//! Black-box coverage for connection inputs, output modes, and artifact states.

mod security;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use pg_probe::{CancelFlag, ConnectionParams, DomainSelection};
use security::{assert_finalized_artifact, canary, full_scan, run_cli, CanarySet};
use tempfile::tempdir;

fn unreachable_env(password: &str) -> Vec<(&str, &str)> {
    vec![
        ("PGHOST", "127.0.0.1"),
        ("PGPORT", "1"),
        ("PGCONNECT_TIMEOUT", "1"),
        ("PGPASSWORD", password),
    ]
}

#[test]
fn connection_inputs_and_output_modes_do_not_leak() {
    for mode in ["--quiet", "--verbose"] {
        let directory = tempdir().unwrap();
        let password = canary("password");
        let environment = canary("environment");
        let token = canary("token");
        let passfile_secret = canary("password-file");
        let passfile = directory.path().join(".pgpass");
        let passfile_contents = format!("127.0.0.1:1:*:probe:{passfile_secret}\n");
        fs::write(&passfile, &passfile_contents).unwrap();
        fs::set_permissions(&passfile, fs::Permissions::from_mode(0o600)).unwrap();

        let data_dir = directory.path().to_string_lossy().into_owned();
        let passfile_path = passfile.to_string_lossy().into_owned();
        let mut env = unreachable_env(&password);
        env.extend([
            ("PGAPPNAME", environment.as_str()),
            ("PGOPTIONS", token.as_str()),
            ("PGPASSFILE", passfile_path.as_str()),
        ]);
        let canaries: CanarySet = vec![
            ("password", password.clone()),
            ("environment", environment.clone()),
            ("token", token.clone()),
            ("password-file", passfile_secret),
        ];

        let output = run_cli(&["-D", &data_dir, mode], &env);
        full_scan(directory.path(), &output.stdout, &output.stderr, &canaries);
        assert_finalized_artifact(directory.path(), &output);
        assert_eq!(fs::read_to_string(&passfile).unwrap(), passfile_contents);
        assert_eq!(output.stderr.is_empty(), mode == "--quiet");
    }
}

#[test]
fn complete_dsn_does_not_leak_on_connection_failure() {
    let directory = tempdir().unwrap();
    let password = canary("dsn-password");
    let dsn = format!("postgresql://probe:{password}@127.0.0.1:1/probe?connect_timeout=1");
    let canaries = vec![("dsn-password", password), ("complete-dsn", dsn.clone())];
    let data_dir = directory.path().to_string_lossy().into_owned();

    let output = run_cli(&["-D", &data_dir, "-d", &dsn], &[]);
    full_scan(directory.path(), &output.stdout, &output.stderr, &canaries);
    assert_finalized_artifact(directory.path(), &output);
}

#[test]
fn interrupted_partial_artifact_does_not_leak() {
    let directory = tempdir().unwrap();
    let dsn = canary("interrupted-dsn");
    let canaries = vec![("interrupted-dsn", dsn.clone())];
    let cancel = CancelFlag::new();
    cancel.request(libc::SIGINT);

    let report = pg_probe::capture_with_cancellation(
        directory.path(),
        &cancel,
        DomainSelection::All,
        &ConnectionParams {
            dbname: Some(dsn),
            ..ConnectionParams::default()
        },
    )
    .unwrap();

    assert!(report.interrupted);
    assert!(report.partial_path.is_some_and(|path| path.is_file()));
    full_scan(directory.path(), &[], &[], &canaries);
}

#[test]
fn unwritable_directory_failure_does_not_leak_or_publish() {
    let directory = tempdir().unwrap();
    let password = canary("fatal-password");
    let canaries = vec![("fatal-password", password.clone())];
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let data_dir = directory.path().to_string_lossy().into_owned();

    let output = run_cli(&["-D", &data_dir], &[("PGPASSWORD", &password)]);
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();

    full_scan(directory.path(), &output.stdout, &output.stderr, &canaries);
    if unsafe { libc::geteuid() } != 0 {
        assert_eq!(output.status.code(), Some(3));
        assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
    }
}
