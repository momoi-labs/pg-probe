//! Minimal safe adapter over the real `libpq` C library.
//!
//! Release builds link a bundled `libpq` (via `pq-sys`'s `bundled` feature) and
//! vendored OpenSSL. All unsafe FFI stays in this module; callers only see
//! typed, sanitized results and never raw libpq diagnostic strings.

use std::ffi::{CStr, CString};
use std::fmt;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::ptr;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::sanitize::{parse_libpq_port, parse_port, sanitize_host};

/// Ensure the vendored OpenSSL archive participates in the final link.
use openssl_sys as _;

/// Public reason a libpq call did not produce the requested value. The error
/// itself is never logged; only the variant name reaches the terminal or
/// artifact, so secrets in libpq's diagnostic strings cannot leak.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdapterError {
    #[error("libpq refused the connection")]
    ConnectionFailed,
    #[error("libpq rejected a parameter value")]
    InvalidParameter,
    #[error("the libpq query failed")]
    QueryFailed,
    #[error("the libpq result was not a single tuple")]
    SingleTupleExpected,
    #[error("a libpq column did not contain data")]
    ColumnMissing,
    #[error("a libpq column contained an invalid value")]
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

    pub const fn is_colloquial_rejected(self) -> bool {
        !matches!(self, Self::Ok | Self::Unsupported)
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

/// Owns a live libpq connection and the sanitized endpoint identity derived
/// from it. Dropping this value closes the underlying connection.
pub struct ConnectedClient {
    conn: *mut pq_sys::PGconn,
    endpoint: EndpointIdentity,
    /// The original libpq connection string used to open this connection.
    /// It is kept only in memory to build per-database connection strings
    /// for extension discovery; it is never persisted or logged.
    connection_string: Option<String>,
}

// libpq connections are not Sync; ConnectedClient is only used on one thread
// in the collector spine.
unsafe impl Send for ConnectedClient {}

impl Drop for ConnectedClient {
    fn drop(&mut self) {
        if !self.conn.is_null() {
            // SAFETY: conn is an owned PGconn pointer created by PQconnectdb.
            unsafe { pq_sys::PQfinish(self.conn) };
            self.conn = ptr::null_mut();
        }
    }
}

impl ConnectedClient {
    /// Open a libpq connection using psql-compatible resolution rules. Pass
    /// the user-supplied `--dbname` value (when present) as `dsn_input` and
    /// leave it `None` to let standard environment resolution apply.
    ///
    /// This function performs no logging and never returns the raw libpq
    /// error message. The connection metadata returned alongside the
    /// connected client is sanitized so the caller can persist or print it.
    pub fn open(
        dsn_input: Option<&str>,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Self, AdapterError> {
        let started_at_utc = now_utc();
        let conninfo = match dsn_input {
            Some(raw) => {
                if raw.is_empty() {
                    let _ = timestamped_failure(
                        outcomes,
                        source,
                        AdapterOutcomeStatus::InvalidParameter,
                    );
                    return Err(AdapterError::InvalidParameter);
                }
                raw
            }
            None => "",
        };

        let c_conninfo = match CString::new(conninfo) {
            Ok(value) => value,
            Err(_) => {
                let _ =
                    timestamped_failure(outcomes, source, AdapterOutcomeStatus::InvalidParameter);
                return Err(AdapterError::InvalidParameter);
            }
        };

        // SAFETY: c_conninfo is a valid NUL-terminated C string.
        let conn = unsafe { pq_sys::PQconnectdb(c_conninfo.as_ptr()) };
        if conn.is_null() {
            let _ = timestamped_failure(outcomes, source, AdapterOutcomeStatus::QueryFailed);
            return Err(AdapterError::ConnectionFailed);
        }

        // SAFETY: conn is a non-null PGconn from PQconnectdb.
        let status = unsafe { pq_sys::PQstatus(conn) };
        if status != pq_sys::ConnStatusType::CONNECTION_OK {
            // SAFETY: owned pointer from PQconnectdb.
            unsafe { pq_sys::PQfinish(conn) };
            let _ = timestamped_failure(outcomes, source, AdapterOutcomeStatus::QueryFailed);
            return Err(AdapterError::ConnectionFailed);
        }

        let endpoint = match current_endpoint(conn) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                unsafe { pq_sys::PQfinish(conn) };
                let _ = timestamped_failure(outcomes, source, adapter_status_for(&error));
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
            conn,
            endpoint,
            connection_string: dsn_input.map(ToOwned::to_owned),
        })
    }

    pub fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }

    /// The original libpq connection string used to open this connection.
    /// Callers must not persist or log this value.
    pub fn connection_string(&self) -> Option<&str> {
        self.connection_string.as_deref()
    }

    /// Connected role name from libpq, without logging.
    pub fn user(&self) -> Result<String, AdapterError> {
        read_conn_field(self.conn, pq_sys::PQuser)
    }

    /// Initial database name from libpq, without logging.
    pub fn db(&self) -> Result<String, AdapterError> {
        read_conn_field(self.conn, pq_sys::PQdb)
    }

    /// Read a bounded row from a catalog query. Returns the raw `&[u8]` values
    /// in column order so callers can apply their own typed parsing without
    /// going through libpq's lossy decoder.
    pub fn exec_bounded_row(
        &self,
        query: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<Vec<Vec<u8>>, AdapterError> {
        let started_at_utc = now_utc();
        let result = exec_query(self.conn, query)?;
        match result.status() {
            QueryStatus::TuplesOk => {}
            other => {
                let status = adapter_status_from_query(other);
                outcomes.push(
                    ComponentOutcome::failed(
                        source,
                        status.as_collection_status(),
                        "query_failed",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return Err(AdapterError::QueryFailed);
            }
        }
        if result.ntuples() != 1 || result.nfields() == 0 {
            outcomes.push(
                ComponentOutcome::failed(
                    source,
                    AdapterOutcomeStatus::InvalidParameter.as_collection_status(),
                    "single_tuple_expected",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return Err(AdapterError::SingleTupleExpected);
        }
        let mut row = Vec::with_capacity(result.nfields());
        for column in 0..result.nfields() {
            let value = match result.value(0, column) {
                Some(bytes) => bytes.to_vec(),
                None => {
                    outcomes.push(
                        ComponentOutcome::failed(
                            source,
                            AdapterOutcomeStatus::InvalidParameter.as_collection_status(),
                            "column_missing",
                            None,
                        )
                        .with_timing(started_at_utc, now_utc()),
                    );
                    return Err(AdapterError::ColumnMissing);
                }
            };
            row.push(value);
        }
        let observed_at_utc = now_utc();
        outcomes.push(
            ComponentOutcome::collected(source, started_at_utc, observed_at_utc, observed_at_utc)
                .with_timing(started_at_utc, observed_at_utc),
        );
        Ok(row)
    }

    /// Run a catalog query that returns many rows. Every cell is returned as
    /// `Vec<u8>` so callers keep full control over byte-level escaping and
    /// truncation. An empty result set is not considered an error.
    pub fn exec_bounded_rows(
        &self,
        query: &str,
        source: &'static str,
        outcomes: &mut Vec<ComponentOutcome>,
    ) -> Result<BoundedRows, AdapterError> {
        let started_at_utc = now_utc();
        let result = exec_query(self.conn, query)?;
        match result.status() {
            QueryStatus::TuplesOk => {}
            other => {
                let status = adapter_status_from_query(other);
                outcomes.push(
                    ComponentOutcome::failed(
                        source,
                        status.as_collection_status(),
                        "query_failed",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return Err(AdapterError::QueryFailed);
            }
        }
        let observed_at_utc = now_utc();
        outcomes.push(ComponentOutcome::collected(
            source,
            started_at_utc,
            observed_at_utc,
            observed_at_utc,
        ));
        let nrows = result.ntuples();
        let ncols = result.nfields();
        let mut rows = Vec::with_capacity(nrows);
        for row in 0..nrows {
            let mut values = Vec::with_capacity(ncols);
            for column in 0..ncols {
                let value = match result.value(row, column) {
                    Some(bytes) => bytes.to_vec(),
                    None => Vec::new(),
                };
                values.push(value);
            }
            rows.push(values);
        }
        Ok(BoundedRows { rows })
    }
}

/// Holds decoded rows from a bounded query. Each row keeps its raw bytes so the
/// caller can apply domain parsers without any further libpq seek.
#[derive(Debug, Clone)]
pub struct BoundedRows {
    pub rows: Vec<Vec<Vec<u8>>>,
}

impl BoundedRows {
    pub fn into_rows(self) -> Vec<Vec<Vec<u8>>> {
        self.rows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryStatus {
    TuplesOk,
    CommandOk,
    FatalError,
    BadResponse,
    Other,
}

struct QueryResult {
    ptr: *mut pq_sys::PGresult,
}

impl Drop for QueryResult {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: owned PGresult from PQexec.
            unsafe { pq_sys::PQclear(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}

impl QueryResult {
    fn status(&self) -> QueryStatus {
        // SAFETY: ptr is a live PGresult for the lifetime of self.
        let status = unsafe { pq_sys::PQresultStatus(self.ptr) };
        match status {
            pq_sys::ExecStatusType::PGRES_TUPLES_OK => QueryStatus::TuplesOk,
            pq_sys::ExecStatusType::PGRES_COMMAND_OK => QueryStatus::CommandOk,
            pq_sys::ExecStatusType::PGRES_FATAL_ERROR => QueryStatus::FatalError,
            pq_sys::ExecStatusType::PGRES_BAD_RESPONSE => QueryStatus::BadResponse,
            _ => QueryStatus::Other,
        }
    }

    fn ntuples(&self) -> usize {
        // SAFETY: ptr is a live PGresult.
        unsafe { pq_sys::PQntuples(self.ptr) as usize }
    }

    fn nfields(&self) -> usize {
        // SAFETY: ptr is a live PGresult.
        unsafe { pq_sys::PQnfields(self.ptr) as usize }
    }

    fn value(&self, row: usize, column: usize) -> Option<Vec<u8>> {
        // SAFETY: ptr is a live PGresult; row/column are within ntuples/nfields
        // when callers follow the contract used above.
        let is_null = unsafe { pq_sys::PQgetisnull(self.ptr, row as c_int, column as c_int) };
        if is_null != 0 {
            return None;
        }
        let len = unsafe { pq_sys::PQgetlength(self.ptr, row as c_int, column as c_int) };
        if len < 0 {
            return None;
        }
        let ptr = unsafe { pq_sys::PQgetvalue(self.ptr, row as c_int, column as c_int) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: PQgetvalue returns a pointer to len bytes owned by the result.
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
        Some(bytes.to_vec())
    }
}

fn exec_query(conn: *mut pq_sys::PGconn, query: &str) -> Result<QueryResult, AdapterError> {
    let c_query = CString::new(query).map_err(|_| AdapterError::InvalidParameter)?;
    // SAFETY: conn is a live CONNECTION_OK handle; c_query is NUL-terminated.
    let ptr = unsafe { pq_sys::PQexec(conn, c_query.as_ptr()) };
    if ptr.is_null() {
        return Err(AdapterError::QueryFailed);
    }
    Ok(QueryResult { ptr })
}

fn read_conn_field(
    conn: *mut pq_sys::PGconn,
    reader: unsafe extern "C" fn(*const pq_sys::PGconn) -> *mut c_char,
) -> Result<String, AdapterError> {
    // SAFETY: conn is a live PGconn; reader is a libpq accessor.
    let ptr = unsafe { reader(conn) };
    if ptr.is_null() {
        return Err(AdapterError::ColumnInvalid);
    }
    // SAFETY: libpq returns a NUL-terminated string owned by the connection.
    let value = unsafe { CStr::from_ptr(ptr) };
    Ok(bounded_cstr_to_string(value))
}

/// Derive the sanitized endpoint identity from a freshly opened libpq
/// connection. libpq reports either an IP/hostname with a port, or a Unix
/// socket path; anything else is rejected so we never persist a malformed
/// endpoint component.
fn current_endpoint(conn: *mut pq_sys::PGconn) -> Result<EndpointIdentity, AdapterError> {
    let raw_host = read_conn_field(conn, pq_sys::PQhost)?;
    let sanitized_host = sanitize_host(&raw_host);
    if sanitized_host.redacted {
        return Err(AdapterError::ColumnInvalid);
    }
    let endpoint_kind = if sanitized_host.value.starts_with('/') {
        EndpointKind::UnixSocket
    } else {
        EndpointKind::Tcp
    };
    let port = match read_conn_field(conn, pq_sys::PQport) {
        Ok(raw_port) => match parse_libpq_port(&raw_port) {
            Ok(value) => {
                if value == 0 {
                    None
                } else {
                    Some(value)
                }
            }
            Err(_) => None,
        },
        Err(_) => None,
    };
    let port = match (endpoint_kind, port) {
        (EndpointKind::UnixSocket, _) => None,
        (EndpointKind::Tcp, None) => {
            return Err(AdapterError::ColumnInvalid);
        }
        (EndpointKind::Tcp, port) => {
            port.filter(|candidate| parse_port(&candidate.to_string()).is_some())
        }
    };

    Ok(EndpointIdentity {
        endpoint_kind,
        host: sanitized_host.value,
        port,
    })
}

fn adapter_status_from_query(status: QueryStatus) -> AdapterOutcomeStatus {
    match status {
        QueryStatus::TuplesOk | QueryStatus::CommandOk => AdapterOutcomeStatus::Ok,
        QueryStatus::FatalError | QueryStatus::BadResponse => AdapterOutcomeStatus::QueryFailed,
        QueryStatus::Other => AdapterOutcomeStatus::InvalidParameter,
    }
}

fn adapter_status_for(error: &AdapterError) -> AdapterOutcomeStatus {
    match error {
        AdapterError::ConnectionFailed => AdapterOutcomeStatus::QueryFailed,
        AdapterError::InvalidParameter => AdapterOutcomeStatus::InvalidParameter,
        AdapterError::QueryFailed => AdapterOutcomeStatus::QueryFailed,
        AdapterError::SingleTupleExpected => AdapterOutcomeStatus::InvalidParameter,
        AdapterError::ColumnMissing => AdapterOutcomeStatus::InvalidParameter,
        AdapterError::ColumnInvalid => AdapterOutcomeStatus::InvalidParameter,
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

/// Read a bounded null-terminated C string into a `String`, replacing invalid
/// bytes with U+FFFD. Used for the few libpq APIs that hand us a raw C
/// pointer rather than a bounded helper.
#[allow(dead_code)]
pub fn bounded_cstr_to_string(raw: &CStr) -> String {
    raw.to_string_lossy().into_owned()
}

/// Convenience for callers that need to inspect `Path`-shaped host names.
#[allow(dead_code)]
pub fn host_is_unix_socket(host: &str) -> bool {
    Path::new(host).is_absolute() || host.starts_with('/')
}

/// Common trait for connection metadata that the rest of the codebase can use
/// without depending on the FFI layer directly.
pub trait Client {
    fn endpoint(&self) -> &EndpointIdentity;
}

impl Client for ConnectedClient {
    fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }
}

/// Parse a textual integer coming back from a libpq row.
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

/// Parse a textual integer coming back from a libpq row into an `i64`.
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

/// Parse a textual boolean coming back from a libpq row. PostgreSQL reports
/// booleans as `t` or `f`.
pub fn parse_text_bool(raw: &[u8]) -> Result<bool, AdapterError> {
    let text = std::str::from_utf8(raw).map_err(|_| AdapterError::ColumnInvalid)?;
    match text.trim() {
        "t" => Ok(true),
        "f" => Ok(false),
        _ => Err(AdapterError::ColumnInvalid),
    }
}

/// Read a null-terminated string stored in a libpq row as UTF-8, replacing
/// invalid bytes with the Unicode replacement character.
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
    use crate::postgres::sanitize::{sanitize_identifier, MAX_IDENTIFIER_BYTES};

    #[test]
    fn bounded_string_handles_invalid_utf8() {
        let raw = c"abc\xff";
        let parsed = bounded_cstr_to_string(raw);
        assert!(parsed.starts_with("abc"));
        assert!(parsed.ends_with('\u{FFFD}'));
    }

    #[test]
    fn parse_i32_handles_trailing_text() {
        assert_eq!(parse_text_i32(b"8192").unwrap(), 8192);
        assert_eq!(parse_text_i32(b" -7 ").unwrap(), -7);
        assert!(matches!(
            parse_text_i32(b"abc"),
            Err(AdapterError::ColumnInvalid)
        ));
    }

    #[test]
    fn parse_text_bool_accepts_postgres_alphabet() {
        assert!(parse_text_bool(b"t").unwrap());
        assert!(!parse_text_bool(b"f").unwrap());
        assert!(matches!(
            parse_text_bool(b"TRUE"),
            Err(AdapterError::ColumnInvalid)
        ));
    }

    #[test]
    fn parse_text_string_trims_padding() {
        assert_eq!(parse_text_string(b"  hello  ").unwrap(), "hello");
        assert!(matches!(
            parse_text_string(&[0xff]),
            Err(AdapterError::ColumnInvalid)
        ));
    }

    #[test]
    fn identifier_sanitization_rejects_overlong_inputs() {
        let long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        let sanitized = sanitize_identifier(&long);
        assert!(sanitized.redacted);
    }
}
