pub mod artifact;
pub mod cancel;
pub mod contract;
pub mod linux;
pub mod postgres;
pub mod sanitize;

use std::path::{Path, PathBuf};

pub use cancel::CancelFlag;
use contract::{now_utc, SnapshotStatus};
use uuid::Uuid;

pub use artifact::ArtifactError;

/// Which collection domains to initialize during a capture run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainSelection {
    All,
    Linux,
    PostgreSql,
}

impl DomainSelection {
    pub fn linux_enabled(self) -> bool {
        matches!(self, Self::All | Self::Linux)
    }

    pub fn postgresql_enabled(self) -> bool {
        matches!(self, Self::All | Self::PostgreSql)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureReport {
    pub status: SnapshotStatus,
    pub interrupted: bool,
    pub final_path: Option<PathBuf>,
    pub partial_path: Option<PathBuf>,
}

pub fn capture(data_dir: &Path) -> Result<CaptureReport, ArtifactError> {
    capture_with_cancellation(data_dir, &CancelFlag::new(), DomainSelection::All, None)
}

/// Captures a snapshot into `data_dir`, honoring a cooperative cancellation
/// flag, domain selection, and optional PostgreSQL database target.
pub fn capture_with_cancellation(
    data_dir: &Path,
    cancel: &CancelFlag,
    domains: DomainSelection,
    dbname: Option<&str>,
) -> Result<CaptureReport, ArtifactError> {
    let hostname = linux::collector_hostname();
    let sanitized = sanitize::sanitize_hostname(&hostname);
    let base_name = artifact::format_base_name(&sanitized, now_utc());

    let snapshot_id = Uuid::new_v4();
    let started_at_utc = now_utc();
    let writer =
        artifact::PartialWriter::create(data_dir, &base_name, snapshot_id, started_at_utc)?;

    run_collection(writer, cancel, domains, dbname)
}

fn run_collection(
    mut writer: artifact::PartialWriter,
    cancel: &CancelFlag,
    domains: DomainSelection,
    dbname: Option<&str>,
) -> Result<CaptureReport, ArtifactError> {
    let partial_path = writer.partial_path().to_path_buf();

    if cancel.is_cancelled() {
        return Ok(interrupted_report(partial_path));
    }

    let mut has_failures = false;

    if domains.linux_enabled() {
        let (identity, id_outcomes) = linux::collect_identity_isolated();
        if id_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_identity(&identity, &id_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (compute, compute_outcomes) = linux::collect_compute_isolated();
        if compute_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_compute(&compute, &compute_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (memory, memory_outcomes) = linux::collect_memory_isolated();
        if memory_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_memory(&memory, &memory_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (pressure, pressure_outcomes) = linux::collect_pressure_isolated();
        if pressure_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_pressure(&pressure, &pressure_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (storage, storage_outcomes) = linux::collect_storage_isolated();
        if storage_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_storage(&storage, &storage_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (limits, limits_outcomes) = linux::collect_limits_isolated();
        if limits_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_limits(&limits, &limits_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (cgroups, cgroups_outcomes) = linux::collect_cgroups_isolated();
        if cgroups_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_cgroups(&cgroups, &cgroups_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        let (kernel, kernel_outcomes) = linux::collect_kernel_isolated();
        if kernel_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_kernel(&kernel, &kernel_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }
    }

    if domains.postgresql_enabled() {
        let (pg_inventory, pg_outcomes) = postgres::collect_target_inventory_isolated(dbname);
        if pg_outcomes
            .iter()
            .any(|o| o.status.makes_snapshot_partial())
        {
            has_failures = true;
        }
        writer.write_postgres(&pg_inventory, &pg_outcomes)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }
    }

    if cancel.is_cancelled() {
        return Ok(interrupted_report(partial_path));
    }

    let finished_at_utc = now_utc();
    let status = if has_failures {
        SnapshotStatus::Partial
    } else {
        SnapshotStatus::Complete
    };

    let final_path = writer.finalize(status, finished_at_utc)?;

    Ok(CaptureReport {
        status,
        interrupted: false,
        final_path: Some(final_path),
        partial_path: None,
    })
}

fn interrupted_report(partial_path: PathBuf) -> CaptureReport {
    CaptureReport {
        status: SnapshotStatus::Partial,
        interrupted: true,
        final_path: None,
        partial_path: Some(partial_path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_selection_all_enables_both() {
        assert!(DomainSelection::All.linux_enabled());
        assert!(DomainSelection::All.postgresql_enabled());
    }

    #[test]
    fn domain_selection_linux_disables_postgresql() {
        assert!(DomainSelection::Linux.linux_enabled());
        assert!(!DomainSelection::Linux.postgresql_enabled());
    }

    #[test]
    fn domain_selection_postgresql_disables_linux() {
        assert!(!DomainSelection::PostgreSql.linux_enabled());
        assert!(DomainSelection::PostgreSql.postgresql_enabled());
    }
}
