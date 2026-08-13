use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::cargo::cargo_bin_cmd;
use flate2::read::GzDecoder;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

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

fn assert_mode_0600(path: &std::path::Path) {
    let mode = fs::metadata(path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "file {} has mode {:o}, expected 0600",
        path.display(),
        mode & 0o777
    );
}

#[test]
fn help_flag_prints_usage() {
    cargo_bin_cmd!("pg-probe")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--data-directory"))
        .stdout(predicate::str::contains("--domain"))
        .stdout(predicate::str::contains("--quiet"))
        .stdout(predicate::str::contains("--verbose"));
}

#[test]
fn version_flag_prints_version() {
    cargo_bin_cmd!("pg-probe")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("pg-probe"));
}

#[test]
fn a_successful_run_publishes_a_valid_gzip_artifact() {
    let directory = tempdir().unwrap();
    let canary = "postgres://private-user:private-password@example.test/database";

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .env("PGPASSWORD", canary)
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );
    output.stderr(predicate::str::contains(canary).not());

    let artifact = sole_gzip(directory.path());
    assert_mode_0600(&artifact);

    let partials: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".sqlite.partial"))
        })
        .collect();
    assert!(partials.is_empty(), "partial files remain after success");

    let mut decoder = GzDecoder::new(fs::File::open(&artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    assert!(!bytes.windows(canary.len()).any(|w| w == canary.as_bytes()));

    let sqlite_path = directory.path().join("decompressed.sqlite");
    fs::write(&sqlite_path, bytes).unwrap();
    let conn =
        Connection::open_with_flags(&sqlite_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let hostname: String = conn
        .query_row("SELECT hostname FROM linux_identity", [], |row| row.get(0))
        .unwrap();
    assert!(!hostname.is_empty());
}

#[test]
fn stdout_contains_only_the_artifact_path() {
    let directory = tempdir().unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();

    let stdout_trimmed = stdout.trim();
    assert!(!stdout_trimmed.is_empty(), "stdout is empty");
    assert!(stdout_trimmed.ends_with(".sqlite.gz"));
    assert!(!stdout.contains("pg-probe:"));
    assert!(!stderr.contains(".sqlite.gz"));
}

#[test]
fn missing_data_directory_exits_2() {
    let directory = tempdir().unwrap();
    let missing = directory.path().join("does-not-exist");

    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(&missing)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("data directory does not exist"));
}

#[test]
fn invalid_domain_exits_2() {
    let directory = tempdir().unwrap();

    cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .args(["--domain", "bogus"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid --domain"));
}

#[test]
fn domain_linux_only_does_not_initialize_postgresql() {
    let directory = tempdir().unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .args(["--domain", "linux"])
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );

    let artifact = sole_gzip(directory.path());
    let mut decoder = GzDecoder::new(fs::File::open(&artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    let sqlite_path = directory.path().join("decompressed.sqlite");
    fs::write(&sqlite_path, bytes).unwrap();
    let conn =
        Connection::open_with_flags(&sqlite_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    let hostname: String = conn
        .query_row("SELECT hostname FROM linux_identity", [], |row| row.get(0))
        .unwrap();
    assert!(!hostname.is_empty());

    let count: i64 = conn
        .query_row("SELECT count(*) FROM collector_outcome", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(count > 0, "expected at least one collector outcome");
}

#[test]
fn domain_postgresql_only_does_not_initialize_linux() {
    let directory = tempdir().unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .args(["--domain", "postgresql"])
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );

    let artifact = sole_gzip(directory.path());
    let mut decoder = GzDecoder::new(fs::File::open(&artifact).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    let sqlite_path = directory.path().join("decompressed.sqlite");
    fs::write(&sqlite_path, bytes).unwrap();
    let conn =
        Connection::open_with_flags(&sqlite_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    let count: i64 = conn
        .query_row("SELECT count(*) FROM linux_identity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "linux_identity should be empty for postgresql-only"
    );

    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM collector_outcome WHERE source LIKE 'linux%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "no Linux collector outcomes expected for postgresql-only"
    );
}

#[test]
fn quiet_suppresses_stderr_warnings() {
    let directory = tempdir().unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .arg("--quiet")
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );

    let stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.is_empty(),
        "stderr should be empty with --quiet, got: {stderr}"
    );
}

#[test]
fn existing_partial_files_are_warned_about_and_left_untouched() {
    let directory = tempdir().unwrap();
    let partial = directory
        .path()
        .join("oldhost-2026-01-01-00h00m00s.sqlite.partial");
    fs::write(&partial, b"pre-existing partial data").unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("existing .sqlite.partial"));
    assert!(stderr.contains("--cleanup"));

    assert_eq!(fs::read(&partial).unwrap(), b"pre-existing partial data");
}

#[test]
fn filename_uses_sanitized_hostname_and_utc_timestamp() {
    let directory = tempdir().unwrap();

    let output = cargo_bin_cmd!("pg-probe")
        .arg("--data-directory")
        .arg(directory.path())
        .assert();

    let code = output.get_output().status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "expected exit 0 (complete) or 1 (partial), got {code}"
    );

    let artifact = sole_gzip(directory.path());
    let name = artifact.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.ends_with(".sqlite.gz"));
    let stem = name.strip_suffix(".sqlite.gz").unwrap();
    assert!(
        regex_like_matches(stem),
        "filename stem '{stem}' does not match expected pattern"
    );
}

fn regex_like_matches(s: &str) -> bool {
    let ts_start = find_timestamp_start(s);
    if ts_start.is_none() {
        return false;
    }
    let ts_start = ts_start.unwrap();
    if ts_start + 20 > s.len() {
        return false;
    }
    let ts = &s[ts_start..ts_start + 20];
    let valid = ts.len() == 20
        && ts[0..4].chars().all(|c| c.is_ascii_digit())
        && &ts[4..5] == "-"
        && ts[5..7].chars().all(|c| c.is_ascii_digit())
        && &ts[7..8] == "-"
        && ts[8..10].chars().all(|c| c.is_ascii_digit())
        && &ts[10..11] == "-"
        && ts[11..13].chars().all(|c| c.is_ascii_digit())
        && &ts[13..14] == "h"
        && ts[14..16].chars().all(|c| c.is_ascii_digit())
        && &ts[16..17] == "m"
        && ts[17..19].chars().all(|c| c.is_ascii_digit())
        && &ts[19..20] == "s";
    if !valid {
        return false;
    }
    let hostname = &s[..ts_start];
    if hostname.is_empty() {
        return false;
    }
    let hostname = hostname.strip_suffix('-').unwrap_or(hostname);
    if hostname.is_empty() {
        return false;
    }
    hostname
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !hostname.starts_with('-')
        && !hostname.ends_with('-')
}

fn find_timestamp_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    for i in 0..=bytes.len() - 20 {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) && bytes.get(i + 4) == Some(&b'-') {
            return Some(i);
        }
    }
    None
}
