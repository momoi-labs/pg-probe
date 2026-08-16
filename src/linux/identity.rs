use std::fs;
use std::io;
use std::path::Path;

use crate::collector::{Collector, Sink};
use crate::contract::{
    now_utc, ClockParameters, CollectionStatus, ComponentOutcome, DistributionIdentity,
    KernelIdentity, LinuxIdentity,
};

const UNAME_SOURCE: &str = "linux.uname";
const OS_RELEASE_SOURCE: &str = "linux.os_release";
const CLOCK_SOURCE: &str = "linux.clock";
const UPTIME_SOURCE: &str = "proc.uptime";

/// Returns the collector hostname (uname nodename) for artifact filename
/// generation. Falls back to `"unknown"` on any failure.
pub fn collector_hostname() -> String {
    let mut value: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut value) } != 0 {
        return String::from("unknown");
    }
    // c_char is i8 on some targets and u8 on others (e.g. aarch64 Linux).
    #[allow(clippy::unnecessary_cast)]
    let bytes: Vec<u8> = value
        .nodename
        .iter()
        .map(|c| *c as u8)
        .take_while(|c| *c != 0)
        .collect();
    String::from_utf8(bytes).unwrap_or_else(|_| String::from("unknown"))
}

pub struct IdentityCollector;

impl Collector for IdentityCollector {
    type Output = LinuxIdentity;

    fn source(&self) -> &'static str {
        "linux.identity"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        collect_identity()
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_identity(output, outcomes)
    }
}

fn collect_identity() -> (LinuxIdentity, Vec<ComponentOutcome>) {
    let mut identity = LinuxIdentity::default();
    let mut outcomes = Vec::with_capacity(4);

    let started_at_utc = now_utc();
    match collect_uname() {
        Ok(value) => {
            let observed_at_utc = now_utc();
            identity.kernel = Some(value);
            outcomes.push(ComponentOutcome::collected(
                UNAME_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error) => outcomes.push(error.with_timing(started_at_utc, now_utc())),
    }

    let started_at_utc = now_utc();
    match collect_distribution() {
        Ok(value) => {
            let observed_at_utc = now_utc();
            identity.distribution = Some(value);
            outcomes.push(ComponentOutcome::collected(
                OS_RELEASE_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error) => outcomes.push(error.with_timing(started_at_utc, now_utc())),
    }

    let started_at_utc = now_utc();
    match collect_clock_parameters() {
        Ok(value) => {
            let observed_at_utc = now_utc();
            identity.clock = Some(value);
            outcomes.push(ComponentOutcome::collected(
                CLOCK_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error) => outcomes.push(error.with_timing(started_at_utc, now_utc())),
    }

    let started_at_utc = now_utc();
    match collect_uptime(Path::new("/proc/uptime")) {
        Ok(value) => {
            let observed_at_utc = now_utc();
            identity.uptime_microseconds = Some(value);
            outcomes.push(ComponentOutcome::collected(
                UPTIME_SOURCE,
                started_at_utc,
                now_utc(),
                observed_at_utc,
            ));
        }
        Err(error) => outcomes.push(error.with_timing(started_at_utc, now_utc())),
    }

    (identity, outcomes)
}

fn collect_uname() -> Result<KernelIdentity, ComponentOutcome> {
    // SAFETY: `utsname` is a plain C structure and `uname` initializes it on success.
    let mut value: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `value` points to writable memory of exactly the expected type.
    if unsafe { libc::uname(&mut value) } != 0 {
        let error = io::Error::last_os_error();
        return Err(io_outcome(UNAME_SOURCE, "uname_failed", &error));
    }

    Ok(KernelIdentity {
        hostname: uts_field(&value.nodename, "invalid_uname_nodename")?,
        sysname: uts_field(&value.sysname, "invalid_uname_sysname")?,
        release: uts_field(&value.release, "invalid_uname_release")?,
        version: uts_field(&value.version, "invalid_uname_version")?,
        machine: uts_field(&value.machine, "invalid_uname_machine")?,
    })
}

fn uts_field(field: &[libc::c_char], error_code: &'static str) -> Result<String, ComponentOutcome> {
    // c_char is i8 on some targets and u8 on others (e.g. aarch64 Linux).
    #[allow(clippy::unnecessary_cast)]
    let bytes: Vec<u8> = field
        .iter()
        .map(|character| *character as u8)
        .take_while(|character| *character != 0)
        .collect();
    String::from_utf8(bytes).map_err(|_| {
        ComponentOutcome::failed(UNAME_SOURCE, CollectionStatus::ParseError, error_code, None)
    })
}

fn collect_distribution() -> Result<DistributionIdentity, ComponentOutcome> {
    let primary = Path::new("/etc/os-release");
    let fallback = Path::new("/usr/lib/os-release");

    let content = match read_bounded(primary) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => match read_bounded(fallback) {
            Ok(content) => content,
            Err(fallback_error) if fallback_error.kind() == io::ErrorKind::NotFound => {
                return Err(ComponentOutcome::failed(
                    OS_RELEASE_SOURCE,
                    CollectionStatus::Unsupported,
                    "os_release_not_found",
                    fallback_error.raw_os_error(),
                ));
            }
            Err(error) => {
                return Err(io_outcome(
                    OS_RELEASE_SOURCE,
                    "os_release_read_failed",
                    &error,
                ));
            }
        },
        Err(error) => {
            return Err(io_outcome(
                OS_RELEASE_SOURCE,
                "os_release_read_failed",
                &error,
            ));
        }
    };

    parse_os_release(&content).map_err(|code| {
        ComponentOutcome::failed(OS_RELEASE_SOURCE, CollectionStatus::ParseError, code, None)
    })
}

fn parse_os_release(content: &str) -> Result<DistributionIdentity, &'static str> {
    let mut identity = DistributionIdentity::default();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        if !matches!(key, "ID" | "VERSION_ID" | "PRETTY_NAME") {
            continue;
        }

        let value = parse_os_release_value(raw_value)?;
        match key {
            "ID" => identity.id = Some(value),
            "VERSION_ID" => identity.version_id = Some(value),
            "PRETTY_NAME" => identity.pretty_name = Some(value),
            _ => unreachable!("the key was allowlisted above"),
        }
    }

    Ok(identity)
}

fn parse_os_release_value(raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }

    let (value, quoted) = match (raw.as_bytes().first(), raw.as_bytes().last()) {
        (Some(b'"'), Some(b'"')) if raw.len() >= 2 => (&raw[1..raw.len() - 1], true),
        (Some(b'\''), Some(b'\'')) if raw.len() >= 2 => (&raw[1..raw.len() - 1], true),
        (Some(b'"') | Some(b'\''), _) => return Err("invalid_os_release_quote"),
        _ => (raw, false),
    };

    if !quoted {
        return Ok(value.to_owned());
    }

    let mut parsed = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            parsed.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            parsed.push(character);
        }
    }
    if escaped {
        return Err("invalid_os_release_escape");
    }
    Ok(parsed)
}

