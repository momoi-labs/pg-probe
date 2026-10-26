#!/usr/bin/env bash
# Captures the pg-probe top screens that the site shows, into
# site/src/assets/top/, from a real top in a terminal 120 columns by 35 rows.
#
# It starts PostgreSQL 17 in Docker, runs pgbench and a report workload
# against it, then stages a lock and a manual vacuum, so every view has
# something to show. top runs in the container as the postgres user, so it
# can read each backend's I/O, inside a detached tmux, and each screen goes
# through scripts/ansi-to-html.mjs. Everything it starts is removed on exit.
#
# Needs docker, tmux and node. Build the release binary first; the
# container must have a glibc at least as new as the one it links:
#
#   cargo build --release --locked
#   scripts/top-screens.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release/pg-probe
out=$root/site/src/assets/top
name=pg-probe-screens
work=$(mktemp -d)

cleanup() {
  tmux -L "$name" kill-server 2>/dev/null || true
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

[ -x "$bin" ] || { echo "build $bin first: cargo build --release --locked" >&2; exit 1; }

psql() { docker exec -i "$name" psql -U postgres -v ON_ERROR_STOP=1 -q "$@"; }

echo "starting PostgreSQL"
# Commits do not wait for the disk, so on a busy host the report query, not
# a pgbench commit, still runs longest.
docker run -d --name "$name" -e POSTGRES_HOST_AUTH_METHOD=trust \
  postgres:17 -c track_functions=all -c max_connections=200 \
  -c synchronous_commit=off >/dev/null
docker cp "$bin" "$name:/usr/local/bin/pg-probe" >/dev/null
# The server that initdb starts first listens on the socket only.
until docker exec "$name" pg_isready -h 127.0.0.1 -U postgres -q; do sleep 1; done

echo "loading data"
docker exec "$name" pgbench -U postgres -i -s 30 -q postgres
psql <<'SQL'
CREATE FUNCTION branch_report(p_bid integer)
RETURNS TABLE(aid integer, abalance integer, history_rows bigint)
LANGUAGE plpgsql AS $$
BEGIN
    RETURN QUERY
    SELECT a.aid, a.abalance, count(h.*)
    FROM pgbench_accounts a
    LEFT JOIN pgbench_history h ON h.aid = a.aid
    WHERE a.bid = p_bid
    GROUP BY a.aid, a.abalance
    ORDER BY a.abalance DESC
    LIMIT 20;
END
$$;
CREATE TABLE audit_log AS
    SELECT g AS id, now() AS at, md5(g::text) AS payload
    FROM generate_series(1, 2000000) g;
SQL
docker exec -i "$name" sh -c 'cat > /tmp/report.sql' <<'SQL'
\set bid random(1, 30)
SELECT r.aid, r.abalance, r.history_rows FROM branch_report(:bid) r;
SQL

echo "starting the workload"
docker exec -d "$name" pgbench -U postgres -n -c 8 -j 2 -T 900 -f /tmp/report.sql postgres
docker exec -d "$name" pgbench -U postgres -n -c 8 -j 2 -T 900 postgres

# A fresh XDG_CONFIG_HOME, so no saved top settings change the screens.
tmux -L "$name" -f /dev/null new-session -d -s top -x 120 -y 35 \
  docker exec -it -u postgres -e TERM=xterm-256color -e COLORTERM=truecolor \
  -e XDG_CONFIG_HOME=/tmp/top "$name" \
  pg-probe top -h 127.0.0.1 -U postgres -d postgres
tmux -L "$name" set -s escape-time 0

key() {
  tmux -L "$name" send-keys -t top "$1"
  sleep "${2:-3}"
}
shot() {
  tmux -L "$name" capture-pane -p -e -N -t top > "$work/$1.ansi"
  node "$root/scripts/ansi-to-html.mjs" < "$work/$1.ansi" > "$out/$1.html"
  echo "captured $1"
}

# A few refreshes, so the history and the rates have samples.
sleep 10
shot running
# Paused, the report query that runs longest stays on the first row.
key Space 1
key Enter
shot query
key e
shot query-plan
key Escape 1
key f
shot query-functions
key Escape 1
key Escape 1
key Space 1

echo "holding a lock on every branch for 30 s"
docker exec -d "$name" psql -U postgres -q -c 'BEGIN' \
  -c 'UPDATE pgbench_branches SET bbalance = bbalance' \
  -c 'SELECT pg_sleep(30)' -c 'COMMIT'
sleep 8
key 2
shot waiting
key 3
shot blocking
sleep 25

echo "vacuuming audit_log"
docker exec -d "$name" psql -U postgres -q -c 'SET vacuum_cost_delay = 100' \
  -c 'SET vacuum_cost_limit = 10' \
  -c 'VACUUM (DISABLE_PAGE_SKIPPING) audit_log'
sleep 4
key 4 5
shot vacuum
key q 1
