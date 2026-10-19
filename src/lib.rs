pub mod artifact;
pub mod cancel;
pub mod collector;
pub mod contract;
pub mod linux;
pub mod panic_guard;
pub mod postgres;
pub mod review;
pub mod sanitize;
pub mod top;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use cancel::CancelFlag;
use contract::{now_utc, SnapshotStatus};
use uuid::Uuid;

use crate::collector::run_collector;
use crate::linux::{
    CgroupsCollector, ComputeCollector, ComputeSampleCollector, IdentityCollector, KernelCollector,
    LimitsCollector, MemoryCollector, NetworkCollector, PressureCollector, StorageCollector,
    StorageSampleCollector,
};
use crate::postgres::{PostgresCollector, PostgresSampler};

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

/// A sampling session: how long a capture keeps taking samples, and how far
/// apart. Without one, a capture takes a single sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sampling {
    duration: Duration,
    interval: Duration,
}

impl Sampling {
    /// The shortest interval allowed. Sampling faster adds load to a server
    /// that may already be in trouble (principle 1).
    pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

    pub fn new(duration: Duration, interval: Duration) -> Result<Self, &'static str> {
        if interval < Self::MIN_INTERVAL {
            return Err("the interval must be at least 1 second");
        }
        if duration < interval {
            return Err("the duration must be at least one interval");
        }
        Ok(Self { duration, interval })
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
    capture_session(data_dir, cancel, domains, params, None)
}

/// Captures a snapshot like [`capture_with_cancellation`]. With `sampling`, it
/// then keeps taking samples of the variant observations until the duration
/// ends, all in the same artifact.
pub fn capture_session(
    data_dir: &Path,
    cancel: &CancelFlag,
    domains: DomainSelection,
    params: &ConnectionParams,
    sampling: Option<Sampling>,
) -> Result<CaptureReport, ArtifactError> {
    let hostname = linux::collector_hostname();
    let sanitized = sanitize::sanitize_hostname(&hostname);
    let base_name = artifact::format_base_name(&sanitized, now_utc());

    let snapshot_id = Uuid::new_v4();
    let started_at_utc = now_utc();
    let writer =
        artifact::PartialWriter::create(data_dir, &base_name, snapshot_id, started_at_utc)?;

    run_collection(writer, cancel, domains, params, sampling)
}

fn run_collection(
    mut writer: artifact::PartialWriter,
    cancel: &CancelFlag,
    domains: DomainSelection,
    params: &ConnectionParams,
    sampling: Option<Sampling>,
) -> Result<CaptureReport, ArtifactError> {
    let partial_path = writer.partial_path().to_path_buf();
    let session_start = Instant::now();

    if cancel.is_cancelled() {
        return Ok(interrupted_report(partial_path));
    }

    tracing::debug!(
        artifact = %partial_path.display(),
        duration_s = sampling.map(|s| s.duration.as_secs()),
        interval_s = sampling.map(|s| s.interval.as_secs()),
        "capture started",
    );
    tracing::debug!(sample = 1_u64, "sample started");

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

        has_failures |= run_collector(&NetworkCollector, &mut writer)?;
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
    tracing::debug!(sample = 1_u64, "sample finished");

    if let Some(sampling) = sampling {
        match run_samples(
            &mut writer,
            cancel,
            domains,
            params,
            sampling,
            session_start,
        )? {
            Some(failed) => has_failures |= failed,
            None => return Ok(interrupted_report(partial_path)),
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

/// Takes the samples after the first, one per interval from the session
/// start, until the duration ends. Only variant observations are read again.
/// A sample that runs past the next tick skips that tick rather than queueing
/// a burst. Returns whether any sample failed, or `None` when cancelled.
fn run_samples(
    writer: &mut artifact::PartialWriter,
    cancel: &CancelFlag,
    domains: DomainSelection,
    params: &ConnectionParams,
    sampling: Sampling,
    session_start: Instant,
) -> Result<Option<bool>, ArtifactError> {
    let postgres = domains
        .postgresql_enabled()
        .then(|| PostgresSampler::new(params.clone()));
    let mut has_failures = false;
    let mut due = sampling.interval;
    let mut sample = 1_u64;

    while due <= sampling.duration {
        if !sleep_until(session_start + due, cancel) {
            return Ok(None);
        }
        writer.begin_sample(now_utc())?;
        sample += 1;
        tracing::debug!(sample, "sample started");

        if domains.linux_enabled() {
            has_failures |= run_collector(&ComputeSampleCollector, writer)?;
            has_failures |= run_collector(&MemoryCollector, writer)?;
            has_failures |= run_collector(&PressureCollector, writer)?;
            has_failures |= run_collector(&StorageSampleCollector, writer)?;
            has_failures |= run_collector(&CgroupsCollector, writer)?;
            has_failures |= run_collector(&NetworkCollector, writer)?;
        }
        if let Some(postgres) = &postgres {
            has_failures |= run_collector(postgres, writer)?;
        }
        tracing::debug!(sample, "sample finished");
        if cancel.is_cancelled() {
            return Ok(None);
        }

        let elapsed = session_start.elapsed();
        due += sampling.interval;
        while due <= elapsed {
            due += sampling.interval;
        }
    }
    Ok(Some(has_failures))
}

/// Sleeps until `deadline`, waking every 200 ms to honor cancellation.
/// Returns false when cancelled.
fn sleep_until(deadline: Instant, cancel: &CancelFlag) -> bool {
    const STEP: Duration = Duration::from_millis(200);
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        std::thread::sleep((deadline - now).min(STEP));
    }
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
    fn sampling_rejects_an_interval_under_one_second() {
        assert!(Sampling::new(Duration::from_secs(60), Duration::from_millis(500)).is_err());
    }

    #[test]
    fn sampling_rejects_a_duration_shorter_than_the_interval() {
        assert!(Sampling::new(Duration::from_secs(5), Duration::from_secs(10)).is_err());
        assert!(Sampling::new(Duration::from_secs(10), Duration::from_secs(10)).is_ok());
    }

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
