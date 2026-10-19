//! PostgreSQL target, inventory, and statistics collector.
//!
//! Issue #16 captures the bounded set of facts that identify the PostgreSQL
//! cluster, collects a small allowlist of runtime settings, walks the
//! database inventory (with sizes when readable), and visits every
//! connectable database to record installed extensions.
//!
//! Issue #17 extends the collector with reset-aware cumulative statistics
//! (per-database activity, standby conflicts, WAL, bgwriter, checkpointer,
//! archiver, I/O, SLRU) and privacy-preserving workload aggregates
//! (activity, locks, replication, slots). Each subcollector is capability-
//! gated: unavailable version-specific sources are recorded as unsupported
//! rather than failed, and permission or query failures remain isolated so
//! they don't make an otherwise complete artifact partial.

mod connection;
mod databases;
mod extensions;
mod sanitize;
mod settings;
mod settings_allowlist;
pub(crate) mod stats;
mod target;

pub use connection::{
    AdapterError, AdapterOutcomeStatus, Client, ConnectedClient, ConnectionParams,
    EndpointIdentity, EndpointKind,
};
pub use databases::{DatabaseEntry, ExtensionVisitStatus};
pub use extensions::ExtensionEntry;
pub use settings::Setting;
pub use stats::{
    ActivityAggregate, ArchiverStats, BgWriterStats, DatabaseStats, IoStatEntry, LockAggregate,
    PostgresStats, ReplicationAggregate, SlotAggregate, SlruStatEntry, StandbyConflictStats,
    WalStats,
};
pub use target::Target;

use std::cell::RefCell;

use crate::collector::{Collector, Sink};
use connection::parse_text_i32;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};

/// The complete observation produced by the PostgreSQL target and inventory
/// collector for one snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetInventory {
    pub target: Option<Target>,
    pub settings: Vec<Setting>,
    pub databases: Vec<DatabaseEntry>,
    pub extensions: Vec<ExtensionEntry>,
    pub stats: PostgresStats,
}

/// Collector for the PostgreSQL target, inventory, and statistics.
pub struct PostgresCollector {
    params: ConnectionParams,
}

impl PostgresCollector {
    pub fn new(params: ConnectionParams) -> Self {
        Self { params }
    }
}

impl Collector for PostgresCollector {
    type Output = TargetInventory;

    fn source(&self) -> &'static str {
        "postgres"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        collect_target_inventory(&self.params)
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_postgres(output, outcomes)
    }
}

/// Collects the variant PostgreSQL statistics for samples after the first.
///
/// The first sample runs [`PostgresCollector`], which also reads the target,
/// settings, database sizes and extensions. Those are invariant, so later
/// samples skip them. The sampler opens one connection on its first sample and
/// keeps it for the whole session instead of opening one per sample. If that
/// first open fails, the next sample tries again.
pub struct PostgresSampler {
    params: ConnectionParams,
    session: RefCell<Option<(ConnectedClient, i32)>>,
}

impl PostgresSampler {
    pub fn new(params: ConnectionParams) -> Self {
        Self {
            params,
            session: RefCell::new(None),
        }
    }
}

const VERSION_QUERY: &str = "SELECT current_setting('server_version_num')";

impl Collector for PostgresSampler {
    type Output = TargetInventory;

    fn source(&self) -> &'static str {
        "postgres"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        let mut outcomes = Vec::new();
        let mut inventory = TargetInventory::default();
        let mut session = self.session.borrow_mut();

        if session.is_none() {
            let source = "postgres.connection";
            let Ok(client) = ConnectedClient::open(&self.params, source, &mut outcomes) else {
                return (inventory, outcomes);
            };
            let version = client
                .exec_bounded_row(VERSION_QUERY, source, &mut outcomes)
                .ok()
                .and_then(|row| row.first().and_then(|raw| parse_text_i32(raw).ok()));
            let Some(version) = version else {
                return (inventory, outcomes);
            };
            *session = Some((client, version));
        }

