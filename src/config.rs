//! Persisting the user's page and theme choice across runs. `serialize_state` and `parse_state`
//! are the pure core: a literal TOML string in, a literal string out, no I/O either way.
//! `state_path`, `load_state` and `save_state` are the thin edge that reads and writes that
//! string to disk, and never let an I/O failure become a panic or a startup crash.

// `state_path` isn't called yet: the startup/shutdown wiring that would use it lands with the
// UI, so clippy would otherwise flag it as dead code.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::app::{AppPage, ThemeId};

#[derive(serde::Deserialize)]
struct StateFile {
    page: Option<String>,
    theme: Option<String>,
}

fn page_str(page: AppPage) -> &'static str {
    match page {
        AppPage::Regatta => "regatta",
        AppPage::Slipstream => "slipstream",
    }
}

fn theme_str(theme: ThemeId) -> &'static str {
    match theme {
        ThemeId::Regatta => "regatta",
        ThemeId::HarborLight => "harbor_light",
    }
}

pub fn serialize_state(page: AppPage, theme: ThemeId) -> String {
    format!(
        "page = \"{}\"\ntheme = \"{}\"\n",
        page_str(page),
        theme_str(theme)
    )
}

/// Returns `None` on anything that isn't exactly the two known fields with a recognised value,
/// so a corrupt or future-versioned state file is ignored rather than fatal.
pub fn parse_state(toml_str: &str) -> Option<(AppPage, ThemeId)> {
    let parsed: StateFile = toml::from_str(toml_str).ok()?;
    let page = match parsed.page.as_deref() {
        Some("regatta") => AppPage::Regatta,
        Some("slipstream") => AppPage::Slipstream,
        _ => return None,
    };
    let theme = match parsed.theme.as_deref() {
        Some("regatta") => ThemeId::Regatta,
        Some("harbor_light") => ThemeId::HarborLight,
        _ => return None,
    };
    Some((page, theme))
}

// edge
/// The state file's path: the `dirs` crate's config dir (falling back to the system temp dir
/// when no config dir is reported) joined with `coxtop/state.toml`.
pub fn state_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("coxtop")
        .join("state.toml")
}

/// Reads and parses the state file. Any I/O failure (missing file, missing directory, unreadable
/// permissions) yields `None`, the same as a corrupt file; it never panics.
pub fn load_state(path: &Path) -> Option<(AppPage, ThemeId)> {
    let contents = std::fs::read_to_string(path).ok()?;
    parse_state(&contents)
}

/// Writes the state file, creating its parent directory first. Any I/O failure is a silent
/// no-op: a missing or unwritable config directory must never crash startup.
pub fn save_state(path: &Path, page: AppPage, theme: ThemeId) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let _ = std::fs::write(path, serialize_state(page, theme));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_state_produces_the_literal_toml() {
        assert_eq!(
            serialize_state(AppPage::Regatta, ThemeId::HarborLight),
            "page = \"regatta\"\ntheme = \"harbor_light\"\n"
        );
        assert_eq!(
            serialize_state(AppPage::Slipstream, ThemeId::Regatta),
            "page = \"slipstream\"\ntheme = \"regatta\"\n"
        );
    }

    #[test]
    fn round_trips_regatta_and_harbor_light() {
        let toml_str = serialize_state(AppPage::Regatta, ThemeId::HarborLight);
        assert_eq!(
            parse_state(&toml_str),
            Some((AppPage::Regatta, ThemeId::HarborLight))
        );
    }

    #[test]
    fn round_trips_slipstream_and_regatta() {
        let toml_str = serialize_state(AppPage::Slipstream, ThemeId::Regatta);
        assert_eq!(
            parse_state(&toml_str),
            Some((AppPage::Slipstream, ThemeId::Regatta))
        );
    }

    #[test]
    fn parse_state_rejects_garbage() {
        assert_eq!(parse_state("not = toml [[["), None);
    }

    #[test]
    fn parse_state_rejects_unrecognised_values() {
        assert_eq!(
            parse_state("page = \"starboard\"\ntheme = \"regatta\"\n"),
            None
        );
    }

    #[test]
    fn load_save_round_trip_through_a_tempfile_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("state.toml");
        save_state(&path, AppPage::Slipstream, ThemeId::HarborLight);
        assert_eq!(
            load_state(&path),
            Some((AppPage::Slipstream, ThemeId::HarborLight))
        );
    }

    #[test]
    fn load_state_on_a_missing_path_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_state(&path), None);
    }
}
