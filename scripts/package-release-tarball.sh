#!/usr/bin/env bash
# Create a deterministic release tarball for one release target.
#
# Usage:
#   package-release-tarball.sh <version> <target> <binary-path> <output-dir>
#
# The archive layout is:
#   pg-probe-v<version>-<target>/
#     pg-probe
#     README.md
#     LICENSE
#
# Archive-level variance is removed by fixing member order, ownership,
# permissions, and mtime (SOURCE_DATE_EPOCH), and by gzipping without an
# embedded filename or timestamp. This does not claim bit-for-bit identical
# Rust machine code across rebuilds.

set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <version> <target> <binary-path> <output-dir>" >&2
  exit 2
fi

version="$1"
target="$2"
binary_path="$3"
output_dir="$4"

case "$version" in
  '' | *[!0-9.]* | .* | *. | *..*)
    echo "invalid version: ${version}" >&2
    exit 2
    ;;
esac

case "$target" in
  x86_64-unknown-linux-musl | aarch64-unknown-linux-musl) ;;
  x86_64-apple-darwin | aarch64-apple-darwin) ;;
  *)
    echo "unsupported target: ${target}" >&2
    exit 2
    ;;
esac

if [[ ! -f "$binary_path" ]]; then
  echo "binary not found: ${binary_path}" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
readme_path="${repo_root}/README.md"
license_path="${repo_root}/LICENSE"

if [[ ! -f "$readme_path" || ! -f "$license_path" ]]; then
  echo "missing README.md or LICENSE in repository root" >&2
  exit 2
fi

mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"

archive_name="pg-probe-v${version}-${target}.tar.gz"
staging_root="$(mktemp -d "${TMPDIR:-/tmp}/pg-probe-package.XXXXXX")"
cleanup() {
  rm -rf "$staging_root"
}
trap cleanup EXIT

prefix="pg-probe-v${version}-${target}"
staging_dir="${staging_root}/${prefix}"
mkdir -p "$staging_dir"

cp "$binary_path" "${staging_dir}/pg-probe"
cp "$readme_path" "${staging_dir}/README.md"
cp "$license_path" "${staging_dir}/LICENSE"

chmod 0755 "${staging_dir}/pg-probe"
chmod 0644 "${staging_dir}/README.md" "${staging_dir}/LICENSE"

source_date_epoch="${SOURCE_DATE_EPOCH:-0}"
if touch -d "@${source_date_epoch}" "${staging_dir}/pg-probe" 2>/dev/null; then
  touch -d "@${source_date_epoch}" "${staging_dir}/README.md" "${staging_dir}/LICENSE"
else
  stamp="$(date -r "${source_date_epoch}" -u +%Y%m%d%H%M.%S)"
  touch -t "$stamp" "${staging_dir}/pg-probe" "${staging_dir}/README.md" "${staging_dir}/LICENSE"
fi

archive_path="${output_dir}/${archive_name}"
rm -f "$archive_path"

tar_bin="tar"
if command -v gtar >/dev/null 2>&1; then
  tar_bin="gtar"
fi

if ! "${tar_bin}" --version 2>&1 | grep -q 'GNU tar'; then
  echo "GNU tar is required to create deterministic release archives" >&2
  exit 2
fi

(
  cd "$staging_root"
  "${tar_bin}" --sort=name \
    --owner=0 --group=0 --numeric-owner \
    --mtime="@${source_date_epoch}" \
    -cf - "${prefix}/LICENSE" "${prefix}/README.md" "${prefix}/pg-probe"
) | gzip -n >"$archive_path"

echo "$archive_path"