        let (client, version) = session.as_ref().expect("session opened above");
        let (pg_stats, stats_outcomes) = stats::collect_postgres_stats_isolated(client, *version);
        outcomes.extend(stats_outcomes);
        inventory.stats = pg_stats;
        (inventory, outcomes)
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_postgres(output, outcomes)
    }
}

fn collect_target_inventory(params: &ConnectionParams) -> (TargetInventory, Vec<ComponentOutcome>) {
    let mut outcomes = Vec::new();
    let mut inventory = TargetInventory::default();

    let connection_source = "postgres.connection";
    let client = match ConnectedClient::open(params, connection_source, &mut outcomes) {
        Ok(client) => client,
        Err(_) => {
            return (inventory, outcomes);
        }
    };

    let target_source = "postgres.target";
    let started_at = now_utc();
    match target::collect_target(&client, &mut outcomes) {
        Ok(target) => {
            outcomes.push(ComponentOutcome::collected(
                target_source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            inventory.target = Some(target);
        }
        Err(_) => {
            outcomes.push(
                ComponentOutcome::failed(
                    target_source,
                    CollectionStatus::ParseError,
                    "target_failed",
                    None,
                )
                .with_timing(started_at, now_utc()),
            );
        }
    }

    let settings_source = "postgres.settings";
    let started_at = now_utc();
    let server_version_num = inventory.target.as_ref().map(|t| t.server_version_num);
    match settings::collect_settings(&client, server_version_num, &mut outcomes) {
        Ok(collected) => {
            outcomes.push(ComponentOutcome::collected(
                settings_source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            inventory.settings = collected;
        }
        Err(_) => {
            outcomes.push(ComponentOutcome::failed(
                settings_source,
                CollectionStatus::ParseError,
                "settings_failed",
                None,
            ));
        }
    }

    let databases_source = "postgres.databases";
    let started_at = now_utc();
    match databases::collect_databases(&client, &mut outcomes) {
        Ok(collected) => {
            outcomes.push(ComponentOutcome::collected(
                databases_source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            inventory.databases = collected;
        }
        Err(_) => {
            outcomes.push(ComponentOutcome::failed(
                databases_source,
                CollectionStatus::ParseError,
                "databases_failed",
                None,
            ));
        }
    }

    let mut extensions = Vec::new();
    for database in inventory.databases.iter_mut() {
        let visit_source = "postgres.extensions.visit";
        let started_at = now_utc();
        if !database.can_connect {
            database.extension_visit_status = ExtensionVisitStatus::NotConnectable;
            outcomes.push(
                ComponentOutcome::failed(
                    visit_source,
                    CollectionStatus::Unsupported,
                    "database_not_connectable",
                    None,
                )
                .with_timing(started_at, now_utc()),
            );
            continue;
        }
        match extensions::visit_database(&client, database, &mut outcomes) {
            Some(found) => {
                outcomes.push(ComponentOutcome::collected(
                    visit_source,
                    started_at,
                    now_utc(),
                    now_utc(),
                ));
                extensions.extend(found);
            }
            None => {
                outcomes.push(
                    ComponentOutcome::failed(
                        visit_source,
                        CollectionStatus::IoError,
                        "extension_visit_failed",
                        None,
                    )
                    .with_timing(started_at, now_utc()),
                );
            }
        }
    }
    inventory.extensions = extensions;

    if let Some(target) = inventory.target.as_ref() {
        let server_version_num = target.server_version_num;
        let (pg_stats, stats_outcomes) =
            stats::collect_postgres_stats_isolated(&client, server_version_num);
        outcomes.extend(stats_outcomes);
        inventory.stats = pg_stats;
    }

    (inventory, outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_target_records_a_failed_outcome_without_panicking() {
        let (inventory, outcomes) = collect_target_inventory(&ConnectionParams {
            dbname: Some(
                "host=127.0.0.1 port=1 user=postgres dbname=postgres connect_timeout=1".to_owned(),
            ),
            ..ConnectionParams::default()
        });
        assert!(inventory.target.is_none());
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.status == CollectionStatus::IoError));
    }
}
