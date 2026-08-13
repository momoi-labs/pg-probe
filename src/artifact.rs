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
                    started_at_utc TEXT NOT NULL,
                    finished_at_utc TEXT NOT NULL,
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
                CREATE TABLE linux_cpu_set (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    set_kind TEXT NOT NULL CHECK (set_kind IN ('online', 'present', 'possible', 'affinity')),
                    cpu_id INTEGER NOT NULL,
                    PRIMARY KEY (snapshot_id, set_kind, cpu_id)
                ) STRICT;
                CREATE TABLE linux_cpu_topology (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    cpu_id INTEGER NOT NULL,
                    package_id INTEGER,
                    die_id INTEGER,
                    core_id INTEGER,
                    sibling_set TEXT,
                    PRIMARY KEY (snapshot_id, cpu_id)
                ) STRICT;
                CREATE TABLE linux_numa_node (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    node_id INTEGER NOT NULL,
                    cpu_set TEXT NOT NULL,
                    PRIMARY KEY (snapshot_id, node_id)
                ) STRICT;
                CREATE TABLE linux_cpu_time (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    cpu_id INTEGER NOT NULL,
                    state TEXT NOT NULL,
                    ticks INTEGER NOT NULL,
                    PRIMARY KEY (snapshot_id, cpu_id, state)
                ) STRICT;
                CREATE TABLE linux_scheduler (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    procs_running INTEGER,
                    procs_blocked INTEGER,
                    ctxt INTEGER,
                    processes INTEGER,
                    intr INTEGER,
                    softirq INTEGER,
                    load_avg_1m REAL,
                    load_avg_5m REAL,
                    load_avg_15m REAL,
                    runnable_tasks INTEGER,
                    total_tasks INTEGER
                ) STRICT;
                CREATE TABLE linux_memory (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    key TEXT NOT NULL,
                    value_bytes INTEGER,
                    unit TEXT NOT NULL CHECK (unit IN ('bytes', 'pages')),
                    PRIMARY KEY (snapshot_id, key)
                ) STRICT;
                CREATE TABLE linux_vm_event (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    key TEXT NOT NULL,
                    value INTEGER NOT NULL,
                    unit TEXT NOT NULL CHECK (unit IN ('events', 'KiB')),
                    PRIMARY KEY (snapshot_id, key)
                ) STRICT;
                CREATE TABLE linux_psi (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    resource TEXT NOT NULL CHECK (resource IN ('cpu', 'memory', 'io')),
                    scope TEXT NOT NULL CHECK (scope IN ('some', 'full')),
                    avg10 REAL NOT NULL,
                    avg60 REAL NOT NULL,
                    avg300 REAL NOT NULL,
                    total_us INTEGER NOT NULL,
                    PRIMARY KEY (snapshot_id, resource, scope)
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
                        snapshot_id, source, started_at_utc, finished_at_utc,
                        status, error_code, os_error_code, observed_at_utc,
                        collector_name, collector_version
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(ArtifactError::Sqlite)?;
            for outcome in &snapshot.outcomes {
                statement
                    .execute(params![
                        snapshot.snapshot_id.to_string(),
                        outcome.source,
                        format_timestamp(outcome.started_at_utc),
                        format_timestamp(outcome.finished_at_utc),
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

        let collected_at = format_timestamp(snapshot.finished_at_utc);
        Self::write_cpu_sets(&transaction, snapshot, &collected_at)?;
        Self::write_cpu_topology(&transaction, snapshot, &collected_at)?;
        Self::write_numa_nodes(&transaction, snapshot, &collected_at)?;
        Self::write_cpu_times(&transaction, snapshot, &collected_at)?;
        Self::write_scheduler(&transaction, snapshot, &collected_at)?;
        Self::write_memory(&transaction, snapshot, &collected_at)?;
        Self::write_vm_events(&transaction, snapshot, &collected_at)?;
        Self::write_psi(&transaction, snapshot, &collected_at)?;

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

    fn write_cpu_sets(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_cpu_set (snapshot_id, collected_at, set_kind, cpu_id)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(ArtifactError::Sqlite)?;

        let sets = [
            ("online", &snapshot.compute_memory.compute.online),
            ("present", &snapshot.compute_memory.compute.present),
            ("possible", &snapshot.compute_memory.compute.possible),
            ("affinity", &snapshot.compute_memory.compute.affinity),
        ];

        for (kind, set_opt) in sets {
            if let Some(set) = set_opt {
                for cpu_id in &set.cpus {
                    statement
                        .execute(params![
                            snapshot.snapshot_id.to_string(),
                            collected_at,
                            kind,
                            i64::from(*cpu_id),
                        ])
                        .map_err(ArtifactError::Sqlite)?;
                }
            }
        }
        Ok(())
    }

    fn write_cpu_topology(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_cpu_topology (
                    snapshot_id, collected_at, cpu_id, package_id, die_id, core_id, sibling_set
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.compute_memory.compute.topology {
            let sibling_set = entry
                .thread_siblings
                .as_ref()
                .map(|set| format_cpu_set(&set.cpus));
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(entry.cpu_id),
                    entry.package_id.map(i64::from),
                    entry.die_id.map(i64::from),
                    entry.core_id.map(i64::from),
                    sibling_set,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_numa_nodes(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_numa_node (snapshot_id, collected_at, node_id, cpu_set)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for node in &snapshot.compute_memory.compute.numa_nodes {
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(node.node_id),
                    format_cpu_set(&node.cpus.cpus),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_cpu_times(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR REPLACE INTO linux_cpu_time (snapshot_id, collected_at, cpu_id, state, ticks)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.compute_memory.compute.cpu_times {
            let states = [
                ("user", entry.user),
                ("nice", entry.nice),
                ("system", entry.system),
                ("idle", entry.idle),
                ("iowait", entry.iowait),
                ("irq", entry.irq),
                ("softirq", entry.softirq),
                ("steal", entry.steal),
                ("guest", entry.guest),
                ("guest_nice", entry.guest_nice),
            ];

            for (state_name, ticks_value) in states {
                let ticks =
                    i64::try_from(ticks_value).map_err(|_| ArtifactError::ValueOutOfRange)?;
                let cpu_id = entry.cpu_id.map(i64::from).unwrap_or(-1);
                statement
                    .execute(params![
                        snapshot.snapshot_id.to_string(),
                        collected_at,
                        cpu_id,
                        state_name,
                        ticks,
                    ])
                    .map_err(ArtifactError::Sqlite)?;
            }
        }
        Ok(())
    }

    fn write_scheduler(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let scheduler = snapshot.compute_memory.compute.scheduler.as_ref();
        let load_avg = snapshot.compute_memory.compute.load_averages.as_ref();

        if scheduler.is_none() && load_avg.is_none() {
            return Ok(());
        }

        let procs_running = scheduler
            .map(|s| i64::try_from(s.procs_running))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let procs_blocked = scheduler
            .map(|s| i64::try_from(s.procs_blocked))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let ctxt = scheduler
            .map(|s| i64::try_from(s.ctxt))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let processes = scheduler
            .map(|s| i64::try_from(s.processes))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let intr = scheduler
            .map(|s| i64::try_from(s.intr))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let softirq = scheduler
            .map(|s| i64::try_from(s.softirq))
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)?;
        let runnable_tasks = load_avg.map(|l| i64::from(l.runnable_tasks));
        let total_tasks = load_avg.map(|l| i64::from(l.total_tasks));

        transaction
            .execute(
                "INSERT INTO linux_scheduler (
                    snapshot_id, collected_at, procs_running, procs_blocked, ctxt, processes,
                    intr, softirq, load_avg_1m, load_avg_5m, load_avg_15m, runnable_tasks, total_tasks
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    procs_running,
                    procs_blocked,
                    ctxt,
                    processes,
                    intr,
                    softirq,
                    load_avg.map(|l| l.load_1m),
                    load_avg.map(|l| l.load_5m),
                    load_avg.map(|l| l.load_15m),
                    runnable_tasks,
                    total_tasks,
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_memory(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO linux_memory (snapshot_id, collected_at, key, value_bytes, unit)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.compute_memory.memory.memory {
            let value = i64::try_from(entry.value).map_err(|_| ArtifactError::ValueOutOfRange)?;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.key,
                    value,
                    entry.unit.as_str(),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_vm_events(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO linux_vm_event (snapshot_id, collected_at, key, value, unit)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.compute_memory.memory.vm_events {
            let value = i64::try_from(entry.value).map_err(|_| ArtifactError::ValueOutOfRange)?;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.key,
                    value,
                    entry.unit.as_str(),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_psi(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO linux_psi (
                    snapshot_id, collected_at, resource, scope, avg10, avg60, avg300, total_us
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.compute_memory.pressure.psi {
            let total_us =
                i64::try_from(entry.total_us).map_err(|_| ArtifactError::ValueOutOfRange)?;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.resource.as_str(),
                    entry.scope.as_str(),
                    entry.avg10,
                    entry.avg60,
                    entry.avg300,
                    total_us,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }
}

fn format_cpu_set(cpus: &[u32]) -> String {
    if cpus.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let mut start = cpus[0];
    let mut end = cpus[0];

    for &cpu in &cpus[1..] {
        if cpu == end + 1 {
            end = cpu;
        } else {
            if !result.is_empty() {
                result.push(',');
            }
            if start == end {
                result.push_str(&start.to_string());
            } else {
                result.push_str(&format!("{start}-{end}"));
            }
            start = cpu;
            end = cpu;
        }
    }

    if !result.is_empty() {
        result.push(',');
    }
    if start == end {
        result.push_str(&start.to_string());
    } else {
        result.push_str(&format!("{start}-{end}"));
    }

    result
}
