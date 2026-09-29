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
}

#[derive(Debug, Deserialize)]
pub struct RunDetail {
    pub schema: u32,
    pub at: String,
    pub run: String,
    pub machine: String,
    pub initiative: String,
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

/// Deserializes one detail line into a [`DetailSnapshot`]. Does nothing but deserialize.
pub fn parse_detail(line: &str) -> Result<DetailSnapshot, serde_json::Error> {
    serde_json::from_str(line)
}

// edge
/// Spawns `cox dash --detail <kind> <id>` with stdout piped. The caller reads the child's
/// stdout; this function only starts the process.
pub fn spawn_detail(kind: &str, id: &str) -> std::process::Child {
    std::process::Command::new("cox")
        .arg("dash")
        .arg("--detail")
        .arg(kind)
        .arg(id)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn `cox dash --detail`")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_run_v1.json");
    const INITIATIVE_FIXTURE: &str =
        include_str!("../tests/fixtures/dash_detail_initiative_v1.json");
    const MACHINE_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_machine_v1.json");

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
    fn parse_detail_rejects_malformed_json() {
        assert!(parse_detail("{ not json").is_err());
    }
}
