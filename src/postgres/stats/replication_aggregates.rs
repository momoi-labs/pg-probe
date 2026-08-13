//! Privacy-safe replication and slot aggregates.
//!
//! This module aggregates replication and slot information by safe dimensions
//! (state, type, sync state) without exposing client addresses, slot names,
//! user identities, LSN values, or command identifiers.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_bool, parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::has_replication_slots;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationAggregate {
    pub state: Option<String>,
    pub sync_state: Option<String>,
    pub count: i64,
    pub max_lag_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotAggregate {
    pub slot_type: String,
    pub active: bool,
    pub count: i64,
    pub max_retained_bytes: Option<i64>,
}

const REPLICATION_QUERY: &str = "\
SELECT \
    state, sync_state, \
    COUNT(*) AS count, \
    COALESCE(MAX(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)), 0)::bigint AS max_lag_bytes \
FROM pg_stat_replication \
WHERE pid IS NOT NULL \
GROUP BY state, sync_state \
ORDER BY state NULLS FIRST, sync_state NULLS FIRST";

const SLOTS_QUERY: &str = "\
SELECT \
    slot_type, active, \
    COUNT(*) AS count, \
    COALESCE(MAX(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)), 0)::bigint AS max_retained_bytes \
FROM pg_replication_slots \
GROUP BY slot_type, active \
ORDER BY slot_type, active";

pub fn collect_replication_aggregates(
    client: &ConnectedClient,
    _server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<ReplicationAggregate>, AdapterError> {
    let source = "postgres.aggregates.replication";
    let rows = match client.exec_bounded_rows(REPLICATION_QUERY, source, outcomes) {
        Ok(rows) => rows,
        Err(_) => return Ok(Vec::new()),
    };
    let mut aggregates = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let state = parse_string_optional(&row[0]).map(|v| sanitize_identifier(&v).value);
        let sync_state = parse_string_optional(&row[1]).map(|v| sanitize_identifier(&v).value);
        let count = parse_text_i64(&row[2]).unwrap_or(0);
        let max_lag_bytes = parse_text_i64(&row[3]).ok();

        aggregates.push(ReplicationAggregate {
            state,
            sync_state,
            count,
            max_lag_bytes,
        });
    }

    Ok(aggregates)
}

pub fn collect_slot_aggregates(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<Vec<SlotAggregate>>, AdapterError> {
    let source = "postgres.aggregates.slots";
    let capability = has_replication_slots(server_version_num);

    if !capability.is_supported() {
        return Ok(None);
    }

    let rows = match client.exec_bounded_rows(SLOTS_QUERY, source, outcomes) {
        Ok(rows) => rows,
        Err(_) => return Ok(None),
    };
    let mut aggregates = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let slot_type = match parse_text_string(&row[0]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let active = match parse_text_bool(&row[1]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let count = parse_text_i64(&row[2]).unwrap_or(0);
        let max_retained_bytes = parse_text_i64(&row[3]).ok();

        aggregates.push(SlotAggregate {
            slot_type,
            active,
            count,
            max_retained_bytes,
        });
    }

    Ok(Some(aggregates))
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
    fn replication_aggregate_stores_safe_dimensions() {
        let agg = ReplicationAggregate {
            state: Some("streaming".to_string()),
            sync_state: Some("async".to_string()),
            count: 2,
            max_lag_bytes: Some(1024000),
        };
        assert_eq!(agg.state, Some("streaming".to_string()));
        assert_eq!(agg.count, 2);
    }

    #[test]
    fn slot_aggregate_stores_safe_dimensions() {
        let agg = SlotAggregate {
            slot_type: "physical".to_string(),
            active: true,
            count: 1,
            max_retained_bytes: Some(5242880),
        };
        assert_eq!(agg.slot_type, "physical");
        assert!(agg.active);
    }

    #[test]
    fn replication_aggregate_handles_null_state() {
        let agg = ReplicationAggregate {
            state: None,
            sync_state: None,
            count: 0,
            max_lag_bytes: None,
        };
        assert!(agg.state.is_none());
    }
}
