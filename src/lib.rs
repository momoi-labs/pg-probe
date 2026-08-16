pub mod artifact;
pub mod cancel;
pub mod collector;
pub mod contract;
pub mod linux;
pub mod postgres;
pub mod sanitize;

use std::path::{Path, PathBuf};

pub use cancel::CancelFlag;
use contract::{now_utc, SnapshotStatus};
use uuid::Uuid;

use crate::collector::run_collector;
use crate::linux::{
    CgroupsCollector, ComputeCollector, IdentityCollector, KernelCollector, LimitsCollector,
    MemoryCollector, PressureCollector, StorageCollector,
};
use crate::postgres::PostgresCollector;

pub use artifact::ArtifactError;
pub use postgres::ConnectionParams;

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
    capture_with_cancellation(
        data_dir,
        &CancelFlag::new(),
        DomainSelection::All,
        &ConnectionParams::default(),
    )
}

/// Captures a snapshot into `data_dir`, honoring a cooperative cancellation
/// flag, domain selection, and optional PostgreSQL connection parameters.
pub fn capture_with_cancellation(
    data_dir: &Path,
    cancel: &CancelFlag,
    domains: DomainSelection,
    params: &ConnectionParams,
) -> Result<CaptureReport, ArtifactError> {
    let hostname = linux::collector_hostname();
    let sanitized = sanitize::sanitize_hostname(&hostname);
    let base_name = artifact::format_base_name(&sanitized, now_utc());

    let snapshot_id = Uuid::new_v4();
    let started_at_utc = now_utc();
    let writer =
        artifact::PartialWriter::create(data_dir, &base_name, snapshot_id, started_at_utc)?;

    run_collection(writer, cancel, domains, params)
}

fn run_collection(
    mut writer: artifact::PartialWriter,
    cancel: &CancelFlag,
    domains: DomainSelection,
    params: &ConnectionParams,
) -> Result<CaptureReport, ArtifactError> {
    let partial_path = writer.partial_path().to_path_buf();

    if cancel.is_cancelled() {
        return Ok(interrupted_report(partial_path));
    }

    let mut has_failures = false;

    if domains.linux_enabled() {
        has_failures |= run_collector(&IdentityCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&ComputeCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&MemoryCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&PressureCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&StorageCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&LimitsCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&CgroupsCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }

        has_failures |= run_collector(&KernelCollector, &mut writer)?;
        if cancel.is_cancelled() {
            return Ok(interrupted_report(partial_path));
        }
    }

    if domains.postgresql_enabled() {
        let collector = PostgresCollector::new(params.clone());
        has_failures |= run_collector(&collector, &mut writer)?;
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
