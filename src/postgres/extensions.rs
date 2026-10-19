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
use crate::postgres::relations::{collect_relations, RelationEntry};
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

/// Connect to the given database name using the initial connection settings
/// and collect its extensions.
/// While connected, it also collects the database's largest relations into
/// `relations`.
pub fn collect_extensions_for_database(
    initial_client: &ConnectedClient,
    database: &str,
    relations: &mut Vec<RelationEntry>,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<ExtensionEntry>, AdapterError> {
    let sanitized = sanitize_identifier(database);
    if !sanitized.redacted && sanitized.value == database {
        let source = "postgres.extensions.visit";
        let secondary_client = initial_client.for_database(database, source, outcomes)?;
        let extensions = collect_extensions(
            &secondary_client,
            database,
            outcomes,
            secondary_client.endpoint().host.as_str(),
        )?;
        relations.extend(collect_relations(&secondary_client, database, outcomes));
        Ok(extensions)
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
    relations: &mut Vec<RelationEntry>,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Option<Vec<ExtensionEntry>> {
    if !database.can_connect {
        database.extension_visit_status = ExtensionVisitStatus::NotConnectable;
        return None;
    }
    match collect_extensions_for_database(
        initial_client,
        &database.database_name,
        relations,
        outcomes,
    ) {
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
