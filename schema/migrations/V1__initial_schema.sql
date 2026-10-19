CREATE TABLE snapshot (
    snapshot_id TEXT PRIMARY KEY NOT NULL,
    schema_version INTEGER NOT NULL,
    collector_name TEXT NOT NULL,
    collector_version TEXT NOT NULL,
    started_at_utc TEXT NOT NULL,
    finished_at_utc TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('complete', 'partial'))
) STRICT;
CREATE TABLE sample (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL CHECK (sample_id >= 1),
    taken_at_utc TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, sample_id)
) STRICT;
CREATE TABLE collector_outcome (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    source TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    started_at_utc TEXT NOT NULL,
    finished_at_utc TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN (
            'collected',
            'unsupported',
            'permission_denied',
            'parse_error',
            'io_error',
            'raced'
        )
    ),
    error_code TEXT,
    os_error_code INTEGER,
    observed_at_utc TEXT,
    collector_name TEXT NOT NULL,
    collector_version TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, source, sequence),
    CHECK (
        (status = 'collected' AND error_code IS NULL AND observed_at_utc IS NOT NULL)
        OR
        (status <> 'collected' AND error_code IS NOT NULL AND observed_at_utc IS NULL)
    )
) STRICT;
CREATE TABLE linux_identity (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    hostname TEXT,
    kernel_sysname TEXT,
    kernel_release TEXT,
    kernel_version TEXT,
    kernel_machine TEXT,
    distribution_id TEXT,
    distribution_version_id TEXT,
    distribution_pretty_name TEXT,
    page_size_bytes INTEGER,
    ticks_per_second INTEGER,
    uptime_microseconds INTEGER
) STRICT;
CREATE TABLE linux_cpu_set (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    set_kind TEXT NOT NULL CHECK (set_kind IN ('online', 'present', 'possible', 'affinity')),
    cpu_id INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, set_kind, cpu_id)
) STRICT;
CREATE TABLE linux_cpu_topology (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    cpu_id INTEGER NOT NULL,
    package_id INTEGER,
    die_id INTEGER,
    core_id INTEGER,
    sibling_set TEXT,
    PRIMARY KEY (snapshot_id, cpu_id)
) STRICT;
CREATE TABLE linux_numa_node (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    node_id INTEGER NOT NULL,
    cpu_set TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, node_id)
) STRICT;
CREATE TABLE linux_cpu_time (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    cpu_id INTEGER NOT NULL,
    state TEXT NOT NULL,
    ticks INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, sample_id, cpu_id, state),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_scheduler (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    procs_running INTEGER,
    procs_blocked INTEGER,
    ctxt INTEGER,
    processes INTEGER,
    intr INTEGER,
    softirq INTEGER,
    load_avg_1m REAL,
    load_avg_5m REAL,
    load_avg_15m REAL,
    runnable_tasks INTEGER,
    total_tasks INTEGER,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_memory (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    key TEXT NOT NULL,
    value_bytes INTEGER,
    unit TEXT NOT NULL CHECK (unit IN ('bytes', 'pages')),
    PRIMARY KEY (snapshot_id, sample_id, key),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_vm_event (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    key TEXT NOT NULL,
    value INTEGER NOT NULL,
    unit TEXT NOT NULL CHECK (unit IN ('events', 'KiB')),
    PRIMARY KEY (snapshot_id, sample_id, key),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_psi (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    resource TEXT NOT NULL CHECK (resource IN ('cpu', 'memory', 'io')),
    scope TEXT NOT NULL CHECK (scope IN ('some', 'full')),
    avg10 REAL NOT NULL,
    avg60 REAL NOT NULL,
    avg300 REAL NOT NULL,
    total_us INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, sample_id, resource, scope),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_mount (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    mount_id INTEGER NOT NULL,
    parent_mount_id INTEGER NOT NULL,
    device_major INTEGER NOT NULL,
    device_minor INTEGER NOT NULL,
    mount_point TEXT,
    filesystem_type TEXT NOT NULL,
    is_read_only INTEGER NOT NULL CHECK (is_read_only IN (0, 1)),
    total_bytes INTEGER,
    available_bytes INTEGER,
    total_inodes INTEGER,
    available_inodes INTEGER,
    PRIMARY KEY (snapshot_id, mount_id)
) STRICT;
CREATE TABLE linux_block_device (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    device_major INTEGER NOT NULL,
    device_minor INTEGER NOT NULL,
    kernel_name TEXT,
    logical_block_size_bytes INTEGER,
    physical_block_size_bytes INTEGER,
    minimum_io_size_bytes INTEGER,
    optimal_io_size_bytes INTEGER,
    read_ahead_kibibytes INTEGER,
    request_queue_depth INTEGER,
    is_rotational INTEGER CHECK (is_rotational IN (0, 1)),
    scheduler TEXT,
    PRIMARY KEY (snapshot_id, device_major, device_minor)
) STRICT;
CREATE TABLE linux_block_io (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    device_major INTEGER NOT NULL,
    device_minor INTEGER NOT NULL,
    reads_completed INTEGER NOT NULL,
    reads_merged INTEGER NOT NULL,
    sectors_read INTEGER NOT NULL,
    read_milliseconds INTEGER NOT NULL,
    writes_completed INTEGER NOT NULL,
    writes_merged INTEGER NOT NULL,
    sectors_written INTEGER NOT NULL,
    write_milliseconds INTEGER NOT NULL,
    io_in_progress INTEGER NOT NULL,
    io_milliseconds INTEGER NOT NULL,
    weighted_io_milliseconds INTEGER NOT NULL,
    discards_completed INTEGER,
    discard_sectors INTEGER,
    discard_milliseconds INTEGER,
    flushes_completed INTEGER,
    flush_milliseconds INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, device_major, device_minor),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_network_io (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    interface TEXT NOT NULL,
    rx_bytes INTEGER NOT NULL,
    rx_packets INTEGER NOT NULL,
    rx_errors INTEGER NOT NULL,
    rx_dropped INTEGER NOT NULL,
    tx_bytes INTEGER NOT NULL,
    tx_packets INTEGER NOT NULL,
    tx_errors INTEGER NOT NULL,
    tx_dropped INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, sample_id, interface),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_resource_limit (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    resource TEXT NOT NULL CHECK (resource IN (
        'nofile', 'nproc', 'memlock', 'address_space', 'core', 'file_size', 'stack'
    )),
    soft_value INTEGER,
    hard_value INTEGER,
    soft_unlimited INTEGER NOT NULL CHECK (soft_unlimited IN (0, 1)),
    hard_unlimited INTEGER NOT NULL CHECK (hard_unlimited IN (0, 1)),
    unit TEXT NOT NULL CHECK (unit IN ('bytes', 'count')),
    PRIMARY KEY (snapshot_id, resource)
) STRICT;
CREATE TABLE linux_cgroup (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    version TEXT NOT NULL CHECK (version IN ('v1', 'v2')),
    cpu_quota_microseconds INTEGER,
    cpu_period_microseconds INTEGER,
    cpu_quota_unlimited INTEGER CHECK (cpu_quota_unlimited IN (0, 1)),
    cpu_weight INTEGER,
    memory_min_bytes INTEGER,
    memory_low_bytes INTEGER,
    memory_high_bytes INTEGER,
    memory_high_unlimited INTEGER CHECK (memory_high_unlimited IN (0, 1)),
    memory_max_bytes INTEGER,
    memory_max_unlimited INTEGER CHECK (memory_max_unlimited IN (0, 1)),
    swap_max_bytes INTEGER,
    swap_max_unlimited INTEGER CHECK (swap_max_unlimited IN (0, 1)),
    pids_max INTEGER,
    pids_max_unlimited INTEGER CHECK (pids_max_unlimited IN (0, 1))
) STRICT;
CREATE TABLE linux_cgroup_usage (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    memory_current_bytes INTEGER,
    swap_current_bytes INTEGER,
    pids_current INTEGER,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_cgroup_event (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    controller TEXT NOT NULL CHECK (controller IN ('cpu', 'memory', 'pids')),
    event_name TEXT NOT NULL,
    cumulative_value INTEGER NOT NULL,
    unit TEXT NOT NULL CHECK (unit IN ('count', 'microseconds')),
    PRIMARY KEY (snapshot_id, sample_id, controller, event_name),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_cgroup_io (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    device_major INTEGER NOT NULL,
    device_minor INTEGER NOT NULL,
    read_bytes INTEGER,
    write_bytes INTEGER,
    discard_bytes INTEGER,
    read_operations INTEGER,
    write_operations INTEGER,
    discard_operations INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, device_major, device_minor),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE linux_kernel_vm (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    overcommit_mode INTEGER,
    overcommit_ratio_percent INTEGER,
    overcommit_kbytes_source INTEGER,
    overcommit_bytes INTEGER,
    max_map_count INTEGER,
    swappiness INTEGER,
    zone_reclaim_mode INTEGER,
    min_free_kbytes_source INTEGER,
    min_free_bytes INTEGER,
    dirty_background_bytes INTEGER,
    dirty_background_ratio_percent INTEGER,
    dirty_bytes INTEGER,
    dirty_ratio_percent INTEGER,
    dirty_expire_microseconds INTEGER,
    dirty_writeback_microseconds INTEGER
) STRICT;
CREATE TABLE linux_kernel_ipc (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    shmmax_bytes INTEGER,
    shmall_pages INTEGER,
    shmall_bytes INTEGER,
    shmmni INTEGER,
    semmsl INTEGER,
    semmns INTEGER,
    semopm INTEGER,
    semmni INTEGER,
    file_max INTEGER,
    nr_open INTEGER
) STRICT;
CREATE TABLE linux_thp (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    enabled_policy TEXT,
    defrag_policy TEXT,
    khugepaged_defrag INTEGER CHECK (khugepaged_defrag IN (0, 1))
) STRICT;
CREATE TABLE postgres_target (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    endpoint_kind TEXT NOT NULL CHECK (endpoint_kind IN ('tcp', 'unix_socket')),
    endpoint TEXT NOT NULL,
    port INTEGER,
    connected_user TEXT NOT NULL,
    initial_database TEXT NOT NULL,
    server_version_num INTEGER NOT NULL,
    server_version TEXT NOT NULL,
    in_recovery INTEGER NOT NULL CHECK (in_recovery IN (0, 1)),
    block_size_bytes INTEGER NOT NULL CHECK (block_size_bytes > 0),
    is_superuser INTEGER NOT NULL CHECK (is_superuser IN (0, 1)),
    has_pg_monitor INTEGER NOT NULL CHECK (has_pg_monitor IN (0, 1)),
    has_pg_read_all_stats INTEGER NOT NULL CHECK (has_pg_read_all_stats IN (0, 1)),
    has_pg_read_all_settings INTEGER NOT NULL CHECK (has_pg_read_all_settings IN (0, 1))
) STRICT;
CREATE TABLE postgres_setting (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    setting_name TEXT NOT NULL,
    setting_value TEXT NOT NULL,
    unit TEXT,
    value_type TEXT NOT NULL,
    source TEXT NOT NULL,
    pending_restart INTEGER NOT NULL CHECK (pending_restart IN (0, 1)),
    PRIMARY KEY (snapshot_id, setting_name)
) STRICT;
CREATE TABLE postgres_database (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    database_name TEXT NOT NULL,
    is_template INTEGER NOT NULL CHECK (is_template IN (0, 1)),
    allows_connections INTEGER NOT NULL CHECK (allows_connections IN (0, 1)),
    can_connect INTEGER NOT NULL CHECK (can_connect IN (0, 1)),
    size_bytes INTEGER,
    extension_visit_status TEXT NOT NULL CHECK (extension_visit_status IN (
        'pending', 'visited', 'not_connectable', 'permission_denied', 'connection_failed'
    )),
    PRIMARY KEY (snapshot_id, database_name)
) STRICT;
CREATE TABLE postgres_extension (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    collected_at TEXT NOT NULL,
    database_name TEXT NOT NULL,
    extension_name TEXT NOT NULL,
    extension_version TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, database_name, extension_name)
) STRICT;
CREATE VIEW database_inventory AS
    SELECT
        d.snapshot_id,
        d.database_name,
        d.is_template,
        d.allows_connections,
        d.can_connect,
        d.size_bytes,
        d.extension_visit_status,
        (SELECT COUNT(*) FROM postgres_extension e
            WHERE e.snapshot_id = d.snapshot_id
              AND e.database_name = d.database_name) AS extension_count
    FROM postgres_database d;
CREATE TABLE postgres_database_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    database_oid INTEGER NOT NULL,
    database_name TEXT NOT NULL,
    num_backends INTEGER,
    xact_commit INTEGER NOT NULL,
    xact_rollback INTEGER NOT NULL,
    blks_read INTEGER NOT NULL,
    blks_hit INTEGER NOT NULL,
    tup_returned INTEGER NOT NULL,
    tup_fetched INTEGER NOT NULL,
    tup_inserted INTEGER NOT NULL,
    tup_updated INTEGER NOT NULL,
    tup_deleted INTEGER NOT NULL,
    conflicts INTEGER NOT NULL,
    temp_files INTEGER NOT NULL,
    temp_bytes INTEGER NOT NULL,
    deadlocks INTEGER NOT NULL,
    checksum_failures INTEGER,
    blk_read_time_ms REAL,
    blk_write_time_ms REAL,
    session_time_ms REAL,
    active_time_ms REAL,
    idle_in_transaction_time_ms REAL,
    sessions INTEGER,
    sessions_abandoned INTEGER,
    sessions_fatal INTEGER,
    sessions_killed INTEGER,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id, database_oid),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_standby_conflicts (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    database_oid INTEGER NOT NULL,
    database_name TEXT NOT NULL,
    confl_tablespace INTEGER NOT NULL,
    confl_lock INTEGER NOT NULL,
    confl_snapshot INTEGER NOT NULL,
    confl_bufferpin INTEGER NOT NULL,
    confl_deadlock INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, sample_id, database_oid),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_wal_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    wal_records INTEGER NOT NULL,
    wal_fpi INTEGER NOT NULL,
    wal_bytes INTEGER NOT NULL,
    wal_buffers_full INTEGER,
    wal_write INTEGER,
    wal_sync INTEGER,
    wal_write_time_ms REAL,
    wal_sync_time_ms REAL,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_bgwriter_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    checkpoints_timed INTEGER,
    checkpoints_req INTEGER,
    checkpoint_write_time_ms REAL,
    checkpoint_sync_time_ms REAL,
    buffers_checkpoint INTEGER,
    buffers_clean INTEGER,
    maxwritten_clean INTEGER,
    buffers_backend INTEGER,
    buffers_backend_fsync INTEGER,
    buffers_alloc INTEGER,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_checkpointer_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    num_timed INTEGER,
    num_requested INTEGER,
    write_time_ms REAL,
    sync_time_ms REAL,
    buffers_written INTEGER,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_archiver_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    archived_count INTEGER NOT NULL,
    last_archived_wal TEXT,
    last_archived_time TEXT,
    failed_count INTEGER NOT NULL,
    last_failed_wal TEXT,
    last_failed_time TEXT,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_io_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    backend_type TEXT NOT NULL,
    io_object TEXT,
    io_context TEXT,
    reads INTEGER NOT NULL,
    read_time_ms REAL,
    writes INTEGER NOT NULL,
    write_time_ms REAL,
    extends INTEGER NOT NULL,
    extend_time_ms REAL,
    op_bytes INTEGER NOT NULL,
    hits INTEGER NOT NULL,
    evictions INTEGER NOT NULL,
    reuses INTEGER NOT NULL,
    fsyncs INTEGER NOT NULL,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id, backend_type, io_object, io_context),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_slru_stats (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    name TEXT NOT NULL,
    blocks_zeroed INTEGER NOT NULL,
    blocks_hit INTEGER NOT NULL,
    blocks_read INTEGER NOT NULL,
    blocks_written INTEGER NOT NULL,
    blocks_exists INTEGER NOT NULL,
    flush_count INTEGER NOT NULL,
    stats_reset TEXT,
    PRIMARY KEY (snapshot_id, sample_id, name),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
) STRICT;
CREATE TABLE postgres_activity_aggregates (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    database_name TEXT,
    backend_type TEXT NOT NULL,
    state TEXT,
    wait_event_type TEXT,
    count INTEGER NOT NULL,
    max_age_seconds INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, database_name, backend_type, state, wait_event_type),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
);
CREATE TABLE postgres_lock_aggregates (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    lock_mode TEXT NOT NULL,
    lock_type TEXT NOT NULL,
    granted INTEGER NOT NULL CHECK (granted IN (0, 1)),
    fast_path INTEGER CHECK (fast_path IN (0, 1)),
    count INTEGER NOT NULL,
    max_wait_age_seconds INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, lock_mode, lock_type, granted, fast_path),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
);
CREATE TABLE postgres_replication_aggregates (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    state TEXT,
    sync_state TEXT,
    count INTEGER NOT NULL,
    max_lag_bytes INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, state, sync_state),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
);
CREATE TABLE postgres_slot_aggregates (
    snapshot_id TEXT NOT NULL REFERENCES snapshot(snapshot_id),
    sample_id INTEGER NOT NULL,
    collected_at TEXT NOT NULL,
    slot_type TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    count INTEGER NOT NULL,
    max_retained_bytes INTEGER,
    PRIMARY KEY (snapshot_id, sample_id, slot_type, active),
    FOREIGN KEY (snapshot_id, sample_id) REFERENCES sample(snapshot_id, sample_id)
);
