//! Canary injection through live PostgreSQL data and error surfaces.
#![allow(dead_code)]

mod common;
mod security;

use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use security::{
    assert_finalized_artifact, canary, full_scan, identifier_safe_canary, run_cli, CanarySet,
};

fn run_sql(cluster: &common::pg::PgCluster, database: &str, sql: &str) {
    let output = Command::new("psql")
        .args([
            "-h",
            &cluster.host,
            "-p",
            &cluster.port.to_string(),
            "-U",
            &cluster.admin_role,
            "-d",
            database,
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ])
        .env("PGPASSWORD", &cluster.bootstrap_password)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn capture(dsn: &str, canaries: &CanarySet) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().to_string_lossy().into_owned();
    let output = run_cli(&["-D", &data_dir, "-d", dsn, "--domain", "postgresql"], &[]);
    full_scan(directory.path(), &output.stdout, &output.stderr, canaries);
    assert_finalized_artifact(directory.path(), &output);
    directory
}

fn open_artifact(directory: &tempfile::TempDir, name: &str) -> Connection {
    let artifact = security::sole_gzip_artifact(directory.path());
    let sqlite = directory.path().join(name);
    std::fs::write(&sqlite, security::decompressed_artifact_bytes(&artifact)).unwrap();
    Connection::open_with_flags(sqlite, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

#[test]
fn live_postgres_canaries_do_not_cross_the_boundary() {
    let Some(cluster) = common::pg::cluster() else {
        return;
    };

    // Complete run: table data and active SQL text are visible to PostgreSQL
    // while pg-probe collects, but neither may enter the artifact.
    let table_data = canary("table-data");
    let query_text = canary("raw-source");
    run_sql(
        cluster,
        "probe_main",
        &format!(
            "CREATE TABLE pgprobe_canary(payload text); \
             INSERT INTO pgprobe_canary VALUES ('{table_data}');"
        ),
    );
    let mut query = Command::new("psql")
        .args([
            "-h",
            &cluster.host,
            "-p",
            &cluster.port.to_string(),
            "-U",
            &cluster.admin_role,
            "-d",
            &cluster.admin_database,
            "-c",
            &format!("SELECT pg_sleep(3) /* {query_text} */;"),
        ])
        .env("PGPASSWORD", &cluster.bootstrap_password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let complete_canaries = vec![("table-data", table_data), ("raw-source", query_text)];
    let complete = capture(&cluster.admin_connection_string(), &complete_canaries);
    let _ = query.wait();
    assert!(open_artifact(&complete, "complete.sqlite")
        .query_row(
            "SELECT count(*) FROM snapshot WHERE status = 'complete'",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .is_ok_and(|count| count == 1));

    // Material failure: a server error embeds the option name in its raw
    // message; only a structured connection failure may cross the adapter.
    let raw_error = identifier_safe_canary("raw-error");
    let options = format!("-c {raw_error}=1");
    let port = cluster.port.to_string();
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().to_string_lossy().into_owned();
    let output = run_cli(
        &["-D", &data_dir, "--domain", "postgresql"],
        &[
            ("PGHOST", &cluster.host),
            ("PGPORT", &port),
            ("PGUSER", &cluster.admin_role),
            ("PGPASSWORD", &cluster.bootstrap_password),
            ("PGDATABASE", &cluster.admin_database),
            ("PGOPTIONS", &options),
        ],
    );
    full_scan(
        directory.path(),
        &output.stdout,
        &output.stderr,
        &vec![("raw-error", raw_error)],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_finalized_artifact(directory.path(), &output);

    // Invalid identifiers are replaced and produce a structured outcome.
    let invalid = canary("invalid-database");
    let invalid_name = format!("bad\n{invalid}");
    run_sql(
        cluster,
        "postgres",
        &format!("CREATE DATABASE \"{invalid_name}\";"),
    );
    let invalid_canaries = vec![("invalid-database", invalid)];
    let rejected = capture(&cluster.admin_connection_string(), &invalid_canaries);
    let connection = open_artifact(&rejected, "invalid.sqlite");
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM postgres_database \
                 WHERE database_name = '<invalid>'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM collector_outcome \
                 WHERE source = 'postgres.databases.inventory' \
                 AND error_code = 'database_name_invalid'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    // Approved identifiers stay literal after validation; the accompanying
    // password remains prohibited.
    let role = canary("connected-user");
    let password = canary("role-password");
    run_sql(
        cluster,
        "postgres",
        &format!("CREATE ROLE \"{role}\" LOGIN PASSWORD '{password}';"),
    );
    run_sql(
        cluster,
        "postgres",
        "CREATE DATABASE pgprobe_canary_db OWNER probe_admin;",
    );
    run_sql(
        cluster,
        "postgres",
        &format!("GRANT CONNECT ON DATABASE pgprobe_canary_db TO \"{role}\";"),
    );
    let dsn = format!(
        "host={} port={} user={role} password={password} \
         dbname=pgprobe_canary_db connect_timeout=5",
        cluster.host, cluster.port
    );
    let literal = capture(&dsn, &vec![("role-password", password)]);
    let connected_user: String = open_artifact(&literal, "literal.sqlite")
        .query_row("SELECT connected_user FROM postgres_target", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(connected_user, role);
}
