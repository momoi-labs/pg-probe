pub mod artifact;
pub mod contract;
pub mod linux;

use std::path::Path;

use contract::{now_utc, Snapshot, SnapshotStatus};
use uuid::Uuid;

pub fn capture(output: &Path) -> Result<SnapshotStatus, artifact::ArtifactError> {
    let snapshot_id = Uuid::new_v4();
    let started_at_utc = now_utc();
    let (identity, outcomes) = linux::collect_identity_isolated();
    let finished_at_utc = now_utc();
    let snapshot = Snapshot::new(
        snapshot_id,
        started_at_utc,
        finished_at_utc,
        identity,
        outcomes,
    );
    let status = snapshot.status;
    artifact::publish(&snapshot, output)?;
    Ok(status)
}
