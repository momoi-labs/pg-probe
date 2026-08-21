//! Enumerates the production collectors on success and material-failure paths.
#![allow(dead_code)]

mod common;

use pg_probe::collector::Collector;
use pg_probe::contract::CollectionStatus;
use pg_probe::linux::{
    CgroupsCollector, ComputeCollector, IdentityCollector, KernelCollector, LimitsCollector,
    MemoryCollector, PressureCollector, StorageCollector,
};
use pg_probe::postgres::{ConnectionParams, PostgresCollector};

const POSTGRES_SOURCES: &[&str] = &[
    "postgres.connection",
    "postgres.target",
    "postgres.settings",
    "postgres.databases.inventory",
    "postgres.databases",
    "postgres.extensions",
    "postgres.stats.database",
    "postgres.stats.standby_conflicts",
    "postgres.stats.wal",
    "postgres.stats.bgwriter",
    "postgres.stats.checkpointer",
    "postgres.stats.archiver",
    "postgres.stats.io",
    "postgres.stats.slru",
    "postgres.aggregates.activity",
    "postgres.aggregates.locks",
    "postgres.aggregates.replication",
    "postgres.aggregates.slots",
];

#[test]
fn every_linux_collector_reports_an_outcome() {
    let outcomes = [
        ("identity", IdentityCollector.collect().1.len()),
        ("compute", ComputeCollector.collect().1.len()),
        ("memory", MemoryCollector.collect().1.len()),
        ("pressure", PressureCollector.collect().1.len()),
        ("storage", StorageCollector.collect().1.len()),
        ("limits", LimitsCollector.collect().1.len()),
        ("cgroups", CgroupsCollector.collect().1.len()),
        ("kernel", KernelCollector.collect().1.len()),
    ];
    assert!(
        outcomes.iter().all(|(_, count)| *count > 0),
        "collector without an outcome: {outcomes:?}"
    );
}

#[test]
fn postgres_connection_failure_is_structured() {
    let collector = PostgresCollector::new(ConnectionParams {
        host: Some("127.0.0.1".into()),
        port: Some(1),
        username: Some("probe".into()),
        dbname: Some("probe".into()),
    });
    let (inventory, outcomes) = collector.collect();

    assert!(inventory.target.is_none());
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].source, "postgres.connection");
    assert_eq!(outcomes[0].status, CollectionStatus::IoError);
    assert_eq!(outcomes[0].error_code, Some("connection_failed"));
}

#[test]
fn postgres_success_runs_every_subcollector_and_preserves_identifiers() {
    let Some(cluster) = common::pg::cluster() else {
        return;
    };
    let collector = PostgresCollector::new(ConnectionParams {
        dbname: Some(cluster.admin_connection_string()),
        ..ConnectionParams::default()
    });
    let (inventory, outcomes) = collector.collect();
    let target = inventory.target.expect("expected PostgreSQL target");

    assert_eq!(target.endpoint.host, cluster.host);
    assert_eq!(target.endpoint.port, Some(cluster.port));
    assert_eq!(target.connected_user, cluster.admin_role);
    assert_eq!(target.initial_database, cluster.admin_database);
    for source in POSTGRES_SOURCES {
        assert!(
            outcomes.iter().any(|outcome| outcome.source == *source
                && outcome.status == CollectionStatus::Collected),
            "missing successful outcome for {source}"
        );
    }
}
