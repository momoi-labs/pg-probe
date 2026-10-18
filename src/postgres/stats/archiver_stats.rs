//! Archiver statistics from `pg_stat_archiver`.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{parse_text_i64, AdapterError, ConnectedClient};
use crate::postgres::stats::capability::has_archiver_stats;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiverStats {
    pub archived_count: i64,
    pub last_archived_wal: Option<String>,
    pub last_archived_time: Option<String>,
    pub failed_count: i64,
    pub last_failed_wal: Option<String>,
    pub last_failed_time: Option<String>,
    pub stats_reset: Option<String>,
}

const ARCHIVER_QUERY: &str = "\
SELECT \
    archived_count, last_archived_wal, last_archived_time, \
    failed_count, last_failed_wal, last_failed_time, stats_reset \
FROM pg_stat_archiver";

pub fn collect_archiver_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<ArchiverStats>, AdapterError> {
    let source = "postgres.stats.archiver";
    if !has_archiver_stats(server_version_num).is_supported() {
        return Ok(None);
    }
    let rows = match client.exec_bounded_rows(ARCHIVER_QUERY, source, outcomes) {
        Ok(rows) => rows,
        Err(_) => return Ok(None),
    };
    if rows.rows.is_empty() {
        return Ok(None);
    }

    let row = &rows.rows[0];
    let archived_count = parse_text_i64(&row[0])?;
    let last_archived_wal = parse_string_optional(&row[1]);
    let last_archived_time = parse_string_optional(&row[2]);
    let failed_count = parse_text_i64(&row[3])?;
    let last_failed_wal = parse_string_optional(&row[4]);
    let last_failed_time = parse_string_optional(&row[5]);
    let stats_reset = parse_string_optional(&row[6]);

    Ok(Some(ArchiverStats {
        archived_count,
        last_archived_wal,
        last_archived_time,
        failed_count,
        last_failed_wal,
        last_failed_time,
        stats_reset,
    }))
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
    fn parse_string_optional_handles_empty() {
        assert_eq!(parse_string_optional(b""), None);
        assert_eq!(parse_string_optional(b"   "), None);
    }

    #[test]
    fn parse_string_optional_passes_through_valid() {
        assert_eq!(
            parse_string_optional(b"000000010000000000000001"),
            Some("000000010000000000000001".to_string())
        );
    }
}
