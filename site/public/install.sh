#!/bin/sh
# Install pg-probe on Linux.
#
#   curl -fsSL https://pg-probe.dev/install.sh | sh
#
# On Debian and Ubuntu it installs the .deb, on RHEL-family and SUSE systems
# the .rpm, and elsewhere the static binary in /usr/local/bin. Each download
# is checked against the release's SHA256SUMS before anything is installed.
#
# Environment:
#   PG_PROBE_VERSION      install this version instead of the latest
#   PG_PROBE_INSTALL_DIR  install the binary in this directory, never a package
#   PG_PROBE_DOWNLOAD_URL where releases live (default https://dl.pg-probe.dev)

# Everything runs from main, so a partial download executes nothing.
set -eu

download_url="${PG_PROBE_DOWNLOAD_URL:-https://dl.pg-probe.dev}"

say() {
  printf 'pg-probe: %s\n' "$*"
}

fail() {
  printf 'pg-probe: error: %s\n' "$*" >&2
  exit 1
}

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https,file' -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    fail "curl or wget is required"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    fail "sha256sum or shasum is required"
  fi
}

# Succeeds when commands can run as root: directly, or through sudo or doas
# once they grant it. A user who is refused, or who has no terminal to type
# a password in, gets no root. The answer is kept for later calls.
can_root() {
  if [ -z "${root:-}" ]; then
    root=no
    sudo=""
    if [ "$(id -u)" -eq 0 ]; then
      root=yes
    else
      for tool in sudo doas; do
        command -v "$tool" >/dev/null 2>&1 || continue
        if "$tool" -n true 2>/dev/null; then
          sudo="$tool"
        elif (: </dev/tty) 2>/dev/null; then
          say "$tool may ask for your password"
          if "$tool" true </dev/tty; then
            sudo="$tool"
          fi
        fi
        break
      done
      [ -z "$sudo" ] || root=yes
    fi
  fi
  [ "$root" = yes ]
}

as_root() {
  if [ -n "$sudo" ]; then
    "$sudo" "$@"
  else
    "$@"
  fi
}

verify() {
  expected="$(awk -v f="$1" '$2 == f { print $1 }' "$tmp/SHA256SUMS")"
  [ -n "$expected" ] || fail "$1 is not listed in SHA256SUMS"
  actual="$(sha256 "$tmp/$1")"
  [ "$actual" = "$expected" ] || fail "checksum mismatch for $1"
}

get() {
  fetch "$base/$1" "$tmp/$1" || fail "could not download $base/$1"
  verify "$1"
}

install_binary() {
  dir="$1"
  file="pg-probe-v${version}-${target}.tar.gz"
  get "$file"
  tar -xzf "$tmp/$file" -C "$tmp"
  binary="$tmp/pg-probe-v${version}-${target}/pg-probe"
  [ -f "$binary" ] || fail "the archive has no pg-probe binary"

  if mkdir -p "$dir" 2>/dev/null && [ -w "$dir" ]; then
    install -m 0755 "$binary" "$dir/pg-probe"
  elif can_root; then
    as_root mkdir -p "$dir"
    as_root install -m 0755 "$binary" "$dir/pg-probe"
  else
    fail "cannot write to $dir; run as root or set PG_PROBE_INSTALL_DIR"
  fi
  installed="$dir/pg-probe"
}

main() {
  case "$(uname -s)" in
    Linux) ;;
    Darwin) fail "on macOS, install with: brew install momoi-labs/packages/pg-probe" ;;
    *) fail "pg-probe runs on Linux and macOS only" ;;
  esac

  case "$(uname -m)" in
    x86_64 | amd64)
      target=x86_64-unknown-linux-musl
      deb_arch=amd64
      rpm_arch=x86_64
      ;;
    aarch64 | arm64)
      target=aarch64-unknown-linux-musl
      deb_arch=arm64
      rpm_arch=aarch64
      ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac

  distro="Linux"
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    distro="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-${ID:-Linux}}")"
  fi

  tmp="$(mktemp -d "${TMPDIR:-/tmp}/pg-probe-install.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 1' INT TERM

  version="${PG_PROBE_VERSION:-}"
  if [ -z "$version" ]; then
    fetch "$download_url/latest.txt" "$tmp/latest.txt" \
      || fail "could not read the latest version from $download_url"
    version="$(tr -d ' \r\n' <"$tmp/latest.txt")"
  fi
  version="${version#v}"
  case "$version" in
    '' | *[!0-9.]*) fail "invalid version: $version" ;;
  esac
  base="$download_url/v$version"

  # A package needs root, so without it the binary goes in ~/.local/bin.
  method=binary
  dir="${PG_PROBE_INSTALL_DIR:-}"
  if [ -z "$dir" ]; then
    if ! can_root; then
      say "no root access; installing for this user only"
      dir="${HOME:?}/.local/bin"
    elif command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
      method=deb
    elif command -v rpm >/dev/null 2>&1 \
      && { command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 \
        || command -v zypper >/dev/null 2>&1; }; then
      method=rpm
    else
      dir=/usr/local/bin
    fi
  fi

  say "installing v$version for $distro ($(uname -m)) as $method"
  fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" \
    || fail "could not download $base/SHA256SUMS"

  case "$method" in
    deb)
      file="pg-probe_${version}_${deb_arch}.deb"
      get "$file"
      as_root dpkg -i "$tmp/$file" >/dev/null
      installed=/usr/bin/pg-probe
      ;;
    rpm)
      file="pg-probe-${version}-1.${rpm_arch}.rpm"
      get "$file"
      as_root rpm -U --replacepkgs "$tmp/$file"
      installed=/usr/bin/pg-probe
      ;;
    binary)
      install_binary "$dir"
      ;;
  esac

  say "installed $installed"
  found="$(command -v pg-probe 2>/dev/null || true)"
  if [ -z "$found" ]; then
    say "$(dirname "$installed") is not in your PATH; add it to run pg-probe"
  elif [ "$found" != "$installed" ]; then
    say "warning: pg-probe in your PATH is $found, not $installed"
  fi
}

main "$@"
