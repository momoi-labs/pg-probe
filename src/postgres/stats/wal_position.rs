//! The current WAL insert position, as bytes since 0/0.
//!
//! `pg_stat_wal` only exists from PostgreSQL 14. The change in this position
//! between samples gives the WAL generated per second on every version from
//! 9.2, which is when `pg_xlog_location_diff` arrived.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{parse_text_i64, AdapterError, ConnectedClient};
use crate::postgres::stats::capability::has_wal_lsn_naming;

/// Oldest version with `pg_xlog_location_diff`.
pub const WAL_POSITION_MIN_VERSION: i32 = 90200;

/// A standby has no insert position: the query returns NULL there instead of
/// raising an error.
fn wal_position_query(server_version_num: i32) -> &'static str {
    if has_wal_lsn_naming(server_version_num).is_supported() {
        "SELECT CASE WHEN pg_is_in_recovery() THEN NULL \
         ELSE pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')::bigint END"
    } else {
        "SELECT CASE WHEN pg_is_in_recovery() THEN NULL \
         ELSE pg_xlog_location_diff(pg_current_xlog_location(), '0/0')::bigint END"
    }
}

/// Returns the position in bytes, or `None` on a standby.
pub fn collect_wal_position(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<i64>, AdapterError> {
    let source = "postgres.stats.wal_position";
    let row = client.exec_bounded_row(wal_position_query(server_version_num), source, outcomes)?;
    match row.first() {
        Some(raw) if !raw.is_empty() => parse_text_i64(raw).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_position_query_follows_the_lsn_rename_in_10() {
        assert!(wal_position_query(90305).contains("pg_current_xlog_location"));
        assert!(wal_position_query(100000).contains("pg_current_wal_lsn"));
    }
}
