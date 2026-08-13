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
    }
}
