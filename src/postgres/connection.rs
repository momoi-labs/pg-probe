//! Safe PostgreSQL connection adapter backed by the Rust `postgres` client.
//!
//! The supported connection contract is intentionally small: PostgreSQL URLs
//! and keyword strings supplied through `--dbname`, the individual `--host`,
//! `--port`, and `--username` options, or the ordinary `PG*` environment
//! variables. It does not resolve service files, passfiles, or multi-host
//! configurations.

use std::env;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use postgres::config::Host;
use postgres::{Client as PostgresClient, Config};
use postgres_rustls::{tokio_rustls::TlsConnector, MakeTlsConnector};
use rustls::{ClientConfig, RootCertStore};
use thiserror::Error;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::sanitize::{parse_port, sanitize_host};

/// Public reason a client call did not produce the requested value. Raw
/// driver diagnostics never cross this boundary because they may contain
/// connection input or credentials.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdapterError {
    #[error("the PostgreSQL connection failed")]
    ConnectionFailed,
    #[error("the PostgreSQL client rejected a parameter value")]
    InvalidParameter,
    #[error("the PostgreSQL query failed")]
    QueryFailed,
    #[error("the PostgreSQL result was not a single tuple")]
    SingleTupleExpected,
    #[error("a PostgreSQL column did not contain data")]
    ColumnMissing,
    #[error("a PostgreSQL column contained an invalid value")]
    ColumnInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterOutcomeStatus {
    Ok,
    Unsupported,
    PermissionDenied,
    QueryFailed,
    InvalidParameter,
}

impl AdapterOutcomeStatus {
    pub const fn as_collection_status(self) -> CollectionStatus {
        match self {
            Self::Ok => CollectionStatus::Collected,
            Self::Unsupported => CollectionStatus::Unsupported,
            Self::PermissionDenied => CollectionStatus::PermissionDenied,
            Self::QueryFailed => CollectionStatus::IoError,
            Self::InvalidParameter => CollectionStatus::ParseError,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointIdentity {
    pub endpoint_kind: EndpointKind,
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Tcp,
    UnixSocket,
}

impl EndpointKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::UnixSocket => "unix_socket",
        }
    }
}

/// Explicit PostgreSQL connection parameters supplied on the command line.
///
/// Every field is optional. Unset fields fall back to the `PG*` environment
/// variables and then to the client defaults, and a conninfo string or URI in
/// `dbname` is the highest-precedence source, matching psql.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionParams {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub dbname: Option<String>,
}

/// Owns a live Rust-native client and its sanitized endpoint identity.
pub struct ConnectedClient {
    client: Mutex<PostgresClient>,
    endpoint: EndpointIdentity,
    config: Config,
}

impl ConnectedClient {
    /// Opens a PostgreSQL connection from explicit connection parameters, or
    /// from the supported `PG*` environment variables when parameters are unset.
    pub fn open(
        params: &ConnectionParams,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let config = match resolve_config(params) {
            Ok(config) => config,
            Err(error) => {
                let _ =
                    timestamped_failure(outcomes, source, AdapterOutcomeStatus::InvalidParameter);
                return Err(error);
            }
        };
        Self::open_config(config, source, outcomes)
    }

    pub fn for_database(
        &self,
        database: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let mut config = self.config.clone();
        config.dbname(database);
        Self::open_config(config, source, outcomes)
    }

    fn open_config(
        config: Config,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let started_at_utc = now_utc();
        let endpoint = endpoint_from_config(&config)?;
        let client = match config.connect(rustls_connector()) {
            Ok(client) => client,
            Err(error) => {
                record_connection_failure(outcomes, source, &error);
                return Err(AdapterError::ConnectionFailed);
            }
        };
        let observed_at_utc = now_utc();
        outcomes.push(ComponentOutcome::collected(
            source,
            started_at_utc,
            observed_at_utc,
            observed_at_utc,
        ));
        Ok(Self {
            client: Mutex::new(client),
            endpoint,
            config,
        })
    }

