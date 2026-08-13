//! Bring up a real PostgreSQL cluster once per test binary and reuse it across
//! the integration tests in this crate. The setup is complete, hermetic,
//! and isolated from the host: it `initdb`s a temporary data directory,
//! starts a dedicated `postgres` process on a randomly chosen TCP port,
//! creates the test roles and databases through `psql`, and shuts everything
//! down when the returned handle is dropped.

use std::env;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// One running PostgreSQL cluster preconfigured with three roles and four
/// databases suitable for exercising every collector branch the module
/// supports.
pub struct PgCluster {
    pub host: String,
    pub port: u16,
    pub bootstrap_password: String,
    pub admin_role: String,
    pub reader_role: String,
    pub unavailable_role: String,
    pub admin_database: String,
    pub reader_database: String,
    pub unavailable_database: String,
    _tempdir: TempDir,
    postgres: Option<Child>,
}

impl PgCluster {
    /// Connection string a privileged operator would build to reach this
    /// cluster.
    pub fn admin_connection_string(&self) -> String {
        format!(
            "host={} port={} user={} password={} dbname={} connect_timeout=5",
            self.host, self.port, self.admin_role, self.bootstrap_password, self.admin_database,
        )
    }

    /// Connection string for a low-privilege role that does not have
    /// `pg_monitor`, exercising the permission-denied branches.
    pub fn reader_connection_string(&self) -> String {
        format!(
            "host={} port={} user={} password={} dbname={} connect_timeout=5",
            self.host, self.port, self.reader_role, self.bootstrap_password, self.reader_database,
        )
    }

    /// Connection string for a database with no entry on `pg_hba.conf`
    /// accepting it from 127.0.0.1 — used to exercise the connection-failed
    /// branch.
    pub fn unavailable_connection_string(&self) -> String {
        format!(
            "host={} port={} user={} password={} dbname={} connect_timeout=5",
            self.host,
            self.port,
            self.unavailable_role,
            self.bootstrap_password,
            self.unavailable_database,
        )
    }
}

impl Drop for PgCluster {
    fn drop(&mut self) {
        if let Some(mut child) = self.postgres.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static CLUSTER: OnceLock<Option<PgCluster>> = OnceLock::new();

/// Idempotently returns the cluster handle, starting it if needed. Returns
/// `None` when the host does not have the `postgres` and `initdb` binaries
/// on `PATH`, so unit-only environments skip the heavy setup.
pub fn cluster() -> Option<&'static PgCluster> {
    CLUSTER
        .get_or_init(|| match try_start() {
            Ok(cluster) => Some(cluster),
            Err(error) => {
                eprintln!(
                    "skipping PostgreSQL integration tests: failed to start cluster: {error}"
                );
                None
            }
        })
        .as_ref()
}

fn try_start() -> Result<PgCluster, String> {
    let postgres_bin = locate_binary("postgres")?;
    let initdb_bin = locate_binary("initdb")?;
    let psql_bin = locate_binary("psql")?;

    let tempdir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let data_dir = tempdir.path().join("pgdata");
    let socket_dir = tempdir.path().join("sockets");
    fs::create_dir_all(&socket_dir).map_err(|error| error.to_string())?;

    bootstrap_cluster(&initdb_bin, &data_dir)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);
    let host = "127.0.0.1".to_owned();

    let child = spawn_postgres(&postgres_bin, &data_dir, &socket_dir, &host, port)?;
    wait_for_port(&host, port, Duration::from_secs(30))?;

    let bootstrap_password = "probe-secret".to_owned();
    let admin_role = "probe_admin".to_owned();
    let reader_role = "probe_reader".to_owned();
    let unavailable_role = "probe_unavailable".to_owned();

    set_roles_and_databases(&psql_bin, port, &bootstrap_password)?;

