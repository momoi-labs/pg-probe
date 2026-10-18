use std::fs;
use std::io;
use std::path::Path;

use crate::collector::{Collector, Sink};
use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const CPU_ONLINE_SOURCE: &str = "sys.cpu.online";
const CPU_PRESENT_SOURCE: &str = "sys.cpu.present";
const CPU_POSSIBLE_SOURCE: &str = "sys.cpu.possible";
const CPU_AFFINITY_SOURCE: &str = "sched.cpu.affinity";
const CPU_TOPOLOGY_SOURCE: &str = "sys.cpu.topology";
const NUMA_TOPOLOGY_SOURCE: &str = "sys.numa.topology";
const CPU_TIME_SOURCE: &str = "proc.stat.cpu";
const SCHEDULER_SOURCE: &str = "proc.stat.scheduler";
const LOAD_AVG_SOURCE: &str = "proc.loadavg";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuSet {
    pub cpus: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuTopologyEntry {
    pub cpu_id: u32,
    pub package_id: Option<u32>,
    pub die_id: Option<u32>,
    pub core_id: Option<u32>,
    pub thread_siblings: Option<CpuSet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumaNode {
    pub node_id: u32,
    pub cpus: CpuSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuTimeEntry {
    pub cpu_id: Option<u32>,
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub guest: u64,
    pub guest_nice: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerCounters {
    pub procs_running: u64,
    pub procs_blocked: u64,
    pub ctxt: u64,
    pub processes: u64,
    pub intr: u64,
    pub softirq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadAverages {
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub runnable_tasks: u32,
    pub total_tasks: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputeObservations {
    pub online: Option<CpuSet>,
    pub present: Option<CpuSet>,
    pub possible: Option<CpuSet>,
    pub affinity: Option<CpuSet>,
    pub topology: Vec<CpuTopologyEntry>,
    pub numa_nodes: Vec<NumaNode>,
    pub cpu_times: Vec<CpuTimeEntry>,
    pub scheduler: Option<SchedulerCounters>,
    pub load_averages: Option<LoadAverages>,
}

pub struct ComputeCollector;

impl Collector for ComputeCollector {
    type Output = ComputeObservations;

    fn source(&self) -> &'static str {
        "linux.compute"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        collect_compute()
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_compute(output, outcomes)
    }
}

fn collect_compute() -> (ComputeObservations, Vec<ComponentOutcome>) {
    let mut observations = ComputeObservations::default();
    let mut outcomes = Vec::new();

    collect_cpu_sets(&mut observations, &mut outcomes);
    collect_cpu_affinity(&mut observations, &mut outcomes);
    collect_cpu_topology(&mut observations, &mut outcomes);
    collect_numa_topology(&mut observations, &mut outcomes);
    collect_cpu_times(&mut observations, &mut outcomes);
    collect_scheduler(&mut observations, &mut outcomes);
    collect_load_averages(&mut observations, &mut outcomes);

    (observations, outcomes)
}

fn collect_cpu_sets(observations: &mut ComputeObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let sets = [
        ("online", CPU_ONLINE_SOURCE, &mut observations.online),
        ("present", CPU_PRESENT_SOURCE, &mut observations.present),
        ("possible", CPU_POSSIBLE_SOURCE, &mut observations.possible),
    ];

    for (name, source, field) in sets {
        let started_at_utc = now_utc();
        let path = format!("/sys/devices/system/cpu/{name}");
        match read_bounded(Path::new(&path)) {
            Ok(content) => match parse_cpu_set(content.trim()) {
                Ok(cpu_set) => {
                    let observed_at_utc = now_utc();
                    *field = Some(cpu_set);
                    outcomes.push(ComponentOutcome::collected(
                        source,
                        started_at_utc,
                        now_utc(),
                        observed_at_utc,
                    ));
                }
                Err(error_code) => {
                    outcomes.push(
                        ComponentOutcome::failed(
                            source,
                            CollectionStatus::ParseError,
                            error_code,
                            None,
                        )
                        .with_timing(started_at_utc, now_utc()),
                    );
                }
            },
            Err(error) => {
                let status = if error.kind() == io::ErrorKind::NotFound {
                    CollectionStatus::Unsupported
                } else if error.kind() == io::ErrorKind::PermissionDenied {
                    CollectionStatus::PermissionDenied
                } else {
                    CollectionStatus::IoError
                };
                outcomes.push(
                    ComponentOutcome::failed(
                        source,
                        status,
                        "cpu_set_read_failed",
                        error.raw_os_error(),
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
            }
        }
    }
}

fn collect_cpu_affinity(
    observations: &mut ComputeObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/self/status")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    CPU_AFFINITY_SOURCE,
                    status,
                    "proc_self_status_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let cpus_allowed_list = content
        .lines()
        .find(|line| line.starts_with("Cpus_allowed_list:"))
        .and_then(|line| line.split(':').nth(1))
        .map(|s| s.trim());

    match cpus_allowed_list {
        Some(list) => match parse_cpu_set(list) {
            Ok(cpu_set) => {
                let observed_at_utc = now_utc();
                observations.affinity = Some(cpu_set);
                outcomes.push(ComponentOutcome::collected(
                    CPU_AFFINITY_SOURCE,
                    started_at_utc,
                    now_utc(),
                    observed_at_utc,
                ));
            }
            Err(error_code) => {
                outcomes.push(
                    ComponentOutcome::failed(
                        CPU_AFFINITY_SOURCE,
                        CollectionStatus::ParseError,
                        error_code,
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
            }
        },
        None => {
            outcomes.push(
                ComponentOutcome::failed(
                    CPU_AFFINITY_SOURCE,
                    CollectionStatus::ParseError,
                    "cpus_allowed_list_not_found",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn collect_cpu_topology(
    observations: &mut ComputeObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let cpu_online = match observations.online.as_ref() {
        Some(set) => set.cpus.clone(),
        None => {
            outcomes.push(
                ComponentOutcome::failed(
                    CPU_TOPOLOGY_SOURCE,
                    CollectionStatus::Unsupported,
                    "cpu_online_unavailable",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let mut topology = Vec::new();
    let mut had_error = false;

    for cpu_id in cpu_online {
        let base_path = format!("/sys/devices/system/cpu/cpu{cpu_id}/topology");

        let package_id = read_sysfs_u32(&format!("{base_path}/physical_package_id"));
        let die_id = read_sysfs_u32(&format!("{base_path}/die_id"));
        let core_id = read_sysfs_u32(&format!("{base_path}/core_id"));

        let thread_siblings =
            match read_bounded(Path::new(&format!("{base_path}/thread_siblings_list"))) {
                Ok(content) => parse_cpu_set(content.trim()).ok(),
                Err(_) => None,
            };

        if package_id.is_none()
            && die_id.is_none()
            && core_id.is_none()
            && thread_siblings.is_none()
        {
            had_error = true;
            break;
        }

        topology.push(CpuTopologyEntry {
            cpu_id,
            package_id,
            die_id,
            core_id,
            thread_siblings,
        });
    }

    if had_error || topology.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                CPU_TOPOLOGY_SOURCE,
                CollectionStatus::IoError,
                "topology_read_failed",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.topology = topology;
    outcomes.push(ComponentOutcome::collected(
        CPU_TOPOLOGY_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_numa_topology(
    observations: &mut ComputeObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let online_content = match read_bounded(Path::new("/sys/devices/system/node/online")) {
        Ok(content) => content,
        Err(error) => {
            let status = if error.kind() == io::ErrorKind::NotFound {
                CollectionStatus::Unsupported
            } else {
                CollectionStatus::IoError
            };
            outcomes.push(
                ComponentOutcome::failed(
                    NUMA_TOPOLOGY_SOURCE,
                    status,
                    "numa_online_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let node_ids = match parse_cpu_set(online_content.trim()) {
        Ok(set) => set.cpus,
        Err(_) => {
            outcomes.push(
                ComponentOutcome::failed(
                    NUMA_TOPOLOGY_SOURCE,
                    CollectionStatus::ParseError,
                    "invalid_numa_online",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let mut nodes = Vec::new();
    for node_id in node_ids {
        let cpulist_path = format!("/sys/devices/system/node/node{node_id}/cpulist");
        match read_bounded(Path::new(&cpulist_path)) {
            Ok(content) => match parse_cpu_set(content.trim()) {
                Ok(cpus) => nodes.push(NumaNode { node_id, cpus }),
                Err(_) => {
                    outcomes.push(
                        ComponentOutcome::failed(
                            NUMA_TOPOLOGY_SOURCE,
                            CollectionStatus::ParseError,
                            "invalid_numa_cpulist",
                            None,
                        )
                        .with_timing(started_at_utc, now_utc()),
                    );
                    return;
                }
            },
            Err(_) => {
                outcomes.push(
                    ComponentOutcome::failed(
                        NUMA_TOPOLOGY_SOURCE,
                        CollectionStatus::IoError,
                        "numa_cpulist_read_failed",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return;
            }
        }
    }

    let observed_at_utc = now_utc();
    observations.numa_nodes = nodes;
    outcomes.push(ComponentOutcome::collected(
        NUMA_TOPOLOGY_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_cpu_times(observations: &mut ComputeObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/stat")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    CPU_TIME_SOURCE,
                    status,
                    "proc_stat_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let mut cpu_times = Vec::new();
    for line in content.lines() {
        if !line.starts_with("cpu") {
            continue;
        }

        match parse_cpu_time_line(line) {
            Some(entry) => cpu_times.push(entry),
            None => {
                outcomes.push(
                    ComponentOutcome::failed(
                        CPU_TIME_SOURCE,
                        CollectionStatus::ParseError,
                        "invalid_cpu_time_line",
                        None,
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return;
            }
        }
    }

    if cpu_times.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                CPU_TIME_SOURCE,
                CollectionStatus::ParseError,
                "no_cpu_time_entries",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.cpu_times = cpu_times;
    outcomes.push(ComponentOutcome::collected(
        CPU_TIME_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_scheduler(observations: &mut ComputeObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/stat")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    SCHEDULER_SOURCE,
                    status,
                    "proc_stat_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let mut procs_running = None;
    let mut procs_blocked = None;
    let mut ctxt = None;
    let mut processes = None;
    let mut intr = None;
    let mut softirq = None;

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        match parts[0] {
            "procs_running" => procs_running = parts[1].parse().ok(),
            "procs_blocked" => procs_blocked = parts[1].parse().ok(),
            "ctxt" => ctxt = parts[1].parse().ok(),
            "processes" => processes = parts[1].parse().ok(),
            "intr" => intr = parts[1].parse().ok(),
            "softirq" => softirq = parts[1].parse().ok(),
            _ => {}
        }
    }

    match (procs_running, procs_blocked, ctxt, processes, intr, softirq) {
        (Some(pr), Some(pb), Some(ct), Some(proc_count), Some(intr_val), Some(si)) => {
            let observed_at_utc = now_utc();
            observations.scheduler = Some(SchedulerCounters {
                procs_running: pr,
                procs_blocked: pb,
                ctxt: ct,
                processes: proc_count,
                intr: intr_val,
                softirq: si,
            });
            outcomes.push(ComponentOutcome::collected(
                SCHEDULER_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        _ => {
            outcomes.push(
                ComponentOutcome::failed(
                    SCHEDULER_SOURCE,
                    CollectionStatus::ParseError,
                    "missing_scheduler_fields",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn collect_load_averages(
    observations: &mut ComputeObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/loadavg")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    LOAD_AVG_SOURCE,
                    status,
                    "loadavg_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_loadavg(&content) {
        Some(load_avg) => {
            let observed_at_utc = now_utc();
            observations.load_averages = Some(load_avg);
            outcomes.push(ComponentOutcome::collected(
                LOAD_AVG_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        None => {
            outcomes.push(
                ComponentOutcome::failed(
                    LOAD_AVG_SOURCE,
                    CollectionStatus::ParseError,
                    "invalid_loadavg",
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn read_sysfs_u32(path: &str) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn parse_cpu_set(content: &str) -> Result<CpuSet, &'static str> {
    if content.is_empty() {
        return Ok(CpuSet { cpus: Vec::new() });
    }

    let mut cpus = Vec::new();
    for part in content.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((start, end)) = part.split_once('-') {
            let start: u32 = start.parse().map_err(|_| "invalid_cpu_range_start")?;
            let end: u32 = end.parse().map_err(|_| "invalid_cpu_range_end")?;
            if start > end {
                return Err("invalid_cpu_range_order");
            }
            for cpu in start..=end {
                cpus.push(cpu);
            }
        } else {
            let cpu: u32 = part.parse().map_err(|_| "invalid_cpu_id")?;
            cpus.push(cpu);
        }
    }

    cpus.sort_unstable();
    cpus.dedup();
    Ok(CpuSet { cpus })
}

fn parse_cpu_time_line(line: &str) -> Option<CpuTimeEntry> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() || !parts[0].starts_with("cpu") {
        return None;
    }

    let cpu_id = if parts[0] == "cpu" {
        None
    } else {
        Some(parts[0][3..].parse::<u32>().ok()?)
    };

    if parts.len() < 8 {
        return None;
    }

    let user = parts[1].parse().ok()?;
    let nice = parts[2].parse().ok()?;
    let system = parts[3].parse().ok()?;
    let idle = parts[4].parse().ok()?;
    let iowait = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
    let irq = parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
    let softirq = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
    let steal = parts.get(8).and_then(|s| s.parse().ok()).unwrap_or(0);
    let guest = parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(0);
    let guest_nice = parts.get(10).and_then(|s| s.parse().ok()).unwrap_or(0);

    Some(CpuTimeEntry {
        cpu_id,
        user,
        nice,
        system,
        idle,
        iowait,
        irq,
        softirq,
        steal,
        guest,
        guest_nice,
    })
}

pub fn parse_loadavg(content: &str) -> Option<LoadAverages> {
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let load_1m = parts[0].parse().ok()?;
    let load_5m = parts[1].parse().ok()?;
    let load_15m = parts[2].parse().ok()?;

    let task_parts: Vec<&str> = parts[3].split('/').collect();
    if task_parts.len() != 2 {
        return None;
    }

    let runnable_tasks = task_parts[0].parse().ok()?;
    let total_tasks = task_parts[1].parse().ok()?;

    Some(LoadAverages {
        load_1m,
        load_5m,
        load_15m,
        runnable_tasks,
        total_tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_set_single_values() {
        let set = parse_cpu_set("0,2,4").unwrap();
        assert_eq!(set.cpus, vec![0, 2, 4]);
    }

    #[test]
    fn parse_cpu_set_ranges() {
        let set = parse_cpu_set("0-3,7").unwrap();
        assert_eq!(set.cpus, vec![0, 1, 2, 3, 7]);
    }

    #[test]
    fn parse_cpu_set_mixed() {
        let set = parse_cpu_set("0,2-4,7,9-11").unwrap();
        assert_eq!(set.cpus, vec![0, 2, 3, 4, 7, 9, 10, 11]);
    }

    #[test]
    fn parse_cpu_set_empty() {
        let set = parse_cpu_set("").unwrap();
        assert_eq!(set.cpus, Vec::<u32>::new());
    }

    #[test]
    fn parse_cpu_set_deduplicates() {
        let set = parse_cpu_set("0,1,0,2-3,2").unwrap();
        assert_eq!(set.cpus, vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_cpu_time_aggregate() {
        let line = "cpu  10132153 290 3882439 512658173 73129 0 1234 0 0 0";
        let entry = parse_cpu_time_line(line).unwrap();
        assert_eq!(entry.cpu_id, None);
        assert_eq!(entry.user, 10132153);
        assert_eq!(entry.nice, 290);
        assert_eq!(entry.system, 3882439);
        assert_eq!(entry.idle, 512658173);
        assert_eq!(entry.iowait, 73129);
        assert_eq!(entry.irq, 0);
        assert_eq!(entry.softirq, 1234);
    }

    #[test]
    fn parse_cpu_time_per_cpu() {
        let line = "cpu0 1234567 89 123456 78901234 1234 0 567 0 0 0";
        let entry = parse_cpu_time_line(line).unwrap();
        assert_eq!(entry.cpu_id, Some(0));
        assert_eq!(entry.user, 1234567);
    }

    #[test]
    fn parse_loadavg_valid() {
        let content = "0.37 0.52 0.61 2/345 12345";
        let load = parse_loadavg(content).unwrap();
        assert_eq!(load.load_1m, 0.37);
        assert_eq!(load.load_5m, 0.52);
        assert_eq!(load.load_15m, 0.61);
        assert_eq!(load.runnable_tasks, 2);
        assert_eq!(load.total_tasks, 345);
    }
}
