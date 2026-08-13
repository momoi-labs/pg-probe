//! Approved PostgreSQL runtime-setting allowlist.
//!
//! Issue #16 only authorizes capturing a small, well-defined slice of
//! `pg_settings`. Anything outside this list must be skipped so unsafe or
//! sensitive parameters (paths, secrets, network endpoints, lists) never
//! reach the artifact.

/// Setting identifier used in `pg_settings.name` to look up the parameter.
pub const SERVER_VERSION: &str = "server_version";
pub const SERVER_VERSION_NUM: &str = "server_version_num";
pub const BLOCK_SIZE: &str = "block_size";
pub const DATA_CHECKSUMS: &str = "data_checksums";
pub const IN_RECOVERY: &str = "in_recovery";
pub const WAL_LEVEL: &str = "wal_level";
pub const MAX_CONNECTIONS: &str = "max_connections";
pub const SHARED_BUFFERS: &str = "shared_buffers";
pub const WORK_MEM: &str = "work_mem";
pub const MAINTENANCE_WORK_MEM: &str = "maintenance_work_mem";
pub const EFFECTIVE_CACHE_SIZE: &str = "effective_cache_size";
pub const RANDOM_PAGE_COST: &str = "random_page_cost";
pub const SEQ_PAGE_COST: &str = "seq_page_cost";
pub const AUTOVACUUM: &str = "autovacuum";
pub const AUTOVACUUM_MAX_WORKERS: &str = "autovacuum_max_workers";
pub const DEFAULT_STATISTICS_TARGET: &str = "default_statistics_target";
pub const CHECKPOINT_COMPLETION_TARGET: &str = "checkpoint_completion_target";
pub const MAX_WAL_SIZE: &str = "max_wal_size";
pub const MIN_WAL_SIZE: &str = "min_wal_size";
pub const MAX_REPLICATION_SLOTS: &str = "max_replication_slots";
pub const MAX_WAL_SENDERS: &str = "max_wal_senders";
pub const LOG_CHECKPOINTS: &str = "log_checkpoints";
pub const SSL: &str = "ssl";

/// Approximate ordering used to make the artifact stable.
pub const ALLOWLIST: &[&str] = &[
    SERVER_VERSION,
    SERVER_VERSION_NUM,
    BLOCK_SIZE,
    DATA_CHECKSUMS,
    IN_RECOVERY,
    WAL_LEVEL,
    MAX_CONNECTIONS,
    SHARED_BUFFERS,
    WORK_MEM,
    MAINTENANCE_WORK_MEM,
    EFFECTIVE_CACHE_SIZE,
    RANDOM_PAGE_COST,
    SEQ_PAGE_COST,
    AUTOVACUUM,
    AUTOVACUUM_MAX_WORKERS,
    DEFAULT_STATISTICS_TARGET,
    CHECKPOINT_COMPLETION_TARGET,
    MAX_WAL_SIZE,
    MIN_WAL_SIZE,
    MAX_REPLICATION_SLOTS,
    MAX_WAL_SENDERS,
    LOG_CHECKPOINTS,
    SSL,
];

pub fn is_allowlisted(name: &str) -> bool {
    ALLOWLIST.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_contains_expected_entries() {
        assert!(is_allowlisted(SHARED_BUFFERS));
        assert!(is_allowlisted(MAX_CONNECTIONS));
    }

    #[test]
    fn allowlist_rejects_settings_outside_the_approved_set() {
        assert!(!is_allowlisted("password_encryption"));
        assert!(!is_allowlisted("shared_preload_libraries"));
        assert!(!is_allowlisted("unix_socket_directories"));
        assert!(!is_allowlisted("listen_addresses"));
        assert!(!is_allowlisted(""));
    }

    #[test]
    fn allowlist_total_matches_known_entries() {
        assert_eq!(ALLOWLIST.len(), 23);
    }
}
