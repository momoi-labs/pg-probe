use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const PSI_CPU_SOURCE: &str = "proc.pressure.cpu";
const PSI_MEMORY_SOURCE: &str = "proc.pressure.memory";
const PSI_IO_SOURCE: &str = "proc.pressure.io";

#[derive(Debug, Clone, PartialEq)]
pub struct PsiEntry {
    pub resource: PsiResource,
    pub scope: PsiScope,
    pub avg10: f64,
    pub avg60: f64,
    pub avg300: f64,
    pub total_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsiResource {
    Cpu,
    Memory,
    Io,
}

impl PsiResource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Io => "io",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsiScope {
    Some,
    Full,
}

impl PsiScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Some => "some",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PressureObservations {
    pub psi: Vec<PsiEntry>,
}

pub fn collect_pressure_isolated() -> (PressureObservations, Vec<ComponentOutcome>) {
    catch_unwind(AssertUnwindSafe(collect_pressure)).unwrap_or_else(|_| {
        (
            PressureObservations::default(),
            vec![ComponentOutcome::failed(
                "linux.pressure",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        )
    })
}

fn collect_pressure() -> (PressureObservations, Vec<ComponentOutcome>) {
    let mut observations = PressureObservations::default();
    let mut outcomes = Vec::new();

    collect_psi_resource(PsiResource::Cpu, PSI_CPU_SOURCE, &mut observations, &mut outcomes);
    collect_psi_resource(PsiResource::Memory, PSI_MEMORY_SOURCE, &mut observations, &mut outcomes);
    collect_psi_resource(PsiResource::Io, PSI_IO_SOURCE, &mut observations, &mut outcomes);

    (observations, outcomes)
}

fn collect_psi_resource(
    resource: PsiResource,
    source: &'static str,
    observations: &mut PressureObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();
    let path = format!("/proc/pressure/{}", resource.as_str());

    let content = match read_bounded(Path::new(&path)) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(source, status, "psi_read_failed", error.raw_os_error())
                    .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_psi_file(&content, resource) {
        Ok(entries) => {
            let observed_at_utc = now_utc();
            observations.psi.extend(entries);
            outcomes.push(ComponentOutcome::collected(
                source,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error_code) => {
            outcomes.push(
                ComponentOutcome::failed(source, CollectionStatus::ParseError, error_code, None)
                    .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn parse_psi_file(content: &str, resource: PsiResource) -> Result<Vec<PsiEntry>, &'static str> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let scope = match parts[0] {
            "some" => PsiScope::Some,
            "full" => PsiScope::Full,
            _ => continue,
        };

        if parts.len() < 2 {
            return Err("invalid_psi_line");
        }

        let mut avg10 = 0.0;
        let mut avg60 = 0.0;
        let mut avg300 = 0.0;
        let mut total_us = 0;

        for part in &parts[1..] {
            if let Some((key, value)) = part.split_once('=') {
                match key {
                    "avg10" => avg10 = value.parse().map_err(|_| "invalid_psi_avg_value")?,
                    "avg60" => avg60 = value.parse().map_err(|_| "invalid_psi_avg_value")?,
                    "avg300" => avg300 = value.parse().map_err(|_| "invalid_psi_avg_value")?,
                    "total" => total_us = value.parse().map_err(|_| "invalid_psi_total_value")?,
                    _ => {}
                }
            }
        }

        entries.push(PsiEntry {
            resource,
            scope,
            avg10,
            avg60,
            avg300,
            total_us,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_psi_file_valid() {
        let content = "some avg10=0.00 avg60=0.00 avg300=0.00 total=12345\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=67890\n";
        let entries = parse_psi_file(content, PsiResource::Cpu).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].resource, PsiResource::Cpu);
        assert_eq!(entries[0].scope, PsiScope::Some);
        assert_eq!(entries[0].avg10, 0.0);
        assert_eq!(entries[0].total_us, 12345);
        assert_eq!(entries[1].scope, PsiScope::Full);
        assert_eq!(entries[1].total_us, 67890);
    }

    #[test]
    fn parse_psi_file_with_values() {
        let content = "some avg10=1.50 avg60=2.25 avg300=3.75 total=999999\n";
        let entries = parse_psi_file(content, PsiResource::Memory).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].resource, PsiResource::Memory);
        assert_eq!(entries[0].avg10, 1.5);
        assert_eq!(entries[0].avg60, 2.25);
        assert_eq!(entries[0].avg300, 3.75);
    }

    #[test]
    fn parse_psi_file_empty() {
        let entries = parse_psi_file("", PsiResource::Io).unwrap();
        assert_eq!(entries.len(), 0);
    }
}
