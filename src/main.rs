mod signal;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use pg_probe::artifact::{self, ArtifactError};
use pg_probe::contract::SnapshotStatus;
use pg_probe::{CaptureReport, DomainSelection};

const EXIT_COMPLETE: u8 = 0;
const EXIT_PARTIAL: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_PERSIST: u8 = 3;
const EXIT_INTERRUPTED: u8 = 130;

/// Capture a portable PostgreSQL and Linux diagnostic snapshot.
#[derive(Debug, Parser)]
#[command(
    name = "pg-probe",
    version,
    disable_help_flag = true,
    subcommand_required = true,
    arg_required_else_help = true,
    about = "Capture a portable PostgreSQL and Linux diagnostic snapshot",
    long_about = "Capture a portable PostgreSQL and Linux diagnostic snapshot.\n\n\
        Exit codes:\n  \
        0   complete snapshot published\n  \
        1   partial snapshot published (a source failed)\n  \
        2   usage error: invalid arguments or bad data directory\n  \
        3   storage or finalization failure; no usable artifact\n  \
        130 interrupted by Ctrl+C (SIGINT)\n\n\
        A single Ctrl+C stops the run cooperatively, preserving committed\n\
        results in a .sqlite.partial file. A second Ctrl+C has no effect."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Print help.
    #[arg(long = "help", action = clap::ArgAction::Help)]
    _help: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Capture a portable PostgreSQL and Linux diagnostic snapshot.
    Capture(CaptureArgs),
    /// Review diagnostic artifacts in a local web console.
    Analyze(AnalyzeArgs),
    /// Show the queries running now, longest first (read-only session).
    Top(TopArgs),
}

#[derive(Debug, Args)]
struct CaptureArgs {
    /// PostgreSQL host or socket directory to connect to (psql `-h`).
    #[arg(short = 'h', long, value_name = "HOST")]
    host: Option<String>,

    /// PostgreSQL port to connect to (psql `-p`).
    #[arg(short = 'p', long, value_name = "PORT")]
    port: Option<u16>,

    /// PostgreSQL user name to connect as (psql `-U`).
    #[arg(short = 'U', long, value_name = "USER")]
    username: Option<String>,

    /// Database name, a `postgresql://` URL, or a keyword/value string
    /// (psql `-d`).
    #[arg(short = 'd', long = "dbname", value_name = "NAME")]
    dbname: Option<String>,

    /// Directory where the artifact is written. Must already exist and be
    /// writable. Defaults to the current directory.
    #[arg(short = 'D', long, value_name = "DIR", default_value = ".")]
    data_directory: PathBuf,

    /// Which collection domains to run: 'all', 'linux', or 'postgresql'.
    #[arg(long, value_name = "DOMAIN", default_value = "all")]
    domain: String,

    /// Suppress progress and warning messages on stderr.
    #[arg(short, long)]
    quiet: bool,

    /// Print detailed progress messages on stderr.
    #[arg(short, long)]
    verbose: bool,

    /// Print help.
    #[arg(short = '?', long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Directory containing .sqlite.gz diagnostic artifacts.
    #[arg(short, long, value_name = "DIR", default_value = ".")]
    data_directory: PathBuf,

    /// Local TCP port for the review server.
    #[arg(long, default_value_t = 3080)]
    port: u16,
}

#[derive(Debug, Args)]
struct TopArgs {
    /// PostgreSQL host or socket directory to connect to (psql `-h`).
    #[arg(short = 'h', long, value_name = "HOST")]
    host: Option<String>,

    /// PostgreSQL port to connect to (psql `-p`).
    #[arg(short = 'p', long, value_name = "PORT")]
    port: Option<u16>,

    /// PostgreSQL user name to connect as (psql `-U`).
    #[arg(short = 'U', long, value_name = "USER")]
    username: Option<String>,

    /// Database name, a `postgresql://` URL, or a keyword/value string
    /// (psql `-d`).
    #[arg(short = 'd', long = "dbname", value_name = "NAME")]
    dbname: Option<String>,

    /// Seconds between refreshes.
    #[arg(short, long, value_name = "SECONDS", default_value_t = 2)]
    interval: u64,

    /// Print help.
    #[arg(short = '?', long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,
}

fn main() -> ExitCode {
    pg_probe::panic_guard::install();

    let cli = Cli::parse();

    match cli.command {
        Command::Analyze(args) => analyze(args),
        Command::Capture(args) => capture(args),
        Command::Top(args) => top(args),
    }
}

