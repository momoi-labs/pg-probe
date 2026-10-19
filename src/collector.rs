//! The `Collector` and `Sink` seams that separate observation producers
//! from the transactional SQLite writer.
//!
//! Panic isolation lives here, applied uniformly by [`run_collector`]. A
//! collector never wraps itself in `catch_unwind`; it implements
//! [`Collector::collect`] as a plain function, and the seam converts a panic
//! into one failed [`ComponentOutcome`] carrying the collector's
//! [`Collector::source`] name.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::contract::{CollectionStatus, ComponentOutcome, LinuxIdentity};
use crate::linux::{
    CgroupObservations, ComputeObservations, KernelObservations, LimitsObservations,
    MemoryObservations, NetworkObservations, PressureObservations, StorageObservations,
};
use crate::postgres::TargetInventory;

/// A component that produces one typed batch of observations plus the
/// structured outcomes describing how each source within it ended.
///
/// Implementations run collection without any panic isolation. The seam applies
/// it uniformly in [`run_collector`], so every collector gets it and none
/// duplicates the `catch_unwind` machinery.
pub trait Collector {
    /// The typed observation batch this collector produces.
    type Output: Default;

    /// The structured source name recorded when this collector panics.
    fn source(&self) -> &'static str;

    /// Runs the collection. Must not apply its own panic isolation.
    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>);

    /// Writes this collector's observations to `sink` in one transaction.
    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error>;
}

/// A transactional destination for typed collector results.
///
/// The SQLite writer is the only implementation today. Each `write_*` method
/// commits one collector's observations atomically.
pub trait Sink {
    type Error;

