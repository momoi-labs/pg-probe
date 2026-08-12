use std::fs;
use std::io::Read;

use assert_cmd::cargo::cargo_bin_cmd;
use flate2::read::GzDecoder;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn snapshot_command_publishes_a_valid_minimum_artifact() {
    let directory = tempdir().unwrap();
    let artifact = directory.path().join("snapshot.sqlite.gz");
    let sqlite = directory.path().join("snapshot.sqlite");
    let canary = "postgres://private-user:private-password@example.test/database";

    cargo_bin_cmd!("pg-probe")
        .args(["snapshot", "--output"])
        .arg(&artifact)
        .env("PGPASSWORD", canary)
        .assert()
        .success()
        .stderr(predicate::str::contains(canary).not());

    let mut decoder = GzDecoder::new(fs::File::open(&artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    assert!(!bytes
        .windows(canary.len())
        .any(|window| window == canary.as_bytes()));
    fs::write(&sqlite, bytes).unwrap();

    let connection =
        Connection::open_with_flags(&sqlite, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT status FROM snapshot", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "complete"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM collector_outcome", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        4
    );
    let hostname: String = connection
        .query_row("SELECT hostname FROM linux_identity", [], |row| row.get(0))
        .unwrap();
    assert!(!hostname.is_empty());
}

#[test]
fn snapshot_command_does_not_replace_an_existing_artifact() {
    let directory = tempdir().unwrap();
    let artifact = directory.path().join("snapshot.sqlite.gz");
    fs::write(&artifact, b"existing artifact").unwrap();

    cargo_bin_cmd!("pg-probe")
        .args(["snapshot", "--output"])
        .arg(&artifact)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the output artifact already exists",
        ));

    assert_eq!(fs::read(&artifact).unwrap(), b"existing artifact");
}