fn top(args: TopArgs) -> ExitCode {
    if args.interval == 0 {
        eprintln!("pg-probe: --interval must be at least 1 second");
        return ExitCode::from(EXIT_USAGE);
    }
    let params = pg_probe::ConnectionParams {
        host: args.host,
        port: args.port,
        username: args.username,
        dbname: args.dbname,
    };
    match pg_probe::top::run(&params, std::time::Duration::from_secs(args.interval)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pg-probe: {error}");
            ExitCode::from(EXIT_PARTIAL)
        }
    }
}

fn analyze(args: AnalyzeArgs) -> ExitCode {
    if !args.data_directory.is_dir() {
        eprintln!(
            "pg-probe: data directory does not exist or is not a directory: {}",
            args.data_directory.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }

    eprintln!(
        "pg-probe: review console listening on http://127.0.0.1:{}/console/",
        args.port
    );
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("pg-probe: failed to start async runtime: {error}");
            return ExitCode::from(EXIT_PERSIST);
        }
    };
    match runtime.block_on(pg_probe::review::serve(args.data_directory, args.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pg-probe: review server failed: {error}");
            ExitCode::from(EXIT_PERSIST)
        }
    }
}

fn capture(cli: CaptureArgs) -> ExitCode {
    init_logger(cli.quiet, cli.verbose);

    if let Err(error) = signal::install() {
        tracing::error!("failed to install signal handlers: {error}");
        return ExitCode::from(EXIT_PERSIST);
    }

    let domains = match parse_domain(&cli.domain) {
        Ok(d) => d,
        Err(msg) => {
            tracing::error!("invalid --domain value: {msg}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let data_dir = cli.data_directory.clone();

    if !data_dir.is_dir() {
        tracing::error!(
            "data directory does not exist or is not a directory: {}",
            data_dir.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }

    warn_about_existing_partials(&data_dir);

    let params = pg_probe::ConnectionParams {
        host: cli.host,
        port: cli.port,
        username: cli.username,
        dbname: cli.dbname,
    };

    let result = pg_probe::capture_with_cancellation(&data_dir, signal::flag(), domains, &params);

    report_exit(result)
}

/// Installs the structured logger on stderr. The level is derived from the
/// verbosity flags: `--quiet` keeps only errors, the default keeps errors and
/// warnings, and `--verbose` adds per-collector progress at info level.
fn init_logger(quiet: bool, verbose: bool) {
    let level = if quiet {
        tracing::Level::ERROR
    } else if verbose {
        tracing::Level::INFO
    } else {
        tracing::Level::WARN
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(true)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

fn parse_domain(value: &str) -> Result<DomainSelection, String> {
    match value.to_lowercase().as_str() {
        "all" => Ok(DomainSelection::All),
        "linux" => Ok(DomainSelection::Linux),
        "postgresql" | "postgres" | "pg" => Ok(DomainSelection::PostgreSql),
        other => Err(format!(
            "'{other}' is not a valid domain; expected 'all', 'linux', or 'postgresql'"
        )),
    }
}

fn warn_about_existing_partials(data_dir: &std::path::Path) {
    let partials = artifact::scan_for_partials(data_dir);
    if partials.is_empty() {
        return;
    }
    let listing = partials
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n  ");
    tracing::warn!(
        "found {} existing .sqlite.partial file(s) in {}:\n  {}\nleft untouched; a future --cleanup option will manage them",
        partials.len(),
        data_dir.display(),
        listing,
    );
}

fn report_exit(result: Result<CaptureReport, ArtifactError>) -> ExitCode {
    match result {
        Ok(report) if report.interrupted => {
            if let Some(ref partial) = report.partial_path {
                tracing::warn!(
                    "interrupted; committed results preserved in {}",
                    partial.display()
                );
            } else {
                tracing::warn!("interrupted before any data was collected");
            }
            ExitCode::from(EXIT_INTERRUPTED)
        }
        Ok(report) => {
            match report.status {
                SnapshotStatus::Complete => {
                    if let Some(ref path) = report.final_path {
                        tracing::info!(artifact = %path.display(), "complete snapshot published");
                    }
                }
                SnapshotStatus::Partial => {
                    if let Some(ref path) = report.final_path {
                        tracing::warn!(
                            artifact = %path.display(),
                            "partial snapshot published (one or more sources failed)"
                        );
                    }
                }
            }
            match report.status {
                SnapshotStatus::Complete => ExitCode::from(EXIT_COMPLETE),
                SnapshotStatus::Partial => ExitCode::from(EXIT_PARTIAL),
            }
        }
        Err(ArtifactError::DataDirectoryMissing) => {
            tracing::error!("the data directory does not exist or is not a directory");
            ExitCode::from(EXIT_USAGE)
        }
        Err(ArtifactError::DataDirectoryNotWritable) => {
            tracing::error!("the data directory is not writable");
            ExitCode::from(EXIT_USAGE)
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::from(EXIT_PERSIST)
        }
    }
}
