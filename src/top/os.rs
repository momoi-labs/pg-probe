//! Host and per-backend metrics for `pg-probe top`, read from the local
//! `/proc`. They only make sense when the server runs on this host, and they
//! cost the server nothing: no query is sent.

use std::path::Path;

use crate::linux::{parse_loadavg, parse_meminfo, read_bounded, LoadAverages};

/// Host-wide memory, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub buff_cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Host {
    pub load: Option<LoadAverages>,
    pub memory: Option<Memory>,
}

/// Cumulative counters of one process at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessSample {
    /// User plus system CPU time, in clock ticks.
    pub cpu_ticks: u64,
    /// `None` when `/proc/<pid>/io` is not readable by this user.
    pub io: Option<(u64, u64)>,
    /// The process is in uninterruptible sleep, usually waiting on I/O.
    pub io_wait: bool,
}

pub fn read_host() -> Host {
    let load = read_bounded(Path::new("/proc/loadavg"))
        .ok()
        .and_then(|content| parse_loadavg(&content));
    let memory = read_bounded(Path::new("/proc/meminfo"))
        .ok()
        .and_then(|content| parse_meminfo(&content).ok())
        .map(|entries| {
            let value = |key: &str| {
                entries
                    .iter()
                    .find(|entry| entry.key == key)
                    .map(|entry| entry.value)
            };
            let buff_cached = value("Buffers").unwrap_or(0) + value("Cached").unwrap_or(0);
            Memory {
                total: value("MemTotal").unwrap_or(0),
                available: value("MemAvailable")
                    .unwrap_or_else(|| value("MemFree").unwrap_or(0) + buff_cached),
                buff_cached,
                swap_total: value("SwapTotal").unwrap_or(0),
                swap_free: value("SwapFree").unwrap_or(0),
            }
        });
    Host { load, memory }
}

/// Samples a backend process. Returns `None` when the pid is not a local
/// PostgreSQL process, for example when the server runs in another PID
/// namespace.
pub fn read_process(pid: &str) -> Option<ProcessSample> {
    let dir = postgres_process_dir(pid)?;
    let (io_wait, cpu_ticks) = parse_stat(&read_bounded(&dir.join("stat")).ok()?)?;
    let io = read_bounded(&dir.join("io"))
        .ok()
        .and_then(|content| parse_io(&content));
    Some(ProcessSample {
        cpu_ticks,
        io,
        io_wait,
    })
}

/// Resident memory of a backend, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemory {
    /// All resident pages, including the shared buffers this backend touched.
    pub rss: u64,
    /// Private memory only (kernel 4.5+), such as `work_mem` sorts and hashes.
    pub anonymous: Option<u64>,
}

pub fn read_memory(pid: &str) -> Option<ProcessMemory> {
    let dir = postgres_process_dir(pid)?;
    parse_status(&read_bounded(&dir.join("status")).ok()?)
}

fn postgres_process_dir(pid: &str) -> Option<std::path::PathBuf> {
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let dir = Path::new("/proc").join(pid);
    let comm = read_bounded(&dir.join("comm")).ok()?;
    matches!(comm.trim(), "postgres" | "postmaster").then_some(dir)
}

/// Reads `VmRSS` and `RssAnon` from `/proc/<pid>/status`, given in kB.
fn parse_status(content: &str) -> Option<ProcessMemory> {
    let kilobytes = |key: &str| {
        content
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value * 1024)
    };
    Some(ProcessMemory {
        rss: kilobytes("VmRSS:")?,
        anonymous: kilobytes("RssAnon:"),
    })
}

pub fn clock_ticks_per_second() -> f64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 {
        ticks as f64
    } else {
        100.0
    }
}

/// Reads the state and `utime + stime` from `/proc/<pid>/stat`. Fields are
/// counted after the closing parenthesis of `comm`, which may hold spaces.
fn parse_stat(content: &str) -> Option<(bool, u64)> {
    let fields: Vec<&str> = content
        .get(content.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    let state = fields.first()?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((*state == "D", utime + stime))
}

/// Reads `read_bytes` and `write_bytes`: bytes this process caused to be
/// fetched from or sent to the storage layer.
fn parse_io(content: &str) -> Option<(u64, u64)> {
    let value = |key: &str| {
        content
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.trim().parse::<u64>().ok())
    };
    Some((value("read_bytes:")?, value("write_bytes:")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_are_counted_after_the_command_name() {
        let stat = "4242 (postgres: a b) D 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 1 0";
        assert_eq!(parse_stat(stat), Some((true, 300)));
    }

    #[test]
    fn io_reads_storage_bytes() {
        let io = "rchar: 10\nwchar: 20\nsyscr: 1\nsyscw: 2\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n";
        assert_eq!(parse_io(io), Some((4096, 8192)));
    }

    #[test]
    fn status_reads_resident_memory_in_bytes() {
        let status = "Name:\tpostgres\nVmRSS:\t  2048 kB\nRssAnon:\t   512 kB\n";
        assert_eq!(
            parse_status(status),
            Some(ProcessMemory {
                rss: 2048 * 1024,
                anonymous: Some(512 * 1024),
            })
        );
        // Kernels before 4.5, such as CentOS 7, have no RssAnon.
        assert_eq!(
            parse_status("VmRSS:\t  2048 kB\n").map(|memory| memory.anonymous),
            Some(None)
        );
    }

    #[test]
    fn non_numeric_pids_are_rejected() {
        assert_eq!(read_process("../self"), None);
    }
}
