mod signal;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
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

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(error) = signal::install() {
        eprintln!("pg-probe: failed to install signal handlers: {error}");
        return ExitCode::from(EXIT_PERSIST);
    }

    let domains = match parse_domain(&cli.domain) {
        Ok(d) => d,
        Err(msg) => {
            eprintln!("pg-probe: invalid --domain value: {msg}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let data_dir = cli.data_directory.clone();

    if !data_dir.is_dir() {
        eprintln!(
            "pg-probe: data directory does not exist or is not a directory: {}",
            data_dir.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }

    if !cli.quiet {
        warn_about_existing_partials(&data_dir);
    }

    let params = pg_probe::ConnectionParams {
        host: cli.host,
        port: cli.port,
        username: cli.username,
        dbname: cli.dbname,
    };

    let result = pg_probe::capture_with_cancellation(&data_dir, signal::flag(), domains, &params);

    report_exit(result, cli.quiet, cli.verbose)
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
    if !partials.is_empty() {
        eprintln!(
            "pg-probe: warning: found {} existing .sqlite.partial file(s) in {}:",
            partials.len(),
            data_dir.display()
        );
        for path in &partials {
            eprintln!("  {}", path.display());
        }
        eprintln!(
            "pg-probe: these files are left untouched. A future --cleanup option will manage them."
        );
    }
}

fn report_exit(
    result: Result<CaptureReport, ArtifactError>,
    quiet: bool,
    verbose: bool,
) -> ExitCode {
    match result {
        Ok(report) if report.interrupted => {
            if !quiet {
                if let Some(ref partial) = report.partial_path {
                    eprintln!(
                        "pg-probe: interrupted; committed results preserved in {}",
                        partial.display()
                    );
                } else {
                    eprintln!("pg-probe: interrupted before any data was collected");
                }
            }
            ExitCode::from(EXIT_INTERRUPTED)
        }
        Ok(report) => {
            if let Some(ref path) = report.final_path {
                println!("{}", path.display());
            }
            if !quiet {
                match report.status {
                    SnapshotStatus::Complete => {
                        if verbose {
                            eprintln!("pg-probe: complete snapshot published");
                        }
                    }
                    SnapshotStatus::Partial => {
                        eprintln!(
                            "pg-probe: partial snapshot published (one or more sources failed)"
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
            eprintln!("pg-probe: the data directory does not exist or is not a directory");
            ExitCode::from(EXIT_USAGE)
        }
        Err(ArtifactError::DataDirectoryNotWritable) => {
            eprintln!("pg-probe: the data directory is not writable");
            ExitCode::from(EXIT_USAGE)
        }
        Err(error) => {
            eprintln!("pg-probe: {error}");
            ExitCode::from(EXIT_PERSIST)
        }
    }
}
