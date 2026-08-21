//! Shared canary generation and observable-surface scanning for issue #19.
#![allow(dead_code)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use flate2::read::GzDecoder;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use uuid::Uuid;

pub type CanarySet = Vec<(&'static str, String)>;

pub fn canary(label: &str) -> String {
    format!("PGPROBE-CANARY-{label}-{}", Uuid::new_v4().simple())
}

pub fn identifier_safe_canary(label: &str) -> String {
    format!(
        "pgprobecanary_{}_{}",
        label.replace(['-', ' '], "_"),
        Uuid::new_v4().simple()
    )
}

pub fn pg_probe_cmd() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pg-probe"));
    command.arg("capture");
    command
}

pub fn run_cli(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = pg_probe_cmd();
    command.args(args);
    command.envs(envs.iter().copied());
    command.output().expect("run pg-probe")
}

fn assert_bytes_free_of(surface: &str, bytes: &[u8], canaries: &CanarySet) {
    for (label, value) in canaries {
        assert!(
            !bytes
                .windows(value.len())
                .any(|window| window == value.as_bytes()),
            "{surface} contains the {label} canary"
        );
    }
}

fn scan_sqlite(surface: &str, bytes: &[u8], canaries: &CanarySet) {
    let scratch = tempfile::NamedTempFile::new().expect("create scratch SQLite file");
    fs::write(scratch.path(), bytes).expect("write scratch SQLite file");
    let Ok(connection) =
        Connection::open_with_flags(scratch.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return;
    };

    let tables = {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .expect("list SQLite tables");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("read SQLite tables")
            .filter_map(Result::ok)
            .filter(|name| !name.starts_with("sqlite_"))
            .collect::<Vec<_>>()
    };

    for table in tables {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{table}\""))
            .expect("query SQLite table");
        let column_count = statement.column_count();
        let mut rows = statement.query([]).expect("read SQLite rows");
        while let Some(row) = rows.next().expect("advance SQLite rows") {
            for column in 0..column_count {
                match row.get_ref(column).expect("read SQLite value") {
                    ValueRef::Text(value) | ValueRef::Blob(value) => {
                        assert_bytes_free_of(surface, value, canaries);
                    }
                    ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => {}
                }
            }
        }
    }
}

fn decompress(path: &Path) -> Vec<u8> {
    let mut decoder = GzDecoder::new(fs::File::open(path).expect("open gzip artifact"));
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .expect("decompress gzip artifact");
    bytes
}

pub fn full_scan(dir: &Path, stdout: &[u8], stderr: &[u8], canaries: &CanarySet) {
    assert_bytes_free_of("stdout", stdout, canaries);
    assert_bytes_free_of("stderr", stderr, canaries);

    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".sqlite.partial") {
            let bytes = fs::read(&path).expect("read partial artifact");
            assert_bytes_free_of("partial artifact", &bytes, canaries);
            scan_sqlite("partial SQLite records", &bytes, canaries);
        } else if name.ends_with(".sqlite.gz") {
            let bytes = decompress(&path);
            assert_bytes_free_of("decompressed artifact", &bytes, canaries);
            scan_sqlite("final SQLite records", &bytes, canaries);
        }
    }
}

pub fn sole_gzip_artifact(dir: &Path) -> PathBuf {
    let artifacts = fs::read_dir(dir)
        .expect("read artifact directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".sqlite.gz"))
        })
        .collect::<Vec<_>>();
    assert_eq!(artifacts.len(), 1, "expected one finalized artifact");
    artifacts.into_iter().next().unwrap()
}

pub fn assert_finalized_artifact(dir: &Path, output: &Output) -> PathBuf {
    assert!(
        matches!(output.status.code(), Some(0) | Some(1)),
        "expected a finalized run, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    sole_gzip_artifact(dir)
}

pub fn decompressed_artifact_bytes(path: &Path) -> Vec<u8> {
    decompress(path)
}
