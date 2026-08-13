pub mod artifact;
pub mod contract;
pub mod linux;

use std::path::Path;

use contract::{now_utc, Snapshot, SnapshotStatus};
use uuid::Uuid;

pub fn capture(output: &Path) -> Result<SnapshotStatus, artifact::ArtifactError> {
    let snapshot_id = Uuid::new_v4();
    let started_at_utc = now_utc();
    let (identity, mut outcomes) = linux::collect_identity_isolated();
    let (compute, compute_outcomes) = linux::collect_compute_isolated();
    outcomes.extend(compute_outcomes);
    let (memory, memory_outcomes) = linux::collect_memory_isolated();
    outcomes.extend(memory_outcomes);
    let (pressure, pressure_outcomes) = linux::collect_pressure_isolated();
    outcomes.extend(pressure_outcomes);
    let (storage, storage_outcomes) = linux::collect_storage_isolated();
    outcomes.extend(storage_outcomes);
    let (limits, limits_outcomes) = linux::collect_limits_isolated();
    outcomes.extend(limits_outcomes);
    let (cgroups, cgroups_outcomes) = linux::collect_cgroups_isolated();
    outcomes.extend(cgroups_outcomes);
    let (kernel, kernel_outcomes) = linux::collect_kernel_isolated();
    outcomes.extend(kernel_outcomes);
    let finished_at_utc = now_utc();
    let compute_memory = contract::LinuxComputeMemory {
        compute,
        memory,
        pressure,
    };
    let storage_limits = contract::LinuxStorageLimits {
        storage,
        limits,
        cgroups,
        kernel,
    };
    let snapshot = Snapshot::new(
        snapshot_id,
        started_at_utc,
        finished_at_utc,
        identity,
        compute_memory,
        storage_limits,
        outcomes,
    );
    let status = snapshot.status;
    artifact::publish(&snapshot, output)?;
    Ok(status)
}