    pub fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }

    /// The effective role name, read from the connected server.
    pub fn user(&self) -> Result<String, AdapterError> {
        self.scalar("SELECT current_user")
    }

    /// The effective initial database name, read from the connected server.
    pub fn db(&self) -> Result<String, AdapterError> {
        self.scalar("SELECT current_database()")
    }

    pub fn exec_bounded_row(
        &self,
        query: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Vec<Vec<u8>>, AdapterError> {
        let rows = self.exec_rows(query, source, outcomes)?;
        if rows.rows.len() != 1 || rows.rows[0].is_empty() {
            outcomes.push(
                ComponentOutcome::failed(
                    source,
                    AdapterOutcomeStatus::InvalidParameter.as_collection_status(),
                    "single_tuple_expected",
                    None,
                )
                .with_timing(now_utc(), now_utc()),
            );
            return Err(AdapterError::SingleTupleExpected);
        }
        Ok(rows.rows.into_iter().next().expect("one checked row"))
    }

    pub fn exec_bounded_rows(
        &self,
        query: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<BoundedRows, AdapterError> {
        self.exec_rows(query, source, outcomes)
    }

    fn scalar(&self, query: &str) -> Result<String, AdapterError> {
        let messages = self
            .client
            .lock()
            .map_err(|_| AdapterError::QueryFailed)?
            .simple_query(query)
            .map_err(|_| AdapterError::QueryFailed)?;
        let mut values = messages.into_iter().filter_map(|message| match message {
            postgres::SimpleQueryMessage::Row(row) => row.get(0).map(ToOwned::to_owned),
            _ => None,
        });
        match (values.next(), values.next()) {
            (Some(value), None) => Ok(value),
            _ => Err(AdapterError::ColumnInvalid),
        }
    }

    fn exec_rows(
        &self,
        query: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<BoundedRows, AdapterError> {
        let started_at_utc = now_utc();
        let messages = match self
            .client
            .lock()
            .map_err(|_| AdapterError::QueryFailed)?
            .simple_query(query)
        {
            Ok(messages) => messages,
            Err(_) => {
                outcomes.push(
                    ComponentOutcome::failed(
                        source,
                        AdapterOutcomeStatus::QueryFailed.as_collection_status(),
                        "query_failed",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return Err(AdapterError::QueryFailed);
            }
        };
        let mut rows = Vec::new();
        for message in messages {
            if let postgres::SimpleQueryMessage::Row(row) = message {
                let mut values = Vec::with_capacity(row.len());
                for index in 0..row.len() {
                    let value = row
                        .get(index)
                        .map_or_else(Vec::new, |value| value.as_bytes().to_vec());
                    values.push(value);
                }
                rows.push(values);
            }
        }
        let observed_at_utc = now_utc();
        outcomes.push(ComponentOutcome::collected(
            source,
            started_at_utc,
            observed_at_utc,
            observed_at_utc,
        ));
        Ok(BoundedRows { rows })
    }
}

/// Resolves the effective client configuration from explicit command-line
/// parameters, the `PG*` environment variables, and the client defaults.
///
/// Precedence, highest first, matches psql:
///
/// 1. a conninfo string or URI passed via `--dbname`;
/// 2. the individual `--host`, `--port`, `--username`, and plain
///    `--dbname` options;
/// 3. the `PG*` environment variables;
/// 4. the client defaults.
fn resolve_config(params: &ConnectionParams) -> Result<Config, AdapterError> {
    if let Some(dbname) = params.dbname.as_deref() {
        if dbname.is_empty() || dbname.contains('\0') {
            return Err(AdapterError::InvalidParameter);
        }
    }

    let mut config = match params.dbname.as_deref() {
        Some(dbname) if is_conninfo(dbname) => {
            Config::from_str(dbname).map_err(|_| AdapterError::InvalidParameter)?
        }
        _ => Config::new(),
    };

    if config.get_hosts().is_empty() {
        if let Some(host) = params.host.as_deref().filter(|host| !host.is_empty()) {
            config.host(host);
        } else {
            let host = env::var("PGHOST").unwrap_or_else(|_| "localhost".to_owned());
            config.host(&host);
        }
    }

    if config.get_ports().is_empty() {
        if let Some(port) = params.port {
            config.port(port);
        } else if let Some(port) = env::var("PGPORT").ok().and_then(|value| parse_port(&value)) {
            config.port(port);
        }
    }

    if config.get_user().is_none() {
        if let Some(user) = params.username.as_deref().filter(|user| !user.is_empty()) {
            config.user(user);
        } else if let Ok(user) = env::var("PGUSER") {
            config.user(&user);
        }
    }

    if config.get_dbname().is_none() {
        match params.dbname.as_deref() {
            Some(dbname) if !is_conninfo(dbname) => {
                config.dbname(dbname);
            }
            _ => {
                if let Ok(dbname) = env::var("PGDATABASE") {
                    config.dbname(&dbname);
                }
            }
        }
    }

    if config.get_password().is_none() {
        if let Ok(password) = env::var("PGPASSWORD") {
            config.password(password);
        }
    }

    if config.get_options().is_none() {
        if let Ok(options) = env::var("PGOPTIONS") {
            config.options(&options);
        }
    }

    if config.get_application_name().is_none() {
        if let Ok(application_name) = env::var("PGAPPNAME") {
            config.application_name(&application_name);
        }
    }

    if config.get_connect_timeout().is_none() {
        if let Some(seconds) = env::var("PGCONNECT_TIMEOUT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
        {
            config.connect_timeout(Duration::from_secs(seconds));
        }
    }

    Ok(config)
}

/// A value is a conninfo string when it is a PostgreSQL URI or a keyword/value
/// string, rather than a bare database name.
fn is_conninfo(value: &str) -> bool {
    value.starts_with("postgresql://") || value.starts_with("postgres://") || value.contains('=')
}

fn rustls_connector() -> MakeTlsConnector {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    MakeTlsConnector::new(TlsConnector::from(Arc::new(config)))
}

fn endpoint_from_config(config: &Config) -> Result<EndpointIdentity, AdapterError> {
    let host = config
        .get_hosts()
        .first()
        .ok_or(AdapterError::InvalidParameter)?;
    match host {
        Host::Tcp(host) => {
            let host = sanitize_host(host);
            if host.redacted {
                return Err(AdapterError::ColumnInvalid);
            }
            let port = config.get_ports().first().copied().unwrap_or(5432);
            Ok(EndpointIdentity {
                endpoint_kind: EndpointKind::Tcp,
                host: host.value,
                port: Some(port),
            })
        }
        Host::Unix(path) => {
            let host = sanitize_host(&path.to_string_lossy());
            if host.redacted {
                return Err(AdapterError::ColumnInvalid);
            }
            Ok(EndpointIdentity {
                endpoint_kind: EndpointKind::UnixSocket,
                host: host.value,
                port: None,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoundedRows {
    pub rows: Vec<Vec<Vec<u8>>>,
}

impl BoundedRows {
    pub fn into_rows(self) -> Vec<Vec<Vec<u8>>> {
        self.rows
    }
}

/// Records a failed connection attempt with a stable error code and, when the
/// driver exposes one, the OS error number. Raw driver messages are deliberately
/// not surfaced here (they may contain connection input or credentials).
fn record_connection_failure(
    outcomes: &mut Vec<ComponentOutcome>,
    source: &'static str,
    error: &postgres::error::Error,
) {
    let (code, os_error) = classify_connection_error(error);
    let mut outcome = ComponentOutcome::failed(source, CollectionStatus::IoError, code, os_error);
    if let Some(detail) = safe_error_detail(error) {
        outcome = outcome.with_detail(detail);
    }
    outcomes.push(outcome);
}

/// Returns a safe, human-readable reason for a connection failure. Only
/// server-reported (DB) and OS-level (IO) messages are surfaced; other driver
/// text (e.g. config parsing) may echo connection input and is withheld.
fn safe_error_detail(error: &postgres::error::Error) -> Option<String> {
    if let Some(db) = error.as_db_error() {
        return Some(db.message().to_string());
    }
    std::error::Error::source(error)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .map(|io| io.to_string())
}

/// Maps a connection error to a stable code plus the OS error number, so an
/// operator can tell an authentication failure from a network failure without
/// exposing raw driver text.
fn classify_connection_error(error: &postgres::error::Error) -> (&'static str, Option<i32>) {
    if let Some(sqlstate) = error.code() {
        let code = if sqlstate.code().starts_with("28") {
            "authentication_failed"
        } else if sqlstate.code().starts_with("3D") {
            "database_does_not_exist"
        } else {
            "connection_failed"
        };
        (code, None)
    } else {
        let os_error = std::error::Error::source(error)
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .and_then(std::io::Error::raw_os_error);
        ("connection_failed", os_error)
    }
}

fn timestamped_failure(
    outcomes: &mut Vec<ComponentOutcome>,
    source: &'static str,
    status: AdapterOutcomeStatus,
) -> DateTime<Utc> {
    let code = match status {
        AdapterOutcomeStatus::Ok => "unexpected_ok",
        AdapterOutcomeStatus::Unsupported => "unsupported",
        AdapterOutcomeStatus::PermissionDenied => "permission_denied",
        AdapterOutcomeStatus::QueryFailed => "connection_failed",
        AdapterOutcomeStatus::InvalidParameter => "invalid_parameter",
    };
    outcomes.push(ComponentOutcome::failed(
        source,
        status.as_collection_status(),
        code,
        None,
    ));
    now_utc()
}

pub trait Client {
    fn endpoint(&self) -> &EndpointIdentity;
}

impl Client for ConnectedClient {
    fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }
}

pub fn parse_text_i32(raw: &[u8]) -> Result<i32, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AdapterError::ColumnInvalid);
    }
    trimmed
        .parse::<i32>()
        .map_err(|_| AdapterError::ColumnInvalid)
}

pub fn parse_text_i64(raw: &[u8]) -> Result<i64, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AdapterError::ColumnInvalid);
    }
    trimmed
        .parse::<i64>()
        .map_err(|_| AdapterError::ColumnInvalid)
}

pub fn parse_text_bool(raw: &[u8]) -> Result<bool, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    match text.trim() {
        "t" => Ok(true),
        "f" => Ok(false),
        _ => Err(AdapterError::ColumnInvalid),
    }
}

pub fn parse_text_string(raw: &[u8]) -> Result<String, AdapterError> {
    Ok(std::str::from_utf8(raw)
        .map_err(|_| AdapterError::ColumnInvalid)?
        .trim()
        .to_owned())
}

impl fmt::Display for EndpointIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.endpoint_kind {
            EndpointKind::Tcp => match self.port {
                Some(port) => write!(f, "{}:{}", self.host, port),
                None => f.write_str(&self.host),
            },
            EndpointKind::UnixSocket => f.write_str(&self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postgres::config::Host;

    #[test]
    fn explicit_postgresql_url_builds_a_client_config() {
        let config = resolve_config(&ConnectionParams {
            dbname: Some("postgresql://probe:secret@localhost:6543/probe_main".to_owned()),
            ..ConnectionParams::default()
        })
        .unwrap();
        assert_eq!(config.get_user(), Some("probe"));
        assert_eq!(config.get_dbname(), Some("probe_main"));
        assert_eq!(config.get_ports(), [6543]);
    }

    #[test]
    fn keyword_string_overrides_individual_options() {
        let config = resolve_config(&ConnectionParams {
            host: Some("cli-host".to_owned()),
            port: Some(1111),
            username: Some("cli-user".to_owned()),
            dbname: Some("host=dsn-host port=2222 user=dsn-user dbname=dsn-db".to_owned()),
        })
        .unwrap();
        assert_eq!(config.get_ports(), [2222]);
        assert_eq!(config.get_user(), Some("dsn-user"));
        assert_eq!(config.get_dbname(), Some("dsn-db"));
        match config.get_hosts().first() {
            Some(Host::Tcp(host)) => assert_eq!(host.as_str(), "dsn-host"),
            other => panic!("expected a TCP host from the conninfo string, got {other:?}"),
        }
    }

    #[test]
    fn individual_options_resolve_without_environment() {
        let config = resolve_config(&ConnectionParams {
            host: Some("db.example".to_owned()),
            port: Some(6543),
            username: Some("probe".to_owned()),
            dbname: Some("app".to_owned()),
        })
        .unwrap();
        assert_eq!(config.get_ports(), [6543]);
        assert_eq!(config.get_user(), Some("probe"));
        assert_eq!(config.get_dbname(), Some("app"));
        match config.get_hosts().first() {
            Some(Host::Tcp(host)) => assert_eq!(host.as_str(), "db.example"),
            other => panic!("expected a TCP host, got {other:?}"),
        }
    }

    #[test]
    fn plain_dbname_defaults_host_to_localhost() {
        let config = resolve_config(&ConnectionParams {
            dbname: Some("app".to_owned()),
            ..ConnectionParams::default()
        })
        .unwrap();
        assert_eq!(config.get_dbname(), Some("app"));
        match config.get_hosts().first() {
            Some(Host::Tcp(host)) => assert_eq!(host.as_str(), "localhost"),
            other => panic!("expected localhost, got {other:?}"),
        }
    }

    #[test]
    fn parse_text_bool_accepts_postgres_alphabet() {
        assert!(parse_text_bool(b"t").unwrap());
        assert!(!parse_text_bool(b"f").unwrap());
    }
}