    fn write_identity(
        &mut self,
        identity: &LinuxIdentity,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_compute(
        &mut self,
        compute: &ComputeObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_memory(
        &mut self,
        memory: &MemoryObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_pressure(
        &mut self,
        pressure: &PressureObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_storage(
        &mut self,
        storage: &StorageObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_limits(
        &mut self,
        limits: &LimitsObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_cgroups(
        &mut self,
        cgroups: &CgroupObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_kernel(
        &mut self,
        kernel: &KernelObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_postgres(
        &mut self,
        inventory: &TargetInventory,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;

    fn write_network(
        &mut self,
        network: &NetworkObservations,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), Self::Error>;
}

/// Runs one collector, isolating any panic to that collector, writing its
/// result to `sink`, and reporting whether any outcome made the snapshot
/// partial.
pub fn run_collector<C, S>(collector: &C, sink: &mut S) -> Result<bool, S::Error>
where
    C: Collector,
    S: Sink,
{
    let (output, outcomes) = match catch_unwind(AssertUnwindSafe(|| collector.collect())) {
        Ok(result) => result,
        Err(_) => (
            C::Output::default(),
            vec![ComponentOutcome::failed(
                collector.source(),
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        ),
    };

    log_outcomes(&outcomes);
    collector.write(sink, &output, &outcomes)?;

    Ok(outcomes
        .iter()
        .any(|outcome| outcome.status.makes_snapshot_partial()))
}

/// Records one structured event per collector outcome, mapping each status to a
/// level so a subscriber can colorize it (collected → info, unsupported → warn,
/// failure → error). Failure and unsupported events also carry the structured
/// `error_code` (and the OS error number when captured) so an operator can see
/// why a collector did not succeed.
///
/// These events are emitted only while info is enabled: the binary prints them
/// one per line under `--verbose`, summarizes them in its default progress
/// view, and leaves `--quiet` runs without them.
fn log_outcomes(outcomes: &[ComponentOutcome]) {
    if !tracing::enabled!(tracing::Level::INFO) {
        return;
    }
    for outcome in outcomes {
        let duration = format_duration(outcome.finished_at_utc - outcome.started_at_utc);
        match outcome.status {
            CollectionStatus::Collected => tracing::info!(
                source = %outcome.source,
                status = %outcome.status.as_str(),
                duration = %duration,
                "collector outcome",
            ),
            CollectionStatus::Unsupported => tracing::warn!(
                source = %outcome.source,
                status = %outcome.status.as_str(),
                duration = %duration,
                error_code = %outcome.error_code.unwrap_or("unsupported"),
                "collector outcome",
            ),
            CollectionStatus::PermissionDenied
            | CollectionStatus::ParseError
            | CollectionStatus::IoError
            | CollectionStatus::Raced => tracing::error!(
                source = %outcome.source,
                status = %outcome.status.as_str(),
                duration = %duration,
                error_code = %outcome.error_code.unwrap_or("unknown"),
                os_error_code = %format_os_error(outcome.os_error_code),
                detail = outcome.detail.as_deref().unwrap_or("-"),
                "collector outcome",
            ),
        }
    }
}

fn format_os_error(code: Option<i32>) -> String {
    code.map_or_else(|| "-".to_string(), |value| value.to_string())
}

fn format_duration(delta: chrono::TimeDelta) -> String {
    match delta.num_microseconds() {
        Some(micros) if micros < 1_000 => format!("{micros}µs"),
        Some(micros) => format!("{:.1}ms", micros as f64 / 1_000.0),
        None => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that captures the last identity batch written to it.
    #[derive(Default)]
    struct RecordingSink {
        identity: Option<LinuxIdentity>,
        outcomes: Vec<ComponentOutcome>,
    }

    impl Sink for RecordingSink {
        type Error = ();

        fn write_identity(
            &mut self,
            identity: &LinuxIdentity,
            outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            self.identity = Some(identity.clone());
            self.outcomes = outcomes.to_vec();
            Ok(())
        }

        fn write_compute(
            &mut self,
            _compute: &ComputeObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_memory(
            &mut self,
            _memory: &MemoryObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_pressure(
            &mut self,
            _pressure: &PressureObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_storage(
            &mut self,
            _storage: &StorageObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_limits(
            &mut self,
            _limits: &LimitsObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_cgroups(
            &mut self,
            _cgroups: &CgroupObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_kernel(
            &mut self,
            _kernel: &KernelObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_postgres(
            &mut self,
            _inventory: &TargetInventory,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn write_network(
            &mut self,
            _network: &NetworkObservations,
            _outcomes: &[ComponentOutcome],
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    struct PanickingCollector;

    impl Collector for PanickingCollector {
        type Output = LinuxIdentity;

        fn source(&self) -> &'static str {
            "test.panicking"
        }

        fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
            panic!("boom");
        }

        fn write<S: Sink>(
            &self,
            sink: &mut S,
            output: &Self::Output,
            outcomes: &[ComponentOutcome],
        ) -> Result<(), S::Error> {
            sink.write_identity(output, outcomes)
        }
    }

    #[test]
    fn run_collector_isolates_a_panic_to_one_failed_outcome() {
        let mut sink = RecordingSink::default();
        let partial = run_collector(&PanickingCollector, &mut sink).unwrap();

        assert!(partial);
        assert_eq!(sink.identity, Some(LinuxIdentity::default()));
        assert_eq!(sink.outcomes.len(), 1);
        assert_eq!(sink.outcomes[0].source, "test.panicking");
        assert_eq!(sink.outcomes[0].status, CollectionStatus::IoError);
        assert_eq!(sink.outcomes[0].error_code, Some("collector_panicked"));
    }

    struct CompleteCollector;

    impl Collector for CompleteCollector {
        type Output = LinuxIdentity;

        fn source(&self) -> &'static str {
            "test.complete"
        }

        fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
            (LinuxIdentity::default(), Vec::new())
        }

        fn write<S: Sink>(
            &self,
            sink: &mut S,
            output: &Self::Output,
            outcomes: &[ComponentOutcome],
        ) -> Result<(), S::Error> {
            sink.write_identity(output, outcomes)
        }
    }

    #[test]
    fn run_collector_reports_complete_when_no_outcome_is_partial() {
        let mut sink = RecordingSink::default();
        let partial = run_collector(&CompleteCollector, &mut sink).unwrap();
        assert!(!partial);
    }
}