    Ok(PgCluster {
        host,
        port,
        bootstrap_password,
        admin_role: admin_role.clone(),
        reader_role: reader_role.clone(),
        unavailable_role: unavailable_role.clone(),
        admin_database: "probe_main".to_owned(),
        reader_database: "probe_reader".to_owned(),
        unavailable_database: "probe_unavailable".to_owned(),
        _tempdir: tempdir,
        postgres: Some(child),
    })
}

fn locate_binary(name: &str) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("PG_BIN_PATH") {
        let candidate = PathBuf::from(path).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let path = env::var_os("PATH").ok_or_else(|| "PATH is not set".to_owned())?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("`{name}` was not found on PATH"))
}

fn bootstrap_cluster(initdb: &Path, data_dir: &Path) -> Result<(), String> {
    run_command(
        Command::new(initdb)
            .arg("-D")
            .arg(data_dir)
            .arg("--auth-host=trust")
            .arg("--auth-local=trust")
            .arg("--username=probe_admin")
            .arg("--no-locale")
            .arg("--encoding=UTF8")
            .env("LC_ALL", "C"),
    )
}

fn spawn_postgres(
    postgres: &Path,
    data_dir: &Path,
    socket_dir: &Path,
    host: &str,
    port: u16,
) -> Result<Child, String> {
    let mut command = Command::new(postgres);
    command
        .arg("-D")
        .arg(data_dir)
        .arg("-h")
        .arg(host)
        .arg("-p")
        .arg(port.to_string())
        .arg("-c")
        .arg(format!("unix_socket_directories={}", socket_dir.display()))
        .arg("-c")
        .arg("listen_addresses=127.0.0.1")
        .arg("-c")
        .arg("shared_buffers=16MB")
        .arg("-c")
        .arg("fsync=off")
        .arg("-c")
        .arg("full_page_writes=off")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|error| error.to_string())
}

fn wait_for_port(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let address: SocketAddr = format!("{host}:{port}")
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("timed out waiting for postgres on {host}:{port}"))
}

fn run_command(command: &mut Command) -> Result<(), String> {
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "`{:?}` exited with status {}: {}",
            command,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn set_roles_and_databases(psql: &Path, port: u16, bootstrap_password: &str) -> Result<(), String> {
    run_psql(
        psql,
        port,
        &format!(
            "ALTER USER probe_admin WITH PASSWORD '{}';",
            bootstrap_password
        ),
    )?;
    run_psql(
        psql,
        port,
        &format!(
            "CREATE ROLE probe_reader WITH LOGIN PASSWORD '{}';",
            bootstrap_password
        ),
    )?;
    run_psql(
        psql,
        port,
        &format!(
            "CREATE ROLE probe_unavailable WITH LOGIN PASSWORD '{}';",
            bootstrap_password
        ),
    )?;
    run_psql(psql, port, "CREATE DATABASE probe_main OWNER probe_admin;")?;
    run_psql(psql, port, "CREATE DATABASE probe_extra OWNER probe_admin;")?;
    run_psql(
        psql,
        port,
        "CREATE DATABASE probe_reader OWNER probe_reader;",
    )?;
    run_psql(
        psql,
        port,
        "CREATE DATABASE probe_unavailable OWNER probe_unavailable;",
    )?;
    run_psql(psql, port, "GRANT pg_monitor TO probe_admin;")?;
    run_psql(
        psql,
        port,
        "REVOKE CONNECT ON DATABASE probe_main, probe_extra FROM PUBLIC;",
    )?;
    run_psql(
        psql,
        port,
        "ALTER DATABASE probe_unavailable WITH ALLOW_CONNECTIONS false;",
    )?;
    Ok(())
}

fn run_psql(psql: &Path, port: u16, sql: &str) -> Result<(), String> {
    let mut command = Command::new(psql);
    command
        .arg("-h")
        .arg("127.0.0.1")
        .arg("-p")
        .arg(port.to_string())
        .arg("-U")
        .arg("probe_admin")
        .arg("-d")
        .arg("postgres")
        .arg("-v")
        .arg("ON_ERROR_STOP=1")
        .arg("-c")
        .arg(sql);
    command.stdin(Stdio::null());
    run_command(&mut command)
}
