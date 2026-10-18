//! Target identity and capability collection.
//!
//! This module collects the bounded set of facts that identify the PostgreSQL
//! cluster and confirm whether the connected role can read settings,
//! statistics, and database internals.

use std::fmt;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_bool, parse_text_i32, parse_text_string, AdapterError, ConnectedClient,
    EndpointIdentity,
};
use crate::postgres::sanitize::sanitize_identifier;

/// Bounded, sanitized record describing the PostgreSQL target the snapshot was
/// taken against. Every field is allowlisted; raw libpq error messages and
/// query bodies never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub endpoint: EndpointIdentity,
    pub connected_user: String,
    pub initial_database: String,
    pub server_version_num: i32,
    pub server_version: String,
    pub in_recovery: bool,
    pub block_size_bytes: i32,
    pub has_pg_monitor: bool,
    pub has_pg_read_all_stats: bool,
    pub has_pg_read_all_settings: bool,
    pub is_superuser: bool,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "endpoint={} user={} database={} version={}",
            self.endpoint, self.connected_user, self.initial_database, self.server_version,
        )
    }
}

const VERSION_QUERY: &str = "SELECT current_setting('server_version_num')";

/// Predefined monitoring roles first shipped in PostgreSQL 10.
const MONITORING_ROLES_MIN_VERSION: i32 = 100000;

/// Build the capability query for the given server version. Before
/// PostgreSQL 10 the monitoring roles do not exist and `pg_has_role` errors
/// on an unknown role, so those columns are reported as `false`.
fn capability_query(server_version_num: i32) -> String {
    let membership = |role: &str| {
        if server_version_num >= MONITORING_ROLES_MIN_VERSION {
            format!("pg_has_role(current_user, '{role}', 'MEMBER')")
        } else {
            "false".to_owned()
        }
    };
    format!(
        "\
SELECT \
    current_setting('server_version_num') AS version_num, \
    current_setting('server_version') AS version, \
    pg_is_in_recovery() AS in_recovery, \
    current_setting('block_size')::int AS block_size, \
    (SELECT rolsuper FROM pg_roles WHERE rolname = current_user) AS is_superuser, \
    {} AS has_pg_monitor, \
    {} AS has_pg_read_all_stats, \
    {} AS has_pg_read_all_settings \
",
        membership("pg_monitor"),
        membership("pg_read_all_stats"),
        membership("pg_read_all_settings"),
    )
}

pub fn collect_target(
    client: &ConnectedClient,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Target, AdapterError> {
    let started_at_utc = now_utc();

    let (connected_user, initial_database) = read_identity_columns(client, outcomes)?;

    let capability_source = "postgres.target.capabilities";
    let version_rows = client.exec_bounded_row(VERSION_QUERY, capability_source, outcomes)?;
    let probed_version_num = match version_rows.first().map(|r| parse_text_i32(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let capability_rows = client.exec_bounded_row(
        &capability_query(probed_version_num),
        capability_source,
        outcomes,
    )?;

    let server_version_num = match capability_rows.first().map(|r| parse_text_i32(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let server_version = match capability_rows.get(1).map(|r| parse_text_string(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let in_recovery = match capability_rows.get(2).map(|r| parse_text_bool(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let block_size_bytes = match capability_rows.get(3).map(|r| parse_text_i32(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let is_superuser = match capability_rows.get(4).map(|r| parse_text_bool(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let has_pg_monitor = match capability_rows.get(5).map(|r| parse_text_bool(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let has_pg_read_all_stats = match capability_rows.get(6).map(|r| parse_text_bool(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };
    let has_pg_read_all_settings = match capability_rows.get(7).map(|r| parse_text_bool(r)) {
        Some(Ok(value)) => value,
        _ => return Err(AdapterError::ColumnInvalid),
    };

    let _ = started_at_utc;

    Ok(Target {
        endpoint: client.endpoint().clone(),
        connected_user,
        initial_database,
        server_version_num,
        server_version,
        in_recovery,
        block_size_bytes,
        has_pg_monitor,
        has_pg_read_all_stats,
        has_pg_read_all_settings,
        is_superuser,
    })
}

fn read_identity_columns(
    client: &ConnectedClient,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<(String, String), AdapterError> {
    let raw_user = client.user().map_err(|_| AdapterError::ColumnInvalid)?;
    let sanitized_user = sanitize_identifier(&raw_user);
    if sanitized_user.redacted {
        outcomes.push(ComponentOutcome::failed(
            "postgres.target.identity",
            CollectionStatus::ParseError,
            "connected_user_invalid",
            None,
        ));
        return Err(AdapterError::ColumnInvalid);
    }
    let raw_db = client.db().map_err(|_| AdapterError::ColumnInvalid)?;
    let sanitized_db = sanitize_identifier(&raw_db);
    if sanitized_db.redacted {
        outcomes.push(ComponentOutcome::failed(
            "postgres.target.identity",
            CollectionStatus::ParseError,
            "initial_database_invalid",
            None,
        ));
        return Err(AdapterError::ColumnInvalid);
    }
    Ok((sanitized_user.value, sanitized_db.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgres::connection::EndpointKind;

    #[test]
    fn parse_text_bool_handles_lowercase_alphabet() {
        assert!(parse_text_bool(b"t").unwrap());
        assert!(!parse_text_bool(b"f").unwrap());
    }

    #[test]
    fn parse_text_i32_accepts_block_size_strings() {
        assert_eq!(parse_text_i32(b"8192").unwrap(), 8192);
        assert!(parse_text_i32(&[]).is_err());
    }

    #[test]
    fn parse_text_string_rejects_non_utf8() {
        assert!(parse_text_string(&[0xff]).is_err());
        assert_eq!(parse_text_string(b"plain").unwrap(), "plain");
    }

    #[test]
    fn capability_query_skips_monitoring_roles_before_pg10() {
        let query = capability_query(90305);
        assert!(!query.contains("pg_has_role"));
        assert!(query.contains("false AS has_pg_monitor"));
    }

    #[test]
    fn capability_query_checks_monitoring_roles_on_pg10() {
        let query = capability_query(100000);
        assert!(query.contains("pg_has_role(current_user, 'pg_monitor', 'MEMBER')"));
        assert!(query.contains("pg_has_role(current_user, 'pg_read_all_settings', 'MEMBER')"));
    }

    #[test]
    fn endpoint_kind_serializes() {
        assert_eq!(EndpointKind::Tcp.as_str(), "tcp");
        assert_eq!(EndpointKind::UnixSocket.as_str(), "unix_socket");
    }
}
