//! Runs a cox command off the UI thread and summarises its outcome. The process spawn is the
//! only edge; the summaries are pure.

use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};

use crate::app::Origin;
use crate::feed::{feed_command, last_nonempty_line};

const CLIP_CHARS: usize = 200;

/// What a finished command left behind. `output` is stdout then stderr. `code` is `None`
/// when the process could not start or died without an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub argv: Vec<String>,
    pub code: Option<i32>,
    pub output: String,
}

/// Runs an argv and reports the result. Tests substitute a fake.
pub trait CmdRunner: Send + Sync {
    fn run(&self, argv: &[String]) -> ExecResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    Ok,
    Failed,
}

/// Runs argv with its first element replaced by `program`.
pub struct RealRunner {
    pub program: String,
}

impl RealRunner {
    /// A runner for the same cox binary the feed uses.
    pub fn cox() -> Self {
        let program = feed_command(None)
            .get_program()
            .to_string_lossy()
            .into_owned();
        Self { program }
    }
}

// edge
impl CmdRunner for RealRunner {
    fn run(&self, argv: &[String]) -> ExecResult {
        let output = Command::new(&self.program)
            .args(argv.iter().skip(1))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(out) => ExecResult {
                argv: argv.to_vec(),
                code: out.status.code(),
                output: format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            },
            Err(err) => ExecResult {
                argv: argv.to_vec(),
                code: None,
                output: err.to_string(),
            },
        }
    }
}

// edge
/// Runs `argv` on one new thread and sends the result with its `origin`, so the UI loop never
/// blocks and results arriving out of start order still reach the right origin.
pub fn start(
    runner: Arc<dyn CmdRunner>,
    origin: Origin,
    argv: Vec<String>,
    tx: mpsc::Sender<(Origin, ExecResult)>,
) {
    start_with(move |argv| runner.run(argv), origin, argv, tx);
}

// edge
/// `start` over any runner, so a test can hand in a closure that answers when it chooses.
fn start_with(
    run: impl FnOnce(&[String]) -> ExecResult + Send + 'static,
    origin: Origin,
    argv: Vec<String>,
    tx: mpsc::Sender<(Origin, ExecResult)>,
) {
    std::thread::spawn(move || {
        // The receiver may be gone by the time the command finishes.
        let _ = tx.send((origin, run(&argv)));
    });
}

/// The status-line level and text for a finished command, clipped to 200 chars.
pub fn status_summary(result: &ExecResult) -> (StatusLevel, String) {
    let last = last_nonempty_line(&result.output);
    let (level, text) = match (result.code, last) {
        (Some(0), Some(line)) => (StatusLevel::Ok, line),
        (Some(0), None) => (StatusLevel::Ok, "done".to_owned()),
        (Some(n), Some(line)) => (StatusLevel::Failed, format!("failed (exit {n}): {line}")),
        (Some(n), None) => (StatusLevel::Failed, format!("failed (exit {n}):")),
        (None, _) => (
            StatusLevel::Failed,
            format!("failed to start: {}", result.output.trim()),
        ),
    };
    (level, text.chars().take(CLIP_CHARS).collect())
}

