#!/usr/bin/env bash
# Smoke-test an exact packaged musl binary on the current host.
#
# Usage:
#   smoke-test-release-binary.sh <extracted-pg-probe-binary> <work-dir>
#
# Runs --version and a Linux-only snapshot, then checks the artifact is a
# valid gzip-compressed SQLite database with the required spine tables.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <binary> <work-dir>" >&2
  exit 2
fi

binary="$1"
work_dir="$2"

if [[ ! -x "$binary" ]]; then
  echo "binary is missing or not executable: ${binary}" >&2
  exit 2
fi

mkdir -p "$work_dir"
work_dir="$(cd "$work_dir" && pwd)"

version_out="$("$binary" --version)"
case "$version_out" in
  pg-probe\ *) ;;
  *)
    echo "unexpected --version output: ${version_out}" >&2
    exit 1
    ;;
esac

artifact_path="$("$binary" --domain linux --data-directory "$work_dir")"
if [[ ! -f "$artifact_path" ]]; then
  echo "snapshot did not produce an artifact at: ${artifact_path}" >&2
  exit 1
fi

case "$artifact_path" in
  *.sqlite.gz) ;;
  *)
    echo "artifact path is not .sqlite.gz: ${artifact_path}" >&2
    exit 1
    ;;
esac

sqlite_path="${artifact_path%.gz}"
rm -f "$sqlite_path"
gunzip -k "$artifact_path"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 is required for release smoke tests" >&2
  exit 2
fi

integrity="$(sqlite3 "$sqlite_path" 'PRAGMA integrity_check;')"
if [[ "$integrity" != "ok" ]]; then
  echo "SQLite integrity_check failed: ${integrity}" >&2
  exit 1
fi

for table in snapshot collector_outcome linux_identity; do
  present="$(sqlite3 "$sqlite_path" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='${table}';")"
  if [[ "$present" != "1" ]]; then
    echo "required table missing: ${table}" >&2
    exit 1
  fi
done

echo "smoke-test ok: ${version_out}"
