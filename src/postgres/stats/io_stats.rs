//! I/O statistics from `pg_stat_io` (PG 16+).

use crate::contract::ComponentOutcome;
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;
use crate::postgres::stats::capability::has_io_stats;

#[derive(Debug, Clone, PartialEq)]
pub struct IoStatEntry {
    pub backend_type: String,
    pub io_object: Option<String>,
    pub io_context: Option<String>,
    pub reads: i64,
    pub read_time_ms: Option<f64>,
    pub writes: i64,
    pub write_time_ms: Option<f64>,
    pub extends: i64,
    pub extend_time_ms: Option<f64>,
    pub op_bytes: i64,
    pub hits: i64,
    pub evictions: i64,
    pub reuses: i64,
    pub fsyncs: i64,
    pub stats_reset: Option<String>,
}

const IO_QUERY: &str = "\
SELECT \
    backend_type, object, context, \
    reads, read_time, writes, write_time, \
    extends, extend_time, op_bytes, \
    hits, evictions, reuses, fsyncs, stats_reset \
FROM pg_stat_io";

const IO_QUERY_PG18: &str = "\
SELECT \
    backend_type, object, context, \
    SUM(reads), SUM(read_time), SUM(writes), SUM(write_time), \
    SUM(extends), SUM(extend_time), \
    SUM(read_bytes) + SUM(write_bytes) + SUM(extend_bytes), \
    SUM(hits), SUM(evictions), SUM(reuses), SUM(fsyncs), MAX(stats_reset) \
FROM pg_stat_io \
GROUP BY backend_type, object, context";

pub fn collect_io_stats(
    client: &ConnectedClient,
    server_version_num: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Option<Vec<IoStatEntry>>, AdapterError> {
    let source = "postgres.stats.io";
    let capability = has_io_stats(server_version_num);

    if !capability.is_supported() {
        return Ok(None);
    }

    let query = if server_version_num >= 180000 {
        IO_QUERY_PG18
    } else {
        IO_QUERY
    };
    let rows = client.exec_bounded_rows(query, source, outcomes)?;
    let mut entries = Vec::with_capacity(rows.rows.len());

    for row in rows.rows {
        let backend_type = match parse_text_string(&row[0]) {
            Ok(v) => sanitize_identifier(&v.replace(' ', "_")).value,
            Err(_) => continue,
        };
        let io_object = parse_string_optional(&row[1]);
        let io_context = parse_string_optional(&row[2]);
        let reads = parse_text_i64(&row[3]).unwrap_or(0);
        let read_time_ms = parse_f64_optional(&row[4]);
        let writes = parse_text_i64(&row[5]).unwrap_or(0);
        let write_time_ms = parse_f64_optional(&row[6]);
        let extends = parse_text_i64(&row[7]).unwrap_or(0);
        let extend_time_ms = parse_f64_optional(&row[8]);
        let op_bytes = parse_text_i64(&row[9]).unwrap_or(0);
        let hits = parse_text_i64(&row[10]).unwrap_or(0);
        let evictions = parse_text_i64(&row[11]).unwrap_or(0);
        let reuses = parse_text_i64(&row[12]).unwrap_or(0);
        let fsyncs = parse_text_i64(&row[13]).unwrap_or(0);
        let stats_reset = parse_string_optional(&row[14]);

        entries.push(IoStatEntry {
            backend_type,
            io_object,
            io_context,
            reads,
            read_time_ms,
            writes,
            write_time_ms,
            extends,
            extend_time_ms,
            op_bytes,
            hits,
            evictions,
            reuses,
            fsyncs,
            stats_reset,
        });
    }

    Ok(Some(entries))
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
    fn io_stat_entry_stores_dimensions() {
        let entry = IoStatEntry {
            backend_type: "client_backend".to_string(),
            io_object: Some("relation".to_string()),
            io_context: Some("normal".to_string()),
            reads: 100,
            read_time_ms: Some(50.0),
            writes: 200,
            write_time_ms: Some(100.0),
            extends: 10,
            extend_time_ms: Some(5.0),
            op_bytes: 8192,
            hits: 5000,
            evictions: 50,
            reuses: 30,
            fsyncs: 5,
            stats_reset: None,
        };
        assert_eq!(entry.backend_type, "client_backend");
        assert_eq!(entry.io_object, Some("relation".to_string()));
    }

    #[test]
    fn backend_type_normalizes_spaces_before_sanitizing() {
        assert_eq!(
            sanitize_identifier(&"client backend".replace(' ', "_")).value,
            "client_backend"
        );
    }
}
