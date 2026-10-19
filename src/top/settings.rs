//! The display options the operator sets in `top`, kept between runs in
//! `$XDG_CONFIG_HOME/pg-probe/top.json` (or `~/.config/pg-probe/top.json`).
//! The view is not kept, so a new run never starts in a view that sends
//! extra queries to the server.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Group;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preview: bool,
    pub tree: bool,
    pub compact: bool,
    pub group: Group,
    pub compact_header: bool,
}

/// Where the settings live, or `None` when neither `XDG_CONFIG_HOME` nor
/// `HOME` is set.
pub fn path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("pg-probe").join("top.json"))
}

/// Reads the settings. A missing or unreadable file gives the defaults,
/// and unknown or missing fields are ignored.
pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the settings through a temporary file, so a crash never leaves
/// half a file behind.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    fs::write(&temporary, text + "\n")?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pg-probe").join("top.json");
        assert_eq!(load(&path), Settings::default());
        let settings = Settings {
            compact: true,
            group: Group::Database,
            ..Settings::default()
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
        assert!(fs::read_to_string(&path).unwrap().contains("\"database\""));
    }

    #[test]
    fn a_broken_or_partial_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("top.json");
        fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        fs::write(&path, r#"{"tree": true, "unknown": 1}"#).unwrap();
        assert_eq!(
            load(&path),
            Settings {
                tree: true,
                ..Settings::default()
            }
        );
    }
}
