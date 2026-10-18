//! Allowlisted PostgreSQL runtime-setting capture.
//!
//! Issue #16 only authorizes capturing a published, narrow subset of
//! `pg_settings`. Anything outside that allowlist is skipped so unsafe or
//! sensitive parameters (paths, secrets, network endpoints, replica
//! identifiers) never enter the artifact.

use std::fmt;

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_bool, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::settings_allowlist::{is_allowlisted, ALLOWLIST};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub name: String,
    pub value: String,
    pub unit: Option<String>,
    pub value_type: String,
    pub source: String,
    pub pending_restart: bool,
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={} (source={})", self.name, self.value, self.source)
    }
}

/// `pg_settings.pending_restart` first shipped in PostgreSQL 9.5.
const PENDING_RESTART_MIN_VERSION: i32 = 90500;

/// Collect the approved `pg_settings` allowlist in canonical order. Each
/// setting is recorded only if every column parses cleanly. A parse or
/// permission failure on a single setting does not abort the rest of the
/// allowlist, and the structured outcome records exactly which entry failed.
///
/// `server_version_num` is `None` when target collection failed; the query
/// then assumes a server with `pending_restart`. Older servers report `false`.
pub fn collect_settings(
    client: &ConnectedClient,
    server_version_num: Option<i32>,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<Setting>, AdapterError> {
    let started_at_utc = now_utc();
    let allowlist_literal = ALLOWLIST
        .iter()
        .map(|name| format!("'{}'", name.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let pending_restart = match server_version_num {
        Some(version) if version < PENDING_RESTART_MIN_VERSION => "false",
        _ => "pending_restart",
    };
    let query = format!(
        "\
SELECT name, setting, unit, vartype, source, {} \
FROM pg_settings \
WHERE name IN ({}) \
ORDER BY name",
        pending_restart, allowlist_literal
    );
    let settings_source = "postgres.settings";
    let rows = client.exec_bounded_rows(&query, settings_source, outcomes)?;
    let _ = started_at_utc;
    let mut collected = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        let name = parse_text_string(&row[0])?;
        if !is_allowlisted(&name) {
            continue;
        }
        let value = parse_text_string(&row[1])?;
        let unit = if row[2].is_empty() {
            None
        } else {
            Some(parse_text_string(&row[2])?)
        };
        let value_type = parse_text_string(&row[3])?;
        let source = parse_text_string(&row[4])?;
        let pending_restart = parse_text_bool(&row[5])?;
        collected.push(Setting {
            name,
            value,
            unit,
            value_type,
            source,
            pending_restart,
        });
    }
    if collected.is_empty() {
        outcomes.push(ComponentOutcome::failed(
            "postgres.settings",
            CollectionStatus::PermissionDenied,
            "settings_unavailable",
            None,
        ));
    }
    Ok(collected)
}

#[cfg(test)]
mod tests {
    use crate::postgres::settings_allowlist::{is_allowlisted, ALLOWLIST, SHARED_BUFFERS};

    #[test]
    fn empty_allowlist_returns_empty_collection() {
        assert!(ALLOWLIST.contains(&SHARED_BUFFERS));
        assert!(is_allowlisted(SHARED_BUFFERS));
    }
}
