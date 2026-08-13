//! Per-database cumulative statistics from `pg_stat_database`.

use chrono::{DateTime, Utc};

use crate::contract::{now_utc, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::has_stats_reset;

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseStats {
    pub database_oid: i64,
    pub database_name: String,
    pub num_backends: Option<i64>,
    pub xact_commit: i64,
    pub xact_rollback: i64,
    pub blks_read: i64,
    pub blks_hit: i64,
    pub tup_returned: i64,
    pub tup_fetched: i64,
    pub tup_inserted: i64,
    pub tup_updated: i64,
    pub tup_deleted: i64,
    pub conflicts: i64,
    pub temp_files: i64,
    pub temp_bytes: i64,
    pub deadlocks: i64,
    pub checksum_failures: Option<i64>,
    pub blk_read_time_ms: Option<f64>,
    pub blk_write_time_ms: Option<f64>,
    pub session_time_ms: Option<f64>,
    pub active_time_ms: Option<f64>,
    pub idle_in_transaction_time_ms: Option<f64>,
    pub sessions: Option<i64>,
    pub sessions_abandoned: Option<i64>,
    pub sessions_fatal: Option<i64>,
    pub sessions_killed: Option<i64>,
    pub stats_reset: Option<DateTime<Utc>>,
}

const BASE_QUERY: &str = "\
SELECT oid, datname \
FROM pg_database \
WHERE datistemplate = false \
ORDER BY datname";

const STATS_QUERY: &str = "\
SELECT \
    datid, datname, numbackends, xact_commit, xact_rollback, \
    blks_read, blks_hit, tup_returned, tup_fetched, \
    tup_inserted, tup_updated, tup_deleted, conflicts, \
    temp_files, temp_bytes, deadlocks \
FROM pg_stat_database \
WHERE datid IS NOT NULL \
ORDER BY datname";

const EXTENDED_STATS_QUERY: &str = "\
SELECT \
    datid, datname, numbackends, xact_commit, xact_rollback, \
    blks_read, blks_hit, tup_returned, tup_fetched, \
    tup_inserted, tup_updated, tup_deleted, conflicts, \
    temp_files, temp_bytes, deadlocks, \
    checksum_failures, blk_read_time, blk_write_time, \
    session_time, active_time, idle_in_transaction_time, \
    sessions, sessions_abandoned, sessions_fatal, sessions_killed, \
    stats_reset \
FROM pg_stat_database \
WHERE datid IS NOT NULL \
ORDER BY datname";

pub fn collect_database_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<DatabaseStats>, AdapterError> {
    let source = "postgres.stats.database";
    let has_reset = has_stats_reset(server_version_num).is_supported();
    let has_sessions = server_version_num >= 140000;
    let has_checksums = server_version_num >= 120000;
    let has_timing = server_version_num >= 90200;

    let query = if has_reset && has_sessions && has_checksums && has_timing {
        EXTENDED_STATS_QUERY
    } else {
        STATS_QUERY
    };

    let rows = client.exec_bounded_rows(query, source, outcomes)?;
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
        let num_backends = parse_text_i64(&row[2]).ok();
        let xact_commit = match parse_text_i64(&row[3]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let xact_rollback = match parse_text_i64(&row[4]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let blks_read = match parse_text_i64(&row[5]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let blks_hit = match parse_text_i64(&row[6]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tup_returned = match parse_text_i64(&row[7]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tup_fetched = match parse_text_i64(&row[8]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tup_inserted = match parse_text_i64(&row[9]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tup_updated = match parse_text_i64(&row[10]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tup_deleted = match parse_text_i64(&row[11]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let conflicts = match parse_text_i64(&row[12]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let temp_files = match parse_text_i64(&row[13]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let temp_bytes = match parse_text_i64(&row[14]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let deadlocks = match parse_text_i64(&row[15]) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let mut checksum_failures = None;
        let mut blk_read_time_ms = None;
        let mut blk_write_time_ms = None;
        let mut session_time_ms = None;
        let mut active_time_ms = None;
        let mut idle_in_transaction_time_ms = None;
        let mut sessions = None;
        let mut sessions_abandoned = None;
        let mut sessions_fatal = None;
        let mut sessions_killed = None;
        let mut stats_reset = None;

        if has_checksums {
            checksum_failures = row.get(16).and_then(|c| parse_text_i64(c).ok());
        }
        if has_timing {
            let time_idx = if has_checksums { 17 } else { 16 };
            blk_read_time_ms = row.get(time_idx).and_then(|c| parse_text_f64(c).ok());
            blk_write_time_ms = row.get(time_idx + 1).and_then(|c| parse_text_f64(c).ok());
        }
        if has_sessions {
            let sess_idx = if has_timing {
                if has_checksums {
                    20
                } else {
                    19
                }
            } else if has_checksums {
                18
            } else {
                17
            };
            session_time_ms = row.get(sess_idx).and_then(|c| parse_text_f64(c).ok());
            active_time_ms = row.get(sess_idx + 1).and_then(|c| parse_text_f64(c).ok());
            idle_in_transaction_time_ms =
                row.get(sess_idx + 2).and_then(|c| parse_text_f64(c).ok());
            sessions = row.get(sess_idx + 3).and_then(|c| parse_text_i64(c).ok());
            sessions_abandoned = row.get(sess_idx + 4).and_then(|c| parse_text_i64(c).ok());
            sessions_fatal = row.get(sess_idx + 5).and_then(|c| parse_text_i64(c).ok());
            sessions_killed = row.get(sess_idx + 6).and_then(|c| parse_text_i64(c).ok());
        }
        if has_reset {
            let reset_idx = if has_sessions {
                if has_timing {
                    if has_checksums {
                        27
                    } else {
                        26
                    }
                } else if has_checksums {
                    25
                } else {
                    24
                }
            } else if has_timing {
                if has_checksums {
                    19
                } else {
                    18
                }
            } else if has_checksums {
                17
            } else {
                16
            };
            stats_reset = row.get(reset_idx).and_then(|c| parse_timestamp(c).ok());
        }

        stats.push(DatabaseStats {
            database_oid,
            database_name,
            num_backends,
            xact_commit,
            xact_rollback,
            blks_read,
            blks_hit,
            tup_returned,
            tup_fetched,
            tup_inserted,
            tup_updated,
            tup_deleted,
            conflicts,
            temp_files,
            temp_bytes,
            deadlocks,
            checksum_failures,
            blk_read_time_ms,
            blk_write_time_ms,
            session_time_ms,
            active_time_ms,
            idle_in_transaction_time_ms,
            sessions,
            sessions_abandoned,
            sessions_fatal,
            sessions_killed,
            stats_reset,
        });
    }

    let _ = now_utc();
    Ok(stats)
}

pub fn parse_text_f64(raw: &[u8]) -> Result<f64, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AdapterError::ColumnInvalid);
    }
    trimmed
        .parse::<f64>()
        .map_err(|_| AdapterError::ColumnInvalid)
}

fn parse_timestamp(raw: &[u8]) -> Result<DateTime<Utc>, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AdapterError::ColumnInvalid);
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f%#z")
                .map(|dt| dt.and_utc())
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f")
                        .map(|dt| dt.and_utc())
                })
        })
        .map_err(|_| AdapterError::ColumnInvalid)
}

#[allow(dead_code)]
pub fn database_names_query() -> &'static str {
    BASE_QUERY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_f64_handles_canonical_values() {
        assert!((parse_text_f64(b"1.5").unwrap() - 1.5).abs() < f64::EPSILON);
        assert!((parse_text_f64(b" 0.0 ").unwrap()).abs() < f64::EPSILON);
        assert!(parse_text_f64(b"").is_err());
        assert!(parse_text_f64(b"abc").is_err());
    }

    #[test]
    fn parse_timestamp_handles_rfc3339() {
        let ts = parse_timestamp(b"2026-01-15T10:30:00+00:00").unwrap();
        assert_eq!(ts.year(), 2026);
    }

    #[test]
    fn parse_timestamp_rejects_empty() {
        assert!(parse_timestamp(b"").is_err());
        assert!(parse_timestamp(b"   ").is_err());
    }

    use chrono::Datelike;
}
