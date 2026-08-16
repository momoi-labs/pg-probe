use std::io;
use std::path::{Path, PathBuf};

use crate::collector::{Collector, Sink};
use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const CGROUPS_SOURCE: &str = "proc.cgroup";
const CGROUP_EVENTS_SOURCE: &str = "proc.cgroup.events";
const CGROUP_IO_SOURCE: &str = "proc.cgroup.io";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupVersion {
    V1,
    V2,
}

impl CgroupVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CgroupLimits {
    pub version: Option<CgroupVersion>,
    pub cpu_quota_microseconds: Option<i64>,
    pub cpu_period_microseconds: Option<i64>,
    pub cpu_quota_unlimited: Option<bool>,
    pub cpu_weight: Option<u64>,
    pub memory_current_bytes: Option<u64>,
    pub memory_min_bytes: Option<u64>,
    pub memory_low_bytes: Option<u64>,
    pub memory_high_bytes: Option<u64>,
    pub memory_high_unlimited: Option<bool>,
    pub memory_max_bytes: Option<u64>,
    pub memory_max_unlimited: Option<bool>,
    pub swap_current_bytes: Option<u64>,
    pub swap_max_bytes: Option<u64>,
    pub swap_max_unlimited: Option<bool>,
    pub pids_current: Option<u64>,
    pub pids_max: Option<i64>,
    pub pids_max_unlimited: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupEventController {
    Cpu,
    Memory,
    Pids,
}

impl CgroupEventController {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Pids => "pids",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupEventUnit {
    Count,
    Microseconds,
}

impl CgroupEventUnit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Microseconds => "microseconds",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupEventEntry {
    pub controller: CgroupEventController,
    pub event_name: String,
    pub cumulative_value: u64,
    pub unit: CgroupEventUnit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CgroupIoEntry {
    pub device_major: u32,
    pub device_minor: u32,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub discard_bytes: Option<u64>,
    pub read_operations: Option<u64>,
    pub write_operations: Option<u64>,
    pub discard_operations: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CgroupObservations {
    pub limits: CgroupLimits,
    pub events: Vec<CgroupEventEntry>,
    pub io: Vec<CgroupIoEntry>,
}

pub struct CgroupsCollector;

impl Collector for CgroupsCollector {
    type Output = CgroupObservations;

    fn source(&self) -> &'static str {
        "linux.cgroups"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        collect_cgroups()
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_cgroups(output, outcomes)
    }
}

fn collect_cgroups() -> (CgroupObservations, Vec<ComponentOutcome>) {
    let mut observations = CgroupObservations::default();
    let mut outcomes = Vec::new();

    collect_cgroup_limits(&mut observations, &mut outcomes);
    collect_cgroup_events(&mut observations, &mut outcomes);
    collect_cgroup_io(&mut observations, &mut outcomes);

    (observations, outcomes)
}

fn resolve_cgroup_v2_path() -> Option<PathBuf> {
    let content = read_bounded(Path::new("/proc/self/mountinfo")).ok()?;
    for line in content.lines() {
        if line.contains(" cgroup2 ") {
            let fields: Vec<&str> = line.split_whitespace().collect();
            for i in 0..fields.len().saturating_sub(1) {
                if fields[i] == "-" && fields.get(i + 1).copied() == Some("cgroup2") && i >= 5 {
                    return Some(PathBuf::from(fields[4]));
                }
            }
        }
    }
    None
}

fn resolve_cgroup_v2_self_path() -> Option<PathBuf> {
    let mount_root = resolve_cgroup_v2_path()?;
    let self_cgroup = read_bounded(Path::new("/proc/self/cgroup")).ok()?;
    for line in self_cgroup.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() == 3 && parts[1].is_empty() {
            let path = parts[2].trim();
            if path == "/" {
                return Some(mount_root);
            }
            return Some(mount_root.join(path.trim_start_matches('/')));
        }
    }
    Some(mount_root)
}

fn resolve_cgroup_v1_path(controller: &str) -> Option<PathBuf> {
    let self_cgroup = read_bounded(Path::new("/proc/self/cgroup")).ok()?;
    for line in self_cgroup.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() == 3 {
            let mut controllers = parts[1].split(',');
            if controllers.any(|c| c == controller) {
                let path = parts[2].trim();
                let mountinfo = read_bounded(Path::new("/proc/self/mountinfo")).ok()?;
                for mline in mountinfo.lines() {
                    let mfields: Vec<&str> = mline.split_whitespace().collect();
                    let mut sep_idx = 0;
                    for (i, field) in mfields.iter().enumerate() {
                        if *field == "-" {
                            sep_idx = i;
                            break;
                        }
                    }
                    if sep_idx + 1 < mfields.len()
                        && mfields[sep_idx + 1] == "cgroup"
                        && sep_idx + 2 < mfields.len()
                    {
                        let super_opts = mfields[sep_idx + 2];
                        if super_opts.split(',').any(|opt| opt == controller) {
                            let mount_point = mfields[4];
                            if path == "/" {
                                return Some(PathBuf::from(mount_point));
                            }
                            return Some(
                                PathBuf::from(mount_point).join(path.trim_start_matches('/')),
                            );
                        }
                    }
                }
            }
        }
    }
    None
}

fn is_cgroup_v2() -> bool {
    resolve_cgroup_v2_path().is_some()
}

fn collect_cgroup_limits(
    observations: &mut CgroupObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let v2 = is_cgroup_v2();

    if v2 {
        let path = match resolve_cgroup_v2_self_path() {
            Some(p) => p,
            None => {
                outcomes.push(
                    ComponentOutcome::failed(
                        CGROUPS_SOURCE,
                        CollectionStatus::Unsupported,
                        "cgroup_v2_path_unresolved",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return;
            }
        };

        let mut limits = CgroupLimits {
            version: Some(CgroupVersion::V2),
            ..Default::default()
        };

        if let Some(content) = read_cgroup_file_string(&path.join("cpu.max")) {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() == 2 {
                if parts[0] == "max" {
                    limits.cpu_quota_unlimited = Some(true);
                } else {
                    limits.cpu_quota_unlimited = Some(false);
                    limits.cpu_quota_microseconds = parts[0].parse().ok();
                }
                limits.cpu_period_microseconds = parts[1].parse().ok();
            }
        }

        limits.cpu_weight = read_cgroup_file_u64(&path.join("cpu.weight"));
        limits.memory_current_bytes = read_cgroup_file_u64(&path.join("memory.current"));
        limits.memory_min_bytes = read_cgroup_file_u64(&path.join("memory.min"));
        limits.memory_low_bytes = read_cgroup_file_u64(&path.join("memory.low"));

        if let Some(s) = read_cgroup_file_string(&path.join("memory.high")) {
            if s.trim() == "max" {
                limits.memory_high_unlimited = Some(true);
            } else {
                limits.memory_high_unlimited = Some(false);
                limits.memory_high_bytes = s.trim().parse().ok();
            }
        }

        if let Some(s) = read_cgroup_file_string(&path.join("memory.max")) {
            if s.trim() == "max" {
                limits.memory_max_unlimited = Some(true);
            } else {
                limits.memory_max_unlimited = Some(false);
                limits.memory_max_bytes = s.trim().parse().ok();
            }
        }

        if let Some(s) = read_cgroup_file_string(&path.join("memory.swap.max")) {
            if s.trim() == "max" {
                limits.swap_max_unlimited = Some(true);
            } else {
                limits.swap_max_unlimited = Some(false);
                limits.swap_max_bytes = s.trim().parse().ok();
            }
        }
        limits.swap_current_bytes = read_cgroup_file_u64(&path.join("memory.swap.current"));

        if let Some(s) = read_cgroup_file_string(&path.join("pids.max")) {
            if s.trim() == "max" {
                limits.pids_max_unlimited = Some(true);
            } else {
                limits.pids_max_unlimited = Some(false);
                limits.pids_max = s.trim().parse().ok();
            }
        }
        limits.pids_current = read_cgroup_file_u64(&path.join("pids.current"));

        let observed_at_utc = now_utc();
        observations.limits = limits;
        outcomes.push(ComponentOutcome::collected(
            CGROUPS_SOURCE,
            started_at_utc,
            now_utc(),
            observed_at_utc,
        ));
    } else {
        let mut limits = CgroupLimits {
            version: Some(CgroupVersion::V1),
            ..Default::default()
        };

        if let Some(p) = resolve_cgroup_v1_path("cpu") {
            if let Some(quota) = read_cgroup_file_signed(&p.join("cpu.cfs_quota_us")) {
                if quota == -1 {
                    limits.cpu_quota_unlimited = Some(true);
                } else {
                    limits.cpu_quota_unlimited = Some(false);
                    limits.cpu_quota_microseconds = Some(quota);
                }
            }
            limits.cpu_period_microseconds = read_cgroup_file_signed(&p.join("cpu.cfs_period_us"));
        }

        if let Some(p) = resolve_cgroup_v1_path("memory") {
            limits.memory_current_bytes = read_cgroup_file_u64(&p.join("memory.usage_in_bytes"));
            limits.memory_max_bytes = read_cgroup_file_u64(&p.join("memory.limit_in_bytes"));
            limits.memory_max_unlimited =
                limits.memory_max_bytes.map(|v| v >= 0x7FFF_FFFF_FFFF_F000);
            if let Some(p2) = resolve_cgroup_v1_path("memory") {
                limits.swap_current_bytes = read_cgroup_file_u64(&p2.join("memsw.usage_in_bytes"));
                limits.swap_max_bytes = read_cgroup_file_u64(&p2.join("memsw.limit_in_bytes"));
                limits.swap_max_unlimited =
                    limits.swap_max_bytes.map(|v| v >= 0x7FFF_FFFF_FFFF_F000);
            }
        }

        if let Some(p) = resolve_cgroup_v1_path("pids") {
            if let Some(s) = read_cgroup_file_string(&p.join("pids.max")) {
                if s.trim() == "max" {
                    limits.pids_max_unlimited = Some(true);
                } else {
                    limits.pids_max_unlimited = Some(false);
                    limits.pids_max = s.trim().parse().ok();
                }
            }
            limits.pids_current = read_cgroup_file_u64(&p.join("pids.current"));
        }

        let observed_at_utc = now_utc();
        observations.limits = limits;
        outcomes.push(ComponentOutcome::collected(
            CGROUPS_SOURCE,
            started_at_utc,
            now_utc(),
            observed_at_utc,
        ));
    }
}

fn collect_cgroup_events(
    observations: &mut CgroupObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let v2 = is_cgroup_v2();
    let mut events = Vec::new();

    if v2 {
        if let Some(path) = resolve_cgroup_v2_self_path() {
            if let Ok(content) = read_bounded(&path.join("cpu.stat")) {
                for line in content.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() != 2 {
                        continue;
                    }
                    let (name, value): (&str, u64) = match (parts[0], parts[1].parse()) {
                        (n, Ok(v)) => (n, v),
                        _ => continue,
                    };
                    let unit = if name.ends_with("_usec") {
                        CgroupEventUnit::Microseconds
                    } else {
                        CgroupEventUnit::Count
                    };
                    events.push(CgroupEventEntry {
                        controller: CgroupEventController::Cpu,
                        event_name: name.to_string(),
                        cumulative_value: value,
                        unit,
                    });
                }
            }

            if let Ok(content) = read_bounded(&path.join("memory.events")) {
                for line in content.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() != 2 {
                        continue;
                    }
                    if let Ok(value) = parts[1].parse::<u64>() {
                        events.push(CgroupEventEntry {
                            controller: CgroupEventController::Memory,
                            event_name: parts[0].to_string(),
                            cumulative_value: value,
                            unit: CgroupEventUnit::Count,
                        });
                    }
                }
            }

            if let Ok(content) = read_bounded(&path.join("pids.events")) {
                for line in content.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() != 2 {
                        continue;
                    }
                    if let Ok(value) = parts[1].parse::<u64>() {
                        events.push(CgroupEventEntry {
                            controller: CgroupEventController::Pids,
                            event_name: parts[0].to_string(),
                            cumulative_value: value,
                            unit: CgroupEventUnit::Count,
                        });
                    }
                }
            }
        }
    } else {
        if let Some(p) = resolve_cgroup_v1_path("cpu") {
            if let Ok(content) = read_bounded(&p.join("cpuacct.usage")) {
                if let Ok(value) = content.trim().parse::<u64>() {
                    events.push(CgroupEventEntry {
                        controller: CgroupEventController::Cpu,
                        event_name: "usage_us".to_string(),
                        cumulative_value: value,
                        unit: CgroupEventUnit::Microseconds,
                    });
                }
            }
        }
    }

