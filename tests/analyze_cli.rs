use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin_cmd;
use flate2::write::GzEncoder;
use flate2::Compression;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

struct Server(Child);

// The managed test environment permits one local listening socket at a time.
static SERVER_TEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_PORT_OFFSET: AtomicU16 = AtomicU16::new(0);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn available_port() -> u16 {
    let base = 40_000 + (std::process::id() % 20_000) as u16;
    base + NEXT_PORT_OFFSET.fetch_add(1, Ordering::Relaxed)
}

fn seed_artifact(directory: &std::path::Path) -> String {
    let sqlite = directory.join("analyze-test.sqlite");
    let connection = Connection::open(&sqlite).unwrap();
    connection
        .execute_batch(include_str!("../schema/migrations/V1__initial_schema.sql"))
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO snapshot VALUES (
            'test-snapshot', 1, 'pg-probe', 'test',
            '2026-08-19T12:00:00Z', '2026-08-19T12:00:01Z', 'complete'
         );
         INSERT INTO linux_identity (
            snapshot_id, hostname, kernel_sysname, kernel_release, kernel_machine
         ) VALUES ('test-snapshot', 'test-host', 'Linux', '6.12.0', 'x86_64');
         INSERT INTO sample VALUES ('test-snapshot', 1, '2026-08-19T12:00:00Z');
         INSERT INTO sample VALUES ('test-snapshot', 2, '2026-08-19T12:00:10Z');
         INSERT INTO linux_memory VALUES
            ('test-snapshot', 1, '2026-08-19T12:00:00Z', 'MemFree', 1024, 'bytes'),
            ('test-snapshot', 2, '2026-08-19T12:00:10Z', 'MemFree', 2048, 'bytes');
         INSERT INTO collector_outcome VALUES (
            'test-snapshot', 'linux.identity', 0,
            '2026-08-19T12:00:00Z', '2026-08-19T12:00:01Z', 'collected',
            NULL, NULL, '2026-08-19T12:00:00Z', 'pg-probe', 'test'
         );",
        )
        .unwrap();
    drop(connection);

    let file_name = "analyze-test.sqlite.gz";
    let output = fs::File::create(directory.join(file_name)).unwrap();
    let mut encoder = GzEncoder::new(output, Compression::default());
    encoder.write_all(&fs::read(&sqlite).unwrap()).unwrap();
    encoder.finish().unwrap();
    fs::remove_file(sqlite).unwrap();
    file_name.to_owned()
}

fn start_server(directory: &std::path::Path) -> (Server, u16) {
    let port = available_port();
    let child = Command::new(env!("CARGO_BIN_EXE_pg-probe"))
        .arg("analyze")
        .arg("--data-directory")
        .arg(directory)
        .arg("--port")
        .arg(port.to_string())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "analyze server did not become ready"
        );
        thread::sleep(Duration::from_millis(25));
    }
    (Server(child), port)
}

fn get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

#[test]
fn analyze_serves_schema_listing_detail_rows_and_console() {
    let _server_test = SERVER_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempdir().unwrap();
    let file_name = seed_artifact(directory.path());
    let (_server, port) = start_server(directory.path());

    let (status, body) = get(port, "/api/schema");
    assert_eq!(status, 200);
    let schema: Value = serde_json::from_str(&body).unwrap();
    assert!(schema["tables"]
        .as_array()
        .unwrap()
        .iter()
        .any(|table| table["name"] == "snapshot"));

    let (status, body) = get(port, "/api/snapshots");
    assert_eq!(status, 200);
    let listing: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(listing["snapshots"][0]["snapshot_id"], "test-snapshot");
    assert_eq!(listing["snapshots"][0]["hostname"], "test-host");

    let (status, body) = get(port, &format!("/api/snapshots/{file_name}"));
    assert_eq!(status, 200);
    let detail: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["snapshot"]["status"], "complete");
    assert_eq!(detail["collector_outcomes"][0]["source"], "linux.identity");

    let (status, body) = get(
        port,
        &format!("/api/snapshots/{file_name}/tables/linux_identity"),
    );
    assert_eq!(status, 200);
    let rows: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows["rows"][0]["hostname"], "test-host");
    assert!(rows["columns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|column| column["name"] == "hostname"));

    let (status, body) = get(port, &format!("/api/snapshots/{file_name}/samples"));
    assert_eq!(status, 200);
    let samples: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(samples["samples"].as_array().unwrap().len(), 2);
    assert_eq!(
        samples["samples"][1]["taken_at_utc"],
        "2026-08-19T12:00:10Z"
    );

    let (status, body) = get(
        port,
        &format!("/api/snapshots/{file_name}/tables/linux_memory?sample_id=2"),
    );
    assert_eq!(status, 200);
    let rows: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
    assert_eq!(rows["rows"][0]["value_bytes"], 2048);
    assert_eq!(rows["truncated"], false);

    let (status, _) = get(
        port,
        &format!("/api/snapshots/{file_name}/tables/linux_memory?bogus=1"),
    );
    assert_eq!(status, 400, "a filter on an unknown column is rejected");

    let (status, body) = get(port, "/console/");
    assert_eq!(status, 200);
    assert!(body.contains("pg-probe review console"));
}

#[test]
fn analyze_returns_not_found_for_unknown_snapshot_and_table() {
    let _server_test = SERVER_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempdir().unwrap();
    let file_name = seed_artifact(directory.path());
    let (_server, port) = start_server(directory.path());

    assert_eq!(get(port, "/api/snapshots/bogus").0, 404);
    assert_eq!(
        get(port, &format!("/api/snapshots/{file_name}/tables/bogus")).0,
        404
    );
}

#[test]
fn capture_is_explicit_and_bare_invocation_shows_help_as_an_error() {
    cargo_bin_cmd!("pg-probe")
        .arg("capture")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--domain"));

    cargo_bin_cmd!("pg-probe")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Usage:"))
        .stderr(predicate::str::contains("capture"))
        .stderr(predicate::str::contains("analyze"));
}
