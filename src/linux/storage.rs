use std::fs;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const MOUNTS_SOURCE: &str = "proc.mounts";
const BLOCK_DEVICE_SOURCE: &str = "sys.block.devices";
const BLOCK_IO_SOURCE: &str = "proc.diskstats";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_id: u32,
    pub parent_mount_id: u32,
    pub device_major: u32,
    pub device_minor: u32,
    pub filesystem_type: String,
    pub is_read_only: bool,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub total_inodes: Option<u64>,
    pub available_inodes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDeviceEntry {
    pub device_major: u32,
    pub device_minor: u32,
    pub kernel_name: Option<String>,
    pub logical_block_size_bytes: Option<u64>,
    pub physical_block_size_bytes: Option<u64>,
    pub minimum_io_size_bytes: Option<u64>,
    pub optimal_io_size_bytes: Option<u64>,
    pub read_ahead_kibibytes: Option<u64>,
    pub request_queue_depth: Option<u64>,
    pub is_rotational: Option<bool>,
    pub scheduler: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockIoEntry {
    pub device_major: u32,
    pub device_minor: u32,
    pub reads_completed: u64,
    pub reads_merged: u64,
    pub sectors_read: u64,
    pub read_milliseconds: u64,
    pub writes_completed: u64,
    pub writes_merged: u64,
    pub sectors_written: u64,
    pub write_milliseconds: u64,
    pub io_in_progress: u64,
    pub io_milliseconds: u64,
    pub weighted_io_milliseconds: u64,
    pub discards_completed: Option<u64>,
    pub discard_sectors: Option<u64>,
    pub discard_milliseconds: Option<u64>,
    pub flushes_completed: Option<u64>,
    pub flush_milliseconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageObservations {
    pub mounts: Vec<MountEntry>,
    pub block_devices: Vec<BlockDeviceEntry>,
    pub block_io: Vec<BlockIoEntry>,
}

pub fn collect_storage_isolated() -> (StorageObservations, Vec<ComponentOutcome>) {
    catch_unwind(AssertUnwindSafe(collect_storage)).unwrap_or_else(|_| {
        (
            StorageObservations::default(),
            vec![ComponentOutcome::failed(
                "linux.storage",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        )
    })
}

fn collect_storage() -> (StorageObservations, Vec<ComponentOutcome>) {
    let mut observations = StorageObservations::default();
    let mut outcomes = Vec::new();

    collect_mounts(&mut observations, &mut outcomes);
    collect_block_devices(&mut observations, &mut outcomes);
    collect_block_io(&mut observations, &mut outcomes);

    (observations, outcomes)
}

fn collect_mounts(observations: &mut StorageObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/self/mountinfo")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    MOUNTS_SOURCE,
                    status,
                    "mountinfo_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_mountinfo(&content) {
        Ok(mounts) => {
            let observed_at_utc = now_utc();
            observations.mounts = mounts;
            outcomes.push(ComponentOutcome::collected(
                MOUNTS_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error_code) => {
            outcomes.push(
                ComponentOutcome::failed(
                    MOUNTS_SOURCE,
                    CollectionStatus::ParseError,
                    error_code,
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

fn collect_block_devices(
    observations: &mut StorageObservations,
    outcomes: &mut Vec<ComponentOutcome>,
) {
    let started_at_utc = now_utc();

    let entries = match fs::read_dir("/sys/block") {
        Ok(entries) => entries,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    BLOCK_DEVICE_SOURCE,
                    status,
                    "sys_block_readdir_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    let mut devices = Vec::new();
    let mut had_error = false;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if error.kind() == io::ErrorKind::NotFound {
                    continue;
                }
                had_error = true;
                break;
            }
        };

        let name = entry.file_name().to_string_lossy().to_string();

        let dev_path = entry.path().join("dev");
        let dev_content = match read_bounded(&dev_path) {
            Ok(content) => content,
            Err(_) => continue,
        };

        let (major, minor) = match parse_dev_file(&dev_content) {
            Some(pair) => pair,
            None => continue,
        };

        let base = entry.path();
        let kernel_name = Some(name.clone());

        let logical_block_size_bytes = read_sysfs_u64(&base.join("queue/logical_block_size"));
        let physical_block_size_bytes = read_sysfs_u64(&base.join("queue/physical_block_size"));
        let minimum_io_size_bytes = read_sysfs_u64(&base.join("queue/minimum_io_size"));
        let optimal_io_size_bytes = read_sysfs_u64(&base.join("queue/optimal_io_size"));
        let read_ahead_kibibytes = read_sysfs_u64(&base.join("queue/read_ahead_kb"));
        let request_queue_depth = read_sysfs_u64(&base.join("queue/nr_requests"));
        let is_rotational = read_sysfs_string(&base.join("queue/rotational"))
            .map(|s| s.trim() == "1")
            .ok();
        let scheduler = read_scheduler(&base.join("queue/scheduler"));

        devices.push(BlockDeviceEntry {
            device_major: major,
            device_minor: minor,
            kernel_name,
            logical_block_size_bytes,
            physical_block_size_bytes,
            minimum_io_size_bytes,
            optimal_io_size_bytes,
            read_ahead_kibibytes,
            request_queue_depth,
            is_rotational,
            scheduler,
        });
    }

    if had_error && devices.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                BLOCK_DEVICE_SOURCE,
                CollectionStatus::IoError,
                "block_device_read_failed",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return;
    }

    let observed_at_utc = now_utc();
    observations.block_devices = devices;
    outcomes.push(ComponentOutcome::collected(
        BLOCK_DEVICE_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));
}

fn collect_block_io(observations: &mut StorageObservations, outcomes: &mut Vec<ComponentOutcome>) {
    let started_at_utc = now_utc();

    let content = match read_bounded(Path::new("/proc/diskstats")) {
        Ok(content) => content,
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                _ => CollectionStatus::IoError,
            };
            outcomes.push(
                ComponentOutcome::failed(
                    BLOCK_IO_SOURCE,
                    status,
                    "diskstats_read_failed",
                    error.raw_os_error(),
                )
                .with_timing(started_at_utc, now_utc()),
            );
            return;
        }
    };

    match parse_diskstats(&content) {
        Ok(entries) => {
            let observed_at_utc = now_utc();
            observations.block_io = entries;
            outcomes.push(ComponentOutcome::collected(
                BLOCK_IO_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error_code) => {
            outcomes.push(
                ComponentOutcome::failed(
                    BLOCK_IO_SOURCE,
                    CollectionStatus::ParseError,
                    error_code,
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            );
        }
    }
}

pub fn parse_mountinfo(content: &str) -> Result<Vec<MountEntry>, &'static str> {
    let mut mounts = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }

        let mount_id: u32 = parts[0].parse().map_err(|_| "invalid_mount_id")?;
        let parent_mount_id: u32 = parts[1].parse().map_err(|_| "invalid_parent_mount_id")?;

        let (major, minor) = parse_dev_file(parts[2]).ok_or("invalid_mount_device")?;

        // mountinfo format: mount_id parent dev root mount_point mount_options optional... - fstype source super_options
        let mount_options = parts[5];

        let mut separator_idx = 6;
        for (i, field) in parts.iter().enumerate().skip(6) {
            if *field == "-" {
                separator_idx = i;
                break;
            }
        }

        if separator_idx + 1 >= parts.len() {
            return Err("invalid_mountinfo_format");
        }

        let filesystem_type = parts[separator_idx + 1].to_string();

        let is_read_only = mount_options.split(',').any(|opt| opt == "ro");

        mounts.push(MountEntry {
            mount_id,
            parent_mount_id,
            device_major: major,
            device_minor: minor,
            filesystem_type,
            is_read_only,
            total_bytes: None,
            available_bytes: None,
            total_inodes: None,
            available_inodes: None,
        });
    }

    Ok(mounts)
}

pub fn parse_dev_file(content: &str) -> Option<(u32, u32)> {
    let content = content.trim();
    let (major_str, minor_str) = content.split_once(':')?;
    let major: u32 = major_str.trim().parse().ok()?;
    let minor: u32 = minor_str.trim().parse().ok()?;
    Some((major, minor))
}

pub fn parse_diskstats(content: &str) -> Result<Vec<BlockIoEntry>, &'static str> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 14 {
            continue;
        }

        let major: u32 = parts[0].parse().map_err(|_| "invalid_diskstats_major")?;
        let minor: u32 = parts[1].parse().map_err(|_| "invalid_diskstats_minor")?;

        let reads_completed: u64 = parts[3].parse().map_err(|_| "invalid_reads_completed")?;
        let reads_merged: u64 = parts[4].parse().map_err(|_| "invalid_reads_merged")?;
        let sectors_read: u64 = parts[5].parse().map_err(|_| "invalid_sectors_read")?;
        let read_milliseconds: u64 = parts[6].parse().map_err(|_| "invalid_read_ms")?;
        let writes_completed: u64 = parts[7].parse().map_err(|_| "invalid_writes_completed")?;
        let writes_merged: u64 = parts[8].parse().map_err(|_| "invalid_writes_merged")?;
        let sectors_written: u64 = parts[9].parse().map_err(|_| "invalid_sectors_written")?;
        let write_milliseconds: u64 = parts[10].parse().map_err(|_| "invalid_write_ms")?;
        let io_in_progress: u64 = parts[11].parse().map_err(|_| "invalid_io_in_progress")?;
        let io_milliseconds: u64 = parts[12].parse().map_err(|_| "invalid_io_ms")?;
        let weighted_io_milliseconds: u64 =
            parts[13].parse().map_err(|_| "invalid_weighted_io_ms")?;

        let discards_completed = parts.get(14).and_then(|s| s.parse().ok());
        let discard_sectors = parts.get(16).and_then(|s| s.parse().ok());
        let discard_milliseconds = parts.get(17).and_then(|s| s.parse().ok());
        let flushes_completed = parts.get(18).and_then(|s| s.parse().ok());
        let flush_milliseconds = parts.get(19).and_then(|s| s.parse().ok());

        entries.push(BlockIoEntry {
            device_major: major,
            device_minor: minor,
            reads_completed,
            reads_merged,
            sectors_read,
            read_milliseconds,
            writes_completed,
            writes_merged,
            sectors_written,
            write_milliseconds,
            io_in_progress,
            io_milliseconds,
            weighted_io_milliseconds,
            discards_completed,
            discard_sectors,
            discard_milliseconds,
            flushes_completed,
            flush_milliseconds,
        });
    }

    Ok(entries)
}

fn read_scheduler(path: &Path) -> Option<String> {
    let content = read_sysfs_string(path).ok()?;
    let content = content.trim();
    for entry in content.split_whitespace() {
        if entry.starts_with('[') && entry.ends_with(']') {
            return Some(entry[1..entry.len() - 1].to_string());
        }
    }
    None
}

fn read_sysfs_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_sysfs_string(path: &Path) -> io::Result<String> {
    let content = read_bounded(path)?;
    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mountinfo_valid() {
        let content = "23 0 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n24 23 0:27 / /proc ro,nosuid,nodev,noexec,relatime - proc proc ro\n";
        let mounts = parse_mountinfo(content).unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].mount_id, 23);
        assert_eq!(mounts[0].parent_mount_id, 0);
        assert_eq!(mounts[0].device_major, 8);
        assert_eq!(mounts[0].device_minor, 1);
        assert_eq!(mounts[0].filesystem_type, "ext4");
        assert!(!mounts[0].is_read_only);
        assert_eq!(mounts[1].mount_id, 24);
        assert_eq!(mounts[1].device_major, 0);
        assert_eq!(mounts[1].device_minor, 27);
        assert_eq!(mounts[1].filesystem_type, "proc");
        assert!(mounts[1].is_read_only);
    }

    #[test]
    fn parse_mountinfo_read_only() {
        let content = "25 23 8:2 / /mnt/ro ro,relatime - ext4 /dev/sda2 ro\n";
        let mounts = parse_mountinfo(content).unwrap();
        assert_eq!(mounts.len(), 1);
        assert!(mounts[0].is_read_only);
    }

    #[test]
    fn parse_mountinfo_empty() {
        let mounts = parse_mountinfo("").unwrap();
        assert!(mounts.is_empty());
    }

    #[test]
    fn parse_dev_file_valid() {
        let (major, minor) = parse_dev_file("8:1").unwrap();
        assert_eq!(major, 8);
        assert_eq!(minor, 1);
    }

    #[test]
    fn parse_dev_file_with_whitespace() {
        let (major, minor) = parse_dev_file("  8 : 16  \n").unwrap();
        assert_eq!(major, 8);
        assert_eq!(minor, 16);
    }

    #[test]
    fn parse_dev_file_invalid() {
        assert!(parse_dev_file("invalid").is_none());
        assert!(parse_dev_file("8").is_none());
        assert!(parse_dev_file("").is_none());
    }

    #[test]
    fn parse_diskstats_basic() {
        let content = "   8       1 sda1 123 45 6789 100 200 50 5432 80 0 300 400\n";
        let entries = parse_diskstats(content).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.device_major, 8);
        assert_eq!(entry.device_minor, 1);
        assert_eq!(entry.reads_completed, 123);
        assert_eq!(entry.reads_merged, 45);
        assert_eq!(entry.sectors_read, 6789);
        assert_eq!(entry.read_milliseconds, 100);
        assert_eq!(entry.writes_completed, 200);
        assert_eq!(entry.writes_merged, 50);
        assert_eq!(entry.sectors_written, 5432);
        assert_eq!(entry.write_milliseconds, 80);
        assert_eq!(entry.io_in_progress, 0);
        assert_eq!(entry.io_milliseconds, 300);
        assert_eq!(entry.weighted_io_milliseconds, 400);
        assert!(entry.discards_completed.is_none());
        assert!(entry.flushes_completed.is_none());
    }

    #[test]
    fn parse_diskstats_with_discards_and_flushes() {
        // Format: major minor name reads_comp reads_merged sectors_read read_ms
        //         writes_comp writes_merged sectors_written write_ms io_in_prog
        //         io_ms weighted_io_ms discards_comp discards_merged
        //         discard_sectors discard_ms flushes_comp flush_ms
        let content =
            " 259 0 nvme0n1 100 10 2000 50 200 20 4000 100 0 150 250 50 100 5000 60 30 400\n";
        let entries = parse_diskstats(content).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.device_major, 259);
        assert_eq!(entry.device_minor, 0);
        assert_eq!(entry.discards_completed, Some(50));
        assert_eq!(entry.discard_sectors, Some(5000));
        assert_eq!(entry.discard_milliseconds, Some(60));
        assert_eq!(entry.flushes_completed, Some(30));
        assert_eq!(entry.flush_milliseconds, Some(400));
    }

    #[test]
    fn parse_diskstats_empty() {
        let entries = parse_diskstats("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_diskstats_skips_short_lines() {
        let content = "   8       1 sda1 123 45\n";
        let entries = parse_diskstats(content).unwrap();
        assert!(entries.is_empty());
    }
}
