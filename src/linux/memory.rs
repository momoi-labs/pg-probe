use std::io;
use std::path::Path;

use crate::collector::{Collector, Sink};
use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const MEMINFO_SOURCE: &str = "proc.meminfo";
const VMSTAT_SOURCE: &str = "proc.vmstat";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub key: &'static str,
    pub value: u64,
    pub unit: MemoryUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUnit {
    Bytes,
    Pages,
}

impl MemoryUnit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Pages => "pages",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmEventEntry {
    pub key: String,
    pub value: u64,
    pub unit: VmEventUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmEventUnit {
    Events,
    KiB,
}

impl VmEventUnit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::KiB => "KiB",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryObservations {
    pub memory: Vec<MemoryEntry>,
    pub vm_events: Vec<VmEventEntry>,
}

const MEMINFO_ALLOWLIST: &[(&str, MemoryUnit)] = &[
    ("MemTotal", MemoryUnit::Bytes),
    ("MemFree", MemoryUnit::Bytes),
    ("MemAvailable", MemoryUnit::Bytes),
    ("Buffers", MemoryUnit::Bytes),
    ("Cached", MemoryUnit::Bytes),
    ("SReclaimable", MemoryUnit::Bytes),
    ("SUnreclaim", MemoryUnit::Bytes),
    ("Slab", MemoryUnit::Bytes),
    ("Shmem", MemoryUnit::Bytes),
    ("SwapTotal", MemoryUnit::Bytes),
    ("SwapFree", MemoryUnit::Bytes),
    ("Dirty", MemoryUnit::Bytes),
    ("Writeback", MemoryUnit::Bytes),
    ("AnonPages", MemoryUnit::Bytes),
    ("Mapped", MemoryUnit::Bytes),
    ("PageTables", MemoryUnit::Bytes),
    ("KernelStack", MemoryUnit::Bytes),
    ("AnonHugePages", MemoryUnit::Bytes),
    ("HugePages_Total", MemoryUnit::Pages),
    ("HugePages_Free", MemoryUnit::Pages),
    ("HugePages_Rsvd", MemoryUnit::Pages),
    ("HugePages_Surp", MemoryUnit::Pages),
    ("Hugepagesize", MemoryUnit::Bytes),
];

const VMSTAT_EXACT_KEYS: &[&str] = &[
    "pgpgin",
    "pgpgout",
    "pswpin",
    "pswpout",
    "pgfault",
    "pgmajfault",
    "oom_kill",
];

const VMSTAT_PREFIX_KEYS: &[&str] = &["allocstall_", "pgscan_", "pgsteal_", "compact_"];

pub struct MemoryCollector;

impl Collector for MemoryCollector {
    type Output = MemoryObservations;

    fn source(&self) -> &'static str {
        "linux.memory"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        collect_memory()
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_memory(output, outcomes)
    }
}

fn collect_memory() -> (MemoryObservations, Vec<ComponentOutcome>) {
    let mut observations = MemoryObservations::default();
    let mut outcomes = Vec::new();

    collect_meminfo(&mut observations, &mut outcomes);
    collect_vmstat(&mut observations, &mut outcomes);

    (observations, outcomes)
}

