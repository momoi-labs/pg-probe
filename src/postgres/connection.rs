//! Safe PostgreSQL connection adapter backed by the Rust `postgres` client.
//!
//! The supported connection contract is intentionally small: PostgreSQL URLs
//! and keyword strings supplied through `--dbname`, the individual `--host`,
//! `--port`, and `--username` options, or the ordinary `PG*` environment
//! variables. It does not resolve service files, passfiles, or multi-host
//! configurations.
//!
//! TLS follows libpq's `sslmode`, read from the conninfo string or
//! `PGSSLMODE`. The default, `prefer`, encrypts without checking the
//! server's certificate and retries without TLS when the handshake fails.

use std::env;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use postgres::config::{Host, SslMode};
use postgres::{Client as PostgresClient, Config, NoTls};
use postgres_rustls::{tokio_rustls::TlsConnector, MakeTlsConnector};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
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

/// How a TCP connection uses TLS, following libpq's `sslmode`. A unix socket
/// never uses TLS. `allow` is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// No TLS.
    Disable,
    /// TLS without checking the certificate; without TLS if it fails.
    Prefer,
    /// TLS without checking the certificate.
    Require,
    /// TLS with a certificate signed by a public certificate authority.
    VerifyCa,
    /// As `VerifyCa`, and the certificate must name the host.
    VerifyFull,
}

impl TlsMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "disable" => Some(Self::Disable),
            "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            "verify-ca" => Some(Self::VerifyCa),
            "verify-full" => Some(Self::VerifyFull),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

/// Socket directories tried, in order, when no host is given. The first is
/// the Debian and Ubuntu default, the second PostgreSQL's own.
const SOCKET_DIRECTORIES: [&str; 2] = ["/var/run/postgresql", "/tmp"];

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
    tls: TlsMode,
}

impl ConnectedClient {
    /// Opens a PostgreSQL connection from explicit connection parameters, or
    /// from the supported `PG*` environment variables when parameters are unset.
    pub fn open(
        params: &ConnectionParams,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let (config, tls) = match resolve_config(params) {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ =
                    timestamped_failure(outcomes, source, AdapterOutcomeStatus::InvalidParameter);
                return Err(error);
            }
        };
        Self::open_config(config, tls, source, outcomes)
    }

    /// Opens a session that cannot write: every transaction starts read-only
    /// and every statement is bounded by `statement_timeout`. The guard is
    /// appended after any `PGOPTIONS`, so it takes precedence.
    pub fn open_read_only(
        params: &ConnectionParams,
        statement_timeout_ms: u64,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let (mut config, tls) = match resolve_config(params) {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ =
                    timestamped_failure(outcomes, source, AdapterOutcomeStatus::InvalidParameter);
                return Err(error);
            }
        };
        let options = read_only_options(config.get_options(), statement_timeout_ms);
        config.options(&options);
        Self::open_config(config, tls, source, outcomes)
    }

    pub fn for_database(
        &self,
        database: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let mut config = self.config.clone();
        config.dbname(database);
        Self::open_config(config, self.tls, source, outcomes)
    }

    fn open_config(
        mut config: Config,
        mut tls: TlsMode,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let started_at_utc = now_utc();
        let endpoint = endpoint_from_config(&config)?;
        log_target(&config, &endpoint, tls);
        let client = match connect(&mut config, endpoint.endpoint_kind, &mut tls) {
            Ok(client) => client,
            Err(error) => {
                tracing::info!(
                    "connection failed: {}",
                    safe_error_detail(&error).unwrap_or_else(|| "no reason given".to_owned())
                );
                record_connection_failure(outcomes, source, &error);
                return Err(AdapterError::ConnectionFailed);
            }
        };
        tracing::info!("connected to {endpoint}");
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
            tls,
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

    /// Runs one statement over the extended query protocol and returns the
    /// first text column of every row. The server refuses a string with more
    /// than one command on this protocol, so nothing after the first statement
    /// can ever run. On failure it returns the server's message, which is
    /// about the statement, never about the connection settings.
    pub fn single_statement_text_rows(&self, statement: &str) -> Result<Vec<String>, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "the session is unavailable".to_owned())?;
        let rows = client.query(statement, &[]).map_err(|error| {
            error
                .as_db_error()
                .map(|db_error| db_error.message().to_owned())
                .unwrap_or_else(|| "the query failed".to_owned())
        })?;
        rows.iter()
            .map(|row| {
                row.try_get::<_, String>(0)
                    .map_err(|_| "the result was not text".to_owned())
            })
            .collect()
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
///
/// Without a host, it uses the first socket directory in `SOCKET_DIRECTORIES`
/// that holds the server's socket, as psql does, and `localhost` otherwise.
fn resolve_config(params: &ConnectionParams) -> Result<(Config, TlsMode), AdapterError> {
    resolve_config_with(params, &SOCKET_DIRECTORIES)
}

