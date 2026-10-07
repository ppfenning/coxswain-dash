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
    #[serde(default)]
    pub watch: Vec<WatchItem>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub spend_series: Vec<SpendPoint>,
    #[serde(default)]
    pub history: Vec<HistoryRow>,
    #[serde(default)]
    pub history_today: HistoryToday,
}

/// How an ended run finished. A value this build does not know parses as `Unknown`.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Landed,
    Approved,
    Quarantined,
    Stopped,
    /// Ended normally having built nothing (waiting on a land or a need), not a failure.
    Idle,
    Crashed,
    #[serde(other)]
    Unknown,
}

/// How a run row ended. Wire shape is a bare kind string (`"landed"`) or an object
/// `{"kind": "died", "cause": "..."}`; only quarantined and died carry a cause.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEnd {
    Running,
    Landed,
    Approved,
    Quarantined { cause: Option<String> },
    Idle,
    Died { cause: Option<String> },
    Stopped,
}

/// An unrecognised kind or a wrong JSON type is `None`, never an error.
pub fn parse_end(value: &serde_json::Value) -> Option<RunEnd> {
    let (kind, cause) = match value {
        serde_json::Value::String(kind) => (kind.as_str(), None),
        serde_json::Value::Object(map) => (
            map.get("kind")?.as_str()?,
            map.get("cause")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        ),
        _ => return None,
    };
    match kind {
        "running" => Some(RunEnd::Running),
        "landed" => Some(RunEnd::Landed),
        "approved" => Some(RunEnd::Approved),
        "quarantined" => Some(RunEnd::Quarantined { cause }),
        "idle" => Some(RunEnd::Idle),
        "died" => Some(RunEnd::Died { cause }),
        "stopped" => Some(RunEnd::Stopped),
        _ => None,
    }
}

fn deserialize_end<'de, D>(deserializer: D) -> Result<Option<RunEnd>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(parse_end))
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct LandedTask {
    pub task: String,
    pub pr: u64,
}

