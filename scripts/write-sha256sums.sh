#!/usr/bin/env bash
# Write a sorted SHA256SUMS manifest for the two release tarballs.
#
# Usage:
#   write-sha256sums.sh <output-dir> <tarball> [<tarball> ...]
#
# Each line is:
#   <lowercase sha256>  <filename>
# sorted by filename. Only regular files whose basenames match the approved
# release asset pattern are accepted.

set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <output-dir> <tarball> [<tarball> ...]" >&2
  exit 2
fi

output_dir="$1"
shift
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
manifest="${output_dir}/SHA256SUMS"
rm -f "$manifest"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/pg-probe-sha256.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

lines_file="${tmpdir}/lines"

for path in "$@"; do
  if [[ ! -f "$path" ]]; then
    echo "missing file: ${path}" >&2
    exit 2
  fi
  base="$(basename "$path")"
  case "$base" in
    pg-probe-v*-x86_64-unknown-linux-musl.tar.gz | pg-probe-v*-aarch64-unknown-linux-musl.tar.gz) ;;
    *)
      echo "unexpected asset name: ${base}" >&2
      exit 2
      ;;
  esac
  if command -v sha256sum >/dev/null 2>&1; then
    digest="$(sha256sum "$path" | awk '{print tolower($1)}')"
  else
    digest="$(shasum -a 256 "$path" | awk '{print tolower($1)}')"
  fi
  printf '%s  %s\n' "$digest" "$base" >>"$lines_file"
done

LC_ALL=C sort -k2,2 "$lines_file" >"$manifest"
echo "$manifest"
