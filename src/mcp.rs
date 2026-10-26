//! A local MCP server over stdio that lets AI agents read artifacts. See
//! `docs/adr/0009-mcp-server-over-artifacts.md`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Statement};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::review::{self, ApiError};

/// The most rows `query` returns. Past this the result says it was truncated,
/// so one `SELECT *` cannot fill the agent's context.
pub const ROW_LIMIT: usize = 1_000;

/// How long one statement may run before the server stops it.
pub const TIME_LIMIT: Duration = Duration::from_secs(30);

const CATALOG: &str = include_str!("../schema/catalog.md");
const ERROR_CODES: &str = include_str!("../schema/error-codes.md");
const SCHEMA_SQL: &str = include_str!("../schema/migrations/V1__initial_schema.sql");

const INSTRUCTIONS: &str = "pg-probe reads PostgreSQL and Linux diagnostic \
snapshots from .sqlite.gz artifacts. Start with list_snapshots, then call \
describe_schema without a table for the overview and with a table before you \
query it. When the person names another directory of artifacts, pass it as \
data_directory to every call. query runs one read-only SQLite statement on one \
snapshot and returns at most 1000 rows; filter and aggregate in SQL. Cumulative \
counters need the difference between consecutive samples. percentile(), \
median() and math functions such as sqrt() are available. export_query writes \
every row to a new CSV file for analysis outside SQLite. State only what a \
query showed. Before you explain a status or an error, query the tables that \
hold its cause, and when the artifact does not show the cause, say so instead \
of guessing. Say the artifact does not record something only after \
describe_schema showed no table or column for it. In your answer, cite the \
table, column and value behind each fact, for example \
postgres_database.is_template = 1. Label any statement that no query showed \
as an inference.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSnapshotsParams {
    /// Absolute path of a directory with .sqlite.gz artifacts. Leave it out
    /// for the directory the server started with.
    pub data_directory: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DescribeSchemaParams {
    /// A table or view name. Leave it out for the list of all tables.
    pub table: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryParams {
    /// The snapshot's id or file name, from list_snapshots.
    pub snapshot: String,
    /// Absolute path of the directory that holds the snapshot. Leave it out
    /// for the directory the server started with.
    pub data_directory: Option<String>,
    /// One read-only SQLite statement.
    pub sql: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportQueryParams {
    /// The snapshot's id or file name, from list_snapshots.
    pub snapshot: String,
    /// Absolute path of the directory that holds the snapshot. Leave it out
    /// for the directory the server started with.
    pub data_directory: Option<String>,
    /// One read-only SQLite statement.
    pub sql: String,
    /// Absolute path of the CSV file to create. It must not exist yet.
    pub path: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// True when the statement returned more than [`ROW_LIMIT`] rows.
    pub truncated: bool,
    /// What each `error_code` in the rows means, so the agent reads the cause
    /// instead of guessing it from the code's name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub error_codes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ExportResult {
    pub path: String,
    pub rows: usize,
}

#[derive(Clone)]
pub struct Server {
    data_directory: Arc<PathBuf>,
}

pub async fn serve(data_directory: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // An absolute path, so the agent can tell the person where it reads.
    let data_directory = fs::canonicalize(&data_directory).unwrap_or(data_directory);
    let server = Server {
        data_directory: Arc::new(data_directory),
    };
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[tool_router]
impl Server {
    #[tool(
        description = "List the snapshots in a data directory: id, host, time span and status, plus the directory's absolute path. Pass data_directory when the person names another directory, and keep passing it to query and export_query."
    )]
    async fn list_snapshots(
        &self,
        Parameters(params): Parameters<ListSnapshotsParams>,
    ) -> CallToolResult {
        let default = Arc::clone(&self.data_directory);
        blocking(move || {
            let directory = data_directory(&default, params.data_directory)?;
            review::list_snapshots(&directory)
                .map(|listings| {
                    serde_json::json!({
                        "data_directory": directory.display().to_string(),
                        "snapshots": listings,
                    })
                })
                .map_err(api_message)
        })
        .await
    }

    #[tool(
        description = "Describe the artifact schema. Without a table, list every table with a summary. With a table, return what each column means, its unit and where pg-probe reads it, plus the CREATE statement."
    )]
    async fn describe_schema(
        &self,
        Parameters(params): Parameters<DescribeSchemaParams>,
    ) -> CallToolResult {
        let result = match params.table {
            None => Ok(Value::String(table_list())),
            Some(table) => describe_table(&table).map(Value::String),
        };
        tool_result(result)
    }

    #[tool(
        description = "Run one read-only SQLite statement on a snapshot. Returns at most 1000 rows and sets truncated when it cut the result; filter and aggregate in SQL. Stops after 30 seconds."
    )]
    async fn query(&self, Parameters(params): Parameters<QueryParams>) -> CallToolResult {
        let default = Arc::clone(&self.data_directory);
        blocking(move || {
            let directory = data_directory(&default, params.data_directory)?;
            let connection = open(&directory, &params.snapshot)?;
            let result = run_query(&connection, &params.sql)?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(
        description = "Run one read-only SQLite statement on a snapshot and write every row to a new CSV file. Use it for results too large for query. Never overwrites a file. Stops after 30 seconds."
    )]
    async fn export_query(
        &self,
        Parameters(params): Parameters<ExportQueryParams>,
    ) -> CallToolResult {
        let default = Arc::clone(&self.data_directory);
        blocking(move || {
            let directory = data_directory(&default, params.data_directory)?;
            let connection = open(&directory, &params.snapshot)?;
            let result = export_query(&connection, &params.sql, Path::new(&params.path))?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("pg-probe", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

async fn blocking<F>(work: F) -> CallToolResult
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    let result = tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    tool_result(result)
}

/// Errors go back to the agent as tool errors, so it can fix its call.
fn tool_result(result: Result<Value, String>) -> CallToolResult {
    match result {
        Ok(Value::String(text)) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
        Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
    }
}

/// The directory a call reads: the one it names, or the server's default.
pub fn data_directory(default: &Path, requested: Option<String>) -> Result<PathBuf, String> {
    let Some(requested) = requested else {
        return Ok(default.to_path_buf());
    };
    let path = PathBuf::from(&requested);
    if !path.is_absolute() {
        return Err(format!(
            "data_directory must be an absolute path: {requested}"
        ));
    }
    if !path.is_dir() {
        return Err(format!("data_directory is not a directory: {requested}"));
    }
    fs::canonicalize(&path).map_err(|error| format!("cannot read {requested}: {error}"))
}

fn api_message((_, body): ApiError) -> String {
    body.0["error"]
        .as_str()
        .unwrap_or("unknown error")
        .to_owned()
}

/// Opens a snapshot for queries: read-only, without `ATTACH`, which could
/// create files.
pub fn open(data_directory: &Path, snapshot: &str) -> Result<Connection, String> {
    let path = review::resolve_artifact(data_directory, snapshot).map_err(api_message)?;
    let connection = review::open_artifact(&path).map_err(api_message)?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

/// Prepares one statement that does not write, and arms the time limit.
fn prepare<'c>(connection: &'c Connection, sql: &str) -> Result<Statement<'c>, String> {
    let statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    if !statement.readonly() {
        return Err("only read-only statements are allowed".to_owned());
    }
    let started = Instant::now();
    connection
        .progress_handler(10_000, Some(move || started.elapsed() > TIME_LIMIT))
        .map_err(|error| error.to_string())?;
    Ok(statement)
}

fn query_error(error: rusqlite::Error) -> String {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        format!(
            "the statement ran longer than {} seconds and was stopped",
            TIME_LIMIT.as_secs()
        )
    } else {
        error.to_string()
    }
}

pub fn run_query(connection: &Connection, sql: &str) -> Result<QueryResult, String> {
    let mut statement = prepare(connection, sql)?;
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let width = columns.len();
    let mut cursor = statement.query([]).map_err(query_error)?;
    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = cursor.next().map_err(query_error)? {
        if rows.len() == ROW_LIMIT {
            truncated = true;
            break;
        }
        let values = (0..width)
            .map(|index| row.get_ref(index).map(review::sqlite_value))
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_error)?;
        rows.push(values);
    }
    let error_codes = explain_error_codes(&columns, &rows);
    Ok(QueryResult {
        columns,
        rows,
        truncated,
        error_codes,
    })
}

/// The meaning of every code in the `error_code` columns of the rows. A join
/// can return more than one.
fn explain_error_codes(columns: &[String], rows: &[Vec<Value>]) -> BTreeMap<String, String> {
    let indexes: Vec<usize> = (0..columns.len())
        .filter(|&index| columns[index] == "error_code")
        .collect();
    rows.iter()
        .flat_map(|row| indexes.iter().filter_map(|&index| row[index].as_str()))
        .filter_map(|code| {
            error_code_entries()
                .find(|(name, _)| *name == code)
                .map(|(name, meaning)| (name.to_owned(), meaning.to_owned()))
        })
        .collect()
}

/// The entries of `schema/error-codes.md`: each code and what it means.
fn error_code_entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    ERROR_CODES.lines().filter_map(|line| {
        let rest = line.strip_prefix("- `")?;
        let (code, meaning) = rest.split_once("`: ")?;
        Some((code, meaning))
    })
}

pub fn export_query(
    connection: &Connection,
    sql: &str,
    path: &Path,
) -> Result<ExportResult, String> {
    if !path.is_absolute() {
        return Err("path must be absolute".to_owned());
    }
    let mut statement = prepare(connection, sql)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    let result = write_csv(&mut statement, BufWriter::new(file));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    Ok(ExportResult {
        path: path.display().to_string(),
        rows: result?,
    })
}

fn write_csv(statement: &mut Statement<'_>, mut out: impl Write) -> Result<usize, String> {
    let io = |error: std::io::Error| error.to_string();
    let header: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(csv_field)
        .collect();
    writeln!(out, "{}", header.join(",")).map_err(io)?;
    let width = header.len();
    let mut cursor = statement.query([]).map_err(query_error)?;
    let mut count = 0;
    while let Some(row) = cursor.next().map_err(query_error)? {
        let fields = (0..width)
            .map(|index| row.get_ref(index).map(csv_value))
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_error)?;
        writeln!(out, "{}", fields.join(",")).map_err(io)?;
        count += 1;
    }
    out.flush().map_err(io)?;
    Ok(count)
}

