//! The pure model of the settings screen: the parsed rows of `cox settings get --json`.

use serde::Deserialize;

/// Every section of the verb's output, in the order the verb printed them.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    pub sections: Vec<SettingsSection>,
}

/// One section: its raw id, its screen label, and its rows in given order.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSection {
    pub id: String,
    pub label: String,
    pub rows: Vec<SettingsRow>,
}

/// One setting. `value` is the display string the verb gives, never interpreted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SettingsRow {
    pub section: String,
    pub scope: String,
    pub key: String,
    pub value: String,
    // The verb names the source file `file`; `source` is accepted as a spelling of the same field.
    #[serde(rename = "file", alias = "source")]
    pub source: String,
    pub tracked: bool,
    pub pat_only: bool,
}

#[derive(Deserialize)]
struct WireSnapshot {
    sections: Vec<WireSection>,
}

#[derive(Deserialize)]
struct WireSection {
    id: String,
    rows: Vec<SettingsRow>,
}

/// Screen name for a section id. An unknown id is shown as itself.
pub fn label(id: &str) -> String {
    match id {
        "lanes" => "lanes and machines",
        "spend" => "spend and pacing",
        "builds" => "builds and budgets",
        "models" => "models and tiers",
        "crew" => "crew seats",
        "housekeeping" => "housekeeping",
        "profiles" => "profile files",
        other => other,
    }
    .to_string()
}

/// Where a row's value is stored: a tracked file or this machine only.
pub fn provenance(row: &SettingsRow) -> &'static str {
    if row.tracked {
        "git-tracked"
    } else {
        "machine-local"
    }
}

/// Parse the verb's JSON. Bad JSON or a missing field is an `Err`, never a panic.
pub fn parse(text: &str) -> Result<SettingsSnapshot, String> {
    let wire: WireSnapshot = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(SettingsSnapshot {
        sections: wire
            .sections
            .into_iter()
            .map(|s| SettingsSection {
                label: label(&s.id),
                id: s.id,
                rows: s.rows,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"{"sections":[
        {"id":"lanes","rows":[{"section":"lanes","scope":"repo","key":"lanes.max","value":"4","file":".cox/settings.toml","tracked":true,"pat_only":false}]},
        {"id":"spend","rows":[{"section":"spend","scope":"machine","key":"spend.cap","value":"$20","file":"~/.cox/local.toml","tracked":false,"pat_only":true}]}
    ]}"#;

    fn row(tracked: bool) -> SettingsRow {
        SettingsRow {
            section: "s".into(),
            scope: "repo".into(),
            key: "k".into(),
            value: "v".into(),
            source: "f".into(),
            tracked,
            pat_only: false,
        }
    }

    #[test]
    fn two_sections_parse_in_order_with_all_row_fields() {
        let snap = parse(TWO).unwrap();
        let ids: Vec<&str> = snap.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["lanes", "spend"]);
        assert_eq!(snap.sections[0].label, "lanes and machines");
        assert_eq!(
            snap.sections[0].rows,
            vec![SettingsRow {
                section: "lanes".into(),
                scope: "repo".into(),
                key: "lanes.max".into(),
                value: "4".into(),
                source: ".cox/settings.toml".into(),
                tracked: true,
                pat_only: false,
            }]
        );
        assert_eq!(snap.sections[1].rows[0].value, "$20");
    }

    #[test]
    fn a_pat_only_row_keeps_its_flag() {
        let snap = parse(TWO).unwrap();
        assert!(!snap.sections[0].rows[0].pat_only);
        assert!(snap.sections[1].rows[0].pat_only);
    }

    #[test]
    fn tracked_and_untracked_rows_give_the_two_provenance_strings() {
        assert_eq!(provenance(&row(true)), "git-tracked");
        assert_eq!(provenance(&row(false)), "machine-local");
    }

    #[test]
    fn an_unknown_section_id_keeps_its_raw_name_as_the_label() {
        let text = r#"{"sections":[{"id":"fresh-section","rows":[]}]}"#;
        let snap = parse(text).unwrap();
        assert_eq!(snap.sections[0].label, "fresh-section");
    }

    #[test]
    fn a_missing_field_is_an_err() {
        let text = r#"{"sections":[{"id":"lanes","rows":[{"section":"lanes","scope":"repo","key":"k","value":"1","file":"f","tracked":true}]}]}"#;
        assert!(parse(text).is_err());
    }

    #[test]
    fn invalid_json_is_an_err() {
        assert!(parse("{not json").is_err());
    }
}
