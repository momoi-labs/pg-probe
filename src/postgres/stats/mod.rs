//! PostgreSQL statistics and safe aggregates orchestrator.
//!
//! Issue #17 collects reset-aware cumulative statistics and privacy-preserving
//! workload aggregates from the PostgreSQL cluster. Each subcollector is
//! capability-gated: unavailable version-specific sources are recorded as
//! unsupported rather than failed, and permission or query failures remain
//! isolated so they don't make an otherwise complete artifact partial.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::ConnectedClient;

mod activity_aggregates;
mod archiver_stats;
mod bgwriter_stats;
pub(crate) mod capability;
mod database_stats;
mod io_stats;
mod lock_aggregates;
mod replication_aggregates;
mod slru_stats;
mod standby_conflicts;
mod wal_stats;

pub use activity_aggregates::{collect_activity_aggregates, ActivityAggregate};
pub use archiver_stats::{collect_archiver_stats, ArchiverStats};
pub use bgwriter_stats::{collect_bgwriter_stats, collect_checkpointer_stats, BgWriterStats};
pub use database_stats::{collect_database_stats, DatabaseStats};
pub use io_stats::{collect_io_stats, IoStatEntry};
pub use lock_aggregates::{collect_lock_aggregates, LockAggregate};
pub use replication_aggregates::{
    collect_replication_aggregates, collect_slot_aggregates, ReplicationAggregate, SlotAggregate,
};
pub use slru_stats::{collect_slru_stats, SlruStatEntry};
pub use standby_conflicts::{collect_standby_conflicts, StandbyConflictStats};
pub use wal_stats::{collect_wal_stats, WalStats};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostgresStats {
    pub database_stats: Vec<DatabaseStats>,
    pub standby_conflicts: Vec<StandbyConflictStats>,
    pub wal_stats: Option<WalStats>,
    pub bgwriter_stats: Option<BgWriterStats>,
    pub checkpointer_stats: Option<BgWriterStats>,
    pub archiver_stats: Option<ArchiverStats>,
    pub io_stats: Option<Vec<IoStatEntry>>,
    pub slru_stats: Option<Vec<SlruStatEntry>>,
    pub activity_aggregates: Vec<ActivityAggregate>,
    pub lock_aggregates: Vec<LockAggregate>,
    pub replication_aggregates: Vec<ReplicationAggregate>,
    pub slot_aggregates: Option<Vec<SlotAggregate>>,
}

type OptionalStatsCollector<T> = fn(
    &ConnectedClient,
    i32,
    &mut Vec<ComponentOutcome>,
)
    -> Result<Option<T>, crate::postgres::connection::AdapterError>;

pub fn collect_postgres_stats_isolated(
    client: &ConnectedClient,
    server_version_num: i32,
) -> (PostgresStats, Vec<ComponentOutcome>) {
    match catch_unwind(AssertUnwindSafe(|| {
        collect_postgres_stats(client, server_version_num)
    })) {
        Ok((stats, outcomes)) => (stats, outcomes),
        Err(_) => (
            PostgresStats::default(),
            vec![ComponentOutcome::failed(
                "postgres.stats",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        ),
    }
}

fn collect_postgres_stats(
    client: &ConnectedClient,
    server_version_num: i32,
) -> (PostgresStats, Vec<ComponentOutcome>) {
    let mut outcomes = Vec::new();
    let mut stats = PostgresStats::default();

    collect_sub(
        &mut stats.database_stats,
        "postgres.stats.database",
        "database_stats_failed",
        client,
        server_version_num,
        collect_database_stats,
        &mut outcomes,
    );

    collect_sub(
        &mut stats.standby_conflicts,
        "postgres.stats.standby_conflicts",
        "standby_conflicts_failed",
        client,
        server_version_num,
        collect_standby_conflicts,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.wal_stats,
        "postgres.stats.wal",
        "wal_stats_failed",
        client,
        server_version_num,
        collect_wal_stats,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.bgwriter_stats,
        "postgres.stats.bgwriter",
        "bgwriter_stats_failed",
        client,
        server_version_num,
        |c, v, o| collect_bgwriter_stats(c, v, o).map(Some),
        &mut outcomes,
    );

    collect_optional(
        &mut stats.checkpointer_stats,
        "postgres.stats.checkpointer",
        "checkpointer_stats_failed",
        client,
        server_version_num,
        collect_checkpointer_stats,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.archiver_stats,
        "postgres.stats.archiver",
        "archiver_stats_failed",
        client,
        server_version_num,
        collect_archiver_stats,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.io_stats,
        "postgres.stats.io",
        "io_stats_failed",
        client,
        server_version_num,
        collect_io_stats,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.slru_stats,
        "postgres.stats.slru",
        "slru_stats_failed",
        client,
        server_version_num,
        collect_slru_stats,
        &mut outcomes,
    );

    collect_sub(
        &mut stats.activity_aggregates,
        "postgres.aggregates.activity",
        "activity_aggregates_failed",
        client,
        server_version_num,
        collect_activity_aggregates,
        &mut outcomes,
    );

    collect_sub(
        &mut stats.lock_aggregates,
        "postgres.aggregates.locks",
        "lock_aggregates_failed",
        client,
        server_version_num,
        collect_lock_aggregates,
        &mut outcomes,
    );

    collect_sub(
        &mut stats.replication_aggregates,
        "postgres.aggregates.replication",
        "replication_aggregates_failed",
        client,
        server_version_num,
        collect_replication_aggregates,
        &mut outcomes,
    );

    collect_optional(
        &mut stats.slot_aggregates,
        "postgres.aggregates.slots",
        "slot_aggregates_failed",
        client,
        server_version_num,
        collect_slot_aggregates,
        &mut outcomes,
    );

    (stats, outcomes)
}

fn collect_sub<T>(
    target: &mut T,
    source: &'static str,
    error_code: &'static str,
    client: &ConnectedClient,
    server_version_num: i32,
    collector: fn(
        &ConnectedClient,
        i32,
        &mut Vec<ComponentOutcome>,
    ) -> Result<T, crate::postgres::connection::AdapterError>,
    outcomes: &mut Vec<ComponentOutcome>,
) where
    T: Default,
{
    let started_at = now_utc();
    match collector(client, server_version_num, outcomes) {
        Ok(collected) => {
            outcomes.push(ComponentOutcome::collected(
                source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            *target = collected;
        }
        Err(_) => {
            outcomes.push(ComponentOutcome::failed(
                source,
                CollectionStatus::IoError,
                error_code,
                None,
            ));
        }
    }
}

fn collect_optional<T>(
    target: &mut Option<T>,
    source: &'static str,
    error_code: &'static str,
    client: &ConnectedClient,
    server_version_num: i32,
    collector: OptionalStatsCollector<T>,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at = now_utc();
    match collector(client, server_version_num, outcomes) {
        Ok(collected) => {
            outcomes.push(ComponentOutcome::collected(
                source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            *target = collected;
        }
        Err(_) => {
            outcomes.push(ComponentOutcome::failed(
                source,
                CollectionStatus::IoError,
                error_code,
                None,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_stats_default_is_empty() {
        let stats = PostgresStats::default();
        assert!(stats.database_stats.is_empty());
        assert!(stats.standby_conflicts.is_empty());
        assert!(stats.wal_stats.is_none());
        assert!(stats.activity_aggregates.is_empty());
        assert!(stats.lock_aggregates.is_empty());
        assert!(stats.replication_aggregates.is_empty());
        assert!(stats.slot_aggregates.is_none());
    }
}
