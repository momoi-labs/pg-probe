//! Privacy-safe activity aggregates from `pg_stat_activity`.
//!
//! This module aggregates backend activity by safe dimensions (database,
//! backend type, state, wait event type) without exposing PIDs, SQL text,
//! client addresses, or user identities.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::{has_backend_type, has_wait_event_type};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityAggregate {
    pub database_name: Option<String>,
    pub backend_type: String,
    pub state: Option<String>,
    pub wait_event_type: Option<String>,
    pub count: i64,
    pub max_age_seconds: Option<i64>,
}

/// Build the activity query for the given server version. Before PG 10,
/// `pg_stat_activity` lists only client backends and has no `backend_type`.
/// Before PG 9.6 it has no `wait_event_type`, so that column is NULL.
fn activity_query(server_version_num: i32) -> String {
    let backend_type = if has_backend_type(server_version_num).is_supported() {
        "backend_type"
    } else {
        "'client backend'::text"
    };
    let wait_event_type = if has_wait_event_type(server_version_num).is_supported() {
        "wait_event_type"
    } else {
        "NULL::text"
    };
    format!(
        "\
SELECT \
    datname, {backend_type} AS backend_type, state, {wait_event_type} AS wait_event_type, \
    COUNT(*) AS count, \
    COALESCE(MAX(EXTRACT(EPOCH FROM (now() - state_change))::bigint), 0) AS max_age_seconds \
FROM pg_stat_activity \
WHERE pid IS NOT NULL \
GROUP BY datname, backend_type, state, wait_event_type \
ORDER BY datname NULLS FIRST, backend_type, state NULLS FIRST, wait_event_type NULLS FIRST"
    )
}

pub fn collect_activity_aggregates(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<ActivityAggregate>, AdapterError> {
    let source = "postgres.aggregates.activity";
    let rows = client.exec_bounded_rows(&activity_query(server_version_num), source, outcomes)?;
    let mut aggregates = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let database_name = parse_string_optional(&row[0]).map(|v| sanitize_identifier(&v).value);
        let backend_type = match parse_text_string(&row[1]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let state = parse_string_optional(&row[2]);
        let wait_event_type = parse_string_optional(&row[3]);
        let count = parse_text_i64(&row[4]).unwrap_or(0);
        let max_age_seconds = parse_text_i64(&row[5]).ok();

        aggregates.push(ActivityAggregate {
            database_name,
            backend_type,
            state,
            wait_event_type,
            count,
            max_age_seconds,
        });
    }

    Ok(aggregates)
}

fn parse_string_optional(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(raw).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_aggregate_stores_safe_dimensions() {
        let agg = ActivityAggregate {
            database_name: Some("testdb".to_string()),
            backend_type: "client backend".to_string(),
            state: Some("active".to_string()),
            wait_event_type: Some("IO".to_string()),
            count: 5,
            max_age_seconds: Some(30),
        };
        assert_eq!(agg.database_name, Some("testdb".to_string()));
        assert_eq!(agg.count, 5);
    }

    #[test]
    fn activity_aggregate_handles_null_dimensions() {
        let agg = ActivityAggregate {
            database_name: None,
            backend_type: "autovacuum worker".to_string(),
            state: None,
            wait_event_type: None,
            count: 1,
            max_age_seconds: None,
        };
        assert!(agg.database_name.is_none());
        assert!(agg.state.is_none());
    }
}