fn resolve_config_with(
    params: &ConnectionParams,
    socket_directories: &[&str],
) -> Result<(Config, TlsMode), AdapterError> {
    if let Some(dbname) = params.dbname.as_deref() {
        if dbname.is_empty() || dbname.contains('\0') {
            return Err(AdapterError::InvalidParameter);
        }
    }

    // The client parses only disable, prefer and require, so a verify mode
    // reaches it as require, and the connector does the checking.
    let mut conninfo_tls = None;
    let mut config = match params.dbname.as_deref() {
        Some(dbname) if is_conninfo(dbname) => {
            let mut conninfo = dbname.to_owned();
            if let Some((span, value)) = find_sslmode(dbname) {
                let mode = TlsMode::parse(&value).ok_or(AdapterError::InvalidParameter)?;
                conninfo.replace_range(span, "require");
                conninfo_tls = Some(mode);
            }
            Config::from_str(&conninfo).map_err(|_| AdapterError::InvalidParameter)?
        }
        _ => Config::new(),
    };

    let tls = match conninfo_tls {
        Some(mode) => mode,
        None => match env::var("PGSSLMODE") {
            Ok(value) => TlsMode::parse(&value).ok_or(AdapterError::InvalidParameter)?,
            Err(_) => TlsMode::Prefer,
        },
    };

    // A socket file given as the host stands for its directory and port.
    let mut socket_port = None;
    let host = if config.get_hosts().is_empty() {
        let host = params
            .host
            .clone()
            .filter(|host| !host.is_empty())
            .or_else(|| env::var("PGHOST").ok().filter(|host| !host.is_empty()));
        host.map(|host| match split_socket_file(&host) {
            Some((directory, port)) => {
                socket_port = Some(port);
                directory
            }
            None => host,
        })
    } else {
        None
    };

    if config.get_ports().is_empty() {
        if let Some(port) = params.port {
            config.port(port);
        } else if let Some(port) = env::var("PGPORT").ok().and_then(|value| parse_port(&value)) {
            config.port(port);
        } else if let Some(port) = socket_port {
            config.port(port);
        }
    }

    if config.get_hosts().is_empty() {
        match host {
            Some(host) => {
                config.host(&host);
            }
            None => {
                let port = config.get_ports().first().copied().unwrap_or(5432);
                match find_socket_directory(socket_directories, port) {
                    Some(directory) => {
                        tracing::info!(
                            "no host given; using the socket in {}",
                            directory.display()
                        );
                        config.host_path(directory);
                    }
                    None => {
                        tracing::info!(
                            "no host given and no socket in {}; using localhost",
                            socket_directories.join(" or ")
                        );
                        config.host("localhost");
                    }
                }
            }
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

    Ok((config, tls))
}

/// Splits `/dir/.s.PGSQL.5432` into its directory and port. psql takes only
/// the directory, but the file is what `ls` shows.
fn split_socket_file(host: &str) -> Option<(String, u16)> {
    if !host.starts_with('/') {
        return None;
    }
    let path = Path::new(host);
    let port = path
        .file_name()?
        .to_str()?
        .strip_prefix(".s.PGSQL.")?
        .parse()
        .ok()?;
    Some((path.parent()?.to_str()?.to_owned(), port))
}

fn find_socket_directory(directories: &[&str], port: u16) -> Option<PathBuf> {
    directories
        .iter()
        .map(PathBuf::from)
        .find(|directory| directory.join(format!(".s.PGSQL.{port}")).exists())
}

/// Finds the `sslmode` value in a conninfo string: its byte range in the
/// string and its text. A URI carries it in the query, a keyword string as
/// a `key=value` pair, maybe quoted.
fn find_sslmode(conninfo: &str) -> Option<(Range<usize>, String)> {
    if conninfo.starts_with("postgresql://") || conninfo.starts_with("postgres://") {
        let query = conninfo.find('?')? + 1;
        let mut start = query;
        for pair in conninfo[query..].split('&') {
            if let Some(value) = pair.strip_prefix("sslmode=") {
                let value_start = start + "sslmode=".len();
                return Some((value_start..value_start + value.len(), value.to_owned()));
            }
            start += pair.len() + 1;
        }
        return None;
    }

    let bytes = conninfo.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &conninfo[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            return None;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value_start = i;
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                value.push(bytes[i] as char);
                i += 1;
            }
            i += 1;
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                value.push(bytes[i] as char);
                i += 1;
            }
        }
        if key == "sslmode" {
            return Some((value_start..i.min(bytes.len()), value));
        }
    }
    None
}