/// The command line, its output lines, then the exit line. A start failure ends `exit none`.
pub fn frame_lines(result: &ExecResult) -> Vec<String> {
    let exit = match result.code {
        Some(n) => format!("exit {n}"),
        None => "exit none".to_owned(),
    };
    std::iter::once(format!("$ {}", result.argv.join(" ")))
        .chain(result.output.lines().map(str::to_owned))
        .chain(std::iter::once(exit))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    struct FakeRunner {
        result: ExecResult,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl CmdRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> ExecResult {
            self.calls.lock().unwrap().push(argv.to_vec());
            self.result.clone()
        }
    }

    fn result(code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: vec!["cox".to_owned(), "land".to_owned(), "t1".to_owned()],
            code,
            output: output.to_owned(),
        }
    }

    #[test]
    fn exit_zero_with_output_is_ok_with_the_last_nonempty_line() {
        assert_eq!(
            status_summary(&result(Some(0), "step one\nlanded t1\n\n")),
            (StatusLevel::Ok, "landed t1".to_owned())
        );
    }

    #[test]
    fn exit_zero_without_output_is_ok_done() {
        assert_eq!(
            status_summary(&result(Some(0), "")),
            (StatusLevel::Ok, "done".to_owned())
        );
    }

    #[test]
    fn exit_two_is_failed_with_the_last_line() {
        assert_eq!(
            status_summary(&result(Some(2), "warming up\nno such task\n")),
            (
                StatusLevel::Failed,
                "failed (exit 2): no such task".to_owned()
            )
        );
    }

    #[test]
    fn a_start_failure_is_failed_to_start() {
        assert_eq!(
            status_summary(&result(None, "No such file or directory (os error 2)")),
            (
                StatusLevel::Failed,
                "failed to start: No such file or directory (os error 2)".to_owned()
            )
        );
    }

    #[test]
    fn the_text_is_clipped_to_200_chars() {
        let (_, text) = status_summary(&result(Some(0), &"x".repeat(300)));
        assert_eq!(text, "x".repeat(200));
    }

    #[test]
    fn frame_lines_is_the_command_the_output_and_the_exit() {
        assert_eq!(
            frame_lines(&result(Some(2), "a\nb\n")),
            vec!["$ cox land t1", "a", "b", "exit 2"]
        );
    }

    #[test]
    fn start_delivers_one_result_and_records_the_argv() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let canned = result(Some(0), "ok\n");
        let runner = Arc::new(FakeRunner {
            result: canned.clone(),
            calls: Arc::clone(&calls),
        });
        let (tx, rx) = mpsc::channel();
        let argv = vec!["cox".to_owned(), "land".to_owned(), "t1".to_owned()];
        start(runner, Origin::Action, argv.clone(), tx);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok((Origin::Action, canned))
        );
        assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
        assert_eq!(*calls.lock().unwrap(), vec![argv]);
    }

    fn answer(words: &[&str], code: i32, output: &str) -> ExecResult {
        ExecResult {
            argv: words.iter().map(|w| (*w).to_owned()).collect(),
            code: Some(code),
            output: output.to_owned(),
        }
    }

    /// A runner that answers `result` only after the test sends on the returned gate, so the
    /// test thread alone decides when it finishes.
    fn gated(
        result: ExecResult,
    ) -> (
        mpsc::Sender<()>,
        impl FnOnce(&[String]) -> ExecResult + Send + 'static,
    ) {
        let (open, gate) = mpsc::channel::<()>();
        let run = move |_: &[String]| {
            let _ = gate.recv();
            result
        };
        (open, run)
    }

    #[test]
    fn results_arrive_paired_with_their_own_origin_in_completion_order() {
        let slow = answer(&["cox", "runs", "move"], 0, "moved\n");
        let fast = answer(&["cox", "runs", "pause"], 1, "no such run\n");
        let (open_slow, slow_run) = gated(slow.clone());
        let (tx, rx) = mpsc::channel();
        start_with(slow_run, Origin::Action, slow.argv.clone(), tx.clone());
        let fast_answer = fast.clone();
        start_with(move |_| fast_answer, Origin::Palette, fast.argv.clone(), tx);

        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait), Ok((Origin::Palette, fast)));
        open_slow.send(()).unwrap();
        assert_eq!(rx.recv_timeout(wait), Ok((Origin::Action, slow)));
    }

    #[test]
    fn settings_load_dry_run_and_apply_keep_their_origins_when_finishing_in_reverse() {
        let scope = || "budgets".to_owned();
        let key = || "max_usd".to_owned();
        let dry = Origin::SettingsDryRun {
            scope: scope(),
            key: key(),
        };
        let apply = Origin::SettingsApply {
            scope: scope(),
            key: key(),
        };
        let load_out = answer(&["cox", "settings", "show"], 0, "rows\n");
        let dry_out = answer(&["cox", "settings", "set", "--dry-run"], 0, "diff\n");
        let apply_out = answer(&["cox", "settings", "set"], 2, "refused\n");
        let (open_load, load_run) = gated(load_out.clone());
        let (open_dry, dry_run) = gated(dry_out.clone());
        let (tx, rx) = mpsc::channel();
        start_with(
            load_run,
            Origin::SettingsLoad,
            load_out.argv.clone(),
            tx.clone(),
        );
        start_with(dry_run, dry.clone(), dry_out.argv.clone(), tx.clone());
        let apply_answer = apply_out.clone();
        start_with(
            move |_| apply_answer,
            apply.clone(),
            apply_out.argv.clone(),
            tx,
        );

        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait), Ok((apply, apply_out)));
        open_dry.send(()).unwrap();
        assert_eq!(rx.recv_timeout(wait), Ok((dry, dry_out)));
        open_load.send(()).unwrap();
        assert_eq!(rx.recv_timeout(wait), Ok((Origin::SettingsLoad, load_out)));
    }

    #[test]
    fn real_runner_reports_the_exit_code_and_a_start_failure() {
        let argv = vec!["cox".to_owned()];
        let run = |program: &str| {
            RealRunner {
                program: program.to_owned(),
            }
            .run(&argv)
        };
        assert_eq!(run("true").code, Some(0));
        assert_eq!(run("false").code, Some(1));
        let missing = run("definitely-not-a-binary-xyz");
        assert_eq!(missing.code, None);
        assert!(!missing.output.is_empty());
    }
}
