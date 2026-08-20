#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output_dir=${1:-"$repo_dir/web/dev-artifacts"}

command -v sqlite3 >/dev/null 2>&1 || {
    echo "seed-dev-artifacts: sqlite3 is required" >&2
    exit 1
}
command -v gzip >/dev/null 2>&1 || {
    echo "seed-dev-artifacts: gzip is required" >&2
    exit 1
}

mkdir -p "$output_dir"
temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

for scenario in complete-small partial-failures dangerous-settings null-heavy large-host standby-recovery extension-rich; do
    database="$temporary_dir/$scenario.sqlite"
    sqlite3 "$database" < "$repo_dir/schema/migrations/V1__initial_schema.sql"
    sqlite3 "$database" <<SQL
PRAGMA foreign_keys = ON;
PRAGMA application_id = 1346850898;
PRAGMA user_version = 1;
CREATE TABLE refinery_schema_history (
    version int4 PRIMARY KEY,
    name VARCHAR(255),
    applied_on VARCHAR(255),
    checksum VARCHAR(255)
);
CREATE TEMP TABLE seed_scenario (name TEXT NOT NULL);
INSERT INTO seed_scenario VALUES ('$scenario');
.read $repo_dir/schema/seed.sql
PRAGMA optimize;
SQL
    gzip -c "$database" > "$output_dir/$scenario.sqlite.gz"
done

echo "Created 7 development artifacts in $output_dir"
