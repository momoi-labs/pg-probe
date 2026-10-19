use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OpenFlags};
use thiserror::Error;
use uuid::Uuid;

use crate::contract::{
    format_timestamp, ComponentOutcome, LinuxIdentity, Snapshot, SnapshotStatus, COLLECTOR_NAME,
    COLLECTOR_VERSION, SCHEMA_VERSION,
};
use crate::linux::{
    CgroupObservations, ComputeObservations, KernelObservations, LimitsObservations,
    MemoryObservations, PressureObservations, StorageObservations,
};
use crate::postgres::TargetInventory;

const SQLITE_APPLICATION_ID: i64 = 0x5047_5052; // "PGPR"
const FILE_MODE: u32 = 0o600;
/// A single-shot run writes one sample, numbered from 1.
const FIRST_SAMPLE: i64 = 1;

mod embedded_migrations {
    refinery::embed_migrations!("schema/migrations");
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("the data directory does not exist or is not a directory")]
    DataDirectoryMissing,
    #[error("the data directory is not writable")]
    DataDirectoryNotWritable,
    #[error("failed to persist the SQLite snapshot")]
    Sqlite(#[source] rusqlite::Error),
    #[error("failed to migrate the SQLite snapshot schema")]
    Migration(#[source] refinery::Error),
    #[error("a collected value exceeds the SQLite integer range")]
    ValueOutOfRange,
    #[error("failed to publish the output artifact")]
    Io(#[source] io::Error),
    #[error("the output artifact already exists")]
    OutputExists,
}

/// Scans `data_dir` for existing `.sqlite.partial` files.
pub fn scan_for_partials(data_dir: &Path) -> Vec<PathBuf> {
    let mut partials = Vec::new();
    if let Ok(entries) = fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".sqlite.partial"))
            {
                partials.push(path);
            }
        }
    }
    partials.sort();
    partials
}

/// Generates the artifact base name from a sanitized hostname and a UTC
/// timestamp in `YYYY-MM-DD-HHhMMmSSs` format.
pub fn format_base_name(sanitized_hostname: &str, timestamp: DateTime<Utc>) -> String {
    use chrono::{Datelike, Timelike};
    format!(
        "{}-{:04}-{:02}-{:02}-{:02}h{:02}m{:02}s",
        sanitized_hostname,
        timestamp.year(),
        timestamp.month(),
        timestamp.day(),
        timestamp.hour(),
        timestamp.minute(),
        timestamp.second(),
    )
}

fn find_unique_paths(
    data_dir: &Path,
    base_name: &str,
) -> Result<(PathBuf, PathBuf), ArtifactError> {
    let mut suffix = 0u32;
    loop {
        let (partial, final_path) = if suffix == 0 {
            (
                data_dir.join(format!("{base_name}.sqlite.partial")),
                data_dir.join(format!("{base_name}.sqlite.gz")),
            )
        } else {
            (
                data_dir.join(format!("{base_name}-{suffix}.sqlite.partial")),
                data_dir.join(format!("{base_name}-{suffix}.sqlite.gz")),
            )
        };
        if !partial.exists() && !final_path.exists() {
            return Ok((partial, final_path));
        }
        suffix = suffix
            .checked_add(1)
            .ok_or(ArtifactError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "exhausted collision suffix space",
            )))?;
    }
}

fn create_secure_file(path: &Path) -> Result<File, ArtifactError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(ArtifactError::Io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(FILE_MODE)).map_err(ArtifactError::Io)?;
    Ok(file)
}

fn compress_to_gzip(source: &Path, destination: &Path) -> Result<(), ArtifactError> {
    let input = File::open(source).map_err(ArtifactError::Io)?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(destination)
        .map_err(ArtifactError::Io)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(FILE_MODE))
        .map_err(ArtifactError::Io)?;
    let mut encoder = GzEncoder::new(BufWriter::new(output), Compression::default());
    io::copy(&mut BufReader::new(input), &mut encoder).map_err(ArtifactError::Io)?;
    let mut writer = encoder.finish().map_err(ArtifactError::Io)?;
    writer.flush().map_err(ArtifactError::Io)?;
    writer.get_ref().sync_all().map_err(ArtifactError::Io)?;
    Ok(())
}

/// Incremental writer that creates a `.sqlite.partial` file, commits each
/// collector domain in its own transaction, and finalizes to `.sqlite.gz`.
pub struct PartialWriter {
    connection: Connection,
    partial_path: PathBuf,
    final_path: PathBuf,
    snapshot_id: String,
    sample_id: i64,
    next_sequence: i64,
}

