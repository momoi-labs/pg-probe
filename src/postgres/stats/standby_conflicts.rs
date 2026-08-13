//! Standby recovery conflict statistics from `pg_stat_database_conflicts`.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandbyConflictStats {
    pub database_oid: i64,
    pub database_name: String,
    pub confl_tablespace: i64,
    pub confl_lock: i64,
    pub confl_snapshot: i64,
    pub confl_bufferpin: i64,
    pub confl_deadlock: i64,
}

const CONFLICTS_QUERY: &str = "\
SELECT \
    datid, datname, confl_tablespace, confl_lock, \
    confl_snapshot, confl_bufferpin, confl_deadlock \
FROM pg_stat_database_conflicts \
WHERE datid IS NOT NULL \
ORDER BY datname";

pub fn collect_standby_conflicts(
    client: &ConnectedClient,
    _server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<StandbyConflictStats>, AdapterError> {
    let source = "postgres.stats.standby_conflicts";
    let rows = client.exec_bounded_rows(CONFLICTS_QUERY, source, outcomes)?;
    let mut stats = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let database_oid = match parse_text_i64(&row[0]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let database_name = match parse_text_string(&row[1]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let confl_tablespace = match parse_text_i64(&row[2]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let confl_lock = match parse_text_i64(&row[3]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let confl_snapshot = match parse_text_i64(&row[4]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let confl_bufferpin = match parse_text_i64(&row[5]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let confl_deadlock = match parse_text_i64(&row[6]) {
            Ok(v) => v,
            Err(_) => continue,
        };

        stats.push(StandbyConflictStats {
            database_oid,
            database_name,
            confl_tablespace,
            confl_lock,
            confl_snapshot,
            confl_bufferpin,
            confl_deadlock,
        });
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standby_conflicts_struct_is_debug() {
        let stats = StandbyConflictStats {
            database_oid: 12345,
            database_name: "testdb".to_string(),
            confl_tablespace: 0,
            confl_lock: 0,
            confl_snapshot: 0,
            confl_bufferpin: 0,
            confl_deadlock: 0,
        };
        assert_eq!(stats.database_name, "testdb");
    }
}
