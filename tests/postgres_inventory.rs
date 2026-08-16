mod common;

use std::fs;
use std::io::Read;

use assert_cmd::cargo::cargo_bin_cmd;
use flate2::read::GzDecoder;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

const CANARY: &str = "probe-canary-very-unique-string-7f7c4b1a";

fn sole_gzip(dir: &std::path::Path) -> std::path::PathBuf {
    let gz_files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "gz"))
        .collect();
    assert_eq!(
        gz_files.len(),
        1,
        "expected exactly one .gz file in {:?}",
        dir
    );
    gz_files[0].clone()
}

#[test]
fn black_box_postgres_collection_publishes_target_settings_and_extensions() {
    let cluster = match common::pg::cluster() {
        Some(cluster) => cluster,
        None => return,
    };
    let directory = tempdir().unwrap();
    let sqlite = directory.path().join("snapshot.sqlite");

    let dsn = cluster.admin_connection_string();
    let assert = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--dbname")
        .arg(&dsn)
        .env("PGPASSWORD", CANARY)
        .assert();
    let output = assert.get_output().clone();
    eprintln!(
        "=== pg-probe stderr ===\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "=== pg-probe stdout ===\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!("=== exit === {:?}", output.status);
    if !output.status.success() {
        panic!("pg-probe exited with status {:?}", output.status);
    }

    let artifact = sole_gzip(directory.path());
    assert!(artifact.exists(), "artifact {:?} does not exist", artifact);

    let mut decoder = GzDecoder::new(fs::File::open(&artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    assert!(!bytes
        .windows(CANARY.len())
        .any(|window| window == CANARY.as_bytes()));
    fs::write(&sqlite, bytes).unwrap();

    let connection =
        Connection::open_with_flags(&sqlite, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let target_row: (String, String, String, i64, i64, i64) = connection
        .query_row(
            "SELECT endpoint_kind, endpoint, connected_user, server_version_num, block_size_bytes, has_pg_monitor
             FROM postgres_target",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(target_row.0, "tcp");
    assert!(!target_row.1.is_empty());
    assert_eq!(target_row.2, "probe_admin");
    assert!(target_row.3 > 0);
    assert!(target_row.4 > 0);
    assert_eq!(target_row.5, 1);

    let setting_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM postgres_setting WHERE snapshot_id IN
             (SELECT snapshot_id FROM snapshot)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(setting_count >= 10);

    let disallowed: i64 = connection
        .query_row(
            "SELECT count(*) FROM postgres_setting WHERE setting_name IN
             ('password_encryption', 'shared_preload_libraries', 'unix_socket_directories', 'listen_addresses')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(disallowed, 0);

    let database_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM postgres_database WHERE snapshot_id IN
             (SELECT snapshot_id FROM snapshot)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(database_count >= 4);

    let probe_main_size: Option<i64> = connection
        .query_row(
            "SELECT size_bytes FROM postgres_database WHERE database_name = 'probe_main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(probe_main_size.is_some());

    let extension_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM postgres_extension WHERE snapshot_id IN
             (SELECT snapshot_id FROM snapshot)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(extension_count >= 1);

    let view_rows: i64 = connection
        .query_row("SELECT count(*) FROM database_inventory", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(view_rows >= 4);
}

#[test]
fn black_box_postgres_collection_uses_standard_pg_environment() {
    let cluster = match common::pg::cluster() {
        Some(cluster) => cluster,
        None => return,
    };
    let directory = tempdir().unwrap();

    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .env("PGHOST", &cluster.host)
        .env("PGPORT", cluster.port.to_string())
        .env("PGDATABASE", &cluster.admin_database)
        .env("PGUSER", &cluster.admin_role)
        .env("PGPASSWORD", &cluster.bootstrap_password)
        .assert()
        .success();

    let artifact = sole_gzip(directory.path());
    let mut decoder = GzDecoder::new(fs::File::open(artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    let sqlite = directory.path().join("snapshot.sqlite");
    fs::write(&sqlite, bytes).unwrap();

    let connection =
        Connection::open_with_flags(&sqlite, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let target: (String, String) = connection
        .query_row(
            "SELECT connected_user, initial_database FROM postgres_target",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(target.0, "probe_admin");
    assert_eq!(target.1, "probe_main");
}

#[test]
fn black_box_postgres_collection_uses_psql_connection_flags() {
    let cluster = match common::pg::cluster() {
        Some(cluster) => cluster,
        None => return,
    };
    let directory = tempdir().unwrap();

    cargo_bin_cmd!("pg-probe")
        .arg("-D")
        .arg(directory.path())
        .arg("-h")
        .arg(&cluster.host)
        .arg("-p")
        .arg(cluster.port.to_string())
        .arg("-U")
        .arg(&cluster.admin_role)
        .arg("-d")
        .arg(&cluster.admin_database)
        .env("PGPASSWORD", &cluster.bootstrap_password)
        .assert()
        .success();

    let artifact = sole_gzip(directory.path());
    let mut decoder = GzDecoder::new(fs::File::open(artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    let sqlite = directory.path().join("snapshot.sqlite");
    fs::write(&sqlite, bytes).unwrap();

    let connection =
        Connection::open_with_flags(&sqlite, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let target: (String, String) = connection
        .query_row(
            "SELECT connected_user, initial_database FROM postgres_target",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(target.0, "probe_admin");
    assert_eq!(target.1, "probe_main");
}

#[test]
fn black_box_postgres_collection_recovers_from_connection_failure() {
    let directory = tempdir().unwrap();
    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--dbname")
        .arg("host=127.0.0.1 port=1 user=postgres dbname=postgres connect_timeout=1")
        .assert()
        .failure();
}

#[test]
fn black_box_postgres_collection_handles_permission_denied_collector() {
    let cluster = match common::pg::cluster() {
        Some(cluster) => cluster,
        None => return,
    };
    let directory = tempdir().unwrap();
    let dsn = cluster.reader_connection_string();
    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--dbname")
        .arg(&dsn)
        .assert()
        .failure();
}

#[test]
fn black_box_postgres_collection_handles_unavailable_database() {
    let cluster = match common::pg::cluster() {
        Some(cluster) => cluster,
        None => return,
    };
    let directory = tempdir().unwrap();
    let dsn = cluster.unavailable_connection_string();
    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--dbname")
        .arg(&dsn)
        .assert()
        .failure();
}

#[test]
fn black_box_postgres_collection_does_not_echo_dsn_password() {
    let directory = tempdir().unwrap();
    let dsn = "host=127.0.0.1 port=1 user=postgres password=probe-secret-secret dbname=postgres connect_timeout=1";
    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--dbname")
        .arg(dsn)
        .assert()
        .failure()
        .stderr(predicate::str::contains("probe-secret-secret").not());
}
