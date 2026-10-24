#!/usr/bin/env bash
# Smoke-test an exact packaged musl binary on the current host.
#
# Usage:
#   smoke-test-release-binary.sh <extracted-pg-probe-binary> <work-dir>
#   smoke-test-release-binary.sh --check-artifact <work-dir>
#
# Runs --version and a Linux-only snapshot, then checks the artifact is a
# valid gzip-compressed SQLite database with the required spine tables.
#
# A sqlite3 older than 3.37 cannot read the artifact's STRICT tables, as on
# Rocky Linux 9. There, set SMOKE_SQL_CHECK=0 to stop after the snapshot and
# gunzip, and run --check-artifact on the work directory from a host whose
# sqlite3 reads STRICT tables.

set -euo pipefail

check_artifact() {
  local sqlite_path="$1"
  if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "sqlite3 is required for release smoke tests" >&2
    exit 2
  fi

  local integrity
  integrity="$(sqlite3 -readonly "$sqlite_path" 'PRAGMA integrity_check;')"
  if [[ "$integrity" != "ok" ]]; then
    echo "SQLite integrity_check failed: ${integrity}" >&2
    exit 1
  fi

  local table present
  for table in snapshot collector_outcome linux_identity; do
    present="$(sqlite3 -readonly "$sqlite_path" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='${table}';")"
    if [[ "$present" != "1" ]]; then
      echo "required table missing: ${table}" >&2
      exit 1
    fi
  done
}

if [[ "${1:-}" == "--check-artifact" && $# -eq 2 ]]; then
  sqlites=("$2"/*.sqlite)
  if [[ ${#sqlites[@]} -ne 1 || ! -f "${sqlites[0]}" ]]; then
    echo "expected exactly one unpacked artifact in: $2" >&2
    exit 1
  fi
  check_artifact "${sqlites[0]}"
  echo "artifact check ok: ${sqlites[0]}"
  exit 0
fi

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <binary> <work-dir> | --check-artifact <work-dir>" >&2
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

# capture keeps stdout empty and logs the artifact path on stderr, so look
# for the one artifact it wrote in the work directory.
"$binary" capture --domain linux --data-directory "$work_dir" >/dev/null
artifacts=("$work_dir"/*.sqlite.gz)
if [[ ${#artifacts[@]} -ne 1 || ! -f "${artifacts[0]}" ]]; then
  echo "snapshot did not produce exactly one artifact in: ${work_dir}" >&2
  exit 1
fi
artifact_path="${artifacts[0]}"

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

if [[ "${SMOKE_SQL_CHECK:-1}" == "0" ]]; then
  # Artifacts are private (0600). In a container they belong to root, so let
  # the host user read the unpacked copy.
  chmod a+r "$sqlite_path"
  echo "smoke-test ok, artifact left for --check-artifact: ${version_out}"
  exit 0
fi

check_artifact "$sqlite_path"

echo "smoke-test ok: ${version_out}"
