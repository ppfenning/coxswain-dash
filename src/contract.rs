//! The coxswain feed contract: the version this towpath reads, the pure check of a feed line's
//! version against it, and the conformance of towpath's own scenario fixtures to the pinned contract.
//!
//! Coxswain sends the version as `schema` and moves it only on a breaking change, so a feed at any
//! other version is refused with a message saying which side to upgrade. `coxswain-contract.toml`
//! pins the coxswain commit whose fixtures `tests/fixtures/coxswain/` holds; the tests below hold
//! that pin, the vendored manifest and `CONTRACT_VERSION` to one number.

use std::fmt;

/// The contract version this towpath parses. Raised only together with a pin bump that brings
/// fixtures at the new version and a parser that reads them.
pub const CONTRACT_VERSION: u64 = 1;

/// Why a feed line is refused before it is parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// The line is JSON but carries no integer `schema`.
    Missing,
    /// Coxswain sends a newer contract than this towpath reads.
    Newer(u64),
    /// Coxswain sends an older contract than this towpath reads.
    Older(u64),
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mismatch::Missing => write!(
                f,
                "the feed carries no contract version (`schema`); this towpath reads contract {CONTRACT_VERSION}"
            ),
            Mismatch::Newer(found) => write!(
                f,
                "the feed is contract {found}, newer than the contract {CONTRACT_VERSION} this towpath reads: upgrade towpath"
            ),
            Mismatch::Older(found) => write!(
                f,
                "the feed is contract {found}, older than the contract {CONTRACT_VERSION} this towpath reads: upgrade coxswain"
            ),
        }
    }
}

/// Whether a feed at contract `found` can be read by a towpath built for `supported`.
pub fn check(found: Option<u64>, supported: u64) -> Result<(), Mismatch> {
    match found {
        None => Err(Mismatch::Missing),
        Some(version) if version > supported => Err(Mismatch::Newer(version)),
        Some(version) if version < supported => Err(Mismatch::Older(version)),
        Some(_) => Ok(()),
    }
}

/// The contract version a parsed feed line carries as `schema`; None when it carries none.
pub fn version_of(value: &serde_json::Value) -> Option<u64> {
    value.get("schema").and_then(serde_json::Value::as_u64)
}