/// One ended run. `cause` is the first cause and is present only on a quarantined row.
#[derive(Debug, Deserialize, PartialEq)]
pub struct HistoryRow {
    pub run: String,
    pub machine: String,
    pub initiative: String,
    /// RFC 3339 UTC, kept as sent.
    pub ended_at: String,
    pub outcome: Outcome,
    pub cost_usd: f64,
    #[serde(default)]
    pub landed: Vec<LandedTask>,
    #[serde(default)]
    pub cause: Option<String>,
    /// A missing key and an explicit null both give `None`.
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryToday {
    pub lands: u32,
    pub quarantines: u32,
    pub cost_usd: f64,
    pub runs: u32,
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
    /// Seconds since the chair's last tick; absent or null on older feeds.
    #[serde(default)]
    pub tick_age_s: Option<u64>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub current_action: Option<CurrentAction>,
    #[serde(default)]
    pub today: ChairToday,
    /// Phases landed today; absent on feeds before the chair fields.
    #[serde(default)]
    pub phases_today: Option<u32>,
    /// Initiatives waiting for approval as drafts.
    #[serde(default)]
    pub drafts: Option<u32>,
    /// Seconds since the chair's last housekeeping run.
    #[serde(default)]
    pub housekeeping_age_s: Option<u64>,
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
    /// At most 60 points. Empty means the run has made no calls yet.
    #[serde(default)]
    pub cost_series: Vec<CostPoint>,
    /// How the run ended; `None` when the feed omits it or sends a kind this build does not know.
    #[serde(default, deserialize_with = "deserialize_end")]
    pub end: Option<RunEnd>,
    /// A missing key and an explicit null both give `None`.
    #[serde(default)]
    pub project: Option<String>,
}

/// One `[at, cumulative_cost_usd, node]` point. `at` is RFC 3339 UTC, kept as sent.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(from = "(String, f64, String)")]
pub struct CostPoint {
    pub at: String,
    pub cumulative_cost_usd: f64,
    pub node: String,
}

impl From<(String, f64, String)> for CostPoint {
    fn from((at, cumulative_cost_usd, node): (String, f64, String)) -> Self {
        Self {
            at,
            cumulative_cost_usd,
            node,
        }
    }
}

/// One `[at, cumulative_cost_usd]` point per 10-minute slot that holds a call. `at` is the
/// slot start in RFC 3339 UTC, kept as sent.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(from = "(String, f64)")]
pub struct SpendPoint {
    pub at: String,
    pub cumulative_cost_usd: f64,
}

impl From<(String, f64)> for SpendPoint {
    fn from((at, cumulative_cost_usd): (String, f64)) -> Self {
        Self {
            at,
            cumulative_cost_usd,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct QueueEntry {
    pub initiative: String,
    pub priority: u32,
    pub phases_landed: u32,
    pub phases_total: u32,
    pub current_phase: String,
    #[serde(default)]
    pub repo: String,
    /// A missing key and an explicit null both give `None`.
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InboxEntry {
    pub kind: String,
    pub target: String,
    pub reason: String,
}

/// One watched item. `kind` is "pr" or "run"; `id` is a pull request reference or a run id.
#[derive(Debug, Deserialize)]
pub struct WatchItem {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub at: String,
}

/// Deserializes one feed line into a [`FeedSnapshot`]. Does nothing but deserialize.
pub fn parse_snapshot(line: &str) -> Result<FeedSnapshot, serde_json::Error> {
    serde_json::from_str(line)
}

/// The distinct non-empty repos of the queue, in order of first appearance.
pub fn queue_repos(snapshot: &FeedSnapshot) -> Vec<String> {
    snapshot
        .queue
        .iter()
        .map(|entry| entry.repo.as_str())
        .filter(|repo| !repo.is_empty())
        .fold(Vec::new(), |seen, repo| {
            if seen.iter().any(|s| s == repo) {
                seen
            } else {
                seen.into_iter().chain([repo.to_string()]).collect()
            }
        })
}

/// What the feed reader delivers to the app: a parsed snapshot, or a one-line failure.
#[derive(Debug)]
pub enum FeedMessage {
    Snapshot(Box<FeedSnapshot>),
    /// A stdout line that did not parse, carrying the parse error text.
    ParseError(String),
    /// The child exited. Carries the exit status and the stderr tail.
    Error(String),
}

/// How many trailing stderr lines the reader keeps.
const STDERR_TAIL_LINES: usize = 20;

/// The longest stderr line kept whole; a longer line is split into pieces this size, so a
/// child that never writes a newline still has a bounded tail.
const STDERR_LINE_BYTES: u64 = 4096;

/// The wait before restart number `attempt` (0-based): 1s doubling each time, capped at 30s.
pub fn restart_delay(attempt: u32) -> std::time::Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(30);
    std::time::Duration::from_secs(secs)
}

/// `tail` with `line` appended, keeping only the last `max` lines.
fn push_tail(tail: Vec<String>, line: String, max: usize) -> Vec<String> {
    let skip = (tail.len() + 1).saturating_sub(max);
    tail.into_iter()
        .skip(skip)
        .chain(std::iter::once(line))
        .collect()
}

/// The error text for an exited feed. Always names the exit status, `Some(0)` included.
/// The last stderr line leads; with more than one line the whole tail follows.
pub fn exit_message(code: Option<i32>, tail: &[String]) -> String {
    let status = match code {
        Some(n) => format!("exit status {n}"),
        None => "no exit status".to_owned(),
    };
    let lines: Vec<&str> = tail
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    match lines.as_slice() {
        [] => format!("feed exited ({status})"),
        [only] => format!("{only} ({status})"),
        [.., last] => format!("{last} ({status}; stderr: {})", lines.join(" | ")),
    }
}

/// The last line that is not blank after trimming, or `None` when there is none.
pub fn last_nonempty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

/// The error text for a finished feed. `code` is `Some(0)` on success and `None` when the
/// child died without an exit code. Stderr text wins, even on a zero exit.
pub fn exit_error(code: Option<i32>, stderr_last: Option<String>) -> Option<String> {
    match (stderr_last, code) {
        (Some(line), _) => Some(line),
        (None, Some(0)) => None,
        (None, Some(n)) => Some(format!("feed exited with status {n}")),
        (None, None) => Some("feed exited without a status".to_owned()),
    }
}

/// The `cox dash --feed` command, with `--interval` when `interval_secs` is `Some`.
pub fn feed_command(interval_secs: Option<u64>) -> std::process::Command {
    let mut command = std::process::Command::new("cox");
    command.arg("dash").arg("--feed");
    if let Some(secs) = interval_secs {
        command.arg("--interval").arg(secs.to_string());
    }
    command
}

// edge
/// Spawns `command` with stdout and stderr piped, so stderr is never inherited. Taking the
/// command lets tests substitute a stub for the real feed.
pub fn spawn_command(mut command: std::process::Command) -> std::process::Child {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn the feed command")
}

// edge
/// Spawns `cox dash --feed` with stdout and stderr piped. The caller hands the child to
/// [`read_feed`]; this function only starts the process.
pub fn spawn_feed(interval_secs: Option<u64>) -> std::process::Child {
    spawn_command(feed_command(interval_secs))
}

// edge
/// Lines of `reader`, split on `\n` with invalid UTF-8 replaced, so a bad byte never stops
/// the drain.
fn lossy_lines(reader: impl std::io::Read) -> impl Iterator<Item = String> {
    use std::io::BufRead;

    std::io::BufReader::new(reader)
        .split(b'\n')
        .map_while(Result::ok)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

// edge
/// Like [`lossy_lines`], but a line longer than `max_bytes` arrives as several pieces of at
/// most `max_bytes` each, so no single read grows without bound.
fn bounded_lines(reader: impl std::io::Read, max_bytes: u64) -> impl Iterator<Item = String> {
    use std::io::{BufRead, Read};

    let mut reader = std::io::BufReader::new(reader);
    std::iter::from_fn(move || {
        let mut piece = Vec::new();
        match (&mut reader).take(max_bytes).read_until(b'\n', &mut piece) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                let text = piece.strip_suffix(b"\n").unwrap_or(&piece);
                Some(String::from_utf8_lossy(text).into_owned())
            }
        }
    })
}

// edge
/// Reads the child to the end and hands each message to `sink`. Stderr drains on its own
/// thread and only its last lines are kept, so a chatty child never fills the pipe. A good
/// stdout line is a `Snapshot`, a bad one a `ParseError`. Once stdout closes, one `Error`
/// follows with the exit status and the stderr tail, whatever the exit code.
fn read_feed_into(mut child: std::process::Child, mut sink: impl FnMut(FeedMessage)) {
    let stderr = child.stderr.take();
    let stderr_reader = std::thread::spawn(move || {
        stderr
            .map(|pipe| {
                bounded_lines(pipe, STDERR_LINE_BYTES).fold(Vec::new(), |tail, line| {
                    push_tail(tail, line, STDERR_TAIL_LINES)
                })
            })
            .unwrap_or_default()
    });

    if let Some(stdout) = child.stdout.take() {
        lossy_lines(stdout)
            .filter(|line| !line.trim().is_empty())
            .for_each(|line| {
                sink(match parse_snapshot(&line) {
                    Ok(snapshot) => FeedMessage::Snapshot(Box::new(snapshot)),
                    Err(err) => FeedMessage::ParseError(err.to_string()),
                })
            });
    }

    let tail = stderr_reader.join().unwrap_or_default();
    let code = child.wait().ok().and_then(|status| status.code());
    sink(FeedMessage::Error(exit_message(code, &tail)));
}

// edge
/// Reads the child to the end, sending every message to `tx`. See [`read_feed_into`].
pub fn read_feed(child: std::process::Child, tx: std::sync::mpsc::Sender<FeedMessage>) {
    read_feed_into(child, |message| {
        let _ = tx.send(message);
    });
}

// edge
/// Runs `spawn` and [`read_feed_into`] in a loop, sleeping `delay(attempt)` after each exit
/// before the next spawn. A parsed snapshot resets `attempt` to 0. Stops when `spawn`
/// returns `None` or the receiver is gone. `delay` and `sleep` are hooks so tests never wait.
/// The counter is local mutable state; this is the imperative edge.
pub fn supervise_feed(
    mut spawn: impl FnMut() -> Option<std::process::Child>,
    tx: std::sync::mpsc::Sender<FeedMessage>,
    delay: impl Fn(u32) -> std::time::Duration,
    mut sleep: impl FnMut(std::time::Duration),
) {
    let mut attempt = 0u32;
    while let Some(child) = spawn() {
        let mut open = true;
        read_feed_into(child, |message| {
            if matches!(message, FeedMessage::Snapshot(_)) {
                attempt = 0;
            }
            open &= tx.send(message).is_ok();
        });
        if !open {
            break;
        }
        sleep(delay(attempt));
        attempt = attempt.saturating_add(1);
    }
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

    const WATCH_FIXTURE: &str = include_str!("../tests/fixtures/dash_feed_watch_v1.json");

    #[test]
    fn parse_snapshot_reads_typed_watch_rows() {
        let snapshot = parse_snapshot(WATCH_FIXTURE).expect("fixture should parse");
        assert_eq!(snapshot.watch.len(), 3);
        assert_eq!(snapshot.watch[0].kind, "pr");
        assert_eq!(snapshot.watch[0].id, "coxswain-tools#41");
        assert_eq!(snapshot.watch[0].state, "open");
        assert_eq!(snapshot.watch[2].kind, "run");
    }

    #[test]
    fn parse_snapshot_leaves_watch_empty_for_the_existing_fixture() {
        let snapshot = parse_snapshot(FIXTURE).expect("fixture should parse");
        assert!(snapshot.watch.is_empty());
    }

    #[test]
    fn parse_snapshot_reads_chair_session_and_decisions() {
        let snapshot = parse_snapshot(FIXTURE).expect("fixture should parse");
        assert_eq!(snapshot.chair.session, "d2820a82");
        assert_eq!(snapshot.decisions.len(), 1);
    }

    fn with_queue(queue: &str) -> FeedSnapshot {
        parse_snapshot(&format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"x","epoch":1,"liveness":"live","beat_age_s":1}},"spend":{{"five_hour_fraction":0.1,"five_hour_source":"meter","weekly_fraction":0.1,"weekly_source":"meter","hard_stop_fraction":0.9,"five_hour_resets_at":"2026-09-29T02:00:00Z","weekly_resets_at":"2026-10-04T04:00:00Z"}},"machines":[],"runs":[],"queue":{queue},"inbox":[],"watch":[]}}"#
        ))
        .expect("literal snapshot should parse")
    }

    #[test]
    fn queue_repos_lists_distinct_repos_in_queue_order() {
        let entry = |id: &str, repo: &str| {
            format!(
                r#"{{"initiative":"{id}","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p1","repo":"{repo}"}}"#
            )
        };
        let queue = format!(
            "[{},{},{}]",
            entry("i1", "coxswain"),
            entry("i2", "coxswain"),
            entry("i3", "coxtop")
        );
        assert_eq!(queue_repos(&with_queue(&queue)), ["coxswain", "coxtop"]);
    }

    #[test]
    fn queue_repos_skips_entries_without_a_repo() {
        let queue = r#"[{"initiative":"i1","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p1"}]"#;
        assert!(queue_repos(&with_queue(queue)).is_empty());
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
    fn parse_snapshot_reads_tick_age_s_when_present() {
        let chair = parse_snapshot(&with_chair(&format!(r#"{OLD_CHAIR},"tick_age_s":42}}"#)))
            .expect("chair with tick age should parse")
            .chair;
        assert_eq!(chair.tick_age_s, Some(42));
    }

    #[test]
    fn parse_snapshot_without_tick_age_s_leaves_it_none_and_the_rest_unchanged() {
        let chair = parse_snapshot(&with_chair(&format!("{OLD_CHAIR}}}")))
            .expect("old chair should parse")
            .chair;
        assert!(chair.tick_age_s.is_none());
        assert_eq!(chair.holder, "h");
        assert_eq!(chair.beat_age_s, 4);
        assert_eq!(chair.session, "");
        assert!(chair.last_tick_at.is_none());
        assert!(chair.last_status.is_none());
        assert!(chair.current_action.is_none());
        assert_eq!(chair.today, ChairToday::default());
        let null_age = parse_snapshot(&with_chair(&format!(r#"{OLD_CHAIR},"tick_age_s":null}}"#)))
            .expect("null tick age should parse")
            .chair;
        assert!(null_age.tick_age_s.is_none());
    }

    fn cost(at: &str, cumulative_cost_usd: f64, node: &str) -> CostPoint {
        CostPoint {
            at: at.to_string(),
            cumulative_cost_usd,
            node: node.to_string(),
        }
    }

    fn spend(at: &str, cumulative_cost_usd: f64) -> SpendPoint {
        SpendPoint {
            at: at.to_string(),
            cumulative_cost_usd,
        }
    }

    #[test]
    fn parse_snapshot_reads_the_cost_and_spend_series_from_the_fixture() {
        let snapshot = parse_snapshot(FIXTURE).expect("fixture should parse");
        assert_eq!(
            snapshot.runs[0].cost_series,
            vec![
                cost("2026-10-04T14:00:10Z", 0.10, "plan"),
                cost("2026-10-04T14:01:20Z", 0.42, "build"),
                cost("2026-10-04T14:02:10Z", 0.84, "build"),
            ]
        );
        assert!(snapshot.runs[1].cost_series.is_empty());
        assert_eq!(
            snapshot.spend_series,
            vec![
                spend("2026-10-04T13:40:00Z", 0.30),
                spend("2026-10-04T13:50:00Z", 0.55),
                spend("2026-10-04T14:00:00Z", 1.20),
            ]
        );
    }

    #[test]
    fn parse_snapshot_defaults_both_series_to_empty_when_the_keys_are_absent() {
        let line = with_chair(&format!("{OLD_CHAIR}}}")).replace(
            r#""runs":[]"#,
            r#""runs":[{"run":"r1","machine":"m","phase":"p","node":"build","attempt":1,"turns":2,"cost":0.5,"verdict":"approve","status":"running"}]"#,
        );
        let snapshot = parse_snapshot(&line).expect("feed without series should parse");
        assert_eq!(snapshot.runs.len(), 1);
        assert!(snapshot.runs[0].cost_series.is_empty());
        assert!(snapshot.spend_series.is_empty());
    }

    const LANDED_ROW: &str = r#"{"run":"dash-feed-0","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-09-28T23:50:00Z","outcome":"landed","cost_usd":2.15,"landed":[{"task":"p1-foundations-task","pr":41}]}"#;
    const QUARANTINED_ROW: &str = r#"{"run":"dash-feed-9","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-09-28T23:40:00Z","outcome":"quarantined","cost_usd":0.42,"landed":[],"cause":"review rejected twice"}"#;

    fn with_history(history: &str, today: &str) -> String {
        with_chair(&format!("{OLD_CHAIR}}}")).replace(
            r#""watch":[]"#,
            &format!(r#""watch":[],"history":[{history}],"history_today":{today}"#),
        )
    }

    #[test]
    fn parse_snapshot_reads_history_rows_and_the_today_object() {
        let line = with_history(
            &format!("{LANDED_ROW},{QUARANTINED_ROW}"),
            r#"{"lands":1,"quarantines":1,"cost_usd":2.57,"runs":2}"#,
        );
        let snapshot = parse_snapshot(&line).expect("feed with history should parse");
        assert_eq!(snapshot.history.len(), 2);
        let landed = &snapshot.history[0];
        assert_eq!(landed.run, "dash-feed-0");
        assert_eq!(landed.ended_at, "2026-09-28T23:50:00Z");
        assert_eq!(landed.outcome, Outcome::Landed);
        assert_eq!(landed.cost_usd, 2.15);
        assert_eq!(
            landed.landed,
            vec![LandedTask {
                task: "p1-foundations-task".to_string(),
                pr: 41
            }]
        );
        assert!(landed.cause.is_none());
        let quarantined = &snapshot.history[1];
        assert_eq!(quarantined.outcome, Outcome::Quarantined);
        assert!(quarantined.landed.is_empty());
        assert_eq!(quarantined.cause.as_deref(), Some("review rejected twice"));
        assert_eq!(
            snapshot.history_today,
            HistoryToday {
                lands: 1,
                quarantines: 1,
                cost_usd: 2.57,
                runs: 2
            }
        );
    }

    #[test]
    fn parse_snapshot_reads_an_unknown_outcome_as_unknown() {
        let row = LANDED_ROW.replace(r#""outcome":"landed""#, r#""outcome":"someday""#);
        let snapshot = parse_snapshot(&with_history(&row, "{}")).expect("should parse");
        assert_eq!(snapshot.history[0].outcome, Outcome::Unknown);
        assert_eq!(snapshot.history_today, HistoryToday::default());
    }

    #[test]
    fn parse_snapshot_defaults_history_when_the_keys_are_absent() {
        let snapshot = parse_snapshot(&with_chair(&format!("{OLD_CHAIR}}}")))
            .expect("feed without history should parse");
        assert!(snapshot.history.is_empty());
        assert_eq!(snapshot.history_today, HistoryToday::default());
    }

    const RUN_JSON: &str = r#"{"run":"r","machine":"m","phase":"p1","node":"build","attempt":1,"turns":1,"cost":0.1,"verdict":"none","status":"running""#;
    const QUEUE_JSON: &str =
        r#"{"initiative":"i","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p1""#;

    fn with_rows(runs: &str, queue: &str, history: &str) -> FeedSnapshot {
        let line = with_history(history, "{}")
            .replace(r#""runs":[]"#, &format!(r#""runs":[{runs}]"#))
            .replace(r#""queue":[]"#, &format!(r#""queue":[{queue}]"#));
        parse_snapshot(&line).expect("literal snapshot should parse")
    }

    #[test]
    fn parse_snapshot_reads_project_on_runs_queue_and_history() {
        let landed = LANDED_ROW.replace(r#""cost_usd""#, r#""project":"alpha","cost_usd""#);
        let quarantined = QUARANTINED_ROW.replace(r#""cost_usd""#, r#""project":null,"cost_usd""#);
        let snapshot = with_rows(
            &format!(r#"{RUN_JSON},"project":"alpha"}},{RUN_JSON},"project":null}}"#),
            &format!(r#"{QUEUE_JSON},"project":"beta"}},{QUEUE_JSON},"project":null}}"#),
            &format!("{landed},{quarantined}"),
        );
        assert_eq!(snapshot.runs[0].project.as_deref(), Some("alpha"));
        assert_eq!(snapshot.runs[1].project, None);
        assert_eq!(snapshot.queue[0].project.as_deref(), Some("beta"));
        assert_eq!(snapshot.queue[1].project, None);
        assert_eq!(snapshot.history[0].project.as_deref(), Some("alpha"));
        assert_eq!(snapshot.history[1].project, None);
    }

    #[test]
    fn parse_snapshot_reads_a_feed_with_no_project_key_as_none() {
        let snapshot = with_rows(
            &format!("{RUN_JSON}}}"),
            &format!("{QUEUE_JSON}}}"),
            &format!("{LANDED_ROW},{QUARANTINED_ROW}"),
        );
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.queue.len(), 1);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.runs[0].project, None);
        assert_eq!(snapshot.queue[0].project, None);
        assert!(snapshot.history.iter().all(|row| row.project.is_none()));
    }

    #[test]
    fn parse_snapshot_rejects_malformed_json() {
        assert!(parse_snapshot("{ not json").is_err());
    }

    fn run_stub(script: &str, line: &str) -> Vec<FeedMessage> {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script]).env("LINE", line);
        let (tx, rx) = std::sync::mpsc::channel();
        read_feed(spawn_command(command), tx);
        rx.iter().collect()
    }

    #[test]
    fn last_nonempty_line_skips_trailing_blank_lines() {
        let text = "Traceback:\n  File x\nValueError: boom\n\n  \n";
        assert_eq!(
            last_nonempty_line(text),
            Some("ValueError: boom".to_owned())
        );
        assert_eq!(last_nonempty_line(" \n\n"), None);
    }

    #[test]
    fn exit_error_prefers_stderr_then_status_then_nothing() {
        assert_eq!(
            exit_error(Some(0), Some("warn".to_owned())),
            Some("warn".to_owned())
        );
        assert_eq!(
            exit_error(Some(3), None),
            Some("feed exited with status 3".to_owned())
        );
        assert_eq!(exit_error(Some(0), None), None);
        assert!(exit_error(None, None).is_some());
    }

    #[test]
    fn read_feed_reports_the_last_traceback_line_as_the_error() {
        let script = "printf 'Traceback (most recent call last):\\n  File \"x\"\\nValueError: boom\\n' >&2; exit 1";
        let messages = run_stub(script, "");
        assert_eq!(messages.len(), 1);
        assert!(
            matches!(&messages[0], FeedMessage::Error(text) if text.starts_with("ValueError: boom (exit status 1; stderr: Traceback"))
        );
    }

    #[test]
    fn read_feed_yields_the_snapshot_first_and_the_error_second() {
        let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is json");
        let line = serde_json::to_string(&value).expect("value serializes");
        let messages = run_stub("printf '%s\\n' \"$LINE\"; echo boom >&2; exit 1", &line);
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], FeedMessage::Snapshot(s) if s.schema == 1));
        assert!(matches!(&messages[1], FeedMessage::Error(text) if text == "boom (exit status 1)"));
    }

    #[test]
    fn read_feed_reports_the_exit_status_when_stderr_is_empty() {
        let messages = run_stub("exit 3", "");
        assert!(
            matches!(&messages[..], [FeedMessage::Error(text)] if text == "feed exited (exit status 3)")
        );
    }

    #[test]
    fn restart_delay_doubles_from_one_second_and_caps_at_thirty() {
        let secs = |attempt| restart_delay(attempt).as_secs();
        assert_eq!(
            [secs(0), secs(1), secs(2), secs(3), secs(4)],
            [1, 2, 4, 8, 16]
        );
        assert_eq!([secs(5), secs(6), secs(50), secs(u32::MAX)], [30; 4]);
    }

    #[test]
    fn push_tail_keeps_only_the_last_lines() {
        let lines = (1..=25).map(|n| n.to_string());
        let tail = lines.fold(Vec::new(), |tail, line| push_tail(tail, line, 20));
        assert_eq!(tail.len(), 20);
        assert_eq!((tail[0].as_str(), tail[19].as_str()), ("6", "25"));
    }

    #[test]
    fn exit_message_always_names_the_status() {
        let tail = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert_eq!(
            exit_message(Some(1), &tail(&["boom"])),
            "boom (exit status 1)"
        );
        assert_eq!(exit_message(Some(0), &[]), "feed exited (exit status 0)");
        assert_eq!(exit_message(None, &[]), "feed exited (no exit status)");
        assert_eq!(
            exit_message(Some(2), &tail(&["a", "", "b"])),
            "b (exit status 2; stderr: a | b)"
        );
    }

    #[test]
    fn read_feed_sends_a_parse_error_per_bad_line() {
        let messages = run_stub("printf '%s\\n' '{ not json' 'also bad'", "");
        assert_eq!(messages.len(), 3);
        assert!(matches!(&messages[0], FeedMessage::ParseError(text) if !text.is_empty()));
        assert!(matches!(&messages[1], FeedMessage::ParseError(text) if !text.is_empty()));
        assert!(matches!(&messages[2], FeedMessage::Error(_)));
    }

    #[test]
    fn read_feed_error_carries_the_exit_status_and_stderr_text() {
        let messages = run_stub("echo oops >&2; exit 1", "");
        assert!(
            matches!(&messages[..], [FeedMessage::Error(text)] if text.contains("status 1") && text.contains("oops"))
        );
    }

    #[test]
    fn read_feed_reports_a_clean_exit_too() {
        let messages = run_stub("exit 0", "");
        assert!(matches!(&messages[..], [FeedMessage::Error(text)] if text.contains("status 0")));
    }

    #[test]
    fn read_feed_does_not_block_on_a_flood_of_stderr_and_keeps_only_the_tail() {
        let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is json");
        let line = serde_json::to_string(&value).expect("value serializes");
        let script = "seq 1 100000 >&2; printf '%s\\n' \"$LINE\"";
        let messages = run_stub(script, &line);
        assert!(matches!(&messages[0], FeedMessage::Snapshot(s) if s.schema == 1));
        let FeedMessage::Error(text) = &messages[1] else {
            panic!("expected the exit error second");
        };
        assert!(text.starts_with("100000 (exit status 0; stderr: 99981 | "));
        assert_eq!(text.matches(" | ").count(), STDERR_TAIL_LINES - 1);
    }

    #[test]
    fn read_feed_bounds_a_stderr_line_with_no_newline() {
        let messages = run_stub("head -c 1000000 /dev/zero | tr '\\0' x >&2; exit 1", "");
        let FeedMessage::Error(text) = &messages[0] else {
            panic!("expected one exit error");
        };
        assert!(text.contains("status 1"));
        assert!(text.len() < STDERR_TAIL_LINES * STDERR_LINE_BYTES as usize * 2);
    }

    #[test]
    fn bounded_lines_splits_long_lines_into_pieces() {
        let lines: Vec<String> = bounded_lines(&b"abcdefg\nhi"[..], 3).collect();
        assert_eq!(lines, ["abc", "def", "g", "hi"]);
    }

    /// Runs `supervise_feed` over one stub script per spawn and returns every sleep it asked for.
    fn supervise_stubs(scripts: &[&str], line: &str) -> Vec<u64> {
        let mut pending: Vec<String> = scripts.iter().rev().map(|s| s.to_string()).collect();
        let line = line.to_owned();
        let spawn = move || {
            pending.pop().map(|script| {
                let mut command = std::process::Command::new("sh");
                command.args(["-c", &script]).env("LINE", &line);
                spawn_command(command)
            })
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut slept = Vec::new();
        supervise_feed(spawn, tx, restart_delay, |d| slept.push(d.as_secs()));
        slept
    }

    #[test]
    fn supervise_feed_backs_off_after_each_failed_run() {
        assert_eq!(
            supervise_stubs(&["exit 1", "exit 1", "exit 1"], ""),
            [1, 2, 4]
        );
    }

    #[test]
    fn supervise_feed_resets_the_backoff_after_a_snapshot() {
        let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is json");
        let line = serde_json::to_string(&value).expect("value serializes");
        let good = "printf '%s\\n' \"$LINE\"; exit 1";
        assert_eq!(
            supervise_stubs(&["exit 1", "exit 1", good, "exit 1"], &line),
            [1, 2, 1, 2]
        );
    }

    #[test]
    fn supervise_feed_stops_when_the_receiver_is_gone() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let mut spawns = 0;
        let spawn = || {
            spawns += 1;
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "exit 1"]);
            Some(spawn_command(command))
        };
        supervise_feed(spawn, tx, restart_delay, |_| {});
        assert_eq!(spawns, 1);
    }

    #[test]
    fn chair_parses_phases_drafts_and_housekeeping_when_present() {
        let json = r#"{"holder":"h","host":"m","epoch":1,"liveness":"live","beat_age_s":5,"phases_today":3,"drafts":2,"housekeeping_age_s":7200}"#;
        let chair: Chair = serde_json::from_str(json).expect("chair parses");
        assert_eq!(chair.phases_today, Some(3));
        assert_eq!(chair.drafts, Some(2));
        assert_eq!(chair.housekeeping_age_s, Some(7200));
        let bare: Chair = serde_json::from_str(
            r#"{"holder":"h","host":"m","epoch":1,"liveness":"live","beat_age_s":5}"#,
        )
        .expect("older feed parses");
        assert_eq!(bare.phases_today, None);
    }

    #[test]
    fn history_parses_idle_as_its_own_outcome() {
        let row: HistoryRow = serde_json::from_str(
            r#"{"run":"r","machine":"m","initiative":"i","ended_at":"2026-10-05T12:00:00Z","outcome":"idle","cost_usd":0.0}"#,
        )
        .expect("idle row parses");
        assert_eq!(row.outcome, Outcome::Idle);
    }

    fn run_with(end: &str) -> Run {
        let json = format!(
            r#"{{"run":"r","machine":"m","phase":"p","node":"n","attempt":1,"turns":2,"cost":0.5,"verdict":"","status":"running"{end}}}"#
        );
        serde_json::from_str(&json).expect("run row parses")
    }

    #[test]
    fn run_end_parses_each_kind_as_a_bare_string() {
        let kinds = [
            ("running", RunEnd::Running),
            ("landed", RunEnd::Landed),
            ("approved", RunEnd::Approved),
            ("quarantined", RunEnd::Quarantined { cause: None }),
            ("idle", RunEnd::Idle),
            ("died", RunEnd::Died { cause: None }),
            ("stopped", RunEnd::Stopped),
        ];
        for (kind, want) in kinds {
            let row = run_with(&format!(r#","end":"{kind}""#));
            assert_eq!(row.end, Some(want), "kind {kind}");
        }
    }

    #[test]
    fn run_end_carries_a_cause_on_quarantined_and_died() {
        let row = run_with(r#","end":{"kind":"quarantined","cause":"tests red"}"#);
        assert_eq!(
            row.end,
            Some(RunEnd::Quarantined {
                cause: Some("tests red".to_string())
            })
        );
        let row = run_with(r#","end":{"kind":"died","cause":"oom"}"#);
        assert_eq!(
            row.end,
            Some(RunEnd::Died {
                cause: Some("oom".to_string())
            })
        );
        let row = run_with(r#","end":{"kind":"landed"}"#);
        assert_eq!(row.end, Some(RunEnd::Landed));
    }

    #[test]
    fn run_end_is_none_when_absent_null_or_unrecognised() {
        assert_eq!(run_with("").end, None);
        assert_eq!(run_with(r#","end":null"#).end, None);
        assert_eq!(run_with(r#","end":"someday""#).end, None);
        assert_eq!(run_with(r#","end":{"kind":"someday"}"#).end, None);
        assert_eq!(run_with(r#","end":7"#).end, None);
    }
}
