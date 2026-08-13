//! Centralized sanitization for connection metadata and filesystem-safe names.
//!
//! All connection metadata (collector hostname, PostgreSQL host/socket, port,
//! database name, connected user) must pass through [`sanitize_metadata`] before
//! logging or persistence. Hostnames used in artifact filenames pass through
//! [`sanitize_hostname`] to guarantee a portable, collision-free name.

/// Redacts known secret patterns from a string and returns a sanitized copy.
pub fn sanitize_metadata(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for segment in value.split(',') {
        let trimmed = segment.trim();
        if is_likely_dsn(trimmed) {
            result.push_str(&redact_dsn(trimmed));
        } else if contains_password_fragment(trimmed) {
            result.push_str("<redacted>");
        } else {
            result.push_str(trimmed);
        }
        result.push(',');
    }
    result.trim_end_matches(',').to_string()
}

/// Sanitizes a hostname for use in an artifact filename.
///
/// The result is lowercase, contains only `[a-z0-9-]`, has no leading or
/// trailing hyphens, and collapses consecutive hyphens. An empty or
/// all-invalid input yields `"unknown"`.
pub fn sanitize_hostname(name: &str) -> String {
    let mut sanitized: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    sanitized = sanitized.trim_matches('-').to_string();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn is_likely_dsn(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.starts_with("postgres://")
        || lower.starts_with("postgresql://")
        || (lower.contains("://") && lower.contains('@'))
}

fn redact_dsn(value: &str) -> String {
    if let Some(scheme_end) = value.find("://") {
        let after_scheme = &value[scheme_end + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            let credentials = &after_scheme[..at_pos];
            let rest = &after_scheme[at_pos..];
            let scheme = &value[..scheme_end + 3];
            let user = credentials.split(':').next().unwrap_or("");
            return format!("{scheme}{user}:<redacted>{rest}");
        }
    }
    "<redacted>".to_string()
}

fn contains_password_fragment(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("password=")
        || lower.contains("passwd=")
        || lower.contains("pwd=")
        || lower.contains("token=")
        || lower.contains("secret=")
        || lower.contains("api_key=")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_keeps_alphanumerics_and_replaces_the_rest() {
        assert_eq!(
            sanitize_hostname("My-Host_01.example.com"),
            "my-host-01-example-com"
        );
    }

    #[test]
    fn hostname_collapses_consecutive_hyphens() {
        assert_eq!(sanitize_hostname("a---b"), "a-b");
    }

    #[test]
    fn hostname_strips_leading_and_trailing_hyphens() {
        assert_eq!(sanitize_hostname("---abc---"), "abc");
    }

    #[test]
    fn hostname_defaults_to_unknown_for_empty_input() {
        assert_eq!(sanitize_hostname(""), "unknown");
        assert_eq!(sanitize_hostname("---"), "unknown");
        assert_eq!(sanitize_hostname("..."), "unknown");
    }

    #[test]
    fn metadata_redacts_password_in_dsn() {
        let canary = "postgres://user:supersecret@dbhost:5432/mydb";
        let sanitized = sanitize_metadata(canary);
        assert!(!sanitized.contains("supersecret"));
        assert!(sanitized.contains("<redacted>"));
    }

    #[test]
    fn metadata_redacts_password_fragment() {
        let sanitized = sanitize_metadata("password=hunter2");
        assert!(!sanitized.contains("hunter2"));
        assert!(sanitized.contains("<redacted>"));
    }

    #[test]
    fn metadata_preserves_non_secret_values() {
        assert_eq!(sanitize_metadata("localhost"), "localhost");
        assert_eq!(sanitize_metadata("5432"), "5432");
    }

    #[test]
    fn metadata_handles_multiple_comma_separated_values() {
        let sanitized = sanitize_metadata("localhost,password=secret,5432");
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains("localhost"));
        assert!(sanitized.contains("5432"));
    }
}
