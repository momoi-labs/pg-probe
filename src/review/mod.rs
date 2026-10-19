use std::fs;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::collections::HashMap;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use flate2::read::GzDecoder;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Clone)]
struct AppState {
    data_directory: Arc<PathBuf>,
}

#[derive(Debug, Serialize)]
struct ColumnMetadata {
    name: String,
    #[serde(rename = "type")]
    data_type: String,
    nullable: bool,
    primary_key: bool,
}

#[derive(Debug, Serialize)]
struct TableMetadata {
    name: String,
    columns: Vec<ColumnMetadata>,
}

#[derive(Debug, Serialize)]
struct SnapshotListing {
    id: String,
    file_name: String,
    size_bytes: u64,
    snapshot_id: String,
    started_at_utc: String,
    finished_at_utc: String,
    status: String,
    hostname: Option<String>,
}

#[derive(Debug, Serialize)]
struct TableRows {
    table: String,
    columns: Vec<ColumnMetadata>,
    rows: Vec<Map<String, Value>>,
    /// True when the table had more rows than [`ROW_LIMIT`].
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct SampleListing {
    sample_id: i64,
    taken_at_utc: String,
}

/// The most rows one table request returns. A long sampling session can hold
/// many thousands of rows; past this, the response says it was truncated.
const ROW_LIMIT: usize = 100_000;

type ApiError = (StatusCode, Json<Value>);

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "error": message.into() })))
}

pub async fn serve(data_directory: PathBuf, port: u16) -> Result<(), std::io::Error> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(data_directory)).await
}

pub fn router(data_directory: PathBuf) -> Router {
    Router::new()
        .route("/api/schema", get(schema))
        .route("/api/snapshots", get(snapshots))
        .route("/api/snapshots/{id}", get(snapshot))
        .route("/api/snapshots/{id}/samples", get(samples))
        .route("/api/snapshots/{id}/tables/{table}", get(table_rows))
        .route("/", get(root))
        .route("/console", get(console))
        .route("/console/", get(console))
        .route("/console/{*path}", get(console))
        .with_state(AppState {
            data_directory: Arc::new(data_directory),
        })
}

async fn schema() -> Result<Json<Value>, ApiError> {
    let connection = Connection::open_in_memory()
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    connection
        .execute_batch(include_str!(
            "../../schema/migrations/V1__initial_schema.sql"
        ))
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let tables = metadata(&connection)?;
    Ok(Json(serde_json::json!({ "tables": tables })))
}

async fn snapshots(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut entries = fs::read_dir(state.data_directory.as_ref())
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".sqlite.gz"))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut listings = Vec::new();
    for entry in entries {
        if let Ok(connection) = open_artifact(&entry.path()) {
            if let Ok(listing) = read_listing(&connection, &entry.path()) {
                listings.push(listing);
            }
        }
    }
    Ok(Json(serde_json::json!({ "snapshots": listings })))
}

async fn snapshot(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let path = resolve_artifact(state.data_directory.as_ref(), &id)?;
    let connection = open_artifact(&path)?;
    let listing = read_listing(&connection, &path)?;
    let outcomes = query_rows(&connection, "collector_outcome", &HashMap::new())?.rows;
    Ok(Json(serde_json::json!({
        "snapshot": listing,
        "collector_outcomes": outcomes
    })))
}

async fn samples(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let path = resolve_artifact(state.data_directory.as_ref(), &id)?;
    let connection = open_artifact(&path)?;
    Ok(Json(
        serde_json::json!({ "samples": read_samples(&connection)? }),
    ))
}

/// Artifacts written before schema version 2 have no `sample` table. They hold
/// one sample, taken when the snapshot started.
fn read_samples(connection: &Connection) -> Result<Vec<SampleListing>, ApiError> {
    let internal =
        |error: rusqlite::Error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    let has_table = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sample')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(internal)?;
    let sql = if has_table {
        "SELECT sample_id, taken_at_utc FROM sample ORDER BY sample_id"
    } else {
        "SELECT 1, started_at_utc FROM snapshot"
    };
    let mut statement = connection.prepare(sql).map_err(internal)?;
    let samples = statement
        .query_map([], |row| {
            Ok(SampleListing {
                sample_id: row.get(0)?,
                taken_at_utc: row.get(1)?,
            })
        })
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    Ok(samples)
}

/// Returns a table's rows. Each query parameter whose name is a column of the
/// table filters on equality, for example `?sample_id=3` or `?cpu_id=-1`.
async fn table_rows(
    State(state): State<AppState>,
    AxumPath((id, table)): AxumPath<(String, String)>,
    Query(filters): Query<HashMap<String, String>>,
) -> Result<Json<TableRows>, ApiError> {
    let path = resolve_artifact(state.data_directory.as_ref(), &id)?;
    let connection = open_artifact(&path)?;
    Ok(Json(query_rows(&connection, &table, &filters)?))
}

mod console_assets {
    include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));
}

