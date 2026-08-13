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
    let finished_at_utc = now_utc();
    let compute_memory = contract::LinuxComputeMemory {
        compute,
        memory,
        pressure,
    };
    let snapshot = Snapshot::new(
        snapshot_id,
        started_at_utc,
        finished_at_utc,
        identity,
        compute_memory,
        outcomes,
    );
    let status = snapshot.status;
    artifact::publish(&snapshot, output)?;
    Ok(status)
}
