//! PostgreSQL target and inventory collector.
//!
//! Issue #16 captures the bounded set of facts that identify the PostgreSQL
//! cluster, collects a small allowlist of runtime settings, walks the
//! database inventory (with sizes when readable), and visits every
//! connectable database to record installed extensions. Each subcollector
//! is isolated: a failure somewhere leaves the rest working and records a
//! single sanitized failure outcome.

mod connection;
mod databases;
mod extensions;
mod sanitize;
mod settings;
mod settings_allowlist;
mod target;

pub use connection::{
    AdapterError, AdapterOutcomeStatus, Client, ConnectedClient, EndpointIdentity, EndpointKind,
};
pub use databases::{DatabaseEntry, ExtensionVisitStatus};
pub use extensions::ExtensionEntry;
pub use settings::Setting;
pub use target::Target;

use std::panic::{catch_unwind, AssertUnwindSafe};

use chrono::{DateTime, Utc};

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};

/// The complete observation produced by the PostgreSQL target and inventory
/// collector for one snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetInventory {
    pub target: Option<Target>,
    pub settings: Vec<Setting>,
    pub databases: Vec<DatabaseEntry>,
    pub extensions: Vec<ExtensionEntry>,
}

/// The panic-safe entry point used from the snapshot spine. A collector
/// panic is recorded as a single failed outcome; the rest of the snapshot is
/// unaffected.
pub fn collect_target_inventory_isolated(
    dsn_input: Option<&str>,
) -> (TargetInventory, Vec<ComponentOutcome>) {
    match catch_unwind(AssertUnwindSafe(|| collect_target_inventory(dsn_input))) {
        Ok((inventory, outcomes)) => (inventory, outcomes),
        Err(_) => (
            TargetInventory::default(),
            vec![ComponentOutcome::failed(
                "postgres",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        ),
    }
}

/// The timed variant used by the snapshot spine. Returns the same data plus
/// explicit start/finish timestamps so the artifact can record the bounded
/// window the inventory was captured in.
#[allow(clippy::type_complexity)]
pub fn collect_target_inventory_with_timing(
    dsn_input: Option<&str>,
) -> (
    TargetInventory,
    Vec<ComponentOutcome>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    let started_at = now_utc();
    let (inventory, outcomes) = collect_target_inventory_isolated(dsn_input);
    let finished_at = now_utc();
    (inventory, outcomes, Some(started_at), Some(finished_at))
}

pub fn collect_target_inventory(
    dsn_input: Option<&str>,
) -> (TargetInventory, Vec<ComponentOutcome>) {
    let mut outcomes = Vec::new();
    let mut inventory = TargetInventory::default();

    let connection_source = "postgres.connection";
    let client = match ConnectedClient::open(dsn_input, connection_source, &mut outcomes) {
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
    match settings::collect_settings(&client, &mut outcomes) {
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

    (inventory, outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_isolation_records_a_single_failed_outcome() {
        let (inventory, outcomes) = collect_target_inventory_isolated(None);
        assert!(inventory.target.is_none());
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, CollectionStatus::IoError);
    }

    #[test]
    fn unreachable_target_records_a_failed_outcome_without_panicking() {
        let (inventory, outcomes) = collect_target_inventory(Some(
            "host=127.0.0.1 port=1 user=postgres dbname=postgres connect_timeout=1",
        ));
        assert!(inventory.target.is_none());
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.status == CollectionStatus::IoError));
    }
}
