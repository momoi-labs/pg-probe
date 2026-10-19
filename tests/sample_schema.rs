use rusqlite::Connection;

const VARIANT: &[&str] = &[
    "linux_cpu_time",
    "linux_scheduler",
    "linux_memory",
    "linux_vm_event",
    "linux_psi",
    "linux_block_io",
    "linux_network_io",
    "linux_cgroup_usage",
    "linux_cgroup_event",
    "linux_cgroup_io",
    "postgres_database_stats",
    "postgres_standby_conflicts",
    "postgres_wal_stats",
    "postgres_wal_position",
    "postgres_bgwriter_stats",
    "postgres_checkpointer_stats",
    "postgres_archiver_stats",
    "postgres_io_stats",
    "postgres_slru_stats",
    "postgres_activity_aggregates",
    "postgres_lock_aggregates",
    "postgres_replication_aggregates",
    "postgres_slot_aggregates",
];

const INVARIANT: &[&str] = &[
    "linux_identity",
    "linux_cpu_topology",
    "linux_numa_node",
    "linux_cpu_set",
    "linux_mount",
    "linux_block_device",
    "linux_resource_limit",
    "linux_cgroup",
    "linux_kernel_vm",
    "linux_kernel_ipc",
    "linux_thp",
    "postgres_target",
    "postgres_setting",
    "postgres_database",
    "postgres_extension",
];

fn schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(include_str!("../schema/migrations/V1__initial_schema.sql"))
        .unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
    connection
}

fn key_columns(connection: &Connection, table: &str) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")
        .unwrap();
    statement
        .query_map([table], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn columns(connection: &Connection, table: &str) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap();
    statement
        .query_map([table], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn every_variant_key_includes_the_sample_and_invariant_tables_have_none() {
    let connection = schema();
    for table in VARIANT {
        let key = key_columns(&connection, table);
        assert!(
            key.iter().any(|column| column == "sample_id"),
            "{table} key {key:?} lacks sample_id"
        );
    }
    for table in INVARIANT {
        assert!(
            !columns(&connection, table)
                .iter()
                .any(|column| column == "sample_id"),
            "{table} is invariant but has sample_id"
        );
    }
}

#[test]
fn two_samples_fit_in_one_artifact() {
    let connection = schema();
    connection
        .execute_batch(
            "
            INSERT INTO snapshot VALUES ('s', 2, 'pg-probe', 'test',
                '2026-08-19T12:00:00Z', '2026-08-19T12:00:20Z', 'complete');
            INSERT INTO sample VALUES ('s', 1, '2026-08-19T12:00:00Z');
            INSERT INTO sample VALUES ('s', 2, '2026-08-19T12:00:10Z');
            ",
        )
        .unwrap();

    for sample in [1, 2] {
        connection
            .execute_batch(&format!(
                "
                INSERT INTO linux_scheduler (snapshot_id, sample_id, collected_at, load_avg_1m)
                    VALUES ('s', {sample}, 't', 1.5);
                INSERT INTO linux_cpu_time VALUES ('s', {sample}, 't', 0, 'user', 100);
                INSERT INTO linux_memory VALUES ('s', {sample}, 't', 'MemTotal', 1024, 'bytes');
                INSERT INTO linux_cgroup_usage (snapshot_id, sample_id, collected_at, pids_current)
                    VALUES ('s', {sample}, 't', 12);
                INSERT INTO postgres_database_stats (
                    snapshot_id, sample_id, collected_at, database_oid, database_name,
                    xact_commit, xact_rollback, blks_read, blks_hit, tup_returned,
                    tup_fetched, tup_inserted, tup_updated, tup_deleted, conflicts,
                    temp_files, temp_bytes, deadlocks
                ) VALUES ('s', {sample}, 't', 5, 'postgres', 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0);
                INSERT INTO postgres_activity_aggregates
                    VALUES ('s', {sample}, 't', 'postgres', 'client backend', 'idle', NULL, 3, 10);
                "
            ))
            .unwrap();
    }

    let violations: i64 = connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM linux_scheduler WHERE snapshot_id = 's'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2);
}

#[test]
fn a_variant_row_needs_its_sample() {
    let connection = schema();
    connection
        .execute_batch(
            "INSERT INTO snapshot VALUES ('s', 2, 'pg-probe', 'test',
                '2026-08-19T12:00:00Z', '2026-08-19T12:00:20Z', 'complete');",
        )
        .unwrap();
    let orphan = connection.execute(
        "INSERT INTO linux_memory VALUES ('s', 1, 't', 'MemTotal', 1024, 'bytes')",
        [],
    );
    assert!(orphan.is_err(), "a row without its sample must be rejected");
}
