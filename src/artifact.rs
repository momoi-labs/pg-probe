use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OpenFlags};
use thiserror::Error;
use uuid::Uuid;

use crate::contract::{
    format_timestamp, Snapshot, COLLECTOR_NAME, COLLECTOR_VERSION, SCHEMA_VERSION,
};

const SQLITE_APPLICATION_ID: i64 = 0x5047_5052; // "PGPR"

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("the output artifact already exists")]
    OutputExists,
    #[error("the output artifact has no parent directory")]
    MissingParent,
    #[error("failed to persist the SQLite snapshot")]
    Sqlite(#[source] rusqlite::Error),
    #[error("a collected value exceeds the SQLite integer range")]
    ValueOutOfRange,
    #[error("failed to publish the output artifact")]
    Io(#[source] io::Error),
}

pub fn publish(snapshot: &Snapshot, output: &Path) -> Result<(), ArtifactError> {
    if output.exists() {
        return Err(ArtifactError::OutputExists);
    }
    let parent = output.parent().ok_or(ArtifactError::MissingParent)?;
    let nonce = Uuid::new_v4();
    let sqlite_path = parent.join(format!(".pg-probe-{nonce}.sqlite.tmp"));
    let gzip_path = parent.join(format!(".pg-probe-{nonce}.sqlite.gz.tmp"));

    let result = publish_staged(snapshot, output, &sqlite_path, &gzip_path);
    if result.is_err() {
        let _ = fs::remove_file(&sqlite_path);
        let _ = fs::remove_file(&gzip_path);
    }
    result
}

fn publish_staged(
    snapshot: &Snapshot,
    output: &Path,
    sqlite_path: &Path,
    gzip_path: &Path,
) -> Result<(), ArtifactError> {
    SqliteWriter::create(sqlite_path)?.write(snapshot)?;

    let source = File::open(sqlite_path).map_err(ArtifactError::Io)?;
    let destination = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(gzip_path)
        .map_err(ArtifactError::Io)?;
    let mut encoder = GzEncoder::new(BufWriter::new(destination), Compression::default());
    io::copy(&mut BufReader::new(source), &mut encoder).map_err(ArtifactError::Io)?;
    let mut destination = encoder.finish().map_err(ArtifactError::Io)?;
    destination.flush().map_err(ArtifactError::Io)?;
    destination
        .get_ref()
        .sync_all()
        .map_err(ArtifactError::Io)?;

    match fs::hard_link(gzip_path, output) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(ArtifactError::OutputExists);
        }
        Err(error) => return Err(ArtifactError::Io(error)),
    }
    sync_parent(output)?;
    fs::remove_file(gzip_path).map_err(ArtifactError::Io)?;
    fs::remove_file(sqlite_path).map_err(ArtifactError::Io)?;
    Ok(())
}

fn sync_parent(output: &Path) -> Result<(), ArtifactError> {
    let parent = output.parent().ok_or(ArtifactError::MissingParent)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(ArtifactError::Io)
}

struct SqliteWriter {
    connection: Connection,
    path: PathBuf,
}

