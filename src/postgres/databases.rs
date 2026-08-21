//! PostgreSQL database inventory, sizes, and per-database extension visit
//! bookkeeping.
//!
//! Database inventory needs no special privileges; sizes and per-database
//! extension discovery do. Independent failures per database means one
//! inaccessible cluster database never erases the working ones.

use std::fmt;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_bool, parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseEntry {
    pub database_name: String,
    pub is_template: bool,
    pub allows_connections: bool,
    pub can_connect: bool,
    pub size_bytes: Option<i64>,
    pub extension_visit_status: ExtensionVisitStatus,
}

impl fmt::Display for DatabaseEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} size={:?} template={} connects={}",
            self.database_name, self.size_bytes, self.is_template, self.allows_connections
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionVisitStatus {
    Pending,
    Visited,
    NotConnectable,
    PermissionDenied,
    ConnectionFailed,
}

impl ExtensionVisitStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Visited => "visited",
            Self::NotConnectable => "not_connectable",
            Self::PermissionDenied => "permission_denied",
            Self::ConnectionFailed => "connection_failed",
        }
    }

    pub const fn is_complete(self) -> bool {
        matches!(
            self,
            Self::Visited | Self::PermissionDenied | Self::NotConnectable | Self::ConnectionFailed
        )
    }
}

/// Discover the database inventory and attempt to compute a bounded size for
/// each connectable database. Sizes can be missing when the active role
/// cannot read `pg_database_size` for the entry; the database is recorded
/// with `size_bytes = NULL` instead of being skipped.
pub fn collect_databases(
    client: &ConnectedClient,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<DatabaseEntry>, AdapterError> {
    let started_at_utc = now_utc();
    let inventory_query = "\
SELECT datname, datistemplate, datallowconn, \
       (pg_database_size(oid) >= 0) AS can_size, \
       pg_database_size(oid) AS size_bytes \
FROM pg_database \
ORDER BY datname";
    let inventory_source = "postgres.databases.inventory";
    let rows = client.exec_bounded_rows(inventory_query, inventory_source, outcomes)?;
    let _ = started_at_utc;

    let mut databases = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        let database_name = {
            let value = parse_text_string(&row[0])?;
            let sanitized = sanitize_identifier(&value);
            if sanitized.redacted {
                outcomes.push(ComponentOutcome::failed(
                    inventory_source,
                    CollectionStatus::ParseError,
                    "database_name_invalid",
                    None,
                ));
            }
            sanitized.value
        };
        let is_template = parse_text_bool(&row[1])?;
        let allows_connections = parse_text_bool(&row[2])?;
        let can_connect = allows_connections && !is_template;
        let size_bytes = parse_database_size(&row[3], &row[4]);
        let extension_visit_status = if can_connect {
            ExtensionVisitStatus::Pending
        } else {
            ExtensionVisitStatus::NotConnectable
        };
        databases.push(DatabaseEntry {
            database_name,
            is_template,
            allows_connections,
            can_connect,
            size_bytes,
            extension_visit_status,
        });
    }
    Ok(databases)
}

/// Parse the `pg_database_size(oid)` projection for a single row. The
/// `can_size` boolean prevents us from treating a permissions failure as a
/// legitimate zero-byte database.
fn parse_database_size(can_size_raw: &[u8], size_raw: &[u8]) -> Option<i64> {
    match parse_text_bool(can_size_raw) {
        Ok(true) => match parse_text_i64(size_raw) {
            Ok(value) if value >= 0 => Some(value),
            _ => None,
        },
        _ => None,
    }
}

/// Mark a pending database as visited after a successful extension query.
pub fn mark_visited(database: &mut DatabaseEntry) {
    database.extension_visit_status = ExtensionVisitStatus::Visited;
}

/// Mark a pending database as permission-denied after a failed extension
/// query.
pub fn mark_permission_denied(database: &mut DatabaseEntry) {
    database.extension_visit_status = ExtensionVisitStatus::PermissionDenied;
}

/// Mark a pending database as connection-failed after a failed extension
/// visit.
pub fn mark_connection_failed(database: &mut DatabaseEntry) {
    database.extension_visit_status = ExtensionVisitStatus::ConnectionFailed;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_database_size_treats_permission_failures_as_unknown() {
        assert_eq!(parse_database_size(b"f", b"0"), None);
        assert_eq!(parse_database_size(b"t", b"-1"), None);
        assert_eq!(parse_database_size(b"t", b"1024"), Some(1024));
    }

    #[test]
    fn extension_status_progresses_monotonically() {
        let mut entry = DatabaseEntry {
            database_name: "app".to_owned(),
            is_template: false,
            allows_connections: true,
            can_connect: true,
            size_bytes: Some(1024),
            extension_visit_status: ExtensionVisitStatus::Pending,
        };
        mark_visited(&mut entry);
        assert_eq!(entry.extension_visit_status, ExtensionVisitStatus::Visited);
        mark_permission_denied(&mut entry);
        assert_eq!(
            entry.extension_visit_status,
            ExtensionVisitStatus::PermissionDenied
        );
    }
}
