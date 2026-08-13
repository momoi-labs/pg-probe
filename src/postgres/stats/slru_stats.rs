//! SLRU statistics from `pg_stat_slru` (PG 13+).

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::has_slru_stats;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlruStatEntry {
    pub name: String,
    pub blocks_zeroed: i64,
    pub blocks_hit: i64,
    pub blocks_read: i64,
    pub blocks_written: i64,
    pub blocks_exists: i64,
    pub flush_count: i64,
    pub stats_reset: Option<String>,
}

const SLRU_QUERY: &str = "\
SELECT \
    name, blks_zeroed, blks_hit, blks_read, \
    blks_written, blks_exists, flushes, stats_reset \
FROM pg_stat_slru \
ORDER BY name";

pub fn collect_slru_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<Vec<SlruStatEntry>>, AdapterError> {
    let source = "postgres.stats.slru";
    let capability = has_slru_stats(server_version_num);

    if !capability.is_supported() {
        return Ok(None);
    }

    let rows = client.exec_bounded_rows(SLRU_QUERY, source, outcomes)?;
    let mut entries = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let name = match parse_text_string(&row[0]) {
            Ok(v) => sanitize_identifier(&v).value,
            Err(_) => continue,
        };
        let blocks_zeroed = parse_text_i64(&row[1]).unwrap_or(0);
        let blocks_hit = parse_text_i64(&row[2]).unwrap_or(0);
        let blocks_read = parse_text_i64(&row[3]).unwrap_or(0);
        let blocks_written = parse_text_i64(&row[4]).unwrap_or(0);
        let blocks_exists = parse_text_i64(&row[5]).unwrap_or(0);
        let flush_count = parse_text_i64(&row[6]).unwrap_or(0);
        let stats_reset = parse_string_optional(&row[7]);

        entries.push(SlruStatEntry {
            name,
            blocks_zeroed,
            blocks_hit,
            blocks_read,
            blocks_written,
            blocks_exists,
            flush_count,
            stats_reset,
        });
    }

    Ok(Some(entries))
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
    fn slru_entry_stores_name_and_counts() {
        let entry = SlruStatEntry {
            name: "commit_timestamp".to_string(),
            blocks_zeroed: 10,
            blocks_hit: 100,
            blocks_read: 5,
            blocks_written: 3,
            blocks_exists: 2,
            flush_count: 1,
            stats_reset: None,
        };
        assert_eq!(entry.name, "commit_timestamp");
        assert_eq!(entry.blocks_hit, 100);
    }
}
