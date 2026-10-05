//! The versioned dash detail snapshot: a pure schema for `cox dash --detail <kind> <id>`
//! output, plus the thin edge that spawns the detail process. The wire-detail-dispatch task
//! reads the child's stdout and calls `parse_detail` on each line; neither of those happens
//! here.

// Most fields exist only to mirror the wire schema; the page tasks that render them land
// later, so clippy would otherwise flag every field a test doesn't touch as dead code.
#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetailSnapshot {
    Run(RunDetail),
    Initiative(InitiativeDetail),
    Machine(MachineDetail),
    Spend(SpendDetail),
    Health(HealthDetail),
}

#[derive(Debug, Deserialize)]
pub struct RunDetail {
    pub schema: u32,
    pub at: String,
    pub run: String,
    // A run the store has not placed yet has no machine, and an early one no initiative or
    // phase: tools sends null, which reads as empty.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub machine: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub initiative: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub phase: String,
    pub steps: Vec<StepDetail>,
    pub stopped_reason: Option<String>,
    pub files: Vec<String>,
    pub last_tool_calls: Vec<ToolCall>,
    pub log_tail: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct StepDetail {
    pub node: String,
    pub turns: u32,
    pub cost: f64,
    pub verdict: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub summary: String,
    pub at: String,
}

#[derive(Debug, Deserialize)]
pub struct InitiativeDetail {
    pub schema: u32,
    pub at: String,
    pub initiative: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub body: String,
    pub phases: Vec<PhaseDetail>,
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PhaseDetail {
    pub id: String,
    pub landed_at: Option<String>,
    pub tasks: Vec<TaskEdge>,
}

#[derive(Debug, Deserialize)]
pub struct TaskEdge {
    pub id: String,
    pub needs: Vec<String>,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryEntry {
    pub at: String,
    pub event: String,
}

#[derive(Debug, Deserialize)]
pub struct MachineDetail {
    pub schema: u32,
    pub at: String,
    pub machine: String,
    pub host_facts: HostFacts,
    pub checkouts: BTreeMap<String, Checkout>,
    pub lanes_in_use: u32,
    pub capacity: u32,
}

#[derive(Debug, Deserialize)]
pub struct HostFacts {
    pub os: String,
    pub cpu_count: u32,
    pub mem_total_gb: f64,
}

#[derive(Debug, Deserialize)]
pub struct Checkout {
    pub behind_main: u32,
    pub branch: String,
}

/// The `cox dash --detail spend` payload. A later task wraps it into [`DetailSnapshot`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SpendDetail {
    pub schema: u32,
    pub at: String,
    pub five_hour: Meter,
    pub weekly: Meter,
    pub history: Vec<MeterPoint>,
    pub daily: Vec<DayCost>,
}

/// One usage meter. `fraction` is used over ceiling, in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Meter {
    pub fraction: f64,
    pub used_usd: f64,
    pub ceiling_usd: f64,
    pub resets_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MeterPoint {
    pub at: String,
    pub five_hour: f64,
    pub weekly: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DayCost {
    pub day: String,
    pub cost: f64,
}

/// The `cox dash --detail health` payload. A later task wraps it into [`DetailSnapshot`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HealthDetail {
    pub schema: u32,
    pub at: String,
    pub hosts: Vec<HostHealth>,
    pub logins: Vec<LoginHealth>,
    pub store: StoreHealth,
    pub chair_lease: ChairLease,
    pub housekeeping: Housekeeping,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HostHealth {
    pub name: String,
    pub state: String,
    pub lanes_in_use: u32,
    pub capacity: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LoginHealth {
    pub provider: String,
    pub host: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StoreHealth {
    pub ok: bool,
    pub detail: String,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChairLease {
    pub holder: Option<String>,
    pub expires_at: Option<String>,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Housekeeping {
    pub last_run_at: Option<String>,
    pub age_hours: Option<f64>,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(de)?.unwrap_or_default())
}

/// Deserializes one detail line into a [`DetailSnapshot`]. Does nothing but deserialize.
pub fn parse_detail(line: &str) -> Result<DetailSnapshot, serde_json::Error> {
    serde_json::from_str(line)
}

/// The result of one detail child: its stdout body, or one line saying why it failed.
#[derive(Debug, PartialEq)]
pub enum DetailOutcome {
    Body(String),
    Error(String),
}

/// The last line of `reader` that is non-empty after trimming, trimmed. Reads to the end,
/// holding one line at a time, so a long traceback costs no more memory than its longest line.
fn last_nonempty_line(reader: impl std::io::BufRead) -> Option<String> {
    reader
        .split(b'\n')
        .map_while(Result::ok)
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .filter(|line| !line.is_empty())
        .last()
}

/// Any stderr line wins over the exit status; `code` is `None` when a signal killed the child.
pub fn classify(stdout: String, stderr_line: Option<String>, code: Option<i32>) -> DetailOutcome {
    match (stderr_line, code) {
        (Some(line), _) => DetailOutcome::Error(line),
        (None, Some(0)) => DetailOutcome::Body(stdout),
        (None, Some(n)) => DetailOutcome::Error(format!("detail exited with status {n}")),
        (None, None) => DetailOutcome::Error("detail terminated by signal".to_string()),
    }
}

/// The argv of `cox dash --detail <kind> [<id>]`. Spend and health take no id, so an empty one
/// is left off rather than passed as an empty argument.
pub fn detail_argv(kind: &str, id: &str) -> Vec<String> {
    ["cox", "dash", "--detail", kind]
        .into_iter()
        .chain((!id.is_empty()).then_some(id))
        .map(str::to_string)
        .collect()
}

// edge
/// Builds the command for `detail_argv(kind, id)`. `spawn_piped` sets the pipes.
fn detail_command(kind: &str, id: &str) -> std::process::Command {
    let argv = detail_argv(kind, id);
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd
}

// edge
/// Spawns `cmd` with stdout and stderr piped and drains stderr on its own thread, so a child
/// that writes more than a pipe buffer to stderr cannot block its stdout. The returned
/// `Child` has no stderr handle left. Join the handle after stdout ends for the last
/// non-empty stderr line, and pass it with the exit code to [`classify`].
pub fn spawn_piped(
    mut cmd: std::process::Command,
) -> std::io::Result<(std::process::Child, std::thread::JoinHandle<Option<String>>)> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take();
    let stderr_line = std::thread::spawn(move || {
        stderr.and_then(|pipe| last_nonempty_line(std::io::BufReader::new(pipe)))
    });
    Ok((child, stderr_line))
}

// edge
/// Spawns `cox dash --detail <kind> <id>` with stdout piped and stderr drained. The caller
/// reads the child's stdout; this function only starts the process.
pub fn spawn_detail(kind: &str, id: &str) -> std::process::Child {
    // The stderr line is dropped here until main.rs moves to `spawn_piped` and keeps it.
    // Dropping the handle detaches the drain thread; it still reads until the child exits.
    let (child, _stderr_line) =
        spawn_piped(detail_command(kind, id)).expect("failed to spawn `cox dash --detail`");
    child
}

// edge
/// Runs `cmd` to completion through [`spawn_piped`] and classifies what it produced.
/// Takes a ready command so tests can substitute a stub for `cox`.
pub fn run_detail(cmd: std::process::Command) -> DetailOutcome {
    use std::io::Read;

    match spawn_piped(cmd) {
        Err(err) => DetailOutcome::Error(format!("failed to spawn detail: {err}")),
        Ok((mut child, stderr_line)) => {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                let _ = pipe.read_to_string(&mut stdout);
            }
            let line = stderr_line.join().unwrap_or(None);
            match child.wait() {
                Ok(status) => classify(stdout, line, status.code()),
                Err(err) => DetailOutcome::Error(format!("failed to wait for detail: {err}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_run_v1.json");
    const INITIATIVE_FIXTURE: &str =
        include_str!("../tests/fixtures/dash_detail_initiative_v1.json");
    const MACHINE_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_machine_v1.json");
    const SPEND_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_spend_v1.json");
    const HEALTH_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_health_v1.json");

    #[test]
    fn parse_detail_reads_the_run_v1_fixture() {
        let snapshot = parse_detail(RUN_FIXTURE).expect("fixture should parse");
        match snapshot {
            DetailSnapshot::Run(run) => {
                assert_eq!(run.schema, 1);
                assert_eq!(run.steps[1].node, "build");
            }
            other => panic!("expected DetailSnapshot::Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_detail_reads_the_initiative_v1_fixture() {
        let snapshot = parse_detail(INITIATIVE_FIXTURE).expect("fixture should parse");
        match snapshot {
            DetailSnapshot::Initiative(initiative) => {
                assert!(initiative.phases[0].tasks[1].error.is_some());
            }
            other => panic!("expected DetailSnapshot::Initiative, got {other:?}"),
        }
    }

    fn initiative_with(extra: &str) -> InitiativeDetail {
        let line = format!(
            r#"{{"schema":1,"kind":"initiative","at":"2026-09-29T12:00:00Z","initiative":"i1",{extra}"phases":[],"history":[]}}"#
        );
        match parse_detail(&line).expect("literal should parse") {
            DetailSnapshot::Initiative(initiative) => initiative,
            other => panic!("expected DetailSnapshot::Initiative, got {other:?}"),
        }
    }

    #[test]
    fn parse_detail_reads_the_initiative_title_repo_and_body() {
        let initiative =
            initiative_with(r#""title":"A title","repo":"coxtop","body":"Some prose.","#);
        assert_eq!(initiative.title, "A title");
        assert_eq!(initiative.repo, "coxtop");
        assert_eq!(initiative.body, "Some prose.");
    }

    #[test]
    fn parse_detail_defaults_a_missing_initiative_body_to_empty() {
        let initiative = initiative_with(r#""title":"A title","repo":"coxtop","#);
        assert_eq!(initiative.body, "");
    }

    #[test]
    fn parse_detail_reads_the_machine_v1_fixture() {
        let snapshot = parse_detail(MACHINE_FIXTURE).expect("fixture should parse");
        match snapshot {
            DetailSnapshot::Machine(machine) => {
                assert_eq!(machine.checkouts["coxswain-dash"].behind_main, 3);
            }
            other => panic!("expected DetailSnapshot::Machine, got {other:?}"),
        }
    }

    #[test]
    fn parse_detail_reads_a_run_with_a_null_machine_initiative_and_phase() {
        let line = r#"{"schema": 1, "kind": "run", "at": "2026-10-05T20:41:22Z", "run": "r-2", "machine": null, "initiative": null, "phase": null, "steps": [{"node": "plan", "turns": 17, "cost": 0.36, "verdict": null, "status": "done"}], "stopped_reason": null, "files": [], "last_tool_calls": [{"tool": "", "summary": "StructuredOutput", "at": ""}], "log_tail": []}"#;
        match parse_detail(line).expect("a null machine should parse") {
            DetailSnapshot::Run(run) => {
                assert_eq!(run.machine, "");
                assert_eq!(run.initiative, "");
                assert_eq!(run.phase, "");
            }
            other => panic!("expected DetailSnapshot::Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_detail_rejects_malformed_json() {
        assert!(parse_detail("{ not json").is_err());
    }

    #[test]
    fn spend_fixture_parses_into_spend_detail() {
        let spend: SpendDetail = serde_json::from_str(SPEND_FIXTURE).expect("fixture should parse");
        assert_eq!(spend.schema, 1);
        assert_eq!(spend.weekly.fraction, 0.61);
        assert_eq!(spend.five_hour.ceiling_usd, 50.0);
        assert_eq!(spend.history.len(), 3);
        assert_eq!(spend.daily.len(), 3);
        assert_eq!(spend.daily[0].day, "2026-10-03");
    }

    #[test]
    fn health_fixture_parses_into_health_detail() {
        let health: HealthDetail =
            serde_json::from_str(HEALTH_FIXTURE).expect("fixture should parse");
        assert_eq!(health.hosts[0].lanes_in_use, 2);
        assert_eq!(health.hosts[1].state, "down");
        assert!(!health.logins[1].ok);
        assert_eq!(health.store.latency_ms, Some(42));
        assert_eq!(
            health.chair_lease.holder.as_deref(),
            Some("chair-loop@omarchy")
        );
        assert_eq!(health.housekeeping.age_hours, Some(3.5));
    }

    #[test]
    fn parse_detail_reads_the_spend_and_health_fixtures() {
        match parse_detail(SPEND_FIXTURE.trim()).expect("spend should parse") {
            DetailSnapshot::Spend(spend) => assert_eq!(spend.weekly.fraction, 0.61),
            other => panic!("expected DetailSnapshot::Spend, got {other:?}"),
        }
        match parse_detail(HEALTH_FIXTURE.trim()).expect("health should parse") {
            DetailSnapshot::Health(health) => assert_eq!(health.hosts[1].state, "down"),
            other => panic!("expected DetailSnapshot::Health, got {other:?}"),
        }
    }

    #[test]
    fn detail_argv_leaves_an_empty_id_off() {
        assert_eq!(
            detail_argv("spend", ""),
            ["cox", "dash", "--detail", "spend"]
        );
    }

    #[test]
    fn detail_argv_appends_a_present_id_last() {
        assert_eq!(
            detail_argv("run", "r1"),
            ["cox", "dash", "--detail", "run", "r1"]
        );
    }

    fn fails<T: serde::de::DeserializeOwned>() -> bool {
        serde_json::from_str::<T>("{ not json").is_err()
    }

    #[test]
    fn each_spend_and_health_struct_rejects_malformed_json() {
        assert!(fails::<SpendDetail>());
        assert!(fails::<Meter>());
        assert!(fails::<MeterPoint>());
        assert!(fails::<DayCost>());
        assert!(fails::<HealthDetail>());
        assert!(fails::<HostHealth>());
        assert!(fails::<LoginHealth>());
        assert!(fails::<StoreHealth>());
        assert!(fails::<ChairLease>());
        assert!(fails::<Housekeeping>());
    }

    fn sh(script: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }

    #[test]
    fn last_nonempty_line_takes_the_final_line_and_none_for_blank_input() {
        assert_eq!(
            last_nonempty_line(&b"a\nb  \n\n"[..]),
            Some("b".to_string())
        );
        assert_eq!(last_nonempty_line(&b"\n\n"[..]), None);
    }

    #[test]
    fn classify_lets_a_stderr_line_win_and_falls_back_to_the_status() {
        let body = || "{}".to_string();
        assert_eq!(
            classify(body(), Some("y".to_string()), Some(0)),
            DetailOutcome::Error("y".to_string())
        );
        assert_eq!(
            classify(body(), None, Some(3)),
            DetailOutcome::Error("detail exited with status 3".to_string())
        );
        assert_eq!(
            classify(body(), None, None),
            DetailOutcome::Error("detail terminated by signal".to_string())
        );
        assert_eq!(classify(body(), None, Some(0)), DetailOutcome::Body(body()));
    }

    #[test]
    fn run_detail_reports_the_last_traceback_line_and_nothing_else() {
        let script = "echo 'Traceback (most recent call last):' >&2; \
                      echo '  File \"dash.py\", line 9, in <module>' >&2; \
                      echo 'ValueError: boom' >&2; exit 1";
        assert_eq!(
            run_detail(sh(script)),
            DetailOutcome::Error("ValueError: boom".to_string())
        );
    }

    #[test]
    fn run_detail_reports_stderr_even_when_the_child_exits_zero() {
        assert_eq!(
            run_detail(sh("echo '{}'; echo 'warning: stale' >&2")),
            DetailOutcome::Error("warning: stale".to_string())
        );
    }

    #[test]
    fn spawn_piped_drains_stderr_past_the_pipe_buffer_while_stdout_is_read() {
        use std::io::Read;

        let script = "yes 'noise from stderr' | head -n 60000 >&2; echo done";
        let (mut child, stderr_line) = spawn_piped(sh(script)).expect("sh should spawn");
        assert!(child.stderr.is_none());
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .expect("stdout should be piped")
            .read_to_string(&mut stdout)
            .expect("stdout should read");
        assert_eq!(stdout, "done\n");
        assert_eq!(
            stderr_line.join().expect("drain thread should finish"),
            Some("noise from stderr".to_string())
        );
        assert!(child.wait().expect("child should exit").success());
    }

    #[test]
    fn run_detail_falls_back_to_the_exit_status_when_stderr_is_empty() {
        assert_eq!(
            run_detail(sh("exit 3")),
            DetailOutcome::Error("detail exited with status 3".to_string())
        );
    }

    #[test]
    fn run_detail_returns_the_body_on_a_clean_exit() {
        assert_eq!(
            run_detail(sh("echo '{}'")),
            DetailOutcome::Body("{}\n".to_string())
        );
    }
}
