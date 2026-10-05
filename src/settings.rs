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
    /// The verb prints the raw value (a number, a boolean or a string); it is kept as display text.
    #[serde(deserialize_with = "display_value")]
    pub value: String,
    // The verb names the source file `source_file`; `file` and `source` are accepted as spellings of the same field.
    #[serde(rename = "source_file", alias = "file", alias = "source")]
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
    // `cox settings get --json` names a section `name`; `id` is accepted as the same field.
    #[serde(alias = "name")]
    id: String,
    rows: Vec<SettingsRow>,
}

/// A JSON value as the text the screen shows: a string as itself, null as empty, anything else
/// in its JSON spelling (`1`, `true`, `0.5`).
fn display_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(text) => text,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
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

    #[test]
    fn parses_the_verbs_own_output_with_raw_values_and_named_sections() {
        let json = r#"{"sections":[{"name":"lanes and machines","rows":[
            {"section":"lanes and machines","scope":"cartridge","key":"policy.dispatch.max_in_flight","value":1,"source_file":"cartridge.yaml","tracked":true,"pat_only":false},
            {"section":"lanes and machines","scope":"cartridge","key":"policy.flag","value":true,"source_file":"cartridge.yaml","tracked":true,"pat_only":false},
            {"section":"lanes and machines","scope":"profile","key":"workspace_dir","value":"~/repos/workspace","source_file":"profile.yaml","tracked":false,"pat_only":false}]}]}"#;
        let snap = parse(json).expect("the verb's output parses");
        assert_eq!(snap.sections[0].id, "lanes and machines");
        let values: Vec<&str> = snap.sections[0]
            .rows
            .iter()
            .map(|r| r.value.as_str())
            .collect();
        assert_eq!(values, ["1", "true", "~/repos/workspace"]);
        assert_eq!(snap.sections[0].rows[0].source, "cartridge.yaml");
    }
}