fn collect_clock_parameters() -> Result<ClockParameters, ComponentOutcome> {
    // SAFETY: sysconf is thread-safe for these immutable process parameters.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    // SAFETY: sysconf is thread-safe for these immutable process parameters.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if page_size <= 0 || ticks_per_second <= 0 {
        return Err(ComponentOutcome::failed(
            CLOCK_SOURCE,
            CollectionStatus::IoError,
            "sysconf_failed",
            io::Error::last_os_error().raw_os_error(),
        ));
    }

    Ok(ClockParameters {
        page_size_bytes: page_size as u64,
        ticks_per_second: ticks_per_second as u64,
    })
}

fn collect_uptime(path: &Path) -> Result<u64, ComponentOutcome> {
    let content = read_bounded(path)
        .map_err(|error| io_outcome(UPTIME_SOURCE, "uptime_read_failed", &error))?;
    let field = content.split_whitespace().next().ok_or_else(|| {
        ComponentOutcome::failed(
            UPTIME_SOURCE,
            CollectionStatus::ParseError,
            "invalid_uptime",
            None,
        )
    })?;
    parse_decimal_microseconds(field).ok_or_else(|| {
        ComponentOutcome::failed(
            UPTIME_SOURCE,
            CollectionStatus::ParseError,
            "invalid_uptime",
            None,
        )
    })
}

fn parse_decimal_microseconds(value: &str) -> Option<u64> {
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let seconds = seconds.parse::<u64>().ok()?;
    let mut micros = 0_u64;
    for byte in fraction.bytes().take(6) {
        micros = micros.checked_mul(10)?.checked_add((byte - b'0') as u64)?;
    }
    for _ in fraction.len().min(6)..6 {
        micros = micros.checked_mul(10)?;
    }
    if fraction
        .as_bytes()
        .get(6)
        .is_some_and(|digit| *digit >= b'5')
    {
        micros = micros.checked_add(1)?;
    }

    seconds.checked_mul(1_000_000)?.checked_add(micros)
}

pub fn read_bounded(path: &Path) -> io::Result<String> {
    const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source exceeds collection bound",
        ));
    }
    let mut content = String::new();
    use std::io::Read;
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source exceeds collection bound",
        ));
    }
    Ok(content)
}

fn io_outcome(
    source: &'static str,
    error_code: &'static str,
    error: &io::Error,
) -> ComponentOutcome {
    let status = match error.kind() {
        io::ErrorKind::NotFound => CollectionStatus::Unsupported,
        io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
        _ => CollectionStatus::IoError,
    };
    ComponentOutcome::failed(source, status, error_code, error.raw_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_parser_keeps_only_allowlisted_fields() {
        let canary = "postgres://secret-user:secret-password@example.test/db";
        let parsed = parse_os_release(&format!(
            "ID=alpine\nVERSION_ID=\"3.22\"\nPRETTY_NAME='Alpine Linux 3.22'\nSECRET={canary}\n"
        ))
        .unwrap();

        assert_eq!(parsed.id.as_deref(), Some("alpine"));
        assert_eq!(parsed.version_id.as_deref(), Some("3.22"));
        assert_eq!(parsed.pretty_name.as_deref(), Some("Alpine Linux 3.22"));
        assert!(!format!("{parsed:?}").contains(canary));
    }

    #[test]
    fn os_release_parser_handles_escaped_quotes() {
        let parsed = parse_os_release("PRETTY_NAME=\"Example \\\"Linux\\\"\"\n").unwrap();
        assert_eq!(parsed.pretty_name.as_deref(), Some("Example \"Linux\""));
    }

    #[test]
    fn decimal_seconds_are_rounded_to_microseconds() {
        assert_eq!(parse_decimal_microseconds("12"), Some(12_000_000));
        assert_eq!(parse_decimal_microseconds("12.3"), Some(12_300_000));
        assert_eq!(parse_decimal_microseconds("12.0000004"), Some(12_000_000));
        assert_eq!(parse_decimal_microseconds("12.0000005"), Some(12_000_001));
        assert_eq!(parse_decimal_microseconds("-1.0"), None);
    }
}
