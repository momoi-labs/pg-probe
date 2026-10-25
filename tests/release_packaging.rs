//! Black-box tests for the deterministic release packaging scripts.
//!
//! Seams under test:
//! - `scripts/package-release-tarball.sh` archive layout and modes
//! - `scripts/write-sha256sums.sh` sorted checksum manifest format
//! - `site/public/install.sh` download, checksum check and binary install

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn require_gnu_tar() {
    let has_gtar = Command::new("gtar")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let tar_is_gnu = Command::new("tar")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("GNU tar"))
        .unwrap_or(false);
    if !(has_gtar || tar_is_gnu) {
        panic!("GNU tar (or gtar) is required for release packaging tests");
    }
}

fn write_fake_binary(path: &Path) {
    fs::write(path, b"#!/bin/sh\necho pg-probe fake\n").unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn package_release_tarball_contains_binary_readme_and_license() {
    require_gnu_tar();
    let root = repo_root();
    let dir = tempdir().unwrap();
    let binary = dir.path().join("pg-probe");
    write_fake_binary(&binary);
    let out = dir.path().join("dist");

    let output = Command::new(root.join("scripts/package-release-tarball.sh"))
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .args([
            "0.1.0",
            "x86_64-unknown-linux-musl",
            binary.to_str().unwrap(),
            out.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run package-release-tarball.sh");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let archive_path = String::from_utf8(output.stdout).unwrap();
    let archive_path = archive_path.trim();
    assert!(
        archive_path.ends_with("pg-probe-v0.1.0-x86_64-unknown-linux-musl.tar.gz"),
        "unexpected archive path: {archive_path}"
    );
    assert!(Path::new(archive_path).is_file());

    let extract = dir.path().join("extract");
    fs::create_dir_all(&extract).unwrap();
    let untar = Command::new("tar")
        .args(["-xzf", archive_path, "-C"])
        .arg(&extract)
        .output()
        .unwrap();
    assert!(untar.status.success());

    let prefix = extract.join("pg-probe-v0.1.0-x86_64-unknown-linux-musl");
    let packed_binary = prefix.join("pg-probe");
    let readme = prefix.join("README.md");
    let license = prefix.join("LICENSE");
    assert!(packed_binary.is_file());
    assert!(readme.is_file());
    assert!(license.is_file());

    let mode = fs::metadata(&packed_binary).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755, "binary mode should be 0755, got {mode:o}");

    let readme_mode = fs::metadata(&readme).unwrap().permissions().mode() & 0o777;
    assert_eq!(readme_mode, 0o644);
}

#[test]
fn write_sha256sums_is_sorted_by_filename() {
    require_gnu_tar();
    let root = repo_root();
    let dir = tempdir().unwrap();
    let binary = dir.path().join("pg-probe");
    write_fake_binary(&binary);
    let out = dir.path().join("dist");

    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        let output = Command::new(root.join("scripts/package-release-tarball.sh"))
            .env("SOURCE_DATE_EPOCH", "1700000000")
            .args([
                "0.1.0",
                target,
                binary.to_str().unwrap(),
                out.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let x86 = out.join("pg-probe-v0.1.0-x86_64-unknown-linux-musl.tar.gz");
    let arm = out.join("pg-probe-v0.1.0-aarch64-unknown-linux-musl.tar.gz");
    let output = Command::new(root.join("scripts/write-sha256sums.sh"))
        .args([
            out.to_str().unwrap(),
            x86.to_str().unwrap(),
            arm.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = fs::read_to_string(out.join("SHA256SUMS")).unwrap();
    let lines: Vec<&str> = manifest.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].ends_with("  pg-probe-v0.1.0-aarch64-unknown-linux-musl.tar.gz"),
        "first line should be aarch64: {}",
        lines[0]
    );
    assert!(
        lines[1].ends_with("  pg-probe-v0.1.0-x86_64-unknown-linux-musl.tar.gz"),
        "second line should be x86_64: {}",
        lines[1]
    );
    for line in &lines {
        let digest = line.split_whitespace().next().unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(digest, digest.to_ascii_lowercase());
    }
}

#[test]
fn write_sha256sums_rejects_unexpected_filenames() {
    let root = repo_root();
    let dir = tempdir().unwrap();
    let rogue = dir.path().join("not-a-release-asset.bin");
    fs::write(&rogue, b"nope").unwrap();
    let output = Command::new(root.join("scripts/write-sha256sums.sh"))
        .args([dir.path().to_str().unwrap(), rogue.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

/// Lays out a release for the host architecture the way the R2 bucket does:
/// `latest.txt` and `v<version>/` holding the tarball and `SHA256SUMS`.
fn fake_bucket(dir: &Path) -> PathBuf {
    require_gnu_tar();
    let root = repo_root();
    let binary = dir.join("pg-probe");
    write_fake_binary(&binary);
    let bucket = dir.join("bucket");
    let release = bucket.join("v0.1.0");
    let target = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
    let output = Command::new(root.join("scripts/package-release-tarball.sh"))
        .args([
            "0.1.0",
            &target,
            binary.to_str().unwrap(),
            release.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let archive = String::from_utf8(output.stdout).unwrap();
    let output = Command::new(root.join("scripts/write-sha256sums.sh"))
        .args([release.to_str().unwrap(), archive.trim()])
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::write(bucket.join("latest.txt"), "0.1.0\n").unwrap();
    bucket
}

fn run_install(bucket: &Path, install_dir: &Path) -> std::process::Output {
    Command::new("sh")
        .arg(repo_root().join("site/public/install.sh"))
        .env(
            "PG_PROBE_DOWNLOAD_URL",
            format!("file://{}", bucket.display()),
        )
        .env("PG_PROBE_INSTALL_DIR", install_dir)
        .output()
        .unwrap()
}

#[test]
fn install_script_installs_the_latest_binary() {
    let dir = tempdir().unwrap();
    let bucket = fake_bucket(dir.path());
    let bin = dir.path().join("bin");

    let output = run_install(&bucket, &bin);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed = bin.join("pg-probe");
    let mode = fs::metadata(&installed).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
    let run = Command::new(&installed).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "pg-probe fake\n");
}

#[test]
fn install_script_rejects_a_checksum_mismatch() {
    let dir = tempdir().unwrap();
    let bucket = fake_bucket(dir.path());
    let sums = bucket.join("v0.1.0/SHA256SUMS");
    let manifest = fs::read_to_string(&sums).unwrap();
    fs::write(&sums, format!("{}{}", "0".repeat(64), &manifest[64..])).unwrap();
    let bin = dir.path().join("bin");

    let output = run_install(&bucket, &bin);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
    assert!(!bin.join("pg-probe").exists());
}
