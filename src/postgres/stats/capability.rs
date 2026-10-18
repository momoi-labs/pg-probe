//! PostgreSQL version capability detection.
//!
//! Different PostgreSQL majors expose different statistics views and columns.
//! This module detects what's available and selects the appropriate queries
//! without querying absent objects, which would fail and make the artifact
//! partial.

/// Minimum PostgreSQL major version that supports a given statistics view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsCapability {
    /// The view or column does not exist in this PostgreSQL version.
    Unsupported,
    /// The view or column exists and can be queried.
    Supported,
}

impl StatsCapability {
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// Detect whether `pg_stat_checkpointer` exists (PG 17+).
pub fn has_checkpointer_stats(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 170000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_io` exists (PG 16+).
pub fn has_io_stats(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 160000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_wal` exists (PG 14+).
pub fn has_wal_stats(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 140000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_slru` exists (PG 13+).
pub fn has_slru_stats(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 130000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_database.stats_reset` exists (PG 11+).
pub fn has_stats_reset(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 110000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_replication_slots` exists (PG 9.4+).
pub fn has_replication_slots(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 90400 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_archiver` exists (PG 9.4+).
pub fn has_archiver_stats(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 90400 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_activity.backend_type` exists (PG 10+).
pub fn has_backend_type(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 100000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_stat_activity.wait_event_type` exists (PG 9.6+).
pub fn has_wait_event_type(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 90600 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether `pg_locks.waitstart` exists (PG 14+).
pub fn has_lock_waitstart(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 140000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

/// Detect whether the `pg_*_wal_lsn*` functions and `*_lsn` columns exist
/// (PG 10+). Older servers use the `xlog`/`location` names.
pub fn has_wal_lsn_naming(server_version_num: i32) -> StatsCapability {
    if server_version_num >= 100000 {
        StatsCapability::Supported
    } else {
        StatsCapability::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_detection_handles_maintained_versions() {
        assert_eq!(has_checkpointer_stats(170000), StatsCapability::Supported);
        assert_eq!(has_checkpointer_stats(160000), StatsCapability::Unsupported);
        assert_eq!(has_io_stats(160000), StatsCapability::Supported);
        assert_eq!(has_io_stats(150000), StatsCapability::Unsupported);
        assert_eq!(has_wal_stats(140000), StatsCapability::Supported);
        assert_eq!(has_wal_stats(130000), StatsCapability::Unsupported);
        assert_eq!(has_slru_stats(130000), StatsCapability::Supported);
        assert_eq!(has_slru_stats(120000), StatsCapability::Unsupported);
        assert_eq!(has_archiver_stats(90400), StatsCapability::Supported);
        assert_eq!(has_archiver_stats(90305), StatsCapability::Unsupported);
        assert_eq!(has_backend_type(100000), StatsCapability::Supported);
        assert_eq!(has_backend_type(90600), StatsCapability::Unsupported);
        assert_eq!(has_wait_event_type(90600), StatsCapability::Supported);
        assert_eq!(has_wait_event_type(90500), StatsCapability::Unsupported);
        assert_eq!(has_lock_waitstart(140000), StatsCapability::Supported);
        assert_eq!(has_lock_waitstart(130000), StatsCapability::Unsupported);
        assert_eq!(has_wal_lsn_naming(100000), StatsCapability::Supported);
        assert_eq!(has_wal_lsn_naming(90600), StatsCapability::Unsupported);
    }
}
