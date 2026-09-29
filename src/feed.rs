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
}

#[derive(Debug, Deserialize)]
pub struct Chair {
    pub holder: String,
    pub host: String,
    pub epoch: u32,
    pub liveness: String,
    pub beat_age_s: u64,
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
    fn parse_snapshot_rejects_malformed_json() {
        assert!(parse_snapshot("{ not json").is_err());
    }
}