    if events.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                CGROUP_EVENTS_SOURCE,
                CollectionStatus::Unsupported,
                "cgroup_events_not_available",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.events = events;
    outcomes.push(ComponentOutcome::collected(
        CGROUP_EVENTS_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_cgroup_io(observations: &mut CgroupObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let v2 = is_cgroup_v2();

    let io_file = if v2 {
        resolve_cgroup_v2_self_path().map(|p| p.join("io.stat"))
    } else {
        resolve_cgroup_v1_path("blkio").map(|p| p.join("blkio.io_service_bytes_recursive"))
    };

    let path = match io_file {
        Some(p) => p,
        None => {
            outcomes.push(
                ComponentOutcome::failed(
                    CGROUP_IO_SOURCE,
                    CollectionStatus::Unsupported,
                    "cgroup_io_not_available",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let content = match read_bounded(&path) {
        Ok(c) => c,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    CGROUP_IO_SOURCE,
                    status,
                    "cgroup_io_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let io_entries = if v2 {
        parse_cgroup_v2_io(&content)
    } else {
        parse_cgroup_v1_io(&content)
    };

    if io_entries.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                CGROUP_IO_SOURCE,
                CollectionStatus::Unsupported,
                "cgroup_io_empty",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.io = io_entries;
    outcomes.push(ComponentOutcome::collected(
        CGROUP_IO_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

pub fn parse_cgroup_v2_io(content: &str) -> Vec<CgroupIoEntry> {
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

        let (major, minor) = match parse_dev(parts[0]) {
            Some(pair) => pair,
            None => continue,
        };

        let mut entry = CgroupIoEntry {
            device_major: major,
            device_minor: minor,
            ..Default::default()
        };

        for token in &parts[1..] {
            if let Some((key, value)) = token.split_once('=') {
                let val = value.parse::<u64>().unwrap_or(0);
                match key {
                    "rbytes" => entry.read_bytes = Some(val),
                    "wbytes" => entry.write_bytes = Some(val),
                    "dbytes" => entry.discard_bytes = Some(val),
                    "rios" => entry.read_operations = Some(val),
                    "wios" => entry.write_operations = Some(val),
                    "dios" => entry.discard_operations = Some(val),
                    _ => {}
                }
            }
        }

        entries.push(entry);
    }

    entries
}

pub fn parse_cgroup_v1_io(content: &str) -> Vec<CgroupIoEntry> {
    let mut entries: std::collections::HashMap<(u32, u32), CgroupIoEntry> =
        std::collections::HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let (major, minor) = match parse_dev(parts[0]) {
            Some(pair) => pair,
            None => continue,
        };

        let key = parts[1];
        let value: u64 = match parts[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry = entries.entry((major, minor)).or_insert(CgroupIoEntry {
            device_major: major,
            device_minor: minor,
            ..Default::default()
        });

        match key {
            "Read" => entry.read_bytes = Some(value),
            "Write" => entry.write_bytes = Some(value),
            "Discard" => entry.discard_bytes = Some(value),
            _ => {}
        }
    }

    entries.into_values().collect()
}

fn parse_dev(token: &str) -> Option<(u32, u32)> {
    let (major, minor) = token.split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn read_cgroup_file_u64(path: &Path) -> Option<u64> {
    let s = read_cgroup_file_string(path)?;
    s.trim().parse().ok()
}

fn read_cgroup_file_signed(path: &Path) -> Option<i64> {
    let s = read_cgroup_file_string(path)?;
    s.trim().parse().ok()
}

fn read_cgroup_file_string(path: &Path) -> Option<String> {
    read_bounded(path).ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cgroup_v2_io_valid() {
        let content = "8:1 rbytes=1024 wbytes=2048 dbytes=512 rios=10 wios=20 dios=5\n8:2 rbytes=4096 wbytes=8192 rios=40 wios=80\n";
        let entries = parse_cgroup_v2_io(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].device_major, 8);
        assert_eq!(entries[0].device_minor, 1);
        assert_eq!(entries[0].read_bytes, Some(1024));
        assert_eq!(entries[0].write_bytes, Some(2048));
        assert_eq!(entries[0].discard_bytes, Some(512));
        assert_eq!(entries[0].read_operations, Some(10));
        assert_eq!(entries[0].write_operations, Some(20));
        assert_eq!(entries[0].discard_operations, Some(5));
        assert_eq!(entries[1].device_major, 8);
        assert_eq!(entries[1].device_minor, 2);
        assert_eq!(entries[1].read_bytes, Some(4096));
        assert_eq!(entries[1].discard_bytes, None);
    }

    #[test]
    fn parse_cgroup_v1_io_valid() {
        let content =
            "8:1 Read 1024\n8:1 Write 2048\n8:1 Discard 512\n8:2 Read 4096\n8:2 Write 8192\n";
        let entries = parse_cgroup_v1_io(content);
        assert_eq!(entries.len(), 2);
        let entry_1 = entries
            .iter()
            .find(|e| e.device_minor == 1)
            .expect("device 8:1 missing");
        assert_eq!(entry_1.device_major, 8);
        assert_eq!(entry_1.read_bytes, Some(1024));
        assert_eq!(entry_1.write_bytes, Some(2048));
        assert_eq!(entry_1.discard_bytes, Some(512));
        let entry_2 = entries
            .iter()
            .find(|e| e.device_minor == 2)
            .expect("device 8:2 missing");
        assert_eq!(entry_2.read_bytes, Some(4096));
        assert_eq!(entry_2.write_bytes, Some(8192));
    }

    #[test]
    fn parse_cgroup_v2_io_empty() {
        let entries = parse_cgroup_v2_io("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_dev_valid() {
        let (major, minor) = parse_dev("8:16").unwrap();
        assert_eq!(major, 8);
        assert_eq!(minor, 16);
    }

    #[test]
    fn parse_dev_invalid() {
        assert!(parse_dev("invalid").is_none());
        assert!(parse_dev("8").is_none());
    }

    #[test]
    fn cgroup_version_str() {
        assert_eq!(CgroupVersion::V1.as_str(), "v1");
        assert_eq!(CgroupVersion::V2.as_str(), "v2");
    }
}
