//! WAL generation statistics from `pg_stat_wal`.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{parse_text_i64, AdapterError, ConnectedClient};
use crate::postgres::stats::capability::has_wal_stats;

#[derive(Debug, Clone, PartialEq)]
pub struct WalStats {
    pub wal_records: i64,
    pub wal_fpi: i64,
    pub wal_bytes: i64,
    pub wal_buffers_full: Option<i64>,
    pub wal_write: Option<i64>,
    pub wal_sync: Option<i64>,
    pub wal_write_time_ms: Option<f64>,
    pub wal_sync_time_ms: Option<f64>,
    pub stats_reset: Option<String>,
}

const WAL_QUERY: &str = "\
SELECT \
    wal_records, wal_fpi, wal_bytes, wal_buffers_full, \
    wal_write, wal_sync, wal_write_time, wal_sync_time, stats_reset \
FROM pg_stat_wal";

const WAL_QUERY_PG18: &str = "\
SELECT \
    wal_records, wal_fpi, wal_bytes, wal_buffers_full, \
    NULL::bigint, NULL::bigint, NULL::double precision, NULL::double precision, stats_reset \
FROM pg_stat_wal";

pub fn collect_wal_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<WalStats>, AdapterError> {
    let source = "postgres.stats.wal";
    let capability = has_wal_stats(server_version_num);

    if !capability.is_supported() {
        return Ok(None);
    }

    let query = if server_version_num >= 180000 {
        WAL_QUERY_PG18
    } else {
        WAL_QUERY
    };
    let rows = client.exec_bounded_rows(query, source, outcomes)?;
    if rows.rows.is_empty() {
        return Ok(None);
    }

    let row = &rows.rows[0];
    let wal_records = parse_text_i64(&row[0])?;
    let wal_fpi = parse_text_i64(&row[1])?;
    let wal_bytes = parse_text_i64(&row[2])?;
    let wal_buffers_full = parse_text_i64(&row[3]).ok();
    let wal_write = parse_text_i64(&row[4]).ok();
    let wal_sync = parse_text_i64(&row[5]).ok();
    let wal_write_time_ms = parse_f64_optional(&row[6]);
    let wal_sync_time_ms = parse_f64_optional(&row[7]);
    let stats_reset = parse_string_optional(&row[8]);

    Ok(Some(WalStats {
        wal_records,
        wal_fpi,
        wal_bytes,
        wal_buffers_full,
        wal_write,
        wal_sync,
        wal_write_time_ms,
        wal_sync_time_ms,
        stats_reset,
    }))
}

fn parse_f64_optional(raw: &[u8]) -> Option<f64> {
    if raw.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(raw).ok()?;
    text.trim().parse::<f64>().ok()
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
    fn parse_f64_optional_handles_empty() {
        assert_eq!(parse_f64_optional(b""), None);
        assert_eq!(parse_f64_optional(&[]), None);
    }

    #[test]
    fn parse_f64_optional_handles_valid() {
        assert_eq!(parse_f64_optional(b"123.45"), Some(123.45));
        assert_eq!(parse_f64_optional(b" 0.0 "), Some(0.0));
    }

    #[test]
    fn parse_string_optional_handles_empty() {
        assert_eq!(parse_string_optional(b""), None);
        assert_eq!(parse_string_optional(b"   "), None);
    }

    #[test]
    fn parse_string_optional_handles_valid() {
        assert_eq!(
            parse_string_optional(b"2026-01-15 10:30:00"),
            Some("2026-01-15 10:30:00".to_string())
        );
    }
}