fn collect_meminfo(observations: &mut MemoryObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/meminfo")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    MEMINFO_SOURCE,
                    status,
                    "meminfo_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_meminfo(&content) {
        Ok(entries) => {
            let observed_at_utc = now_utc();
            observations.memory = entries;
            outcomes.push(ComponentOutcome::collected(
                MEMINFO_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error_code) => {
            outcomes.push(
                ComponentOutcome::failed(
                    MEMINFO_SOURCE,
                    CollectionStatus::ParseError,
                    error_code,
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn collect_vmstat(observations: &mut MemoryObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/vmstat")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    VMSTAT_SOURCE,
                    status,
                    "vmstat_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_vmstat(&content) {
        Ok(entries) => {
            let observed_at_utc = now_utc();
            observations.vm_events = entries;
            outcomes.push(ComponentOutcome::collected(
                VMSTAT_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error_code) => {
            outcomes.push(
                ComponentOutcome::failed(
                    VMSTAT_SOURCE,
                    CollectionStatus::ParseError,
                    error_code,
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

pub fn parse_meminfo(content: &str) -> Result<Vec<MemoryEntry>, &'static str> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let key = parts[0].trim_end_matches(':');
        let value_str = parts[1];

        let unit = match find_meminfo_unit(key) {
            Some(unit) => unit,
            None => continue,
        };

        let value: u64 = value_str.parse().map_err(|_| "invalid_meminfo_value")?;

        let converted_value = if parts.len() >= 3 && parts[2] == "kB" && unit == MemoryUnit::Bytes {
            value * 1024
        } else {
            value
        };

        entries.push(MemoryEntry {
            key: find_meminfo_key(key).unwrap(),
            value: converted_value,
            unit,
        });
    }

    Ok(entries)
}

fn find_meminfo_unit(key: &str) -> Option<MemoryUnit> {
    MEMINFO_ALLOWLIST
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, u)| *u)
}

fn find_meminfo_key(key: &str) -> Option<&'static str> {
    MEMINFO_ALLOWLIST
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(k, _)| *k)
}

fn parse_vmstat(content: &str) -> Result<Vec<VmEventEntry>, &'static str> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 2 {
            continue;
        }

        let key = parts[0];
        let value: u64 = parts[1].parse().map_err(|_| "invalid_vmstat_value")?;

        if let Some(unit) = find_vmstat_unit(key) {
            entries.push(VmEventEntry {
                key: key.to_string(),
                value,
                unit,
            });
        }
    }

    Ok(entries)
}

fn find_vmstat_unit(key: &str) -> Option<VmEventUnit> {
    if VMSTAT_EXACT_KEYS.contains(&key) {
        if key == "pgpgin" || key == "pgpgout" {
            Some(VmEventUnit::KiB)
        } else {
            Some(VmEventUnit::Events)
        }
    } else if VMSTAT_PREFIX_KEYS
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        Some(VmEventUnit::Events)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_meminfo_converts_kb_to_bytes() {
        let content = "MemTotal:       16384000 kB\nMemFree:         8192000 kB\n";
        let entries = parse_meminfo(content).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "MemTotal");
        assert_eq!(entries[0].value, 16384000 * 1024);
        assert_eq!(entries[0].unit, MemoryUnit::Bytes);
        assert_eq!(entries[1].key, "MemFree");
        assert_eq!(entries[1].value, 8192000 * 1024);
    }

    #[test]
    fn parse_meminfo_hugepages_are_pages() {
        let content = "HugePages_Total:     100\nHugePages_Free:       50\n";
        let entries = parse_meminfo(content).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "HugePages_Total");
        assert_eq!(entries[0].value, 100);
        assert_eq!(entries[0].unit, MemoryUnit::Pages);
    }

    #[test]
    fn parse_meminfo_ignores_unknown_keys() {
        let content = "MemTotal:       16384000 kB\nUnknownKey:     12345 kB\n";
        let entries = parse_meminfo(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "MemTotal");
    }

    #[test]
    fn parse_vmstat_exact_keys() {
        let content = "pgpgin 123456\npgfault 789012\n";
        let entries = parse_vmstat(content).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "pgpgin");
        assert_eq!(entries[0].value, 123456);
        assert_eq!(entries[0].unit, VmEventUnit::KiB);
        assert_eq!(entries[1].key, "pgfault");
        assert_eq!(entries[1].value, 789012);
        assert_eq!(entries[1].unit, VmEventUnit::Events);
    }

    #[test]
    fn parse_vmstat_prefix_keys() {
        let content = "allocstall_dma 100\npgscan_dma 200\ncompact_success 300\n";
        let entries = parse_vmstat(content).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "allocstall_dma");
        assert_eq!(entries[0].unit, VmEventUnit::Events);
        assert_eq!(entries[1].key, "pgscan_dma");
        assert_eq!(entries[2].key, "compact_success");
    }

    #[test]
    fn parse_vmstat_ignores_non_allowlisted() {
        let content = "pgpgin 123456\nnr_free_pages 789012\n";
        let entries = parse_vmstat(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "pgpgin");
    }
}