const CONSOLE_NOT_BUILT: &str = "<!doctype html><html><body><main>\
    <h1>pg-probe review console</h1>\
    <p>This binary was built without the console. Run <code>pnpm install</code> and \
    <code>pnpm build</code> in <code>web/</code>, then build pg-probe again.</p>\
    </main></body></html>";

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Serves the embedded console. A path that is not a file is a client-side
/// route, so it gets `index.html` and the router takes over.
async fn console(path: Option<AxumPath<String>>) -> Response {
    let assets = console_assets::ASSETS;
    let requested = path.map(|AxumPath(path)| path).unwrap_or_default();
    let asset = assets
        .iter()
        .find(|(name, _)| *name == requested)
        .or_else(|| assets.iter().find(|(name, _)| *name == "index.html"));
    match asset {
        Some((name, bytes)) => {
            ([(header::CONTENT_TYPE, content_type(name))], bytes.to_vec()).into_response()
        }
        None => Html(CONSOLE_NOT_BUILT).into_response(),
    }
}

async fn root() -> Redirect {
    Redirect::to("/console/")
}

fn resolve_artifact(data_directory: &Path, id: &str) -> Result<PathBuf, ApiError> {
    if id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(api_error(StatusCode::NOT_FOUND, "snapshot not found"));
    }
    let candidates = [id.to_owned(), format!("{id}.sqlite.gz")];
    for candidate in candidates {
        let path = data_directory.join(candidate);
        if path.is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".sqlite.gz"))
        {
            return Ok(path);
        }
    }

    let entries = fs::read_dir(data_directory)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_name().to_string_lossy().ends_with(".sqlite.gz") {
            continue;
        }
        if let Ok(connection) = open_artifact(&entry.path()) {
            let found = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM snapshot WHERE snapshot_id = ?1)",
                [id],
                |row| row.get::<_, bool>(0),
            );
            if found == Ok(true) {
                return Ok(entry.path());
            }
        }
    }
    Err(api_error(StatusCode::NOT_FOUND, "snapshot not found"))
}

fn open_artifact(path: &Path) -> Result<Connection, ApiError> {
    let file = fs::File::open(path)
        .map_err(|error| api_error(StatusCode::NOT_FOUND, error.to_string()))?;
    let mut decoder = GzDecoder::new(file);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let mut connection = Connection::open_in_memory()
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let size = bytes.len();
    connection
        .deserialize_read_exact("main", std::io::Cursor::new(bytes), size, true)
        .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    Ok(connection)
}

fn read_listing(connection: &Connection, path: &Path) -> Result<SnapshotListing, ApiError> {
    let size_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    connection
        .query_row(
            "SELECT s.snapshot_id, s.started_at_utc, s.finished_at_utc, s.status, l.hostname
             FROM snapshot s LEFT JOIN linux_identity l USING (snapshot_id) LIMIT 1",
            [],
            |row| {
                let file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                Ok(SnapshotListing {
                    id: file_name.clone(),
                    file_name,
                    size_bytes,
                    snapshot_id: row.get(0)?,
                    started_at_utc: row.get(1)?,
                    finished_at_utc: row.get(2)?,
                    status: row.get(3)?,
                    hostname: row.get(4)?,
                })
            },
        )
        .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
}

fn metadata(connection: &Connection) -> Result<Vec<TableMetadata>, ApiError> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' AND name <> 'refinery_schema_history' ORDER BY name")
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    names
        .into_iter()
        .map(|name| {
            Ok(TableMetadata {
                columns: columns(connection, &name)?,
                name,
            })
        })
        .collect()
}

fn columns(connection: &Connection, table: &str) -> Result<Vec<ColumnMetadata>, ApiError> {
    let mut statement = connection
        .prepare("SELECT name, type, [notnull], pk FROM pragma_table_info(?1) ORDER BY cid")
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let columns = statement
        .query_map([table], |row| {
            Ok(ColumnMetadata {
                name: row.get(0)?,
                data_type: row.get(1)?,
                nullable: !row.get::<_, bool>(2)?,
                primary_key: row.get::<_, bool>(3)?,
            })
        })
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(columns)
}

fn query_rows(
    connection: &Connection,
    table: &str,
    filters: &HashMap<String, String>,
) -> Result<TableRows, ApiError> {
    let valid = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type IN ('table', 'view') AND name = ?1 AND name NOT LIKE 'sqlite_%' AND name <> 'refinery_schema_history')",
        [table],
        |row| row.get::<_, bool>(0),
    ).map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if !valid {
        return Err(api_error(StatusCode::NOT_FOUND, "table not found"));
    }
    let columns = columns(connection, table)?;
    let mut conditions = Vec::new();
    let mut values = Vec::new();
    for (name, value) in filters {
        if !columns.iter().any(|column| &column.name == name) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("{table} has no column {name}"),
            ));
        }
        values.push(value.as_str());
        conditions.push(format!(
            "\"{}\" = ?{}",
            name.replace('"', "\"\""),
            values.len()
        ));
    }
    let filter = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    // One row past the limit tells the reader the result was truncated.
    let sql = format!(
        "SELECT * FROM \"{}\"{filter} LIMIT {}",
        table.replace('"', "\"\""),
        ROW_LIMIT + 1
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let names = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = statement
        .query_map(rusqlite::params_from_iter(values), |row| {
            let mut object = Map::new();
            for (index, name) in names.iter().enumerate() {
                object.insert(name.clone(), sqlite_value(row.get_ref(index)?));
            }
            Ok(object)
        })
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let truncated = rows.len() > ROW_LIMIT;
    rows.truncate(ROW_LIMIT);
    Ok(TableRows {
        table: table.to_owned(),
        columns,
        rows,
        truncated,
    })
}

fn sqlite_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => value.into(),
        ValueRef::Real(value) => value.into(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned().into(),
        ValueRef::Blob(value) => format!("<{} byte blob>", value.len()).into(),
    }
}
