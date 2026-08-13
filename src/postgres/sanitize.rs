use std::fmt;

/// Maximum length accepted for a single sanitized identifier. PostgreSQL caps
/// most identifiers at 63 bytes (NAMEDATALEN minus one) by default; anything
/// longer than that is rejected so it cannot be persisted verbatim and used as
/// an injection vector inside SQLite identifiers or log lines.
pub const MAX_IDENTIFIER_BYTES: usize = 63;

/// Maximum length accepted for an endpoint host or directory string. PostgreSQL
/// Unix socket paths and hostnames both fit well under this bound; we truncate
/// anything longer so that bounded reads stay predictable.
pub const MAX_HOST_BYTES: usize = 1024;

const SAFE_MARKER: &str = "<invalid>";

/// Result of sanitizing an operational identifier. `value` is always safe to
/// persist or print; `redacted` is true when the input contained characters we
/// do not accept and was replaced with the safe marker instead of being echoed
/// back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedIdentifier {
    pub value: String,
    pub redacted: bool,
}

impl fmt::Display for SanitizedIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

/// Accept ASCII printable characters other than controls and the double quote.
///
/// PostgreSQL identifiers may legitimately contain Unicode in QUOTE_IDENT
/// contexts, but we keep the allowlist strict to avoid leaking arbitrary user
/// input back into diagnostic artifacts. Inputs that fail this rule are
/// replaced with a published safe marker rather than echoing the original.
fn is_allowed_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_graphic() && byte != b'"' && byte != b'\''
}

pub fn sanitize_identifier(input: &str) -> SanitizedIdentifier {
    let trimmed = input.trim_matches('\0').trim();
    if trimmed.is_empty() {
        return SanitizedIdentifier {
            value: SAFE_MARKER.to_owned(),
            redacted: true,
        };
    }
    if trimmed.len() > MAX_IDENTIFIER_BYTES || !trimmed.bytes().all(is_allowed_identifier_byte) {
        return SanitizedIdentifier {
            value: SAFE_MARKER.to_owned(),
            redacted: true,
        };
    }
    SanitizedIdentifier {
        value: trimmed.to_owned(),
        redacted: false,
    }
}

/// Accept endpoint hosts the way PostgreSQL itself accepts them: printable
/// ASCII without control characters or quotes.
///
/// Hostnames may include dashes and dots. IPv6 zones are rare in practice and
/// rejected here; libpq will fall back to numeric addresses on its own.
pub fn sanitize_host(input: &str) -> SanitizedIdentifier {
    let trimmed = input.trim_matches('\0').trim();
    if trimmed.is_empty() {
        return SanitizedIdentifier {
            value: SAFE_MARKER.to_owned(),
            redacted: true,
        };
    }
    if trimmed.len() > MAX_HOST_BYTES || trimmed.contains('\n') || trimmed.contains('\r') {
        return SanitizedIdentifier {
            value: SAFE_MARKER.to_owned(),
            redacted: true,
        };
    }
    let mut bytes = trimmed.bytes();
    let first = bytes.next().expect("trimmed non-empty");
    let is_unix_socket = first == b'/';
    let allowed = |byte: u8| byte.is_ascii_graphic() && byte != b'"' && byte != b'\'';
    if is_unix_socket {
        if !trimmed.bytes().all(allowed) {
            return SanitizedIdentifier {
                value: SAFE_MARKER.to_owned(),
                redacted: true,
            };
        }
    } else {
        let first_ok =
            is_unix_socket || (first.is_ascii_alphanumeric() || first == b'.' || first == b'[');
        if !first_ok || !trimmed.bytes().all(allowed) {
            return SanitizedIdentifier {
                value: SAFE_MARKER.to_owned(),
                redacted: true,
            };
        }
    }
    SanitizedIdentifier {
        value: trimmed.to_owned(),
        redacted: false,
    }
}

/// Parse a PostgreSQL port from a libpq-style numeric string. Returns None
/// when the input is not a valid port in the IANA-registered range.
pub fn parse_port(input: &str) -> Option<u16> {
    let trimmed = input.trim_matches('\0').trim();
    if trimmed.is_empty() || trimmed.len() > 6 {
        return None;
    }
    if !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match trimmed.parse::<u16>() {
        Ok(port) if port > 0 => Some(port),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_passes_through_printable_ascii() {
        let sanitized = sanitize_identifier("postgres_user");
        assert_eq!(sanitized.value, "postgres_user");
        assert!(!sanitized.redacted);
    }

    #[test]
    fn identifier_rejects_double_quote() {
        let sanitized = sanitize_identifier("a\"b");
        assert_eq!(sanitized.value, SAFE_MARKER);
        assert!(sanitized.redacted);
    }

    #[test]
    fn identifier_rejects_control_characters() {
        let sanitized = sanitize_identifier("a\nb");
        assert_eq!(sanitized.value, SAFE_MARKER);
        assert!(sanitized.redacted);
    }

    #[test]
    fn identifier_rejects_empty_and_overlong() {
        assert_eq!(sanitize_identifier("").value, SAFE_MARKER);
        assert_eq!(sanitize_identifier("   ").value, SAFE_MARKER);
        let long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        assert_eq!(sanitize_identifier(&long).value, SAFE_MARKER);
    }

    #[test]
    fn host_accepts_unix_socket_paths() {
        let sanitized = sanitize_host("/var/run/postgresql/.s.PGSQL.5432");
        assert_eq!(sanitized.value, "/var/run/postgresql/.s.PGSQL.5432");
        assert!(!sanitized.redacted);
    }

    #[test]
    fn host_accepts_ip_literal_forms() {
        assert_eq!(sanitize_host("127.0.0.1").value, "127.0.0.1");
        assert_eq!(sanitize_host("[::1]").value, "[::1]");
    }

    #[test]
    fn host_rejects_newline_injection() {
        let sanitized = sanitize_host("good\nbad");
        assert_eq!(sanitized.value, SAFE_MARKER);
        assert!(sanitized.redacted);
    }

    #[test]
    fn host_rejects_quotes_and_overlong_values() {
        let sanitized = sanitize_host("a'b");
        assert_eq!(sanitized.value, SAFE_MARKER);
        let long = "a".repeat(MAX_HOST_BYTES + 1);
        assert_eq!(sanitize_host(&long).value, SAFE_MARKER);
    }

    #[test]
    fn port_validation_accepts_canonical_values() {
        assert_eq!(parse_port("5432"), Some(5432));
        assert_eq!(parse_port(" 65432 "), Some(65432));
    }

    #[test]
    fn port_validation_rejects_invalid_inputs() {
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("65536"), None);
        assert_eq!(parse_port("abc"), None);
        assert_eq!(parse_port("5432a"), None);
        assert_eq!(parse_port(""), None);
    }
}