impl PartialWriter {
    fn try_create(
        partial_path: &Path,
        final_path: &Path,
        snapshot_id: Uuid,
        started_at_utc: DateTime<Utc>,
    ) -> Result<Self, ArtifactError> {
        create_secure_file(partial_path)?;
        let connection = Connection::open_with_flags(
            partial_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(ArtifactError::Sqlite)?;
        let snapshot_id_str = snapshot_id.to_string();
        let mut writer = Self {
            connection,
            partial_path: partial_path.to_owned(),
            final_path: final_path.to_owned(),
            snapshot_id: snapshot_id_str.clone(),
            sample_id: FIRST_SAMPLE,
            next_sequence: 0,
        };
        writer.init_schema()?;
        writer.write_envelope(started_at_utc)?;
        Ok(writer)
    }

    fn init_schema(&mut self) -> Result<(), ArtifactError> {
        self.connection
            .execute_batch(&format!(
                "
                PRAGMA application_id = {SQLITE_APPLICATION_ID};
                PRAGMA user_version = {SCHEMA_VERSION};
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = DELETE;
                "
            ))
            .map_err(ArtifactError::Sqlite)?;
        embedded_migrations::migrations::runner()
            .run(&mut self.connection)
            .map_err(ArtifactError::Migration)?;
        Ok(())
    }

    fn write_envelope(&mut self, started_at_utc: DateTime<Utc>) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        tx.execute(
            "INSERT INTO snapshot (
                snapshot_id, schema_version, collector_name, collector_version,
                started_at_utc, finished_at_utc, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &self.snapshot_id,
                SCHEMA_VERSION,
                COLLECTOR_NAME,
                COLLECTOR_VERSION,
                format_timestamp(started_at_utc),
                format_timestamp(started_at_utc),
                SnapshotStatus::Partial.as_str(),
            ],
        )
        .map_err(ArtifactError::Sqlite)?;
        write_sample(&tx, &self.snapshot_id, self.sample_id, started_at_utc)?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }
}

impl crate::collector::Sink for PartialWriter {
    type Error = ArtifactError;