/// An empty field is NULL.
fn csv_value(value: ValueRef<'_>) -> String {
    match review::sqlite_value(value) {
        Value::Null => String::new(),
        Value::String(text) => csv_field(&text),
        other => other.to_string(),
    }
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The catalog's sections, one per table or view: its name and its text.
fn catalog_sections() -> impl Iterator<Item = (&'static str, &'static str)> {
    CATALOG.split("\n## ").skip(1).map(|section| {
        let name = section.lines().next().unwrap_or_default().trim();
        (name, section)
    })
}

fn table_list() -> String {
    let intro = CATALOG.split("\n## ").next().unwrap_or_default().trim();
    let mut text = format!("{intro}\n\n## Tables\n\n");
    for (name, section) in catalog_sections() {
        let summary = section
            .split("\n\n")
            .nth(1)
            .unwrap_or_default()
            .replace('\n', " ");
        text.push_str(&format!("- `{name}`: {summary}\n"));
    }
    text.push_str(&format!("\n#{}", ERROR_CODES.trim_end()));
    text
}

fn describe_table(table: &str) -> Result<String, String> {
    let (_, section) = catalog_sections()
        .find(|(name, _)| *name == table)
        .ok_or_else(|| {
            format!(
                "no table or view named {table}; call describe_schema without a table for the list"
            )
        })?;
    let schema = Connection::open_in_memory().map_err(|error| error.to_string())?;
    schema
        .execute_batch(SCHEMA_SQL)
        .map_err(|error| error.to_string())?;
    let sql: String = schema
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(format!("## {}\n```sql\n{sql}\n```\n", section.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE t (id INTEGER, label TEXT);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1500)
                 INSERT INTO t SELECT i, 'row ' || i FROM n;",
            )
            .unwrap();
        connection
            .set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
            .unwrap();
        connection
    }

    #[test]
    fn query_returns_columns_and_rows() {
        let result = run_query(&artifact(), "SELECT id, label FROM t WHERE id <= 2").unwrap();
        assert_eq!(result.columns, ["id", "label"]);
        assert_eq!(
            result.rows,
            [
                vec![Value::from(1), Value::from("row 1")],
                vec![Value::from(2), Value::from("row 2")]
            ]
        );
        assert!(!result.truncated);
    }

    #[test]
    fn query_cuts_the_result_at_the_row_limit() {
        let result = run_query(&artifact(), "SELECT * FROM t").unwrap();
        assert_eq!(result.rows.len(), ROW_LIMIT);
        assert!(result.truncated);
    }

    #[test]
    fn query_rejects_writes_and_attach() {
        let connection = artifact();
        assert!(run_query(&connection, "DELETE FROM t").is_err());
        assert!(run_query(&connection, "ATTACH '/tmp/pg-probe-attach.db' AS other").is_err());
        assert!(!Path::new("/tmp/pg-probe-attach.db").exists());
    }

    #[test]
    fn query_has_percentile_and_math_functions() {
        let result = run_query(
            &artifact(),
            "SELECT percentile(id, 95), median(id), sqrt(16.0) FROM t",
        )
        .unwrap();
        assert_eq!(
            result.rows[0],
            [Value::from(1425.05), Value::from(750.5), Value::from(4.0)]
        );
    }

    #[test]
    fn query_stops_at_the_time_limit() {
        let connection = artifact();
        let started = Instant::now() - TIME_LIMIT;
        connection
            .progress_handler(1, Some(move || started.elapsed() > TIME_LIMIT))
            .unwrap();
        let mut statement = connection.prepare("SELECT count(*) FROM t").unwrap();
        let error = statement
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap_err();
        assert!(query_error(error).contains("was stopped"));
    }

    #[test]
    fn export_writes_every_row_and_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("out.csv");
        let connection = artifact();
        let result = export_query(
            &connection,
            "SELECT id, label || ', \"x\"' AS label FROM t",
            &path,
        )
        .unwrap();
        assert_eq!(result.rows, 1500);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("id,label\n1,\"row 1, \"\"x\"\"\"\n"));
        assert!(export_query(&connection, "SELECT 1", &path).is_err());
        assert!(export_query(&connection, "SELECT 1", Path::new("relative.csv")).is_err());
    }

    #[test]
    fn a_call_can_name_another_data_directory() {
        let default = Path::new("/default");
        let other = tempfile::tempdir().unwrap();
        let requested = other.path().display().to_string();
        assert_eq!(data_directory(default, None).unwrap(), default);
        assert_eq!(
            data_directory(default, Some(requested)).unwrap(),
            fs::canonicalize(other.path()).unwrap()
        );
        assert!(data_directory(default, Some("relative".to_owned())).is_err());
        assert!(data_directory(default, Some("/does/not/exist".to_owned())).is_err());
    }

    #[test]
    fn query_explains_the_error_codes_it_returns() {
        let connection = Connection::open_in_memory().unwrap();
        let result = run_query(
            &connection,
            "SELECT 'database_not_connectable' AS error_code UNION ALL SELECT NULL",
        )
        .unwrap();
        assert_eq!(result.error_codes.len(), 1);
        assert!(result.error_codes["database_not_connectable"].contains("template"));
        let joined = run_query(
            &connection,
            "SELECT NULL AS error_code, 'database_not_connectable' AS error_code",
        )
        .unwrap();
        assert!(joined.error_codes.contains_key("database_not_connectable"));
        let plain = run_query(&connection, "SELECT 1 AS n").unwrap();
        assert!(plain.error_codes.is_empty());
    }

    /// Every `.rs` file under `src/`, cut at its tests.
    fn production_sources() -> Vec<String> {
        fn walk(directory: &Path, sources: &mut Vec<String>) {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, sources);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let text = fs::read_to_string(&path).unwrap();
                    let end = text.find("#[cfg(test)]").unwrap_or(text.len());
                    sources.push(text[..end].to_owned());
                }
            }
        }
        let mut sources = Vec::new();
        walk(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut sources,
        );
        sources
    }

    #[test]
    fn error_codes_match_the_code() {
        let sources = production_sources();
        let documented: Vec<&str> = error_code_entries().map(|(code, _)| code).collect();
        let unknown: Vec<&&str> = documented
            .iter()
            .filter(|code| {
                !sources
                    .iter()
                    .any(|text| text.contains(&format!("\"{code}\"")))
            })
            .collect();
        assert!(
            unknown.is_empty(),
            "schema/error-codes.md names codes the code never writes: {unknown:?}"
        );

        // Literal codes passed straight to ComponentOutcome::failed. Codes that
        // reach it through variables are not checked here.
        let mut missing = Vec::new();
        for text in &sources {
            for call in text.split("ComponentOutcome::failed(").skip(1) {
                let arguments = &call[..call.find(')').unwrap_or(call.len())];
                for literal in arguments.split('"').skip(1).step_by(2) {
                    let is_code = literal
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                    if is_code && !documented.contains(&literal) {
                        missing.push(literal.to_owned());
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "schema/error-codes.md does not describe: {missing:?}"
        );
    }

    #[test]
    fn catalog_describes_every_table_and_column() {
        let schema = Connection::open_in_memory().unwrap();
        schema.execute_batch(SCHEMA_SQL).unwrap();
        let mut tables = schema
            .prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let names: Vec<String> = tables
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut missing = Vec::new();
        for name in names {
            let Some((_, section)) = catalog_sections().find(|(heading, _)| *heading == name)
            else {
                missing.push(name);
                continue;
            };
            let mut columns = schema
                .prepare("SELECT name FROM pragma_table_info(?1)")
                .unwrap();
            let columns: Vec<String> = columns
                .query_map([&name], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            for column in columns {
                if !section.contains(&format!("\n- `{column}`:")) {
                    missing.push(format!("{name}.{column}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "schema/catalog.md does not describe: {missing:?}"
        );
    }

    #[test]
    fn describe_table_returns_the_section_and_create_statement() {
        let text = describe_table("snapshot").unwrap();
        assert!(text.starts_with("## snapshot\n"));
        assert!(text.contains("CREATE TABLE snapshot"));
        assert!(describe_table("nope").is_err());
    }
}