impl SqliteWriter {
    fn create(path: &Path) -> Result<Self, ArtifactError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(ArtifactError::Sqlite)?;
        connection
            .execute_batch(&format!(
                "
                PRAGMA application_id = {SQLITE_APPLICATION_ID};
                PRAGMA user_version = {SCHEMA_VERSION};
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = DELETE;
                CREATE TABLE snapshot (
                    snapshot_id TEXT PRIMARY KEY NOT NULL,
                    schema_version INTEGER NOT NULL,
                    collector_name TEXT NOT NULL,
                    collector_version TEXT NOT NULL,
                    started_at_utc TEXT NOT NULL,
                    finished_at_utc TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('complete', 'partial'))
                ) STRICT;
                CREATE TABLE collector_outcome (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    source TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (
                        status IN (
                            'collected',
                            'unsupported',
                            'permission_denied',
                            'parse_error',
                            'io_error',
                            'raced'
                        )
                    ),
                    error_code TEXT,
                    os_error_code INTEGER,
                    observed_at_utc TEXT,
                    collector_name TEXT NOT NULL,
                    collector_version TEXT NOT NULL,
                    PRIMARY KEY (snapshot_id, source),
                    CHECK (
                        (status = 'collected' AND error_code IS NULL AND observed_at_utc IS NOT NULL)
                        OR
                        (status <> 'collected' AND error_code IS NOT NULL AND observed_at_utc IS NULL)
                    )
                ) STRICT;
                CREATE TABLE linux_identity (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    hostname TEXT,
                    kernel_sysname TEXT,
                    kernel_release TEXT,
                    kernel_version TEXT,
                    kernel_machine TEXT,
                    distribution_id TEXT,
                    distribution_version_id TEXT,
                    distribution_pretty_name TEXT,
                    page_size_bytes INTEGER,
                    ticks_per_second INTEGER,
                    uptime_microseconds INTEGER
                ) STRICT;
                "
            ))
            .map_err(ArtifactError::Sqlite)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    fn write(mut self, snapshot: &Snapshot) -> Result<(), ArtifactError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO snapshot (
                    snapshot_id, schema_version, collector_name, collector_version,
                    started_at_utc, finished_at_utc, status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    snapshot.snapshot_id.to_string(),
                    SCHEMA_VERSION,
                    COLLECTOR_NAME,
                    COLLECTOR_VERSION,
                    format_timestamp(snapshot.started_at_utc),
                    format_timestamp(snapshot.finished_at_utc),
                    snapshot.status.as_str(),
                ],
            )
            .map_err(ArtifactError::Sqlite)?;

        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO collector_outcome (
                        snapshot_id, source, status, error_code, os_error_code,
                        observed_at_utc, collector_name, collector_version
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(ArtifactError::Sqlite)?;
            for outcome in &snapshot.outcomes {
                statement
                    .execute(params![
                        snapshot.snapshot_id.to_string(),
                        outcome.source,
                        outcome.status.as_str(),
                        outcome.error_code,
                        outcome.os_error_code,
                        outcome.observed_at_utc.map(format_timestamp),
                        COLLECTOR_NAME,
                        COLLECTOR_VERSION,
                    ])
                    .map_err(ArtifactError::Sqlite)?;
            }
        }

        let kernel = snapshot.identity.kernel.as_ref();
        let distribution = snapshot.identity.distribution.as_ref();
        let clock = snapshot.identity.clock.as_ref();
        let page_size_bytes = clock
            .map(|value| i64::try_from(value.page_size_bytes))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let ticks_per_second = clock
            .map(|value| i64::try_from(value.ticks_per_second))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let uptime_microseconds = snapshot
            .identity
            .uptime_microseconds
            .map(i64::try_from)
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        transaction
            .execute(
                "INSERT INTO linux_identity (
                    snapshot_id, hostname, kernel_sysname, kernel_release, kernel_version,
                    kernel_machine, distribution_id, distribution_version_id,
                    distribution_pretty_name, page_size_bytes, ticks_per_second,
                    uptime_microseconds
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    snapshot.snapshot_id.to_string(),
                    kernel.map(|value| value.hostname.as_str()),
                    kernel.map(|value| value.sysname.as_str()),
                    kernel.map(|value| value.release.as_str()),
                    kernel.map(|value| value.version.as_str()),
                    kernel.map(|value| value.machine.as_str()),
                    distribution.and_then(|value| value.id.as_deref()),
                    distribution.and_then(|value| value.version_id.as_deref()),
                    distribution.and_then(|value| value.pretty_name.as_deref()),
                    page_size_bytes,
                    ticks_per_second,
                    uptime_microseconds,
                ],
            )
            .map_err(ArtifactError::Sqlite)?;

        transaction.commit().map_err(ArtifactError::Sqlite)?;
        self.connection
            .execute_batch("PRAGMA optimize;")
            .map_err(ArtifactError::Sqlite)?;
        self.connection
            .close()
            .map_err(|(_, error)| ArtifactError::Sqlite(error))?;
        File::open(&self.path)
            .and_then(|file| file.sync_all())
            .map_err(ArtifactError::Io)
    }
}
