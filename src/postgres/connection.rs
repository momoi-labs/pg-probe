//! Safe PostgreSQL connection adapter backed by the Rust `postgres` client.
//!
//! The supported connection contract is intentionally small: PostgreSQL URLs
//! and keyword strings supplied through `--database`, or the ordinary `PG*`
//! environment variables when no explicit connection input is supplied. It
//! does not resolve service files, passfiles, or multi-host configurations.

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

/// Owns a live Rust-native client and its sanitized endpoint identity.
pub struct ConnectedClient {
    client: Mutex<PostgresClient>,
    endpoint: EndpointIdentity,
    config: Config,
}

impl ConnectedClient {
    /// Opens a PostgreSQL connection from an explicit URL/keyword string, or
    /// from the supported `PG*` environment variables when no input is given.
    pub fn open(
        dsn_input: Option<&str>,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let config = match connection_config(dsn_input) {
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
        let client = config
            .connect(rustls_connector())
            .map_err(|_| AdapterError::ConnectionFailed);
        let client = match client {
            Ok(client) => client,
            Err(error) => {
                let _ = timestamped_failure(outcomes, source, AdapterOutcomeStatus::QueryFailed);
                return Err(error);
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

fn connection_config(dsn_input: Option<&str>) -> Result<Config, AdapterError> {
    match dsn_input {
        Some(raw) if raw.is_empty() || raw.contains('\0') => Err(AdapterError::InvalidParameter),
        Some(raw) => Config::from_str(raw).map_err(|_| AdapterError::InvalidParameter),
        None => Ok(config_from_environment()),
    }
}

fn config_from_environment() -> Config {
    let mut config = Config::new();
    config.host(&env::var("PGHOST").unwrap_or_else(|_| "localhost".to_owned()));
    if let Some(port) = env::var("PGPORT").ok().and_then(|value| parse_port(&value)) {
        config.port(port);
    }
    if let Ok(database) = env::var("PGDATABASE") {
        config.dbname(&database);
    }
    if let Ok(user) = env::var("PGUSER") {
        config.user(&user);
    }
    if let Ok(password) = env::var("PGPASSWORD") {
        config.password(password);
    }
    if let Ok(options) = env::var("PGOPTIONS") {
        config.options(&options);
    }
    if let Ok(application_name) = env::var("PGAPPNAME") {
        config.application_name(&application_name);
    }
    if let Some(seconds) = env::var("PGCONNECT_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        config.connect_timeout(Duration::from_secs(seconds));
    }
    config
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

    #[test]
    fn explicit_postgresql_url_builds_a_client_config() {
        let config =
            connection_config(Some("postgresql://probe:secret@localhost:6543/probe_main")).unwrap();
        assert_eq!(config.get_user(), Some("probe"));
        assert_eq!(config.get_dbname(), Some("probe_main"));
        assert_eq!(config.get_ports(), [6543]);
    }

    #[test]
    fn parse_text_bool_accepts_postgres_alphabet() {
        assert!(parse_text_bool(b"t").unwrap());
        assert!(!parse_text_bool(b"f").unwrap());
    }
}
