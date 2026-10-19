-- The seed script defines temp table seed_scenario(name) before loading this file.
INSERT INTO snapshot (
    snapshot_id, schema_version, collector_name, collector_version,
    started_at_utc, finished_at_utc, status
)
SELECT
    name, 2, 'pg-probe', 'dev-seed',
    '2026-08-19T12:00:00Z', '2026-08-19T12:00:05Z',
    CASE WHEN name IN ('partial-failures', 'null-heavy') THEN 'partial' ELSE 'complete' END
FROM seed_scenario;

INSERT INTO sample (snapshot_id, sample_id, taken_at_utc)
SELECT name, 1, '2026-08-19T12:00:00Z' FROM seed_scenario;

INSERT INTO linux_identity (
    snapshot_id, hostname, kernel_sysname, kernel_release, kernel_version,
    kernel_machine, distribution_id, distribution_version_id,
    distribution_pretty_name, page_size_bytes, ticks_per_second, uptime_microseconds
)
SELECT
    name,
    CASE name
        WHEN 'large-host' THEN 'warehouse-db-128cpu'
        WHEN 'null-heavy' THEN NULL
        ELSE name || '-host'
    END,
    CASE WHEN name = 'null-heavy' THEN NULL ELSE 'Linux' END,
    CASE WHEN name = 'null-heavy' THEN NULL ELSE '6.12.0' END,
    NULL, 'x86_64', 'debian', '13', 'Debian GNU/Linux 13',
    4096, 100,
    CASE WHEN name = 'large-host' THEN 7776000000000 ELSE 86400000000 END
FROM seed_scenario;

INSERT INTO collector_outcome (
    snapshot_id, source, sequence, started_at_utc, finished_at_utc, status,
    error_code, os_error_code, observed_at_utc, collector_name, collector_version
)
SELECT
    name, 'linux.identity', 0, '2026-08-19T12:00:00Z', '2026-08-19T12:00:01Z',
    CASE WHEN name = 'partial-failures' THEN 'permission_denied' ELSE 'collected' END,
    CASE WHEN name = 'partial-failures' THEN 'proc_permission_denied' ELSE NULL END,
    CASE WHEN name = 'partial-failures' THEN 13 ELSE NULL END,
    CASE WHEN name = 'partial-failures' THEN NULL ELSE '2026-08-19T12:00:00Z' END,
    'pg-probe', 'dev-seed'
FROM seed_scenario;

INSERT INTO postgres_target (
    snapshot_id, collected_at, endpoint_kind, endpoint, port, connected_user,
    initial_database, server_version_num, server_version, in_recovery,
    block_size_bytes, is_superuser, has_pg_monitor, has_pg_read_all_stats,
    has_pg_read_all_settings
)
SELECT
    name, '2026-08-19T12:00:02Z', 'tcp', 'db.internal', 5432, 'probe',
    'postgres', 170005, '17.5', CASE WHEN name = 'standby-recovery' THEN 1 ELSE 0 END,
    8192, 0, 1, 1, 1
FROM seed_scenario WHERE name <> 'partial-failures';

INSERT INTO postgres_setting (
    snapshot_id, collected_at, setting_name, setting_value, unit,
    value_type, source, pending_restart
)
SELECT name, '2026-08-19T12:00:03Z', 'fsync',
       CASE WHEN name = 'dangerous-settings' THEN 'off' ELSE 'on' END,
       NULL, 'bool', 'configuration file', 0
FROM seed_scenario WHERE name <> 'partial-failures';

INSERT INTO linux_memory (snapshot_id, sample_id, collected_at, key, value_bytes, unit)
SELECT name, 1, '2026-08-19T12:00:02Z', 'MemTotal',
       CASE WHEN name = 'large-host' THEN 1099511627776 ELSE 17179869184 END,
       'bytes'
FROM seed_scenario WHERE name <> 'null-heavy';

INSERT INTO postgres_database (
    snapshot_id, collected_at, database_name, is_template, allows_connections,
    can_connect, size_bytes, extension_visit_status
)
SELECT name, '2026-08-19T12:00:04Z', 'postgres', 0, 1, 1,
       CASE WHEN name = 'large-host' THEN 2199023255552 ELSE 134217728 END,
       'visited'
FROM seed_scenario WHERE name NOT IN ('partial-failures', 'null-heavy');