fn log_target(config: &Config, endpoint: &EndpointIdentity, tls: TlsMode) {
    let kind = match endpoint.endpoint_kind {
        EndpointKind::Tcp => "",
        EndpointKind::UnixSocket => "the socket in ",
    };
    let tls = match endpoint.endpoint_kind {
        EndpointKind::Tcp => format!(", sslmode {}", tls.as_str()),
        EndpointKind::UnixSocket => String::new(),
    };
    tracing::info!(
        "connecting to {kind}{endpoint} as user {:?}, database {:?}{tls}",
        config.get_user().unwrap_or("(default)"),
        config.get_dbname().unwrap_or("(default)"),
    );
}

/// Connects with the TLS that `tls` asks for. Under `prefer`, a failed
/// handshake leads to one more attempt without TLS, and `tls` becomes
/// `disable` so later connections skip the failing handshake.
fn connect(
    config: &mut Config,
    kind: EndpointKind,
    tls: &mut TlsMode,
) -> Result<PostgresClient, postgres::Error> {
    // PostgreSQL never offers TLS on a unix socket.
    if kind == EndpointKind::UnixSocket {
        return config.connect(NoTls);
    }
    match *tls {
        TlsMode::Disable => {
            config.ssl_mode(SslMode::Disable);
            config.connect(NoTls)
        }
        TlsMode::Prefer => {
            config.ssl_mode(SslMode::Prefer);
            match config.connect(tls_connector(Check::Nothing)) {
                Err(error) if is_tls_failure(&error) => {
                    tracing::warn!(
                        "TLS failed ({}); retrying without TLS",
                        safe_error_detail(&error).unwrap_or_else(|| "no reason given".to_owned())
                    );
                    config.ssl_mode(SslMode::Disable);
                    *tls = TlsMode::Disable;
                    config.connect(NoTls)
                }
                other => other,
            }
        }
        TlsMode::Require => {
            config.ssl_mode(SslMode::Require);
            config.connect(tls_connector(Check::Nothing))
        }
        TlsMode::VerifyCa => {
            config.ssl_mode(SslMode::Require);
            config.connect(tls_connector(Check::Authority))
        }
        TlsMode::VerifyFull => {
            config.ssl_mode(SslMode::Require);
            config.connect(tls_connector(Check::AuthorityAndName))
        }
    }
}

fn is_tls_failure(error: &postgres::error::Error) -> bool {
    error
        .to_string()
        .starts_with("error performing TLS handshake")
}

fn read_only_options(existing: Option<&str>, statement_timeout_ms: u64) -> String {
    let guard =
        format!("-c default_transaction_read_only=on -c statement_timeout={statement_timeout_ms}");
    match existing.filter(|options| !options.trim().is_empty()) {
        Some(options) => format!("{options} {guard}"),
        None => guard,
    }
}

/// A value is a conninfo string when it is a PostgreSQL URI or a keyword/value
/// string, rather than a bare database name.
fn is_conninfo(value: &str) -> bool {
    value.starts_with("postgresql://") || value.starts_with("postgres://") || value.contains('=')
}

/// What the TLS connector checks in the server's certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    Nothing,
    Authority,
    AuthorityAndName,
}

fn tls_connector(check: Check) -> MakeTlsConnector {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = Arc::new(RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    ));
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("the default provider supports the default TLS versions");
    let config = match check {
        Check::AuthorityAndName => builder.with_root_certificates(roots),
        Check::Authority => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyName(
                WebPkiServerVerifier::builder_with_provider(roots, provider)
                    .build()
                    .expect("the bundled roots are valid"),
            ))),
        Check::Nothing => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider))),
    }
    .with_no_client_auth();
    MakeTlsConnector::new(TlsConnector::from(Arc::new(config)))
}