/// Every key path `value` shows: `.a.b` for objects, `[]` for any array element. A key under one
/// of `maps` (a path whose object is keyed by data, such as a repo name) is written `*`.
#[cfg(test)]
pub fn key_paths(value: &serde_json::Value, maps: &[&str]) -> std::collections::BTreeSet<String> {
    fn walk(
        value: &serde_json::Value,
        path: String,
        maps: &[&str],
        out: std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeSet<String> {
        match value {
            serde_json::Value::Object(map) => map.iter().fold(out, |acc, (key, child)| {
                let step = if maps.contains(&path.as_str()) {
                    "*"
                } else {
                    key.as_str()
                };
                let next = format!("{path}.{step}");
                let acc = acc.into_iter().chain([next.clone()]).collect();
                walk(child, next, maps, acc)
            }),
            serde_json::Value::Array(items) => items.iter().fold(out, |acc, child| {
                walk(child, format!("{path}[]"), maps, acc)
            }),
            _ => out,
        }
    }
    walk(
        value,
        String::new(),
        maps,
        std::collections::BTreeSet::new(),
    )
}

/// Test-only: the conformance check below runs in `cargo test`, never in the binary.
/// The key paths a towpath scenario fixture shows that no pinned contract fixture shows, less the
/// `allowed` ones; empty when the scenario only uses what coxswain's contract fixes.
#[cfg(test)]
pub fn unknown_paths(
    scenario: &serde_json::Value,
    contract: &[serde_json::Value],
    maps: &[&str],
    allowed: &[&str],
) -> Vec<String> {
    let known = contract
        .iter()
        .flat_map(|fixture| key_paths(fixture, maps))
        .collect::<std::collections::BTreeSet<_>>();
    key_paths(scenario, maps)
        .into_iter()
        .filter(|path| !known.contains(path) && !allowed.contains(&path.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_same_contract_is_read() {
        assert_eq!(check(Some(1), 1), Ok(()));
    }

    #[test]
    fn a_newer_contract_is_refused_with_upgrade_towpath() {
        let refused = check(Some(2), 1).unwrap_err();
        assert_eq!(refused, Mismatch::Newer(2));
        assert!(
            refused.to_string().ends_with("upgrade towpath"),
            "{refused}"
        );
    }

    #[test]
    fn an_older_contract_is_refused_with_upgrade_coxswain() {
        let refused = check(Some(0), 1).unwrap_err();
        assert_eq!(refused, Mismatch::Older(0));
        assert!(
            refused.to_string().ends_with("upgrade coxswain"),
            "{refused}"
        );
    }

    #[test]
    fn a_feed_without_a_version_is_refused() {
        assert_eq!(check(None, 1), Err(Mismatch::Missing));
    }

    #[test]
    fn the_version_is_read_from_schema_and_only_as_an_integer() {
        assert_eq!(version_of(&json!({"schema": 3, "at": ""})), Some(3));
        assert_eq!(version_of(&json!({"schema": "1"})), None);
        assert_eq!(version_of(&json!({"at": ""})), None);
        assert_eq!(version_of(&json!([1])), None);
    }

    #[test]
    fn key_paths_name_objects_arrays_and_data_keyed_maps() {
        let value = json!({"a": {"b": 1}, "rows": [{"c": 1}, {"d": 2}], "repos": {"x": {"e": 1}}});
        let paths = key_paths(&value, &[".repos"]);
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            vec![
                ".a",
                ".a.b",
                ".repos",
                ".repos.*",
                ".repos.*.e",
                ".rows",
                ".rows[].c",
                ".rows[].d"
            ]
        );
    }

    #[test]
    fn a_scenario_key_the_contract_does_not_fix_is_unknown_unless_allowed() {
        let contract = [json!({"spend": {"five_hour_fraction": 0.1}})];
        let scenario = json!({"spend": {"five_hour_fraction": 0.2, "made_up": 1}});
        assert_eq!(
            unknown_paths(&scenario, &contract, &[], &[]),
            vec![".spend.made_up"]
        );
        assert!(unknown_paths(&scenario, &contract, &[], &[".spend.made_up"]).is_empty());
    }

    // The pin, the vendored manifest and this build agree; the vendored fixtures and towpath's own
    // scenarios against them. These read files compiled in, so they need no I/O at run time.

    const PIN: &str = include_str!("../coxswain-contract.toml");
    const MANIFEST: &str = include_str!("../tests/fixtures/coxswain/dash_contract.json");

    /// Every vendored coxswain fixture, by name. `scripts/coxswain_contract.py verify` holds this
    /// folder to the pinned commit; `every_vendored_fixture_is_compiled_in` holds this list to it.
    const PINNED: &[(&str, &str)] = &[
        (
            "dash_activity_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_activity_v1.json"),
        ),
        (
            "dash_budget_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_budget_v1.json"),
        ),
        (
            "dash_detail_health_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_detail_health_v1.json"),
        ),
        (
            "dash_detail_initiative_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_detail_initiative_v1.json"),
        ),
        (
            "dash_detail_machine_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_detail_machine_v1.json"),
        ),
        (
            "dash_detail_run_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_detail_run_v1.json"),
        ),
        (
            "dash_detail_spend_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_detail_spend_v1.json"),
        ),
        (
            "dash_feed_board_slim_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_board_slim_v1.json"),
        ),
        (
            "dash_feed_board_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_board_v1.json"),
        ),
        (
            "dash_feed_cost_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_cost_v1.json"),
        ),
        (
            "dash_feed_epics_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_epics_v1.json"),
        ),
        (
            "dash_feed_intake_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_intake_v1.json"),
        ),
        (
            "dash_feed_schedule_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_schedule_v1.json"),
        ),
        (
            "dash_feed_task_checks_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_task_checks_v1.json"),
        ),
        (
            "dash_feed_tasks_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_tasks_v1.json"),
        ),
        (
            "dash_feed_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_v1.json"),
        ),
        (
            "dash_feed_waves_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_feed_waves_v1.json"),
        ),
        (
            "dash_sessions_v1.json",
            include_str!("../tests/fixtures/coxswain/dash_sessions_v1.json"),
        ),
    ];

    /// Towpath's own scenario fixtures: inputs its rendering tests pin values against. They are
    /// not copies of coxswain's, but they may only use keys coxswain's pinned fixtures fix.
    const SCENARIOS: &[(&str, &str)] = &[
        (
            "dash_detail_health_v1.json",
            include_str!("../tests/fixtures/towpath/dash_detail_health_v1.json"),
        ),
        (
            "dash_detail_spend_v1.json",
            include_str!("../tests/fixtures/towpath/dash_detail_spend_v1.json"),
        ),
        (
            "dash_feed_board_v1.json",
            include_str!("../tests/fixtures/towpath/dash_feed_board_v1.json"),
        ),
        (
            "dash_feed_tasks_v1.json",
            include_str!("../tests/fixtures/towpath/dash_feed_tasks_v1.json"),
        ),
        (
            "dash_feed_v1.json",
            include_str!("../tests/fixtures/towpath/dash_feed_v1.json"),
        ),
        (
            "dash_feed_watch_v1.json",
            include_str!("../tests/fixtures/towpath/dash_feed_watch_v1.json"),
        ),
        (
            "dash_sessions_v1.json",
            include_str!("../tests/fixtures/towpath/dash_sessions_v1.json"),
        ),
    ];

    /// Objects keyed by data rather than by schema: a machine's checkouts are keyed by repo name.
    const DATA_KEYED: &[&str] = &[".machines[].checkouts"];

    /// Watch rows: coxswain's fixtures carry `watch: []`, so none fixes the row's keys yet. Towpath
    /// reads them as `WatchEntry`; drop this once a pinned fixture carries a watch row.
    const NOT_YET_IN_A_PINNED_FIXTURE: &[&str] = &[
        ".watch[].at",
        ".watch[].id",
        ".watch[].kind",
        ".watch[].state",
        ".watch[].title",
        ".watch[].url",
    ];

    fn parsed(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("fixture is JSON")
    }

    #[test]
    fn the_pin_records_the_contract_this_towpath_reads() {
        let pin: toml::Table = PIN.parse().expect("the pin file is TOML");
        assert_eq!(pin["contract"].as_integer(), Some(CONTRACT_VERSION as i64));
    }

    #[test]
    fn the_pinned_manifest_publishes_the_contract_this_towpath_reads() {
        assert_eq!(
            parsed(MANIFEST)["contract"].as_u64(),
            Some(CONTRACT_VERSION)
        );
    }

    #[test]
    fn every_vendored_fixture_is_compiled_in() {
        let listed: Vec<String> = parsed(MANIFEST)["fixtures"]
            .as_array()
            .expect("fixtures list")
            .iter()
            .map(|name| name.as_str().expect("name").to_string())
            .collect();
        let compiled: Vec<String> = PINNED.iter().map(|(name, _)| name.to_string()).collect();
        assert_eq!(compiled, listed);
    }

    #[test]
    fn every_pinned_fixture_that_carries_a_version_carries_this_one() {
        PINNED.iter().for_each(|(name, text)| {
            let value = parsed(text);
            if value.get("schema").is_some() {
                assert_eq!(
                    check(version_of(&value), CONTRACT_VERSION),
                    Ok(()),
                    "{name}"
                );
            }
        });
    }

    #[test]
    fn the_pinned_feed_and_details_parse() {
        let feed = PINNED
            .iter()
            .find(|(name, _)| *name == "dash_feed_v1.json")
            .expect("feed")
            .1;
        crate::feed::read_snapshot(feed).expect("the pinned feed fixture parses");
        PINNED
            .iter()
            .filter(|(name, _)| name.starts_with("dash_detail_"))
            .for_each(|(name, text)| {
                crate::detail::read_detail(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            });
    }

    #[test]
    fn the_fixture_folder_holds_only_the_readme_and_the_two_fixture_sets() {
        // Read at test time, not compiled in: a stray file or folder beside the two sets would
        // be a fixture no test reads and `verify` does not hold to the pin.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let entries: std::collections::BTreeSet<String> = std::fs::read_dir(&root)
            .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
            .map(|entry| {
                entry
                    .expect("a readable entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            entries,
            ["README.md", "coxswain", "towpath"]
                .map(String::from)
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn towpaths_scenarios_use_only_keys_the_pinned_contract_fixes() {
        let contract: Vec<serde_json::Value> =
            PINNED.iter().map(|(_, text)| parsed(text)).collect();
        SCENARIOS.iter().for_each(|(name, text)| {
            let unknown = unknown_paths(
                &parsed(text),
                &contract,
                DATA_KEYED,
                NOT_YET_IN_A_PINNED_FIXTURE,
            );
            assert!(
                unknown.is_empty(),
                "{name} uses keys coxswain does not send: {unknown:?}"
            );
        });
    }
}
