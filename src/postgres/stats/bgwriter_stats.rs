//! Background writer and checkpointer statistics.

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{parse_text_i64, AdapterError, ConnectedClient};
use crate::postgres::stats::capability::has_checkpointer_stats;

#[derive(Debug, Clone, PartialEq)]
pub struct BgWriterStats {
    pub checkpoints_timed: Option<i64>,
    pub checkpoints_req: Option<i64>,
    pub checkpoint_write_time_ms: Option<f64>,
    pub checkpoint_sync_time_ms: Option<f64>,
    pub buffers_checkpoint: Option<i64>,
    pub buffers_clean: Option<i64>,
    pub maxwritten_clean: Option<i64>,
    pub buffers_backend: Option<i64>,
    pub buffers_backend_fsync: Option<i64>,
    pub buffers_alloc: Option<i64>,
    pub stats_reset: Option<String>,
}

const BGWRITER_QUERY_PG16: &str = "\
SELECT \
    buffers_clean, maxwritten_clean, buffers_backend, \
    buffers_backend_fsync, buffers_alloc, stats_reset \
FROM pg_stat_bgwriter";

const BGWRITER_QUERY_PG17: &str = "\
SELECT \
    buffers_clean, maxwritten_clean, NULL::bigint, \
    NULL::bigint, buffers_alloc, stats_reset \
FROM pg_stat_bgwriter";

const CHECKPOINTER_QUERY_PG17: &str = "\
SELECT \
    num_timed, num_requested, write_time, sync_time, \
    buffers_written, stats_reset \
FROM pg_stat_checkpointer";

pub fn collect_bgwriter_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<BgWriterStats, AdapterError> {
    let source = "postgres.stats.bgwriter";
    let query = if server_version_num >= 170000 {
        BGWRITER_QUERY_PG17
    } else {
        BGWRITER_QUERY_PG16
    };

    let rows = client.exec_bounded_rows(query, source, outcomes)?;
    if rows.rows.is_empty() {
        return Ok(BgWriterStats {
            checkpoints_timed: None,
            checkpoints_req: None,
            checkpoint_write_time_ms: None,
            checkpoint_sync_time_ms: None,
            buffers_checkpoint: None,
            buffers_clean: None,
            maxwritten_clean: None,
            buffers_backend: None,
            buffers_backend_fsync: None,
            buffers_alloc: None,
            stats_reset: None,
        });
    }

    let row = &rows.rows[0];
    let buffers_clean = parse_text_i64(&row[0]).ok();
    let maxwritten_clean = parse_text_i64(&row[1]).ok();
    let buffers_backend = parse_text_i64(&row[2]).ok();
    let buffers_backend_fsync = parse_text_i64(&row[3]).ok();
    let buffers_alloc = parse_text_i64(&row[4]).ok();
    let stats_reset = parse_string_optional(&row[5]);

    Ok(BgWriterStats {
        checkpoints_timed: None,
        checkpoints_req: None,
        checkpoint_write_time_ms: None,
        checkpoint_sync_time_ms: None,
        buffers_checkpoint: None,
        buffers_clean,
        maxwritten_clean,
        buffers_backend,
        buffers_backend_fsync,
        buffers_alloc,
        stats_reset,
    })
}

pub fn collect_checkpointer_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<BgWriterStats>, AdapterError> {
    let source = "postgres.stats.checkpointer";
    let capability = has_checkpointer_stats(server_version_num);

    if !capability.is_supported() {
        return Ok(None);
    }

    let rows = client.exec_bounded_rows(CHECKPOINTER_QUERY_PG17, source, outcomes)?;
    if rows.rows.is_empty() {
        return Ok(None);
    }

    let row = &rows.rows[0];
    let num_timed = parse_text_i64(&row[0]).ok();
    let num_requested = parse_text_i64(&row[1]).ok();
    let write_time = parse_f64_optional(&row[2]);
    let sync_time = parse_f64_optional(&row[3]);
    let buffers_written = parse_text_i64(&row[4]).ok();
    let stats_reset = parse_string_optional(&row[5]);

    Ok(Some(BgWriterStats {
        checkpoints_timed: num_timed,
        checkpoints_req: num_requested,
        checkpoint_write_time_ms: write_time,
        checkpoint_sync_time_ms: sync_time,
        buffers_checkpoint: buffers_written,
        buffers_clean: None,
        maxwritten_clean: None,
        buffers_backend: None,
        buffers_backend_fsync: None,
        buffers_alloc: None,
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
    fn bgwriter_stats_default_values() {
        let stats = BgWriterStats {
            checkpoints_timed: None,
            checkpoints_req: None,
            checkpoint_write_time_ms: None,
            checkpoint_sync_time_ms: None,
            buffers_checkpoint: None,
            buffers_clean: Some(100),
            maxwritten_clean: Some(5),
            buffers_backend: Some(50),
            buffers_backend_fsync: Some(0),
            buffers_alloc: Some(200),
            stats_reset: None,
        };
        assert_eq!(stats.buffers_clean, Some(100));
    }
}
