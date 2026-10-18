//! Privacy-safe lock aggregates from `pg_locks`.
//!
//! This module aggregates lock information by safe dimensions (lock mode,
//! granted state, fast-path state) without exposing PIDs, transaction IDs,
//! object identifiers, or advisory lock keys.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_bool, parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::has_lock_waitstart;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAggregate {
    pub lock_mode: String,
    pub lock_type: String,
    pub granted: bool,
    pub fast_path: Option<bool>,
    pub count: i64,
    pub max_wait_age_seconds: Option<i64>,
}

/// Build the locks query for the given server version. `pg_locks.waitstart`
/// exists only on PG 14+, so older servers report the wait age as NULL.
fn locks_query(server_version_num: i32) -> String {
    let max_wait_age = if has_lock_waitstart(server_version_num).is_supported() {
        "COALESCE(MAX(EXTRACT(EPOCH FROM (now() - waitstart))::bigint), 0)"
    } else {
        "NULL::bigint"
    };
    format!(
        "\
SELECT \
    mode, locktype, granted, fastpath, \
    COUNT(*) AS count, \
    {max_wait_age} AS max_wait_age_seconds \
FROM pg_locks \
WHERE pid IS NOT NULL \
GROUP BY mode, locktype, granted, fastpath \
ORDER BY locktype, mode, granted, fastpath NULLS FIRST"
    )
}

pub fn collect_lock_aggregates(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<LockAggregate>, AdapterError> {
    let source = "postgres.aggregates.locks";
    let rows = client.exec_bounded_rows(&locks_query(server_version_num), source, outcomes)?;
    let mut aggregates = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let lock_mode = match parse_text_string(&row[0]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let lock_type = match parse_text_string(&row[1]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let granted = match parse_text_bool(&row[2]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let fast_path = parse_text_bool(&row[3]).ok();
        let count = parse_text_i64(&row[4]).unwrap_or(0);
        let max_wait_age_seconds = parse_text_i64(&row[5]).ok();

        aggregates.push(LockAggregate {
            lock_mode,
            lock_type,
            granted,
            fast_path,
            count,
            max_wait_age_seconds,
        });
    }

    Ok(aggregates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_aggregate_stores_safe_dimensions() {
        let agg = LockAggregate {
            lock_mode: "AccessShareLock".to_string(),
            lock_type: "relation".to_string(),
            granted: true,
            fast_path: Some(false),
            count: 10,
            max_wait_age_seconds: None,
        };
        assert_eq!(agg.lock_mode, "AccessShareLock");
        assert_eq!(agg.count, 10);
    }

    #[test]
    fn lock_aggregate_handles_null_fast_path() {
        let agg = LockAggregate {
            lock_mode: "RowExclusiveLock".to_string(),
            lock_type: "tuple".to_string(),
            granted: false,
            fast_path: None,
            count: 1,
            max_wait_age_seconds: Some(5),
        };
        assert!(agg.fast_path.is_none());
    }
}
