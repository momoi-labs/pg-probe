use std::fs;
use std::os::unix::fs::PermissionsExt;

use pg_probe::artifact;
use pg_probe::{capture, capture_with_cancellation, CancelFlag, ConnectionParams, DomainSelection};
use tempfile::tempdir;

const SIGINT: i32 = libc::SIGINT;

#[test]
fn a_successful_capture_leaves_only_the_finalized_artifact() {
    let directory = tempdir().unwrap();
    let report = capture(directory.path()).unwrap();

    assert!(!report.interrupted);
    assert!(report.final_path.is_some());
    assert!(report.partial_path.is_none());

    let final_path = report.final_path.unwrap();
    assert!(final_path.exists());
    let mode = fs::metadata(&final_path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let partials: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".sqlite.partial"))
        })
        .collect();
    assert!(partials.is_empty());
}

#[test]
fn cancellation_before_collection_preserves_partial_without_gzip() {
    let directory = tempdir().unwrap();
    let cancel = CancelFlag::new();
    cancel.request(SIGINT);

    let report = capture_with_cancellation(
        directory.path(),
        &cancel,
        DomainSelection::All,
        &ConnectionParams::default(),
    )
    .unwrap();

    assert!(report.interrupted);
    assert!(
        report.final_path.is_none(),
        "no .sqlite.gz should be published"
    );
    assert!(report.partial_path.is_some(), "partial should be preserved");

    let partial = report.partial_path.unwrap();
    assert!(partial.exists(), "partial file must exist");

    let gz_files: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "gz"))
        .collect();
    assert!(gz_files.is_empty());

    let mode = fs::metadata(&partial).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn second_cancel_signal_has_no_effect() {
    let cancel = CancelFlag::new();
    cancel.request(SIGINT);
    cancel.request(SIGINT);
    cancel.request(SIGINT);
    assert_eq!(cancel.signal(), Some(SIGINT));
}

#[test]
fn filename_uses_sanitized_hostname_and_timestamp() {
    let directory = tempdir().unwrap();
    let report = capture(directory.path()).unwrap();
    let path = report.final_path.unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.ends_with(".sqlite.gz"));
    let stem = name.strip_suffix(".sqlite.gz").unwrap();
    assert!(stem.contains('-'), "stem should contain hyphens: {stem}");
}

#[test]
fn collision_suffix_is_appended_when_file_exists() {
    let directory = tempdir().unwrap();
    let report1 = capture(directory.path()).unwrap();
    let path1 = report1.final_path.unwrap();
    assert!(path1.exists());
    let report2 = capture(directory.path()).unwrap();
    let path2 = report2.final_path.unwrap();
    assert!(path2.exists());
    assert_ne!(path1, path2);
}

#[test]
fn scan_for_partials_finds_existing_partials() {
    let directory = tempdir().unwrap();
    assert!(artifact::scan_for_partials(directory.path()).is_empty());
    fs::write(
        directory
            .path()
            .join("host-2026-01-01-00h00m00s.sqlite.partial"),
        b"",
    )
    .unwrap();
    fs::write(directory.path().join("other.txt"), b"").unwrap();
    let partials = artifact::scan_for_partials(directory.path());
    assert_eq!(partials.len(), 1);
}

#[test]
fn existing_partials_are_not_modified_by_new_run() {
    let directory = tempdir().unwrap();
    let pre_existing = directory
        .path()
        .join("oldhost-2026-01-01-00h00m00s.sqlite.partial");
    let original_content = b"pre-existing partial data";
    fs::write(&pre_existing, original_content).unwrap();
    let _ = capture(directory.path()).unwrap();
    assert_eq!(fs::read(&pre_existing).unwrap(), original_content);
}

#[test]
fn domain_postgresql_only_produces_artifact_without_linux_data() {
    let directory = tempdir().unwrap();
    let report = capture_with_cancellation(
        directory.path(),
        &CancelFlag::new(),
        DomainSelection::PostgreSql,
        &ConnectionParams::default(),
    )
    .unwrap();

    assert!(!report.interrupted);
    assert!(report.final_path.is_some());
    let final_path = report.final_path.unwrap();
    assert!(final_path.exists());
    assert!(final_path.to_string_lossy().ends_with(".sqlite.gz"));
    assert!(report.partial_path.is_none());
}

#[test]
fn domain_linux_only_produces_artifact_with_linux_data() {
    let directory = tempdir().unwrap();
    let report = capture_with_cancellation(
        directory.path(),
        &CancelFlag::new(),
        DomainSelection::Linux,
        &ConnectionParams::default(),
    )
    .unwrap();

    assert!(!report.interrupted);
    assert!(report.final_path.is_some());
    let final_path = report.final_path.unwrap();
    assert!(final_path.exists());
    assert!(final_path.to_string_lossy().ends_with(".sqlite.gz"));
}
