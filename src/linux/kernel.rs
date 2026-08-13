use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const KERNEL_VM_SOURCE: &str = "sys.kernel.vm";
const KERNEL_IPC_SOURCE: &str = "sys.kernel.ipc";
const THP_SOURCE: &str = "sys.kernel.thp";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelVmEntry {
    pub overcommit_mode: Option<i64>,
    pub overcommit_ratio_percent: Option<i64>,
    pub overcommit_kbytes_source: Option<i64>,
    pub overcommit_bytes: Option<i64>,
    pub max_map_count: Option<i64>,
    pub swappiness: Option<i64>,
    pub zone_reclaim_mode: Option<i64>,
    pub min_free_kbytes_source: Option<i64>,
    pub min_free_bytes: Option<i64>,
    pub dirty_background_bytes: Option<i64>,
    pub dirty_background_ratio_percent: Option<i64>,
    pub dirty_bytes: Option<i64>,
    pub dirty_ratio_percent: Option<i64>,
    pub dirty_expire_microseconds: Option<i64>,
    pub dirty_writeback_microseconds: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelIpcEntry {
    pub shmmax_bytes: Option<i64>,
    pub shmall_pages: Option<i64>,
    pub shmall_bytes: Option<i64>,
    pub shmmni: Option<i64>,
    pub semmsl: Option<i64>,
    pub semmns: Option<i64>,
    pub semopm: Option<i64>,
    pub semmni: Option<i64>,
    pub file_max: Option<i64>,
    pub nr_open: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThpEntry {
    pub enabled_policy: Option<String>,
    pub defrag_policy: Option<String>,
    pub khugepaged_defrag: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelObservations {
    pub vm: Option<KernelVmEntry>,
    pub ipc: Option<KernelIpcEntry>,
    pub thp: Option<ThpEntry>,
}

pub fn collect_kernel_isolated() -> (KernelObservations, Vec<ComponentOutcome>) {
    catch_unwind(AssertUnwindSafe(collect_kernel)).unwrap_or_else(|_| {
        (
            KernelObservations::default(),
            vec![ComponentOutcome::failed(
                "linux.kernel",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        )
    })
}

fn collect_kernel() -> (KernelObservations, Vec<ComponentOutcome>) {
    let mut observations = KernelObservations::default();
    let mut outcomes = Vec::new();

    collect_kernel_vm(&mut observations, &mut outcomes);
    collect_kernel_ipc(&mut observations, &mut outcomes);
    collect_thp(&mut observations, &mut outcomes);

    (observations, outcomes)
}

fn collect_kernel_vm(observations: &mut KernelObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let mut entry = KernelVmEntry::default();
    let mut any_ok = false;

    let vm_keys = [
        "overcommit_memory",
        "overcommit_ratio",
        "overcommit_kbytes",
        "max_map_count",
        "swappiness",
        "zone_reclaim_mode",
        "min_free_kbytes",
        "dirty_background_bytes",
        "dirty_background_ratio",
        "dirty_bytes",
        "dirty_ratio",
        "dirty_expire_centisecs",
        "dirty_writeback_centisecs",
    ];

    for key in vm_keys {
        let path = format!("/proc/sys/vm/{key}");
        let Ok(content) = read_bounded(Path::new(&path)) else {
            continue;
        };
        let Ok(value) = content.trim().parse::<i64>() else {
            continue;
        };
        any_ok = true;
        match key {
            "overcommit_memory" => entry.overcommit_mode = Some(value),
            "overcommit_ratio" => entry.overcommit_ratio_percent = Some(value),
            "overcommit_kbytes" => {
                entry.overcommit_kbytes_source = Some(value);
                entry.overcommit_bytes = Some(value.saturating_mul(1024));
            }
            "max_map_count" => entry.max_map_count = Some(value),
            "swappiness" => entry.swappiness = Some(value),
            "zone_reclaim_mode" => entry.zone_reclaim_mode = Some(value),
            "min_free_kbytes" => {
                entry.min_free_kbytes_source = Some(value);
                entry.min_free_bytes = Some(value.saturating_mul(1024));
            }
            "dirty_background_bytes" => entry.dirty_background_bytes = Some(value),
            "dirty_background_ratio" => entry.dirty_background_ratio_percent = Some(value),
            "dirty_bytes" => entry.dirty_bytes = Some(value),
            "dirty_ratio" => entry.dirty_ratio_percent = Some(value),
            "dirty_expire_centisecs" => {
                entry.dirty_expire_microseconds = Some(value.saturating_mul(10_000));
            }
            "dirty_writeback_centisecs" => {
                entry.dirty_writeback_microseconds = Some(value.saturating_mul(10_000));
            }
            _ => {}
        }
    }

    if !any_ok {
        outcomes.push(
            ComponentOutcome::failed(
                KERNEL_VM_SOURCE,
                CollectionStatus::Unsupported,
                "kernel_vm_not_available",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.vm = Some(entry);
    outcomes.push(ComponentOutcome::collected(
        KERNEL_VM_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_kernel_ipc(observations: &mut KernelObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let mut entry = KernelIpcEntry::default();
    let mut any_ok = false;

    if let Ok(content) = read_bounded(Path::new("/proc/sys/kernel/shmmax")) {
        if let Ok(value) = content.trim().parse::<i64>() {
            entry.shmmax_bytes = Some(value);
            any_ok = true;
        }
    }

    if let Ok(content) = read_bounded(Path::new("/proc/sys/kernel/shmall")) {
        if let Ok(value) = content.trim().parse::<i64>() {
            entry.shmall_pages = Some(value);
            entry.shmall_bytes = Some(value.saturating_mul(4096));
            any_ok = true;
        }
    }

    if let Ok(content) = read_bounded(Path::new("/proc/sys/kernel/shmmni")) {
        if let Ok(value) = content.trim().parse::<i64>() {
            entry.shmmni = Some(value);
            any_ok = true;
        }
    }

    if let Ok(content) = read_bounded(Path::new("/proc/sys/kernel/sem")) {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 4 {
            if let Ok(v) = parts[0].parse::<i64>() {
                entry.semmsl = Some(v);
                any_ok = true;
            }
            if let Ok(v) = parts[1].parse::<i64>() {
                entry.semmns = Some(v);
            }
            if let Ok(v) = parts[2].parse::<i64>() {
                entry.semopm = Some(v);
            }
            if let Ok(v) = parts[3].parse::<i64>() {
                entry.semmni = Some(v);
            }
        }
    }

    if let Ok(content) = read_bounded(Path::new("/proc/sys/fs/file-max")) {
        if let Ok(value) = content.trim().parse::<i64>() {
            entry.file_max = Some(value);
            any_ok = true;
        }
    }

    if let Ok(content) = read_bounded(Path::new("/proc/sys/fs/nr_open")) {
        if let Ok(value) = content.trim().parse::<i64>() {
            entry.nr_open = Some(value);
            any_ok = true;
        }
    }

    if !any_ok {
        outcomes.push(
            ComponentOutcome::failed(
                KERNEL_IPC_SOURCE,
                CollectionStatus::Unsupported,
                "kernel_ipc_not_available",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.ipc = Some(entry);
    outcomes.push(ComponentOutcome::collected(
        KERNEL_IPC_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_thp(observations: &mut KernelObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let enabled = read_bounded(Path::new("/sys/kernel/mm/transparent_hugepage/enabled"))
        .ok()
        .and_then(|c| parse_thp_policy(&c));

    let defrag = read_bounded(Path::new("/sys/kernel/mm/transparent_hugepage/defrag"))
        .ok()
        .and_then(|c| parse_thp_policy(&c));

    let khugepaged = read_bounded(Path::new(
        "/sys/kernel/mm/transparent_hugepage/khugepaged/defrag",
    ))
    .ok()
    .map(|c| c.trim() == "1");

    if enabled.is_none() && defrag.is_none() && khugepaged.is_none() {
        outcomes.push(
            ComponentOutcome::failed(
                THP_SOURCE,
                CollectionStatus::Unsupported,
                "thp_not_available",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.thp = Some(ThpEntry {
        enabled_policy: enabled,
        defrag_policy: defrag,
        khugepaged_defrag: khugepaged,
    });
    outcomes.push(ComponentOutcome::collected(
        THP_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

pub fn parse_thp_policy(content: &str) -> Option<String> {
    for token in content.split_whitespace() {
        if token.starts_with('[') && token.ends_with(']') {
            return Some(token[1..token.len() - 1].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_thp_policy_bracketed() {
        let content = "[always] madvise never\n";
        assert_eq!(parse_thp_policy(content), Some("always".to_string()));
    }

    #[test]
    fn parse_thp_policy_madvise() {
        let content = "always [madvise] never\n";
        assert_eq!(parse_thp_policy(content), Some("madvise".to_string()));
    }

    #[test]
    fn parse_thp_policy_never() {
        let content = "always madvise [never]\n";
        assert_eq!(parse_thp_policy(content), Some("never".to_string()));
    }

    #[test]
    fn parse_thp_policy_empty() {
        assert!(parse_thp_policy("").is_none());
    }
}
