//! Sanitization for artifact filenames.
//!
//! PostgreSQL connection metadata uses its own bounded positive allowlist.
//! Generic pattern-matching redaction is intentionally not offered because it
//! cannot establish the sensitive-data boundary.

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
}