    fn write_identity(
        &mut self,
        identity: &LinuxIdentity,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_identity_row(&tx, &self.snapshot_id, identity)?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_compute(
        &mut self,
        compute: &ComputeObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_cpu_sets(&tx, &self.snapshot_id, compute, &collected_at)?;
        write_cpu_topology(&tx, &self.snapshot_id, compute, &collected_at)?;
        write_numa_nodes(&tx, &self.snapshot_id, compute, &collected_at)?;
        write_cpu_times(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            compute,
            &collected_at,
        )?;
        write_scheduler(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            compute,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_memory(
        &mut self,
        memory: &MemoryObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_memory_rows(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            memory,
            &collected_at,
        )?;
        write_vm_events(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            memory,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_pressure(
        &mut self,
        pressure: &PressureObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_psi(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            pressure,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_storage(
        &mut self,
        storage: &StorageObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_mounts(&tx, &self.snapshot_id, storage, &collected_at)?;
        write_block_devices(&tx, &self.snapshot_id, storage, &collected_at)?;
        write_block_io(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            storage,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_limits(
        &mut self,
        limits: &LimitsObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_resource_limits(&tx, &self.snapshot_id, limits, &collected_at)?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_cgroups(
        &mut self,
        cgroups: &CgroupObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_cgroup_limits(&tx, &self.snapshot_id, cgroups, &collected_at)?;
        write_cgroup_usage(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            cgroups,
            &collected_at,
        )?;
        write_cgroup_events(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            cgroups,
            &collected_at,
        )?;
        write_cgroup_io(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            cgroups,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_kernel(
        &mut self,
        kernel: &KernelObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_kernel_vm(&tx, &self.snapshot_id, kernel, &collected_at)?;
        write_kernel_ipc(&tx, &self.snapshot_id, kernel, &collected_at)?;
        write_thp(&tx, &self.snapshot_id, kernel, &collected_at)?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }

    fn write_postgres(
        &mut self,
        inventory: &TargetInventory,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), ArtifactError> {
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        let collected_at = format_timestamp(crate::contract::now_utc());
        write_outcomes(&tx, &self.snapshot_id, self.next_sequence, outcomes)?;
        self.next_sequence += outcome_count(outcomes)?;
        write_postgres_target(&tx, &self.snapshot_id, inventory, &collected_at)?;
        write_postgres_settings(&tx, &self.snapshot_id, inventory, &collected_at)?;
        write_postgres_databases(&tx, &self.snapshot_id, inventory, &collected_at)?;
        write_postgres_extensions(&tx, &self.snapshot_id, inventory, &collected_at)?;
        write_postgres_database_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_standby_conflicts(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_wal_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_bgwriter_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_checkpointer_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_archiver_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_io_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_slru_stats(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_activity_aggregates(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_lock_aggregates(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_replication_aggregates(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        write_postgres_slot_aggregates(
            &tx,
            &self.snapshot_id,
            self.sample_id,
            inventory,
            &collected_at,
        )?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.fsync_partial()
    }
}

impl PartialWriter {
    pub fn create(
        data_dir: &Path,
        base_name: &str,
        snapshot_id: Uuid,
        started_at_utc: DateTime<Utc>,
    ) -> Result<Self, ArtifactError> {
        if !data_dir.is_dir() {
            return Err(ArtifactError::DataDirectoryMissing);
        }
        let (partial_path, final_path) = find_unique_paths(data_dir, base_name)?;
        let writer = Self::try_create(&partial_path, &final_path, snapshot_id, started_at_utc);
        if writer.is_err() {
            let _ = fs::remove_file(&partial_path);
        }
        writer
    }

    pub fn finalize(
        self,
        status: SnapshotStatus,
        finished_at_utc: DateTime<Utc>,
    ) -> Result<PathBuf, ArtifactError> {
        self.connection
            .execute(
                "UPDATE snapshot SET finished_at_utc = ?1, status = ?2 WHERE snapshot_id = ?3",
                params![
                    format_timestamp(finished_at_utc),
                    status.as_str(),
                    &self.snapshot_id,
                ],
            )
            .map_err(ArtifactError::Sqlite)?;
        self.connection
            .execute_batch("PRAGMA optimize;")
            .map_err(ArtifactError::Sqlite)?;
        self.connection
            .close()
            .map_err(|(_, error)| ArtifactError::Sqlite(error))?;

        File::open(&self.partial_path)
            .and_then(|file| file.sync_all())
            .map_err(ArtifactError::Io)?;

        let partial_path = self.partial_path.clone();
        let final_path = self.final_path.clone();
        compress_to_gzip(&partial_path, &final_path)?;
        fs::remove_file(&partial_path).map_err(ArtifactError::Io)?;
        Ok(final_path)
    }

    /// Starts the next sample. Variant observations written after this call
    /// are keyed to it.
    pub fn begin_sample(&mut self, taken_at_utc: DateTime<Utc>) -> Result<(), ArtifactError> {
        let next = self.sample_id + 1;
        let tx = self
            .connection
            .transaction()
            .map_err(ArtifactError::Sqlite)?;
        write_sample(&tx, &self.snapshot_id, next, taken_at_utc)?;
        tx.commit().map_err(ArtifactError::Sqlite)?;
        self.sample_id = next;
        self.fsync_partial()
    }

    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }

    fn fsync_partial(&self) -> Result<(), ArtifactError> {
        File::open(&self.partial_path)
            .and_then(|file| file.sync_all())
            .map_err(ArtifactError::Io)
    }
}

pub fn publish(snapshot: &Snapshot, output: &Path) -> Result<(), ArtifactError> {
    if output.exists() {
        return Err(ArtifactError::OutputExists);
    }

    let data_dir = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !data_dir.is_dir() {
        return Err(ArtifactError::DataDirectoryMissing);
    }

    let nonce = Uuid::new_v4();
    let sqlite_path = data_dir.join(format!(".pg-probe-{nonce}.sqlite.tmp"));
    let gzip_path = data_dir.join(format!(".pg-probe-{nonce}.sqlite.gz.tmp"));

    let result = (|| {
        create_secure_file(&sqlite_path)?;
        let mut connection = Connection::open_with_flags(
            &sqlite_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(ArtifactError::Sqlite)?;

        connection
            .execute_batch(&format!(
                "PRAGMA application_id = {SQLITE_APPLICATION_ID};
                 PRAGMA user_version = {SCHEMA_VERSION};
                 PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = DELETE;"
            ))
            .map_err(ArtifactError::Sqlite)?;
        embedded_migrations::migrations::runner()
            .run(&mut connection)
            .map_err(ArtifactError::Migration)?;

        let snapshot_id = snapshot.snapshot_id.to_string();
        let tx = connection.transaction().map_err(ArtifactError::Sqlite)?;

        tx.execute(
            "INSERT INTO snapshot (
                snapshot_id, schema_version, collector_name, collector_version,
                started_at_utc, finished_at_utc, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &snapshot_id,
                SCHEMA_VERSION,
                COLLECTOR_NAME,
                COLLECTOR_VERSION,
                format_timestamp(snapshot.started_at_utc),
                format_timestamp(snapshot.finished_at_utc),
                snapshot.status.as_str(),
            ],
        )
        .map_err(ArtifactError::Sqlite)?;

        write_sample(&tx, &snapshot_id, FIRST_SAMPLE, snapshot.started_at_utc)?;
        write_outcomes(&tx, &snapshot_id, 0, &snapshot.outcomes)?;
        write_identity_row(&tx, &snapshot_id, &snapshot.identity)?;

        let collected_at = format_timestamp(snapshot.finished_at_utc);
        write_cpu_sets(
            &tx,
            &snapshot_id,
            &snapshot.compute_memory.compute,
            &collected_at,
        )?;
        write_cpu_topology(
            &tx,
            &snapshot_id,
            &snapshot.compute_memory.compute,
            &collected_at,
        )?;
        write_numa_nodes(
            &tx,
            &snapshot_id,
            &snapshot.compute_memory.compute,
            &collected_at,
        )?;
        write_cpu_times(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.compute_memory.compute,
            &collected_at,
        )?;
        write_scheduler(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.compute_memory.compute,
            &collected_at,
        )?;
        write_memory_rows(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.compute_memory.memory,
            &collected_at,
        )?;
        write_vm_events(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.compute_memory.memory,
            &collected_at,
        )?;
        write_psi(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.compute_memory.pressure,
            &collected_at,
        )?;
        write_mounts(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.storage,
            &collected_at,
        )?;
        write_block_devices(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.storage,
            &collected_at,
        )?;
        write_block_io(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.storage_limits.storage,
            &collected_at,
        )?;
        write_resource_limits(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.limits,
            &collected_at,
        )?;
        write_cgroup_limits(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.cgroups,
            &collected_at,
        )?;
        write_cgroup_usage(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.storage_limits.cgroups,
            &collected_at,
        )?;
        write_cgroup_events(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.storage_limits.cgroups,
            &collected_at,
        )?;
        write_cgroup_io(
            &tx,
            &snapshot_id,
            FIRST_SAMPLE,
            &snapshot.storage_limits.cgroups,
            &collected_at,
        )?;
        write_kernel_vm(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.kernel,
            &collected_at,
        )?;
        write_kernel_ipc(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.kernel,
            &collected_at,
        )?;
        write_thp(
            &tx,
            &snapshot_id,
            &snapshot.storage_limits.kernel,
            &collected_at,
        )?;
        write_postgres_target(
            &tx,
            &snapshot_id,
            &snapshot.target_inventory.inventory,
            &collected_at,
        )?;
        write_postgres_settings(
            &tx,
            &snapshot_id,
            &snapshot.target_inventory.inventory,
            &collected_at,
        )?;
        write_postgres_databases(
            &tx,
            &snapshot_id,
            &snapshot.target_inventory.inventory,
            &collected_at,
        )?;
        write_postgres_extensions(
            &tx,
            &snapshot_id,
            &snapshot.target_inventory.inventory,
            &collected_at,
        )?;

        tx.commit().map_err(ArtifactError::Sqlite)?;
        connection
            .execute_batch("PRAGMA optimize;")
            .map_err(ArtifactError::Sqlite)?;
        connection
            .close()
            .map_err(|(_, error)| ArtifactError::Sqlite(error))?;

        File::open(&sqlite_path)
            .and_then(|file| file.sync_all())
            .map_err(ArtifactError::Io)?;

        compress_to_gzip(&sqlite_path, &gzip_path)?;
        fs::rename(&gzip_path, output).map_err(ArtifactError::Io)?;
        fs::remove_file(&sqlite_path).map_err(ArtifactError::Io)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&sqlite_path);
        let _ = fs::remove_file(&gzip_path);
    }
    result
}

fn write_sample(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    taken_at_utc: DateTime<Utc>,
) -> Result<(), ArtifactError> {
    tx.execute(
        "INSERT INTO sample (snapshot_id, sample_id, taken_at_utc) VALUES (?1, ?2, ?3)",
        params![snapshot_id, sample_id, format_timestamp(taken_at_utc)],
    )
    .map_err(ArtifactError::Sqlite)?;
    Ok(())
}

fn outcome_count(outcomes: &[ComponentOutcome]) -> Result<i64, ArtifactError> {
    i64::try_from(outcomes.len()).map_err(|_| ArtifactError::ValueOutOfRange)
}

/// Writes outcomes numbered from `first_sequence`. The writer keeps one
/// running sequence for the whole snapshot, so a source that reports again in
/// a later sample does not collide with its earlier outcome.
fn write_outcomes(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    first_sequence: i64,
    outcomes: &[ComponentOutcome],
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO collector_outcome (
                snapshot_id, source, sequence, started_at_utc, finished_at_utc,
                status, error_code, os_error_code, observed_at_utc,
                collector_name, collector_version
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for (sequence, outcome) in outcomes.iter().enumerate() {
        statement
            .execute(params![
                snapshot_id,
                outcome.source,
                first_sequence
                    + i64::try_from(sequence).map_err(|_| ArtifactError::ValueOutOfRange)?,
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
    Ok(())
}

fn write_identity_row(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    identity: &LinuxIdentity,
) -> Result<(), ArtifactError> {
    let kernel = identity.kernel.as_ref();
    let distribution = identity.distribution.as_ref();
    let clock = identity.clock.as_ref();
    let page_size_bytes = clock
        .map(|value| i64::try_from(value.page_size_bytes))
        .transpose()
        .map_err(|_| ArtifactError::ValueOutOfRange)?;
    let ticks_per_second = clock
        .map(|value| i64::try_from(value.ticks_per_second))
        .transpose()
        .map_err(|_| ArtifactError::ValueOutOfRange)?;
    let uptime_microseconds = identity
        .uptime_microseconds
        .map(i64::try_from)
        .transpose()
        .map_err(|_| ArtifactError::ValueOutOfRange)?;
    tx.execute(
        "INSERT INTO linux_identity (
            snapshot_id, hostname, kernel_sysname, kernel_release, kernel_version,
            kernel_machine, distribution_id, distribution_version_id,
            distribution_pretty_name, page_size_bytes, ticks_per_second,
            uptime_microseconds
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            snapshot_id,
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
    Ok(())
}

fn write_cpu_sets(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    compute: &ComputeObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_cpu_set (snapshot_id, collected_at, set_kind, cpu_id)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(ArtifactError::Sqlite)?;

    let sets = [
        ("online", &compute.online),
        ("present", &compute.present),
        ("possible", &compute.possible),
        ("affinity", &compute.affinity),
    ];

    for (kind, set_opt) in sets {
        if let Some(set) = set_opt {
            for cpu_id in &set.cpus {
                statement
                    .execute(params![snapshot_id, collected_at, kind, i64::from(*cpu_id),])
                    .map_err(ArtifactError::Sqlite)?;
            }
        }
    }
    Ok(())
}

fn write_cpu_topology(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    compute: &ComputeObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_cpu_topology (
                snapshot_id, collected_at, cpu_id, package_id, die_id, core_id, sibling_set
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &compute.topology {
        let sibling_set = entry
            .thread_siblings
            .as_ref()
            .map(|set| format_cpu_set(&set.cpus));
        statement
            .execute(params![
                snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    compute: &ComputeObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_numa_node (snapshot_id, collected_at, node_id, cpu_set)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for node in &compute.numa_nodes {
        statement
            .execute(params![
                snapshot_id,
                collected_at,
                i64::from(node.node_id),
                format_cpu_set(&node.cpus.cpus),
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_cpu_times(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    compute: &ComputeObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR REPLACE INTO linux_cpu_time (snapshot_id, sample_id, collected_at, cpu_id, state, ticks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &compute.cpu_times {
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
            let ticks = i64::try_from(ticks_value).map_err(|_| ArtifactError::ValueOutOfRange)?;
            let cpu_id = entry.cpu_id.map(i64::from).unwrap_or(-1);
            statement
                .execute(params![
                    snapshot_id,
                    sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    compute: &ComputeObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let scheduler = compute.scheduler.as_ref();
    let load_avg = compute.load_averages.as_ref();

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

    tx.execute(
        "INSERT INTO linux_scheduler (
                snapshot_id, sample_id, collected_at, procs_running, procs_blocked, ctxt, processes,
                intr, softirq, load_avg_1m, load_avg_5m, load_avg_15m, runnable_tasks, total_tasks
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            snapshot_id,
            sample_id,
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

fn write_memory_rows(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    memory: &MemoryObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO linux_memory (snapshot_id, sample_id, collected_at, key, value_bytes, unit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &memory.memory {
        let value = i64::try_from(entry.value).map_err(|_| ArtifactError::ValueOutOfRange)?;
        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    memory: &MemoryObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO linux_vm_event (snapshot_id, sample_id, collected_at, key, value, unit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &memory.vm_events {
        let value = i64::try_from(entry.value).map_err(|_| ArtifactError::ValueOutOfRange)?;
        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    pressure: &PressureObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO linux_psi (
                snapshot_id, sample_id, collected_at, resource, scope, avg10, avg60, avg300, total_us
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &pressure.psi {
        let total_us = i64::try_from(entry.total_us).map_err(|_| ArtifactError::ValueOutOfRange)?;
        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    storage: &StorageObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_mount (
                snapshot_id, collected_at, mount_id, parent_mount_id,
                device_major, device_minor, filesystem_type, is_read_only,
                total_bytes, available_bytes, total_inodes, available_inodes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &storage.mounts {
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
                snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    storage: &StorageObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_block_device (
                snapshot_id, collected_at, device_major, device_minor,
                kernel_name, logical_block_size_bytes, physical_block_size_bytes,
                minimum_io_size_bytes, optimal_io_size_bytes, read_ahead_kibibytes,
                request_queue_depth, is_rotational, scheduler
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &storage.block_devices {
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
                snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    storage: &StorageObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_block_io (
                snapshot_id, sample_id, collected_at, device_major, device_minor,
                reads_completed, reads_merged, sectors_read, read_milliseconds,
                writes_completed, writes_merged, sectors_written, write_milliseconds,
                io_in_progress, io_milliseconds, weighted_io_milliseconds,
                discards_completed, discard_sectors, discard_milliseconds,
                flushes_completed, flush_milliseconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &storage.block_io {
        let to_i64 = |v: u64| i64::try_from(v).map_err(|_| ArtifactError::ValueOutOfRange);
        let opt_to_i64 = |v: Option<u64>| {
            v.map(i64::try_from)
                .transpose()
                .map_err(|_| ArtifactError::ValueOutOfRange)
        };

        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    limits: &LimitsObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_resource_limit (
                snapshot_id, collected_at, resource, soft_value, hard_value,
                soft_unlimited, hard_unlimited, unit
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &limits.limits {
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
                snapshot_id,
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

fn write_cgroup_limits(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    cgroups: &CgroupObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let limits = &cgroups.limits;
    let Some(version) = limits.version else {
        return Ok(());
    };
    let version_str = version.as_str();

    let opt_u64 = |v: Option<u64>| {
        v.map(i64::try_from)
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)
    };

    tx.execute(
        "INSERT OR IGNORE INTO linux_cgroup (
                snapshot_id, collected_at, version,
                cpu_quota_microseconds, cpu_period_microseconds, cpu_quota_unlimited,
                cpu_weight, memory_min_bytes, memory_low_bytes,
                memory_high_bytes, memory_high_unlimited, memory_max_bytes,
                memory_max_unlimited, swap_max_bytes,
                swap_max_unlimited, pids_max, pids_max_unlimited
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            snapshot_id,
            collected_at,
            version_str,
            limits.cpu_quota_microseconds,
            limits.cpu_period_microseconds,
            limits.cpu_quota_unlimited.map(|b| b as i64),
            opt_u64(limits.cpu_weight)?,
            opt_u64(limits.memory_min_bytes)?,
            opt_u64(limits.memory_low_bytes)?,
            opt_u64(limits.memory_high_bytes)?,
            limits.memory_high_unlimited.map(|b| b as i64),
            opt_u64(limits.memory_max_bytes)?,
            limits.memory_max_unlimited.map(|b| b as i64),
            opt_u64(limits.swap_max_bytes)?,
            limits.swap_max_unlimited.map(|b| b as i64),
            limits.pids_max,
            limits.pids_max_unlimited.map(|b| b as i64),
        ],
    )
    .map_err(ArtifactError::Sqlite)?;
    Ok(())
}

// Usage is variant: it is recorded per sample, apart from the limits.
fn write_cgroup_usage(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    cgroups: &CgroupObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let limits = &cgroups.limits;
    if limits.version.is_none() {
        return Ok(());
    }

    let opt_u64 = |v: Option<u64>| {
        v.map(i64::try_from)
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)
    };

    tx.execute(
        "INSERT OR IGNORE INTO linux_cgroup_usage (
                snapshot_id, sample_id, collected_at,
                memory_current_bytes, swap_current_bytes, pids_current
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot_id,
            sample_id,
            collected_at,
            opt_u64(limits.memory_current_bytes)?,
            opt_u64(limits.swap_current_bytes)?,
            opt_u64(limits.pids_current)?,
        ],
    )
    .map_err(ArtifactError::Sqlite)?;
    Ok(())
}

fn write_cgroup_events(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    cgroups: &CgroupObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_cgroup_event (
                snapshot_id, sample_id, collected_at, controller, event_name,
                cumulative_value, unit
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(ArtifactError::Sqlite)?;

    for entry in &cgroups.events {
        let value =
            i64::try_from(entry.cumulative_value).map_err(|_| ArtifactError::ValueOutOfRange)?;
        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    cgroups: &CgroupObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT OR IGNORE INTO linux_cgroup_io (
                snapshot_id, sample_id, collected_at, device_major, device_minor,
                read_bytes, write_bytes, discard_bytes,
                read_operations, write_operations, discard_operations
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(ArtifactError::Sqlite)?;

    let opt_to_i64 = |v: Option<u64>| {
        v.map(i64::try_from)
            .transpose()
            .map_err(|_| ArtifactError::ValueOutOfRange)
    };

    for entry in &cgroups.io {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    kernel: &KernelObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let Some(entry) = &kernel.vm else {
        return Ok(());
    };

    tx.execute(
        "INSERT OR IGNORE INTO linux_kernel_vm (
                snapshot_id, collected_at, overcommit_mode, overcommit_ratio_percent,
                overcommit_kbytes_source, overcommit_bytes, max_map_count, swappiness,
                zone_reclaim_mode, min_free_kbytes_source, min_free_bytes,
                dirty_background_bytes, dirty_background_ratio_percent, dirty_bytes,
                dirty_ratio_percent, dirty_expire_microseconds, dirty_writeback_microseconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    kernel: &KernelObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let Some(entry) = &kernel.ipc else {
        return Ok(());
    };

    tx.execute(
        "INSERT OR IGNORE INTO linux_kernel_ipc (
                snapshot_id, collected_at, shmmax_bytes, shmall_pages, shmall_bytes,
                shmmni, semmsl, semmns, semopm, semmni, file_max, nr_open
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    kernel: &KernelObservations,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let Some(entry) = &kernel.thp else {
        return Ok(());
    };

    tx.execute(
        "INSERT OR IGNORE INTO linux_thp (
                snapshot_id, collected_at, enabled_policy, defrag_policy,
                khugepaged_defrag
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let target = match inventory.target.as_ref() {
        Some(target) => target,
        None => return Ok(()),
    };
    let port = target.endpoint.port.map(i64::from);
    tx.execute(
        "INSERT INTO postgres_target (
                snapshot_id, collected_at, endpoint_kind, endpoint, port,
                connected_user, initial_database,
                server_version_num, server_version, in_recovery, block_size_bytes,
                is_superuser, has_pg_monitor, has_pg_read_all_stats, has_pg_read_all_settings
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
            )",
        params![
            snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_setting (
                snapshot_id, collected_at, setting_name, setting_value,
                unit, value_type, source, pending_restart
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for setting in &inventory.settings {
        statement
            .execute(params![
                snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_database (
                snapshot_id, collected_at, database_name,
                is_template, allows_connections, can_connect, size_bytes,
                extension_visit_status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for database in &inventory.databases {
        let size = database.size_bytes;
        statement
            .execute(params![
                snapshot_id,
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
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_extension (
                snapshot_id, collected_at, database_name,
                extension_name, extension_version
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for extension in &inventory.extensions {
        statement
            .execute(params![
                snapshot_id,
                collected_at,
                extension.database_name.as_str(),
                extension.extension_name.as_str(),
                extension.extension_version.as_str(),
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_database_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_database_stats (
                snapshot_id, sample_id, collected_at, database_oid, database_name,
                num_backends, xact_commit, xact_rollback, blks_read, blks_hit,
                tup_returned, tup_fetched, tup_inserted, tup_updated, tup_deleted,
                conflicts, temp_files, temp_bytes, deadlocks, checksum_failures,
                blk_read_time_ms, blk_write_time_ms, session_time_ms, active_time_ms,
                idle_in_transaction_time_ms, sessions, sessions_abandoned, sessions_fatal,
                sessions_killed, stats_reset
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for stats in &inventory.stats.database_stats {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.database_oid,
                stats.database_name.as_str(),
                stats.num_backends,
                stats.xact_commit,
                stats.xact_rollback,
                stats.blks_read,
                stats.blks_hit,
                stats.tup_returned,
                stats.tup_fetched,
                stats.tup_inserted,
                stats.tup_updated,
                stats.tup_deleted,
                stats.conflicts,
                stats.temp_files,
                stats.temp_bytes,
                stats.deadlocks,
                stats.checksum_failures,
                stats.blk_read_time_ms,
                stats.blk_write_time_ms,
                stats.session_time_ms,
                stats.active_time_ms,
                stats.idle_in_transaction_time_ms,
                stats.sessions,
                stats.sessions_abandoned,
                stats.sessions_fatal,
                stats.sessions_killed,
                stats.stats_reset.as_ref().map(|ts| ts.to_rfc3339()),
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_standby_conflicts(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_standby_conflicts (
                snapshot_id, sample_id, collected_at, database_oid, database_name,
                confl_tablespace, confl_lock, confl_snapshot, confl_bufferpin, confl_deadlock
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for stats in &inventory.stats.standby_conflicts {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.database_oid,
                stats.database_name.as_str(),
                stats.confl_tablespace,
                stats.confl_lock,
                stats.confl_snapshot,
                stats.confl_bufferpin,
                stats.confl_deadlock,
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_wal_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(stats) = &inventory.stats.wal_stats {
        tx.execute(
            "INSERT INTO postgres_wal_stats (
                snapshot_id, sample_id, collected_at, wal_records, wal_fpi, wal_bytes,
                wal_buffers_full, wal_write, wal_sync, wal_write_time_ms, wal_sync_time_ms, stats_reset
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.wal_records,
                stats.wal_fpi,
                stats.wal_bytes,
                stats.wal_buffers_full,
                stats.wal_write,
                stats.wal_sync,
                stats.wal_write_time_ms,
                stats.wal_sync_time_ms,
                stats.stats_reset,
            ],
        )
        .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_bgwriter_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(stats) = &inventory.stats.bgwriter_stats {
        tx.execute(
            "INSERT INTO postgres_bgwriter_stats (
                snapshot_id, sample_id, collected_at, checkpoints_timed, checkpoints_req,
                checkpoint_write_time_ms, checkpoint_sync_time_ms, buffers_checkpoint,
                buffers_clean, maxwritten_clean, buffers_backend, buffers_backend_fsync,
                buffers_alloc, stats_reset
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.checkpoints_timed,
                stats.checkpoints_req,
                stats.checkpoint_write_time_ms,
                stats.checkpoint_sync_time_ms,
                stats.buffers_checkpoint,
                stats.buffers_clean,
                stats.maxwritten_clean,
                stats.buffers_backend,
                stats.buffers_backend_fsync,
                stats.buffers_alloc,
                stats.stats_reset,
            ],
        )
        .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_checkpointer_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(stats) = &inventory.stats.checkpointer_stats {
        tx.execute(
            "INSERT INTO postgres_checkpointer_stats (
                snapshot_id, sample_id, collected_at, num_timed, num_requested,
                write_time_ms, sync_time_ms, buffers_written, stats_reset
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.checkpoints_timed,
                stats.checkpoints_req,
                stats.checkpoint_write_time_ms,
                stats.checkpoint_sync_time_ms,
                stats.buffers_checkpoint,
                stats.stats_reset,
            ],
        )
        .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_archiver_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(stats) = &inventory.stats.archiver_stats {
        tx.execute(
            "INSERT INTO postgres_archiver_stats (
                snapshot_id, sample_id, collected_at, archived_count, last_archived_wal,
                last_archived_time, failed_count, last_failed_wal, last_failed_time, stats_reset
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                snapshot_id,
                sample_id,
                collected_at,
                stats.archived_count,
                stats.last_archived_wal,
                stats.last_archived_time,
                stats.failed_count,
                stats.last_failed_wal,
                stats.last_failed_time,
                stats.stats_reset,
            ],
        )
        .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_io_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(entries) = &inventory.stats.io_stats {
        let mut statement = tx
            .prepare(
                "INSERT INTO postgres_io_stats (
                    snapshot_id, sample_id, collected_at, backend_type, io_object, io_context,
                    reads, read_time_ms, writes, write_time_ms, extends, extend_time_ms,
                    op_bytes, hits, evictions, reuses, fsyncs, stats_reset
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for entry in entries {
            statement
                .execute(params![
                    snapshot_id,
                    sample_id,
                    collected_at,
                    entry.backend_type.as_str(),
                    entry.io_object,
                    entry.io_context,
                    entry.reads,
                    entry.read_time_ms,
                    entry.writes,
                    entry.write_time_ms,
                    entry.extends,
                    entry.extend_time_ms,
                    entry.op_bytes,
                    entry.hits,
                    entry.evictions,
                    entry.reuses,
                    entry.fsyncs,
                    entry.stats_reset,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
    }
    Ok(())
}

fn write_postgres_slru_stats(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(entries) = &inventory.stats.slru_stats {
        let mut statement = tx
            .prepare(
                "INSERT INTO postgres_slru_stats (
                    snapshot_id, sample_id, collected_at, name, blocks_zeroed, blocks_hit,
                    blocks_read, blocks_written, blocks_exists, flush_count, stats_reset
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for entry in entries {
            statement
                .execute(params![
                    snapshot_id,
                    sample_id,
                    collected_at,
                    entry.name.as_str(),
                    entry.blocks_zeroed,
                    entry.blocks_hit,
                    entry.blocks_read,
                    entry.blocks_written,
                    entry.blocks_exists,
                    entry.flush_count,
                    entry.stats_reset,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
    }
    Ok(())
}

fn write_postgres_activity_aggregates(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_activity_aggregates (
                snapshot_id, sample_id, collected_at, database_name, backend_type, state,
                wait_event_type, count, max_age_seconds
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for agg in &inventory.stats.activity_aggregates {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
                collected_at,
                agg.database_name,
                agg.backend_type.as_str(),
                agg.state,
                agg.wait_event_type,
                agg.count,
                agg.max_age_seconds,
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_lock_aggregates(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_lock_aggregates (
                snapshot_id, sample_id, collected_at, lock_mode, lock_type, granted,
                fast_path, count, max_wait_age_seconds
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for agg in &inventory.stats.lock_aggregates {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
                collected_at,
                agg.lock_mode.as_str(),
                agg.lock_type.as_str(),
                agg.granted as i32,
                agg.fast_path.map(|b| b as i32),
                agg.count,
                agg.max_wait_age_seconds,
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_replication_aggregates(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO postgres_replication_aggregates (
                snapshot_id, sample_id, collected_at, state, sync_state, count, max_lag_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(ArtifactError::Sqlite)?;
    for agg in &inventory.stats.replication_aggregates {
        statement
            .execute(params![
                snapshot_id,
                sample_id,
                collected_at,
                agg.state,
                agg.sync_state,
                agg.count,
                agg.max_lag_bytes,
            ])
            .map_err(ArtifactError::Sqlite)?;
    }
    Ok(())
}

fn write_postgres_slot_aggregates(
    tx: &rusqlite::Transaction,
    snapshot_id: &str,
    sample_id: i64,
    inventory: &TargetInventory,
    collected_at: &str,
) -> Result<(), ArtifactError> {
    if let Some(aggregates) = &inventory.stats.slot_aggregates {
        let mut statement = tx
            .prepare(
                "INSERT INTO postgres_slot_aggregates (
                    snapshot_id, sample_id, collected_at, slot_type, active, count, max_retained_bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(ArtifactError::Sqlite)?;
        for agg in aggregates {
            statement
                .execute(params![
                    snapshot_id,
                    sample_id,
                    collected_at,
                    agg.slot_type.as_str(),
                    agg.active as i32,
                    agg.count,
                    agg.max_retained_bytes,
                ])
                .map_err(ArtifactError::Sqlite)?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_base_name_uses_sanitized_hostname_and_utc_timestamp() {
        let ts = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 8, 12, 14, 30, 0).unwrap();
        assert_eq!(
            format_base_name("myhost", ts),
            "myhost-2026-08-12-14h30m00s"
        );
    }

    #[test]
    fn find_unique_paths_appends_collision_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let base = "host-2026-08-12-14h30m00s";
        let (p1, f1) = find_unique_paths(dir.path(), base).unwrap();
        assert!(p1
            .to_string_lossy()
            .ends_with("host-2026-08-12-14h30m00s.sqlite.partial"));
        assert!(f1
            .to_string_lossy()
            .ends_with("host-2026-08-12-14h30m00s.sqlite.gz"));

        fs::write(&f1, b"x").unwrap();
        let (p2, f2) = find_unique_paths(dir.path(), base).unwrap();
        assert!(p2
            .to_string_lossy()
            .ends_with("host-2026-08-12-14h30m00s-1.sqlite.partial"));
        assert!(f2
            .to_string_lossy()
            .ends_with("host-2026-08-12-14h30m00s-1.sqlite.gz"));
    }

    #[test]
    fn scan_for_partials_finds_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan_for_partials(dir.path()).is_empty());

        fs::write(
            dir.path().join("host-2026-01-01-00h00m00s.sqlite.partial"),
            b"",
        )
        .unwrap();
        fs::write(dir.path().join("host-2026-01-01-00h00m01s.sqlite.gz"), b"").unwrap();
        fs::write(dir.path().join("other.txt"), b"").unwrap();

        let partials = scan_for_partials(dir.path());
        assert_eq!(partials.len(), 1);
    }
}
