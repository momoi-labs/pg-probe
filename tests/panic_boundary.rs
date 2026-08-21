//! End-to-end proof that collector panics stay safe and diagnosable.

mod security;

use security::run_cli;

#[test]
fn collector_panic_hides_payload_and_records_failure() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().to_string_lossy().into_owned();
    let output = run_cli(
        &["-D", &data_dir, "--domain", "linux"],
        &[("PG_PROBE_FORCE_PANIC_LINUX_IDENTITY", "1")],
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("PG_PROBE_FORCE_PANIC_LINUX_IDENTITY"));
    assert!(!stderr.contains("panicked at"));
    assert!(!stderr.contains("stack backtrace"));
    assert!(stderr.contains(pg_probe::panic_guard::SAFE_PANIC_NOTICE));

    let artifact = security::sole_gzip_artifact(directory.path());
    let sqlite = directory.path().join("panic.sqlite");
    std::fs::write(&sqlite, security::decompressed_artifact_bytes(&artifact)).unwrap();
    let connection =
        rusqlite::Connection::open_with_flags(sqlite, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let outcome: (String, Option<String>) = connection
        .query_row(
            "SELECT status, error_code FROM collector_outcome \
             WHERE source = 'linux.identity'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(outcome.0, "io_error");
    assert_eq!(outcome.1.as_deref(), Some("collector_panicked"));
}
