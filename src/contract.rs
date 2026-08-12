use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

pub const SCHEMA_VERSION: i64 = 1;
pub const COLLECTOR_NAME: &str = "pg-probe";
pub const COLLECTOR_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotStatus {
    Complete,
    Partial,
}

impl SnapshotStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionStatus {
    Collected,
    Unsupported,
    PermissionDenied,
    ParseError,
    IoError,
    Raced,
}

impl CollectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Collected => "collected",
            Self::Unsupported => "unsupported",
            Self::PermissionDenied => "permission_denied",
            Self::ParseError => "parse_error",
            Self::IoError => "io_error",
            Self::Raced => "raced",
        }
    }

    pub const fn makes_snapshot_partial(self) -> bool {
        !matches!(self, Self::Collected | Self::Unsupported)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentOutcome {
    pub source: &'static str,
    pub started_at_utc: DateTime<Utc>,
    pub finished_at_utc: DateTime<Utc>,
    pub status: CollectionStatus,
    pub error_code: Option<&'static str>,
    pub os_error_code: Option<i32>,
    pub observed_at_utc: Option<DateTime<Utc>>,
}

impl ComponentOutcome {
    pub fn collected(
        source: &'static str,
        started_at_utc: DateTime<Utc>,
        finished_at_utc: DateTime<Utc>,
        observed_at_utc: DateTime<Utc>,
    ) -> Self {
        Self {
            source,
            started_at_utc,
            finished_at_utc,
            status: CollectionStatus::Collected,
            error_code: None,
            os_error_code: None,
            observed_at_utc: Some(observed_at_utc),
        }
    }

    pub fn failed(
        source: &'static str,
        status: CollectionStatus,
        error_code: &'static str,
        os_error_code: Option<i32>,
    ) -> Self {
        let timestamp = now_utc();
        Self {
            source,
            started_at_utc: timestamp,
            finished_at_utc: timestamp,
            status,
            error_code: Some(error_code),
            os_error_code,
            observed_at_utc: None,
        }
    }

    pub fn with_timing(
        mut self,
        started_at_utc: DateTime<Utc>,
        finished_at_utc: DateTime<Utc>,
    ) -> Self {
        self.started_at_utc = started_at_utc;
        self.finished_at_utc = finished_at_utc;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelIdentity {
    pub hostname: String,
    pub sysname: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DistributionIdentity {
    pub id: Option<String>,
    pub version_id: Option<String>,
    pub pretty_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockParameters {
    pub page_size_bytes: u64,
    pub ticks_per_second: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinuxIdentity {
    pub kernel: Option<KernelIdentity>,
    pub distribution: Option<DistributionIdentity>,
    pub clock: Option<ClockParameters>,
    pub uptime_microseconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub snapshot_id: Uuid,
    pub started_at_utc: DateTime<Utc>,
    pub finished_at_utc: DateTime<Utc>,
    pub status: SnapshotStatus,
    pub identity: LinuxIdentity,
    pub outcomes: Vec<ComponentOutcome>,
}

impl Snapshot {
    pub fn new(
        snapshot_id: Uuid,
        started_at_utc: DateTime<Utc>,
        finished_at_utc: DateTime<Utc>,
        identity: LinuxIdentity,
        outcomes: Vec<ComponentOutcome>,
    ) -> Self {
        let status = if outcomes
            .iter()
            .any(|outcome| outcome.status.makes_snapshot_partial())
        {
            SnapshotStatus::Partial
        } else {
            SnapshotStatus::Complete
        };

        Self {
            snapshot_id,
            started_at_utc,
            finished_at_utc,
            status,
            identity,
            outcomes,
        }
    }
}

pub fn now_utc() -> DateTime<Utc> {
    let micros = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX),
        Err(before_epoch) => {
            i64::try_from(before_epoch.duration().as_micros()).map_or(i64::MIN, |micros| -micros)
        }
    };
    DateTime::from_timestamp_micros(micros).unwrap_or(DateTime::UNIX_EPOCH)
}

pub fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_component_does_not_make_snapshot_partial() {
        let now = now_utc();
        let snapshot = Snapshot::new(
            Uuid::new_v4(),
            now,
            now,
            LinuxIdentity::default(),
            vec![ComponentOutcome::failed(
                "linux.optional",
                CollectionStatus::Unsupported,
                "not_available",
                None,
            )],
        );

        assert_eq!(snapshot.status, SnapshotStatus::Complete);
    }

    #[test]
    fn errors_make_snapshot_partial() {
        let now = now_utc();
        let snapshot = Snapshot::new(
            Uuid::new_v4(),
            now,
            now,
            LinuxIdentity::default(),
            vec![ComponentOutcome::failed(
                "proc.uptime",
                CollectionStatus::ParseError,
                "invalid_uptime",
                None,
            )],
        );

        assert_eq!(snapshot.status, SnapshotStatus::Partial);
    }
}
