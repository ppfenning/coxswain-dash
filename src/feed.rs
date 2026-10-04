//! The versioned dash feed snapshot: a pure schema for `cox dash --feed` output, plus the
//! thin edge that spawns the feed process. The terminal loop task reads the child's stdout
//! and calls `parse_snapshot` on each line; neither of those happens here.

// Most fields exist only to mirror the wire schema; the page tasks that render them land
// later, so clippy would otherwise flag every field a test doesn't touch as dead code.
#![allow(dead_code)]

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct FeedSnapshot {
    pub schema: u32,
    pub at: String,
    pub chair: Chair,
    pub spend: Spend,
    pub machines: Vec<Machine>,
    pub runs: Vec<Run>,
    pub queue: Vec<QueueEntry>,
    pub inbox: Vec<InboxEntry>,
    pub watch: Vec<serde_json::Value>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
}

#[derive(Debug, Deserialize)]
pub struct Chair {
    pub holder: String,
    pub host: String,
    pub epoch: u32,
    pub liveness: String,
    pub beat_age_s: u64,
    #[serde(default)]
    pub session: String,
    /// A local display string such as "09-28 19:59 EDT", not RFC 3339. Do not pass it to `ui::local_time`.
    #[serde(default)]
    pub last_tick_at: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub current_action: Option<CurrentAction>,
    #[serde(default)]
    pub today: ChairToday,
}

/// What the chair is doing now. `since` is RFC 3339 UTC.
#[derive(Debug, Deserialize)]
pub struct CurrentAction {
    pub kind: String,
    pub target: String,
    pub since: String,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChairToday {
    pub lands: u32,
    pub launches: u32,
    pub refused_or_failed: u32,
    pub needs_chair_open: u32,
}

/// An open question for the chair. `asked_at` is RFC 3339 UTC, as `ui::local_time` expects.
#[derive(Debug, Deserialize)]
pub struct Decision {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    pub context: String,
    pub asked_at: String,
}

#[derive(Debug, Deserialize)]
pub struct Spend {
    pub five_hour_fraction: f64,
    pub five_hour_source: String,
    pub weekly_fraction: f64,
    pub weekly_source: String,
    pub hard_stop_fraction: f64,
    pub five_hour_resets_at: String,
    pub weekly_resets_at: String,
}

#[derive(Debug, Deserialize)]
pub struct Machine {
    pub name: String,
    pub state: String,
    pub lanes_in_use: u32,
    pub capacity: u32,
    pub login_ok: bool,
    pub login_checked_at: String,
    pub beat_age_s: u64,
    pub checkouts: HashMap<String, Checkout>,
}

#[derive(Debug, Deserialize)]
pub struct Checkout {
    pub behind_main: u32,
}

#[derive(Debug, Deserialize)]
pub struct Run {
    pub run: String,
    pub machine: String,
    pub phase: String,
    pub node: String,
    pub attempt: u32,
    pub turns: u32,
    pub cost: f64,
    pub verdict: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct QueueEntry {
    pub initiative: String,
    pub priority: u32,
    pub phases_landed: u32,
    pub phases_total: u32,
    pub current_phase: String,
}

#[derive(Debug, Deserialize)]
pub struct InboxEntry {
    pub kind: String,
    pub target: String,
    pub reason: String,
}

/// Deserializes one feed line into a [`FeedSnapshot`]. Does nothing but deserialize.
pub fn parse_snapshot(line: &str) -> Result<FeedSnapshot, serde_json::Error> {
    serde_json::from_str(line)
}

// edge
/// Spawns `cox dash --feed` (with `--interval` when `interval_secs` is `Some`) with stdout
/// piped. The caller reads the child's stdout; this function only starts the process.
pub fn spawn_feed(interval_secs: Option<u64>) -> std::process::Child {
    let mut command = std::process::Command::new("cox");
    command.arg("dash").arg("--feed");
    if let Some(secs) = interval_secs {
        command.arg("--interval").arg(secs.to_string());
    }
    command
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn `cox dash --feed`")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/dash_feed_v1.json");

    #[test]
    fn parse_snapshot_reads_the_v1_fixture() {
        let snapshot = parse_snapshot(FIXTURE).expect("fixture should parse");
        assert_eq!(snapshot.schema, 1);
        assert_eq!(snapshot.chair.host, "omarchy");
        assert_eq!(snapshot.runs[0].node, "build");
        assert_eq!(snapshot.inbox[0].kind, "needs_chair");
    }

    #[test]
    fn parse_snapshot_reads_chair_session_and_decisions() {
        let snapshot = parse_snapshot(FIXTURE).expect("fixture should parse");
        assert_eq!(snapshot.chair.session, "d2820a82");
        assert_eq!(snapshot.decisions.len(), 1);
    }

    fn with_chair(chair: &str) -> String {
        format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{chair},"spend":{{"five_hour_fraction":0.1,"five_hour_source":"meter","weekly_fraction":0.1,"weekly_source":"meter","hard_stop_fraction":0.9,"five_hour_resets_at":"2026-09-29T02:00:00Z","weekly_resets_at":"2026-10-04T04:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[]}}"#
        )
    }

    const OLD_CHAIR: &str =
        r#"{"holder":"h","host":"omarchy","epoch":1,"liveness":"live","beat_age_s":4"#;

    #[test]
    fn parse_snapshot_reads_the_chair_tick_fields() {
        let chair = parse_snapshot(FIXTURE).expect("fixture should parse").chair;
        assert_eq!(chair.last_tick_at.as_deref(), Some("09-28 19:59 EDT"));
        assert_eq!(
            chair.last_status.as_deref(),
            Some("chair 09-28 19:59 EDT | landed 2, launched 1")
        );
        let action = chair.current_action.expect("fixture has an action");
        assert_eq!(action.kind, "land");
        assert_eq!(action.target, "dash-feed/p2-feed");
        assert_eq!(action.since, "2026-09-28T23:59:50Z");
        assert_eq!(
            chair.today,
            ChairToday {
                lands: 2,
                launches: 1,
                refused_or_failed: 1,
                needs_chair_open: 1
            }
        );
    }

    #[test]
    fn parse_snapshot_reads_an_older_chair_without_the_tick_fields() {
        let chair = parse_snapshot(&with_chair(&format!("{OLD_CHAIR}}}")))
            .expect("old chair should parse")
            .chair;
        assert!(chair.last_tick_at.is_none());
        assert!(chair.last_status.is_none());
        assert!(chair.current_action.is_none());
        assert_eq!(chair.today, ChairToday::default());
    }

    #[test]
    fn parse_snapshot_reads_a_null_current_action_between_ticks() {
        let chair = parse_snapshot(&with_chair(&format!(
            r#"{OLD_CHAIR},"current_action":null}}"#
        )))
        .expect("null action should parse")
        .chair;
        assert!(chair.current_action.is_none());
    }

    #[test]
    fn parse_snapshot_rejects_malformed_json() {
        assert!(parse_snapshot("{ not json").is_err());
    }
}
