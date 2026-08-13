//! Per-database extension inventory.
//!
//! Extension list is database-local, so we open a separate connection per
//! connectable database.

use std::fmt;

use crate::contract::{now_utc, ComponentOutcome};
use crate::postgres::connection::{parse_text_string, AdapterError, ConnectedClient};
use crate::postgres::databases::{
    mark_connection_failed, mark_permission_denied, mark_visited, DatabaseEntry,
    ExtensionVisitStatus,
};
use crate::postgres::sanitize::sanitize_identifier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionEntry {
    pub database_name: String,
    pub extension_name: String,
    pub extension_version: String,
}

impl fmt::Display for ExtensionEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{} in {}",
            self.extension_name, self.extension_version, self.database_name
        )
    }
}

/// Connect to the given database name (using all of the existing libpq
/// resolution rules via a new connection) and collect its extensions.
pub fn collect_extensions_for_database(
    initial_client: &ConnectedClient,
    database: &str,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<ExtensionEntry>, AdapterError> {
    let sanitized = sanitize_identifier(database);
    if !sanitized.redacted && sanitized.value == database {
        let source = "postgres.extensions.visit";
        let dsn = build_per_database_dsn(initial_client, database);
        let secondary_client = ConnectedClient::open(Some(&dsn), source, outcomes)?;
        collect_extensions(
            &secondary_client,
            database,
            outcomes,
            secondary_client.endpoint().host.as_str(),
        )
    } else {
        outcomes.push(ComponentOutcome::failed(
            "postgres.extensions.visit",
            crate::contract::CollectionStatus::ParseError,
            "datname_invalid",
            None,
        ));
        Err(AdapterError::ColumnInvalid)
    }
}

fn build_per_database_dsn(initial: &ConnectedClient, database: &str) -> String {
    let mut dsn = String::new();
    if let Some(initial_dsn) = initial.connection_string() {
        dsn.push_str(initial_dsn);
        dsn.push(' ');
    }
    dsn.push_str("dbname=");
    dsn.push_str(&escape_dsn_value(database));
    dsn
}

fn escape_dsn_value(value: &str) -> String {
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/' | ':')
    }) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "''").replace('\\', "\\\\"))
    }
}

fn collect_extensions(
    client: &ConnectedClient,
    database: &str,
    outcomes: &mut Vec<ComponentOutcome>,
    source_qualifier: &str,
) -> Result<Vec<ExtensionEntry>, AdapterError> {
    let started_at_utc = now_utc();
    let query = "SELECT extname, extversion FROM pg_extension ORDER BY extname";
    let source = "postgres.extensions";
    let rows = client.exec_bounded_rows(query, source, outcomes)?;
    let _ = started_at_utc;
    let _ = source_qualifier;
    let mut entries = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        let extension_name = {
            let value = parse_text_string(&row[0])?;
            sanitize_identifier(&value).value
        };
        let extension_version = {
            let value = parse_text_string(&row[1])?;
            sanitize_identifier(&value).value
        };
        entries.push(ExtensionEntry {
            database_name: database.to_owned(),
            extension_name,
            extension_version,
        });
    }
    Ok(entries)
}

/// Visit a database and collect its extensions. Updates the `DatabaseEntry`
/// in place when the visit succeeded or failed in an expected way.
pub fn visit_database(
    initial_client: &ConnectedClient,
    database: &mut DatabaseEntry,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Option<Vec<ExtensionEntry>> {
    if !database.can_connect {
        database.extension_visit_status = ExtensionVisitStatus::NotConnectable;
        return None;
    }
    match collect_extensions_for_database(initial_client, &database.database_name, outcomes) {
        Ok(extensions) => {
            mark_visited(database);
            Some(extensions)
        }
        Err(AdapterError::QueryFailed) => {
            mark_permission_denied(database);
            None
        }
        Err(AdapterError::ConnectionFailed)
        | Err(AdapterError::InvalidParameter)
        | Err(AdapterError::SingleTupleExpected)
        | Err(AdapterError::ColumnMissing)
        | Err(AdapterError::ColumnInvalid) => {
            mark_connection_failed(database);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_dsn_value_passes_through_safe_inputs() {
        assert_eq!(escape_dsn_value("localhost"), "localhost");
        assert_eq!(escape_dsn_value("/tmp"), "/tmp");
        assert_eq!(escape_dsn_value("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn escape_dsn_value_quotes_special_characters() {
        let escaped = escape_dsn_value("a b");
        assert_eq!(escaped, "'a b'");
        let escaped = escape_dsn_value("a'b");
        assert_eq!(escaped, "'a''b'");
    }
}
