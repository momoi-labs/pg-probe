use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use pg_probe::contract::SnapshotStatus;

#[derive(Debug, Parser)]
#[command(
    name = "pg-probe",
    version,
    about = "Capture a portable PostgreSQL and Linux diagnostic snapshot"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Capture the minimum host identity snapshot.
    Snapshot {
        /// Destination for the compressed SQLite artifact.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Snapshot { output } => match pg_probe::capture(&output) {
            Ok(SnapshotStatus::Complete) => {
                eprintln!("snapshot artifact published");
                ExitCode::SUCCESS
            }
            Ok(SnapshotStatus::Partial) => {
                eprintln!("partial snapshot artifact published");
                ExitCode::from(1)
            }
            Err(error) => {
                eprintln!("pg-probe: {error}");
                ExitCode::from(1)
            }
        },
    }
}
