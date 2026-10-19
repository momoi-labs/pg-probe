//! The largest relations and those with the most dead tuples, per database.
//!
//! `pg_class` and `pg_stat_user_tables` are database-local, so this runs on
//! the per-database connection that the extension visit already opens, once
//! per snapshot. Sizes come from `relpages`, a catalog read, rather than
//! `pg_total_relation_size()`, which stats every file of every relation.

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;

const SOURCE: &str = "postgres.relations";

/// How many relations each list keeps per database.
const TOP: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationEntry {
    pub database_name: String,
    pub schema_name: String,
    pub relation_name: String,
    pub relkind: String,
    pub heap_pages: i64,
    pub index_pages: i64,
    pub toast_pages: i64,
    pub estimated_rows: Option<i64>,
    pub live_tuples: Option<i64>,
    pub dead_tuples: Option<i64>,
    pub last_autovacuum: Option<String>,
    pub autovacuum_count: Option<i64>,
    pub last_autoanalyze: Option<String>,
}

fn relations_query() -> String {
    format!(
        "\
WITH rel AS ( \
    SELECT c.oid, n.nspname, c.relname, c.relkind, c.relpages::bigint AS heap_pages, \
           c.reltuples::bigint AS estimated_rows, c.reltoastrelid \
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
    WHERE c.relkind IN ('r', 'm') \
      AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
      AND n.nspname !~ '^pg_toast' \
), picked AS ( \
    (SELECT oid FROM rel ORDER BY heap_pages DESC LIMIT {TOP}) \
    UNION \
    (SELECT relid FROM pg_stat_user_tables ORDER BY n_dead_tup DESC LIMIT {TOP}) \
) \
SELECT r.nspname, r.relname, r.relkind::text, r.heap_pages, \
    COALESCE((SELECT sum(i.relpages) FROM pg_index x \
              JOIN pg_class i ON i.oid = x.indexrelid WHERE x.indrelid = r.oid), 0)::bigint, \
    COALESCE(t.relpages, 0)::bigint, r.estimated_rows, \
    s.n_live_tup, s.n_dead_tup, s.last_autovacuum::text, s.autovacuum_count, \
    s.last_autoanalyze::text \
FROM rel r \
JOIN picked p ON p.oid = r.oid \
LEFT JOIN pg_class t ON t.oid = r.reltoastrelid \
LEFT JOIN pg_stat_user_tables s ON s.relid = r.oid \
ORDER BY r.heap_pages DESC"
    )
}

fn optional_i64(raw: &[u8]) -> Option<i64> {
    if raw.is_empty() {
        None
    } else {
        parse_text_i64(raw).ok()
    }
}

fn optional_text(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        None
    } else {
        parse_text_string(raw).ok()
    }
}

fn read_relations(
    client: &ConnectedClient,
    database: &str,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<RelationEntry>, AdapterError> {
    let rows = client.exec_bounded_rows(&relations_query(), SOURCE, outcomes)?;
    let mut relations = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        relations.push(RelationEntry {
            database_name: database.to_owned(),
            schema_name: sanitize_identifier(&parse_text_string(&row[0])?).value,
            relation_name: sanitize_identifier(&parse_text_string(&row[1])?).value,
            relkind: parse_text_string(&row[2])?,
            heap_pages: parse_text_i64(&row[3])?,
            index_pages: parse_text_i64(&row[4])?,
            toast_pages: parse_text_i64(&row[5])?,
            estimated_rows: optional_i64(&row[6]),
            live_tuples: optional_i64(&row[7]),
            dead_tuples: optional_i64(&row[8]),
            last_autovacuum: optional_text(&row[9]),
            autovacuum_count: optional_i64(&row[10]),
            last_autoanalyze: optional_text(&row[11]),
        });
    }
    Ok(relations)
}

/// Collects the relations of one database and records the outcome. A
/// failure here never fails the extension visit that shares the connection.
pub fn collect_relations(
    client: &ConnectedClient,
    database: &str,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Vec<RelationEntry> {
    let started_at = now_utc();
    match read_relations(client, database, outcomes) {
        Ok(relations) => {
            outcomes.push(ComponentOutcome::collected(
                SOURCE,
                started_at,
                now_utc(),
                now_utc(),
            ));
            relations
        }
        Err(_) => {
            outcomes.push(
                ComponentOutcome::failed(
                    SOURCE,
                    CollectionStatus::IoError,
                    "relations_failed",
                    None,
                )
                .with_timing(started_at, now_utc()),
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relations_query_is_bounded_and_skips_system_schemas() {
        let query = relations_query();
        assert!(query.contains("LIMIT 20"));
        assert!(query.contains("'pg_catalog', 'information_schema'"));
        assert!(!query.contains("pg_total_relation_size"));
    }
}
