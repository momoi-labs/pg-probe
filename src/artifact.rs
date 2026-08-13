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
                    sequence INTEGER NOT NULL,
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
                    PRIMARY KEY (snapshot_id, source, sequence),
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
                CREATE TABLE linux_mount (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    mount_id INTEGER NOT NULL,
                    parent_mount_id INTEGER NOT NULL,
                    device_major INTEGER NOT NULL,
                    device_minor INTEGER NOT NULL,
                    filesystem_type TEXT NOT NULL,
                    is_read_only INTEGER NOT NULL CHECK (is_read_only IN (0, 1)),
                    total_bytes INTEGER,
                    available_bytes INTEGER,
                    total_inodes INTEGER,
                    available_inodes INTEGER,
                    PRIMARY KEY (snapshot_id, mount_id)
                ) STRICT;
                CREATE TABLE linux_block_device (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    device_major INTEGER NOT NULL,
                    device_minor INTEGER NOT NULL,
                    kernel_name TEXT,
                    logical_block_size_bytes INTEGER,
                    physical_block_size_bytes INTEGER,
                    minimum_io_size_bytes INTEGER,
                    optimal_io_size_bytes INTEGER,
                    read_ahead_kibibytes INTEGER,
                    request_queue_depth INTEGER,
                    is_rotational INTEGER CHECK (is_rotational IN (0, 1)),
                    scheduler TEXT,
                    PRIMARY KEY (snapshot_id, device_major, device_minor)
                ) STRICT;
                CREATE TABLE linux_block_io (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    device_major INTEGER NOT NULL,
                    device_minor INTEGER NOT NULL,
                    reads_completed INTEGER NOT NULL,
                    reads_merged INTEGER NOT NULL,
                    sectors_read INTEGER NOT NULL,
                    read_milliseconds INTEGER NOT NULL,
                    writes_completed INTEGER NOT NULL,
                    writes_merged INTEGER NOT NULL,
                    sectors_written INTEGER NOT NULL,
                    write_milliseconds INTEGER NOT NULL,
                    io_in_progress INTEGER NOT NULL,
                    io_milliseconds INTEGER NOT NULL,
                    weighted_io_milliseconds INTEGER NOT NULL,
                    discards_completed INTEGER,
                    discard_sectors INTEGER,
                    discard_milliseconds INTEGER,
                    flushes_completed INTEGER,
                    flush_milliseconds INTEGER,
                    PRIMARY KEY (snapshot_id, device_major, device_minor)
                ) STRICT;
                CREATE TABLE linux_resource_limit (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    resource TEXT NOT NULL CHECK (resource IN (
                        'nofile', 'nproc', 'memlock', 'address_space', 'core', 'file_size', 'stack'
                    )),
                    soft_value INTEGER,
                    hard_value INTEGER,
                    soft_unlimited INTEGER NOT NULL CHECK (soft_unlimited IN (0, 1)),
                    hard_unlimited INTEGER NOT NULL CHECK (hard_unlimited IN (0, 1)),
                    unit TEXT NOT NULL CHECK (unit IN ('bytes', 'count')),
                    PRIMARY KEY (snapshot_id, resource)
                ) STRICT;
                CREATE TABLE linux_cgroup (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    version TEXT NOT NULL CHECK (version IN ('v1', 'v2')),
                    cpu_quota_microseconds INTEGER,
                    cpu_period_microseconds INTEGER,
                    cpu_quota_unlimited INTEGER CHECK (cpu_quota_unlimited IN (0, 1)),
                    cpu_weight INTEGER,
                    memory_current_bytes INTEGER,
                    memory_min_bytes INTEGER,
                    memory_low_bytes INTEGER,
                    memory_high_bytes INTEGER,
                    memory_high_unlimited INTEGER CHECK (memory_high_unlimited IN (0, 1)),
                    memory_max_bytes INTEGER,
                    memory_max_unlimited INTEGER CHECK (memory_max_unlimited IN (0, 1)),
                    swap_current_bytes INTEGER,
                    swap_max_bytes INTEGER,
                    swap_max_unlimited INTEGER CHECK (swap_max_unlimited IN (0, 1)),
                    pids_current INTEGER,
                    pids_max INTEGER,
                    pids_max_unlimited INTEGER CHECK (pids_max_unlimited IN (0, 1))
                ) STRICT;
                CREATE TABLE linux_cgroup_event (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    controller TEXT NOT NULL CHECK (controller IN ('cpu', 'memory', 'pids')),
                    event_name TEXT NOT NULL,
                    cumulative_value INTEGER NOT NULL,
                    unit TEXT NOT NULL CHECK (unit IN ('count', 'microseconds')),
                    PRIMARY KEY (snapshot_id, controller, event_name)
                ) STRICT;
                CREATE TABLE linux_cgroup_io (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    device_major INTEGER NOT NULL,
                    device_minor INTEGER NOT NULL,
                    read_bytes INTEGER,
                    write_bytes INTEGER,
                    discard_bytes INTEGER,
                    read_operations INTEGER,
                    write_operations INTEGER,
                    discard_operations INTEGER,
                    PRIMARY KEY (snapshot_id, device_major, device_minor)
                ) STRICT;
                CREATE TABLE linux_kernel_vm (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    overcommit_mode INTEGER,
                    overcommit_ratio_percent INTEGER,
                    overcommit_kbytes_source INTEGER,
                    overcommit_bytes INTEGER,
                    max_map_count INTEGER,
                    swappiness INTEGER,
                    zone_reclaim_mode INTEGER,
                    min_free_kbytes_source INTEGER,
                    min_free_bytes INTEGER,
                    dirty_background_bytes INTEGER,
                    dirty_background_ratio_percent INTEGER,
                    dirty_bytes INTEGER,
                    dirty_ratio_percent INTEGER,
                    dirty_expire_microseconds INTEGER,
                    dirty_writeback_microseconds INTEGER
                ) STRICT;
                CREATE TABLE linux_kernel_ipc (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    shmmax_bytes INTEGER,
                    shmall_pages INTEGER,
                    shmall_bytes INTEGER,
                    shmmni INTEGER,
                    semmsl INTEGER,
                    semmns INTEGER,
                    semopm INTEGER,
                    semmni INTEGER,
                    file_max INTEGER,
                    nr_open INTEGER
                ) STRICT;
                CREATE TABLE linux_thp (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    enabled_policy TEXT,
                    defrag_policy TEXT,
                    khugepaged_defrag INTEGER CHECK (khugepaged_defrag IN (0, 1))
                ) STRICT;
                CREATE TABLE postgres_target (
                    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    endpoint_kind TEXT NOT NULL CHECK (endpoint_kind IN ('tcp', 'unix_socket')),
                    endpoint TEXT NOT NULL,
                    port INTEGER,
                    connected_user TEXT NOT NULL,
                    initial_database TEXT NOT NULL,
                    server_version_num INTEGER NOT NULL,
                    server_version TEXT NOT NULL,
                    in_recovery INTEGER NOT NULL CHECK (in_recovery IN (0, 1)),
                    block_size_bytes INTEGER NOT NULL CHECK (block_size_bytes > 0),
                    is_superuser INTEGER NOT NULL CHECK (is_superuser IN (0, 1)),
                    has_pg_monitor INTEGER NOT NULL CHECK (has_pg_monitor IN (0, 1)),
                    has_pg_read_all_stats INTEGER NOT NULL CHECK (has_pg_read_all_stats IN (0, 1)),
                    has_pg_read_all_settings INTEGER NOT NULL CHECK (has_pg_read_all_settings IN (0, 1))
                ) STRICT;
                CREATE TABLE postgres_setting (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    setting_name TEXT NOT NULL,
                    setting_value TEXT NOT NULL,
                    unit TEXT,
                    value_type TEXT NOT NULL,
                    source TEXT NOT NULL,
                    pending_restart INTEGER NOT NULL CHECK (pending_restart IN (0, 1)),
                    PRIMARY KEY (snapshot_id, setting_name)
                ) STRICT;
                CREATE TABLE postgres_database (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    database_name TEXT NOT NULL,
                    is_template INTEGER NOT NULL CHECK (is_template IN (0, 1)),
                    allows_connections INTEGER NOT NULL CHECK (allows_connections IN (0, 1)),
                    can_connect INTEGER NOT NULL CHECK (can_connect IN (0, 1)),
                    size_bytes INTEGER,
                    extension_visit_status TEXT NOT NULL CHECK (extension_visit_status IN (
                        'pending', 'visited', 'not_connectable', 'permission_denied', 'connection_failed'
                    )),
                    PRIMARY KEY (snapshot_id, database_name)
                ) STRICT;
                CREATE TABLE postgres_extension (
                    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
                    collected_at TEXT NOT NULL,
                    database_name TEXT NOT NULL,
                    extension_name TEXT NOT NULL,
                    extension_version TEXT NOT NULL,
                    PRIMARY KEY (snapshot_id, database_name, extension_name)
                ) STRICT;
                CREATE VIEW database_inventory AS
                    SELECT
                        d.snapshot_id,
                        d.database_name,
                        d.is_template,
                        d.allows_connections,
                        d.can_connect,
                        d.size_bytes,
                        d.extension_visit_status,
                        (SELECT COUNT(*) FROM postgres_extension e
                            WHERE e.snapshot_id = d.snapshot_id
                              AND e.database_name = d.database_name) AS extension_count
                    FROM postgres_database d;
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
                        snapshot_id, source, sequence, started_at_utc, finished_at_utc,
                        status, error_code, os_error_code, observed_at_utc,
                        collector_name, collector_version
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                )
                .map_err(ArtifactError::Sqlite)?;
            for (sequence, outcome) in snapshot.outcomes.iter().enumerate() {
                statement
                    .execute(params![
                        snapshot.snapshot_id.to_string(),
                        outcome.source,
                        i64::try_from(sequence).map_err(|_| ArtifactError::ValueOutOfRange)?,
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
        Self::write_mounts(&transaction, snapshot, &collected_at)?;
        Self::write_block_devices(&transaction, snapshot, &collected_at)?;
        Self::write_block_io(&transaction, snapshot, &collected_at)?;
        Self::write_resource_limits(&transaction, snapshot, &collected_at)?;
        Self::write_cgroups(&transaction, snapshot, &collected_at)?;
        Self::write_cgroup_events(&transaction, snapshot, &collected_at)?;
        Self::write_cgroup_io(&transaction, snapshot, &collected_at)?;
        Self::write_kernel_vm(&transaction, snapshot, &collected_at)?;
        Self::write_kernel_ipc(&transaction, snapshot, &collected_at)?;
        Self::write_thp(&transaction, snapshot, &collected_at)?;
        Self::write_postgres_target(&transaction, snapshot, &collected_at)?;
        Self::write_postgres_settings(&transaction, snapshot, &collected_at)?;
        Self::write_postgres_databases(&transaction, snapshot, &collected_at)?;
        Self::write_postgres_extensions(&transaction, snapshot, &collected_at)?;

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

    fn write_mounts(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_mount (
                    snapshot_id, collected_at, mount_id, parent_mount_id,
                    device_major, device_minor, filesystem_type, is_read_only,
                    total_bytes, available_bytes, total_inodes, available_inodes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.storage_limits.storage.mounts {
            let total_bytes = entry
                .total_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let available_bytes = entry
                .available_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let total_inodes = entry
                .total_inodes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let available_inodes = entry
                .available_inodes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(entry.mount_id),
                    i64::from(entry.parent_mount_id),
                    i64::from(entry.device_major),
                    i64::from(entry.device_minor),
                    entry.filesystem_type.as_str(),
                    entry.is_read_only as i64,
                    total_bytes,
                    available_bytes,
                    total_inodes,
                    available_inodes,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_block_devices(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_block_device (
                    snapshot_id, collected_at, device_major, device_minor,
                    kernel_name, logical_block_size_bytes, physical_block_size_bytes,
                    minimum_io_size_bytes, optimal_io_size_bytes, read_ahead_kibibytes,
                    request_queue_depth, is_rotational, scheduler
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.storage_limits.storage.block_devices {
            let logical = entry
                .logical_block_size_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let physical = entry
                .physical_block_size_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let min_io = entry
                .minimum_io_size_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let opt_io = entry
                .optimal_io_size_bytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let read_ahead = entry
                .read_ahead_kibibytes
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let queue_depth = entry
                .request_queue_depth
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;

            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(entry.device_major),
                    i64::from(entry.device_minor),
                    entry.kernel_name.as_deref(),
                    logical,
                    physical,
                    min_io,
                    opt_io,
                    read_ahead,
                    queue_depth,
                    entry.is_rotational.map(|b| b as i64),
                    entry.scheduler.as_deref(),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_block_io(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_block_io (
                    snapshot_id, collected_at, device_major, device_minor,
                    reads_completed, reads_merged, sectors_read, read_milliseconds,
                    writes_completed, writes_merged, sectors_written, write_milliseconds,
                    io_in_progress, io_milliseconds, weighted_io_milliseconds,
                    discards_completed, discard_sectors, discard_milliseconds,
                    flushes_completed, flush_milliseconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.storage_limits.storage.block_io {
            let to_i64 = |v: u64| i64::try_from(v).map_err(|_| ArtifactError::ValueOutOfRange);
            let opt_to_i64 = |v: Option<u64>| {
                v.map(i64::try_from)
                    .transpose()
                    .map_err(|_| ArtifactError::ValueOutOfRange)
            };

            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(entry.device_major),
                    i64::from(entry.device_minor),
                    to_i64(entry.reads_completed)?,
                    to_i64(entry.reads_merged)?,
                    to_i64(entry.sectors_read)?,
                    to_i64(entry.read_milliseconds)?,
                    to_i64(entry.writes_completed)?,
                    to_i64(entry.writes_merged)?,
                    to_i64(entry.sectors_written)?,
                    to_i64(entry.write_milliseconds)?,
                    to_i64(entry.io_in_progress)?,
                    to_i64(entry.io_milliseconds)?,
                    to_i64(entry.weighted_io_milliseconds)?,
                    opt_to_i64(entry.discards_completed)?,
                    opt_to_i64(entry.discard_sectors)?,
                    opt_to_i64(entry.discard_milliseconds)?,
                    opt_to_i64(entry.flushes_completed)?,
                    opt_to_i64(entry.flush_milliseconds)?,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_resource_limits(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_resource_limit (
                    snapshot_id, collected_at, resource, soft_value, hard_value,
                    soft_unlimited, hard_unlimited, unit
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.storage_limits.limits.limits {
            let soft = entry
                .soft
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            let hard = entry
                .hard
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)?;

            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.resource.as_str(),
                    soft,
                    hard,
                    entry.soft_unlimited as i64,
                    entry.hard_unlimited as i64,
                    entry.unit,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_cgroups(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let limits = &snapshot.storage_limits.cgroups.limits;
        let Some(version) = limits.version else {
            return Ok(());
        };
        let version_str = version.as_str();

        let opt_u64 = |v: Option<u64>| {
            v.map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)
        };

        transaction
            .execute(
                "INSERT OR IGNORE INTO linux_cgroup (
                    snapshot_id, collected_at, version,
                    cpu_quota_microseconds, cpu_period_microseconds, cpu_quota_unlimited,
                    cpu_weight, memory_current_bytes, memory_min_bytes, memory_low_bytes,
                    memory_high_bytes, memory_high_unlimited, memory_max_bytes,
                    memory_max_unlimited, swap_current_bytes, swap_max_bytes,
                    swap_max_unlimited, pids_current, pids_max, pids_max_unlimited
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    version_str,
                    limits.cpu_quota_microseconds,
                    limits.cpu_period_microseconds,
                    limits.cpu_quota_unlimited.map(|b| b as i64),
                    opt_u64(limits.cpu_weight)?,
                    opt_u64(limits.memory_current_bytes)?,
                    opt_u64(limits.memory_min_bytes)?,
                    opt_u64(limits.memory_low_bytes)?,
                    opt_u64(limits.memory_high_bytes)?,
                    limits.memory_high_unlimited.map(|b| b as i64),
                    opt_u64(limits.memory_max_bytes)?,
                    limits.memory_max_unlimited.map(|b| b as i64),
                    opt_u64(limits.swap_current_bytes)?,
                    opt_u64(limits.swap_max_bytes)?,
                    limits.swap_max_unlimited.map(|b| b as i64),
                    opt_u64(limits.pids_current)?,
                    limits.pids_max,
                    limits.pids_max_unlimited.map(|b| b as i64),
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_cgroup_events(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_cgroup_event (
                    snapshot_id, collected_at, controller, event_name,
                    cumulative_value, unit
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(ArtifactError::Sqlite)?;

        for entry in &snapshot.storage_limits.cgroups.events {
            let value = i64::try_from(entry.cumulative_value)
                .map_err(|_| ArtifactError::ValueOutOfRange)?;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.controller.as_str(),
                    entry.event_name.as_str(),
                    value,
                    entry.unit.as_str(),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_cgroup_io(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT OR IGNORE INTO linux_cgroup_io (
                    snapshot_id, collected_at, device_major, device_minor,
                    read_bytes, write_bytes, discard_bytes,
                    read_operations, write_operations, discard_operations
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(ArtifactError::Sqlite)?;

        let opt_to_i64 = |v: Option<u64>| {
            v.map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)
        };

        for entry in &snapshot.storage_limits.cgroups.io {
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    i64::from(entry.device_major),
                    i64::from(entry.device_minor),
                    opt_to_i64(entry.read_bytes)?,
                    opt_to_i64(entry.write_bytes)?,
                    opt_to_i64(entry.discard_bytes)?,
                    opt_to_i64(entry.read_operations)?,
                    opt_to_i64(entry.write_operations)?,
                    opt_to_i64(entry.discard_operations)?,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_kernel_vm(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let Some(entry) = &snapshot.storage_limits.kernel.vm else {
            return Ok(());
        };

        transaction
            .execute(
                "INSERT OR IGNORE INTO linux_kernel_vm (
                    snapshot_id, collected_at, overcommit_mode, overcommit_ratio_percent,
                    overcommit_kbytes_source, overcommit_bytes, max_map_count, swappiness,
                    zone_reclaim_mode, min_free_kbytes_source, min_free_bytes,
                    dirty_background_bytes, dirty_background_ratio_percent, dirty_bytes,
                    dirty_ratio_percent, dirty_expire_microseconds, dirty_writeback_microseconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.overcommit_mode,
                    entry.overcommit_ratio_percent,
                    entry.overcommit_kbytes_source,
                    entry.overcommit_bytes,
                    entry.max_map_count,
                    entry.swappiness,
                    entry.zone_reclaim_mode,
                    entry.min_free_kbytes_source,
                    entry.min_free_bytes,
                    entry.dirty_background_bytes,
                    entry.dirty_background_ratio_percent,
                    entry.dirty_bytes,
                    entry.dirty_ratio_percent,
                    entry.dirty_expire_microseconds,
                    entry.dirty_writeback_microseconds,
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_kernel_ipc(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let Some(entry) = &snapshot.storage_limits.kernel.ipc else {
            return Ok(());
        };

        transaction
            .execute(
                "INSERT OR IGNORE INTO linux_kernel_ipc (
                    snapshot_id, collected_at, shmmax_bytes, shmall_pages, shmall_bytes,
                    shmmni, semmsl, semmns, semopm, semmni, file_max, nr_open
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.shmmax_bytes,
                    entry.shmall_pages,
                    entry.shmall_bytes,
                    entry.shmmni,
                    entry.semmsl,
                    entry.semmns,
                    entry.semopm,
                    entry.semmni,
                    entry.file_max,
                    entry.nr_open,
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_thp(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let Some(entry) = &snapshot.storage_limits.kernel.thp else {
            return Ok(());
        };

        transaction
            .execute(
                "INSERT OR IGNORE INTO linux_thp (
                    snapshot_id, collected_at, enabled_policy, defrag_policy,
                    khugepaged_defrag
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    entry.enabled_policy.as_deref(),
                    entry.defrag_policy.as_deref(),
                    entry.khugepaged_defrag.map(|b| b as i64),
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_postgres_target(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let target = match snapshot.target_inventory.inventory.target.as_ref() {
            Some(target) => target,
            None => return Ok(()),
        };
        let port = target.endpoint.port.map(i64::from);
        transaction
            .execute(
                "INSERT INTO postgres_target (
                    snapshot_id, collected_at, endpoint_kind, endpoint, port,
                    connected_user, initial_database,
                    server_version_num, server_version, in_recovery, block_size_bytes,
                    is_superuser, has_pg_monitor, has_pg_read_all_stats, has_pg_read_all_settings
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
                )",
                params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    target.endpoint.endpoint_kind.as_str(),
                    target.endpoint.host.as_str(),
                    port,
                    target.connected_user.as_str(),
                    target.initial_database.as_str(),
                    i64::from(target.server_version_num),
                    target.server_version.as_str(),
                    i64::from(target.in_recovery),
                    i64::from(target.block_size_bytes),
                    i64::from(target.is_superuser),
                    i64::from(target.has_pg_monitor),
                    i64::from(target.has_pg_read_all_stats),
                    i64::from(target.has_pg_read_all_settings),
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        Ok(())
    }

    fn write_postgres_settings(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO postgres_setting (
                    snapshot_id, collected_at, setting_name, setting_value,
                    unit, value_type, source, pending_restart
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for setting in &snapshot.target_inventory.inventory.settings {
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    setting.name.as_str(),
                    setting.value.as_str(),
                    setting.unit.as_deref(),
                    setting.value_type.as_str(),
                    setting.source.as_str(),
                    i64::from(setting.pending_restart),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_postgres_databases(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO postgres_database (
                    snapshot_id, collected_at, database_name,
                    is_template, allows_connections, can_connect, size_bytes,
                    extension_visit_status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for database in &snapshot.target_inventory.inventory.databases {
            let size = database.size_bytes;
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    database.database_name.as_str(),
                    i64::from(database.is_template),
                    i64::from(database.allows_connections),
                    i64::from(database.can_connect),
                    size,
                    database.extension_visit_status.as_str(),
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
        Ok(())
    }

    fn write_postgres_extensions(
        transaction: &rusqlite::Transaction,
        snapshot: &Snapshot,
        collected_at: &str,
    ) -> Result<(), ArtifactError> {
        let mut statement = transaction
            .prepare(
                "INSERT INTO postgres_extension (
                    snapshot_id, collected_at, database_name,
                    extension_name, extension_version
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for extension in &snapshot.target_inventory.inventory.extensions {
            statement
                .execute(params![
                    snapshot.snapshot_id.to_string(),
                    collected_at,
                    extension.database_name.as_str(),
                    extension.extension_name.as_str(),
                    extension.extension_version.as_str(),
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