/// Accepts any certificate, as libpq does under `prefer` and `require`. The
/// handshake signatures are still checked, so the session is encrypted to
/// whoever holds the certificate's key.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Checks the certificate's authority but not the name in it, as libpq does
/// under `verify-ca`.
#[derive(Debug)]
struct AnyName(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for AnyName {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
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
    fn read_only_guard_is_appended_after_existing_options() {
        assert_eq!(
            read_only_options(None, 5000),
            "-c default_transaction_read_only=on -c statement_timeout=5000"
        );
        assert_eq!(
            read_only_options(Some("-c default_transaction_read_only=off"), 5000),
            "-c default_transaction_read_only=off \
             -c default_transaction_read_only=on -c statement_timeout=5000"
        );
    }

    #[test]
    fn explicit_postgresql_url_builds_a_client_config() {
        let config = resolve_config(&ConnectionParams {
            dbname: Some("postgresql://probe:secret@localhost:6543/probe_main".to_owned()),
            ..ConnectionParams::default()
        })
        .unwrap()
        .0;
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
        .unwrap()
        .0;
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
        .unwrap()
        .0;
        assert_eq!(config.get_ports(), [6543]);
        assert_eq!(config.get_user(), Some("probe"));
        assert_eq!(config.get_dbname(), Some("app"));
        match config.get_hosts().first() {
            Some(Host::Tcp(host)) => assert_eq!(host.as_str(), "db.example"),
            other => panic!("expected a TCP host, got {other:?}"),
        }
    }

    #[test]
    fn plain_dbname_defaults_host_to_localhost_without_a_socket() {
        let config = resolve_config_with(
            &ConnectionParams {
                dbname: Some("app".to_owned()),
                ..ConnectionParams::default()
            },
            &[],
        )
        .unwrap()
        .0;
        assert_eq!(config.get_dbname(), Some("app"));
        match config.get_hosts().first() {
            Some(Host::Tcp(host)) => assert_eq!(host.as_str(), "localhost"),
            other => panic!("expected localhost, got {other:?}"),
        }
    }

    #[test]
    fn no_host_uses_the_first_directory_with_the_socket() {
        let empty = tempfile::tempdir().unwrap();
        let with_socket = tempfile::tempdir().unwrap();
        std::fs::write(with_socket.path().join(".s.PGSQL.6543"), b"").unwrap();
        let directories = [
            empty.path().to_str().unwrap(),
            with_socket.path().to_str().unwrap(),
        ];
        let config = resolve_config_with(
            &ConnectionParams {
                port: Some(6543),
                ..ConnectionParams::default()
            },
            &directories,
        )
        .unwrap()
        .0;
        match config.get_hosts().first() {
            Some(Host::Unix(path)) => assert_eq!(path, with_socket.path()),
            other => panic!("expected the socket directory, got {other:?}"),
        }
    }

    #[test]
    fn socket_file_host_means_its_directory_and_port() {
        let config = resolve_config(&ConnectionParams {
            host: Some("/var/run/postgresql/.s.PGSQL.5433".to_owned()),
            ..ConnectionParams::default()
        })
        .unwrap()
        .0;
        assert_eq!(config.get_ports(), [5433]);
        match config.get_hosts().first() {
            Some(Host::Unix(path)) => assert_eq!(path, Path::new("/var/run/postgresql")),
            other => panic!("expected the socket directory, got {other:?}"),
        }
        assert_eq!(split_socket_file("/var/run/postgresql"), None);
        assert_eq!(split_socket_file("db.example"), None);
    }

    #[test]
    fn sslmode_defaults_to_prefer() {
        let (_, tls) = resolve_config(&ConnectionParams {
            host: Some("db.example".to_owned()),
            ..ConnectionParams::default()
        })
        .unwrap();
        assert_eq!(tls, TlsMode::Prefer);
    }

    #[test]
    fn sslmode_is_read_from_a_keyword_string_or_a_url() {
        for (dbname, expected) in [
            ("host=db sslmode=disable", TlsMode::Disable),
            (
                "host=db sslmode = 'verify-full' dbname=app",
                TlsMode::VerifyFull,
            ),
            ("postgresql://db/app?sslmode=verify-ca", TlsMode::VerifyCa),
            (
                "postgresql://db/app?application_name=x&sslmode=require",
                TlsMode::Require,
            ),
        ] {
            let (config, tls) = resolve_config(&ConnectionParams {
                dbname: Some(dbname.to_owned()),
                ..ConnectionParams::default()
            })
            .unwrap();
            assert_eq!(tls, expected, "{dbname}");
            assert!(!config.get_hosts().is_empty(), "{dbname}");
        }
    }

    #[test]
    fn unknown_sslmode_is_rejected() {
        for dbname in ["host=db sslmode=allow", "postgresql://db/app?sslmode=on"] {
            let result = resolve_config(&ConnectionParams {
                dbname: Some(dbname.to_owned()),
                ..ConnectionParams::default()
            });
            assert_eq!(
                result.err(),
                Some(AdapterError::InvalidParameter),
                "{dbname}"
            );
        }
    }

    #[test]
    fn parse_text_bool_accepts_postgres_alphabet() {
        assert!(parse_text_bool(b"t").unwrap());
        assert!(!parse_text_bool(b"f").unwrap());
    }
}
