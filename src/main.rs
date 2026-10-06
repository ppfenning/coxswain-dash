//! coxtop, the Coxswain fleet dashboard. Streams `cox dash --feed` into the `App`, dispatches
//! rendering by page, and turns Tab/BackTab/t/digit keys into `App` mutations. `q` and Ctrl-C
//! save the page and theme and exit.

mod actions;
mod app;
// The panel's drawing and `start_session` are not wired yet.
#[allow(dead_code)]
mod chair_panel;
mod config;
mod confirm;
mod decision_card;
mod detail;
mod exec;
mod feed;
mod form;
mod form_initiative;
mod form_machine;
mod input;
mod palette;
// Only `RealPty` is used outside tests; `FakePty` is the test seam.
#[allow(dead_code)]
mod pty;
mod settings;
// `pane` and `refusal_for_selected` are read only by the settings frame, which is not drawn yet.
#[allow(dead_code)]
mod settings_screen;
// `diff_for` is read only by the settings frame, which is not drawn yet.
#[allow(dead_code)]
mod settings_stage;
mod theme;
mod ui;

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use app::{App, AppPage, DetailKind, Origin, ThemeId};
use chair_panel::CloseOutcome;
use exec::{CmdRunner, ExecResult};

fn version_line() -> String {
    format!("coxtop {}", env!("CARGO_PKG_VERSION"))
}

/// Without these flags a terminal reports Shift+Enter as a bare Enter. Only a confirmed
/// `Ok(true)` pushes them; an unsupported terminal or a failed query pushes nothing.
fn enhancement_flags(support: Result<bool, ()>) -> Option<KeyboardEnhancementFlags> {
    match support {
        Ok(true) => Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        Ok(false) | Err(()) => None,
    }
}

/// The `cox dash --detail` argument naming `kind`. The `src/ui/mod.rs` twin, `kind_label`,
/// picks the same three strings for the loading paragraph.
fn kind_arg(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Run => "run",
        DetailKind::Initiative => "initiative",
        DetailKind::Machine => "machine",
        DetailKind::Watch => "watch",
        DetailKind::Spend => "spend",
        DetailKind::Health => "health",
    }
}

/// The id the detail child is asked for. Spend and health take none, so their app-side id, the
/// kind's own word, must not reach the command line.
fn child_id(kind: DetailKind, id: &str) -> &str {
    match kind {
        DetailKind::Spend | DetailKind::Health => "",
        DetailKind::Run | DetailKind::Initiative | DetailKind::Machine | DetailKind::Watch => id,
    }
}

/// Whether the loop runs a detail child for `kind`. Watch reads the feed and has none.
fn has_detail_child(kind: DetailKind) -> bool {
    kind != DetailKind::Watch
}

/// Spawns `cox dash --detail <kind_arg(kind)> <id>` and reads its stdout on a thread into a
/// fresh channel, the same way the feed's child is read.
fn spawn_detail_reader(
    kind: DetailKind,
    id: &str,
) -> (std::process::Child, mpsc::Receiver<String>) {
    let mut child = detail::spawn_detail(kind_arg(kind), child_id(kind, id));
    let stdout = child.stdout.take().expect("detail stdout should be piped");
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    (child, rx)
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// The editor command line. `$EDITOR` may carry arguments, so `code -w` works. Unset or blank
/// falls back to `vi`.
fn editor_argv(env: Option<&str>, path: &str) -> Vec<String> {
    let words: Vec<&str> = env.unwrap_or_default().split_whitespace().collect();
    let program = if words.is_empty() { vec!["vi"] } else { words };
    program
        .into_iter()
        .chain(std::iter::once(path))
        .map(str::to_string)
        .collect()
}

/// The temp file a body is edited in. The pid keeps two coxtops apart, the counter two edits.
fn editor_path(pid: u32, seq: usize) -> PathBuf {
    std::env::temp_dir().join(format!("coxtop-{pid}-{seq}.md"))
}

/// The detail child's body parsed as an initiative. Reads the last non-empty line.
fn initiative_of(body: &str) -> Result<detail::InitiativeDetail, String> {
    let line = body.lines().rev().find(|l| !l.trim().is_empty());
    match line.map(detail::parse_detail) {
        Some(Ok(detail::DetailSnapshot::Initiative(initiative))) => Ok(initiative),
        Some(Ok(_)) => Err("detail was not an initiative".to_string()),
        Some(Err(err)) => Err(format!("detail did not parse: {err}")),
        None => Err("detail was empty".to_string()),
    }
}

/// The result an edit's failed fetch is reported as, so the status line shows it as Failed.
fn failed_fetch(id: &str, message: String) -> ExecResult {
    ExecResult {
        argv: detail::detail_argv("initiative", id),
        code: Some(1),
        output: message,
    }
}

// edge
/// Runs `cox dash --detail initiative <id>` to completion and parses its body.
fn fetch_initiative(id: &str) -> Result<detail::InitiativeDetail, String> {
    let argv = detail::detail_argv(kind_arg(DetailKind::Initiative), id);
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    match detail::run_detail(cmd) {
        detail::DetailOutcome::Body(body) => initiative_of(&body),
        detail::DetailOutcome::Error(line) => Err(line),
    }
}

// edge
/// Leaves raw mode and the alternate screen so a child can own the terminal.
fn suspend_terminal(terminal: &mut Term, pushed: Option<KeyboardEnhancementFlags>) {
    if pushed.is_some() {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
}

// edge
/// Re-enters the alternate screen and raw mode, then clears so the next draw is a full repaint.
fn resume_terminal(
    terminal: &mut Term,
    pushed: Option<KeyboardEnhancementFlags>,
) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    if let Some(flags) = pushed {
        execute!(terminal.backend_mut(), PushKeyboardEnhancementFlags(flags))?;
    }
    terminal.clear()
}

// edge
/// Edits `text` in `$EDITOR` on a temp file with the terminal handed over, and returns the path
/// and what the file held afterwards. The terminal is restored before any failure is returned.
/// A non-zero editor exit still returns the file: the user may have saved before quitting.
fn run_editor(
    terminal: &mut Term,
    pushed: Option<KeyboardEnhancementFlags>,
    seq: usize,
    text: &str,
) -> (Option<PathBuf>, Result<String, String>) {
    let path = editor_path(std::process::id(), seq);
    if let Err(err) = std::fs::write(&path, text) {
        return (
            None,
            Err(format!("could not write {}: {err}", path.display())),
        );
    }
    let argv = editor_argv(
        std::env::var("EDITOR").ok().as_deref(),
        &path.to_string_lossy(),
    );
    suspend_terminal(terminal, pushed);
    let ran = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status();
    let resumed = resume_terminal(terminal, pushed);
    let outcome = match (ran, resumed) {
        (Err(err), _) => Err(format!("could not start {}: {err}", argv[0])),
        (Ok(_), Err(err)) => Err(format!("could not restore the terminal: {err}")),
        (Ok(_), Ok(())) => std::fs::read_to_string(&path)
            .map_err(|err| format!("could not read {}: {err}", path.display())),
    };
    (Some(path), outcome)
}

/// Spawns `cox dash --feed` and reads its stdout on a thread into a fresh channel.
fn spawn_feed_reader() -> (std::process::Child, mpsc::Receiver<String>) {
    let mut child = feed::spawn_feed(Some(2));
    let stdout = child.stdout.take().expect("feed stdout should be piped");
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    (child, rx)
}

/// Replaces the feed child with a fresh one, which emits a snapshot at once. The loop has no
/// refresh tick of its own: the feed's `--interval` is the tick, so this is the early one.
fn restart_feed(mut child: std::process::Child) -> (std::process::Child, mpsc::Receiver<String>) {
    let _ = child.kill();
    let _ = child.wait();
    spawn_feed_reader()
}

/// The origin a finished command's result goes to: the oldest started command still waiting.
/// `None` when no command is waiting, and the result is dropped.
fn pair_origin(queue: &mut VecDeque<Origin>, _result: &ExecResult) -> Option<Origin> {
    queue.pop_front()
}

fn main() {
    eprintln!("{}", version_line());

    let (page, theme_id) =
        config::load_state(&config::state_path()).unwrap_or((AppPage::Regatta, ThemeId::Regatta));
    let mut app = App::new(page, theme_id).with_utc_offset(*chrono::Local::now().offset());

    let (mut child, mut rx) = spawn_feed_reader();

    let runner: Arc<dyn CmdRunner> = Arc::new(exec::RealRunner::cox());
    let (exec_tx, exec_rx) = mpsc::channel::<ExecResult>();
    let mut origins: VecDeque<Origin> = VecDeque::new();

    enable_raw_mode().expect("failed to enable raw mode");
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)
        .expect("failed to enter the alternate screen");
    let pushed = enhancement_flags(supports_keyboard_enhancement().map_err(|_| ()));
    if let Some(flags) = pushed {
        execute!(out, PushKeyboardEnhancementFlags(flags))
            .expect("failed to push the keyboard enhancement flags");
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
            previous_hook(info);
        }));
    }
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).expect("failed to build the terminal");

    let mut detail_child: Option<std::process::Child> = None;
    let mut detail_rx: Option<mpsc::Receiver<String>> = None;
    let mut detail_key: Option<(DetailKind, String)> = None;

    let mut changed = true;
    let mut bells_rung = 0;
    let mut editor_runs: usize = 0;
    // Set when `q` met the close prompt; cleared when the answer keeps the session open.
    let mut quit_pending = false;
    loop {
        while let Ok(line) = rx.try_recv() {
            if let Ok(snapshot) = feed::parse_snapshot(&line) {
                app.apply_snapshot(snapshot);
                changed = true;
            }
        }

        let wanted = app.detail().map(|(kind, id, _)| (*kind, id.clone()));
        if wanted != detail_key {
            if let Some(mut child) = detail_child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            detail_rx = None;
            detail_key = wanted.clone();
            if let Some((kind, id)) = wanted.filter(|(kind, _)| has_detail_child(*kind)) {
                let (child, rx) = spawn_detail_reader(kind, &id);
                detail_child = Some(child);
                detail_rx = Some(rx);
            }
            changed = true;
        }

        if let Some(rx) = &detail_rx {
            while let Ok(line) = rx.try_recv() {
                // A line that does not parse replaces the loading paragraph with why,
                // instead of leaving the drill on loading forever.
                match detail::parse_detail(&line) {
                    Ok(snapshot) => app.apply_detail_snapshot(snapshot),
                    Err(err) => app.apply_detail_error(format!("detail did not parse: {err}")),
                }
                changed = true;
            }
        }

        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    // A focused panel owns every key, `q` and Ctrl-C included.
                    let is_quit = !app.chair_panel().focused()
                        && !app.chair_panel().confirming_close()
                        && (key.code == KeyCode::Char('q')
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL)));
                    if is_quit {
                        // A started session asks first, so exit waits for the prompt's answer.
                        match app.chair_panel_mut().request_close() {
                            CloseOutcome::Closed => break,
                            CloseOutcome::NeedsConfirm => quit_pending = true,
                        }
                    } else {
                        input::handle_event(&mut app, key);
                    }
                    if quit_pending && !app.chair_panel().confirming_close() {
                        if !app.chair_panel().is_open() {
                            break;
                        }
                        quit_pending = false;
                    }
                    changed = true;
                }
                Ok(Event::Resize(cols, rows)) => {
                    app.chair_panel_mut().resize(rows, cols);
                    changed = true;
                }
                Ok(Event::Mouse(mouse)) if app.page() == AppPage::Regatta => {
                    let size = terminal.size().expect("failed to read the terminal size");
                    let area = Rect::new(0, 0, size.width, size.height);
                    let rects = ui::regatta_frame_rects(area, &app);
                    input::handle_mouse(&mut app, mouse, &rects);
                    changed = true;
                }
                _ => {}
            }
        }

        if let Some(id) = app.take_detail_request() {
            match fetch_initiative(&id) {
                Ok(initiative) => app.open_edit_initiative(&initiative),
                Err(message) => app.apply_exec_result(failed_fetch(&id, message), Origin::Action),
            }
            changed = true;
        }

        if let Some(idx) = app.take_editor_request() {
            let text = app
                .form()
                .and_then(|form| form.fields.get(idx))
                .map(|field| field.text.clone())
                .unwrap_or_default();
            let (path, edited) = run_editor(&mut terminal, pushed, editor_runs, &text);
            editor_runs += 1;
            app.editor_done(idx, path, edited);
            changed = true;
        }

        if let Some(pending) = app.take_pending() {
            origins.push_back(pending.origin);
            exec::start(Arc::clone(&runner), pending.argv, exec_tx.clone());
            changed = true;
        }

        let mut refresh = false;
        while let Ok(result) = exec_rx.try_recv() {
            let code = result.code;
            if let Some(origin) = pair_origin(&mut origins, &result) {
                app.apply_exec_result(result, origin);
                refresh = refresh || code == Some(0);
                changed = true;
            }
        }
        if refresh {
            (child, rx) = restart_feed(child);
        }

        if app.chair_panel().is_open() {
            app.poll_chair_panel();
            changed = true;
        }

        if app.bells() != bells_rung {
            bells_rung = app.bells();
            let bell = terminal.backend_mut();
            let _ = bell.write_all(b"\x07").and_then(|()| bell.flush());
        }

        if changed {
            let theme = theme::resolve(app.theme());
            terminal
                .draw(|f| ui::render(f, &app, &theme))
                .expect("failed to draw the frame");
            changed = false;
        }
    }

    config::save_state(&config::state_path(), app.page(), app.theme());
    // Kills the local attach process only. The chair's remote session keeps running.
    app.chair_panel_mut().close();

    if pushed.is_some() {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)
            .expect("failed to pop the keyboard enhancement flags");
    }
    disable_raw_mode().expect("failed to disable raw mode");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .expect("failed to leave the alternate screen");
    terminal.show_cursor().expect("failed to show the cursor");
    let _ = child.kill();
    let _ = child.wait();
    if let Some(mut child) = detail_child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_names_the_binary() {
        assert!(version_line().starts_with("coxtop "));
    }

    #[test]
    fn a_supported_terminal_gets_the_disambiguate_flag() {
        assert_eq!(
            enhancement_flags(Ok(true)),
            Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    #[test]
    fn an_unsupported_terminal_gets_no_flags() {
        assert_eq!(enhancement_flags(Ok(false)), None);
    }

    #[test]
    fn a_failed_support_query_gets_no_flags() {
        assert_eq!(enhancement_flags(Err(())), None);
    }

    fn done() -> ExecResult {
        ExecResult {
            argv: vec!["cox".to_owned()],
            code: Some(0),
            output: String::new(),
        }
    }

    #[test]
    fn results_pair_with_origins_in_start_order() {
        let mut queue = VecDeque::from([Origin::Action, Origin::Palette]);
        assert_eq!(pair_origin(&mut queue, &done()), Some(Origin::Action));
        assert_eq!(pair_origin(&mut queue, &done()), Some(Origin::Palette));
    }

    #[test]
    fn an_extra_result_with_an_empty_queue_has_no_origin() {
        let mut queue = VecDeque::new();
        assert_eq!(pair_origin(&mut queue, &done()), None);
    }

    struct FakeRunner {
        result: ExecResult,
        calls: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl CmdRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> ExecResult {
            self.calls.lock().unwrap().push(argv.to_vec());
            self.result.clone()
        }
    }

    #[test]
    fn a_confirmed_action_runs_once_with_its_argv_and_sets_the_status() {
        let json = include_str!("../tests/fixtures/dash_feed_v1.json");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(feed::parse_snapshot(json).expect("fixture should parse"));
        assert!(app.begin_action('k'));
        app.modal_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        ));

        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let argv: Vec<String> = ["cox", "runs", "stop", "dash-feed-1"]
            .map(str::to_owned)
            .to_vec();
        let runner: Arc<dyn CmdRunner> = Arc::new(FakeRunner {
            result: ExecResult {
                argv: argv.clone(),
                code: Some(0),
                output: "stopped dash-feed-1\n".to_owned(),
            },
            calls: Arc::clone(&calls),
        });
        let (tx, rx) = mpsc::channel::<ExecResult>();
        let mut origins = VecDeque::new();

        let pending = app.take_pending().expect("a confirmed action is pending");
        origins.push_back(pending.origin);
        exec::start(runner, pending.argv, tx);
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the runner should deliver a result");
        assert_eq!(app.status(), None);
        let origin = pair_origin(&mut origins, &result).expect("the origin is queued");
        app.apply_exec_result(result, origin);

        assert_eq!(*calls.lock().unwrap(), vec![argv]);
        assert_eq!(
            app.status(),
            Some(&(exec::StatusLevel::Ok, "stopped dash-feed-1".to_owned()))
        );
    }

    #[test]
    fn settings_origins_pair_in_start_order() {
        let dry = Origin::SettingsDryRun {
            scope: "budgets".to_owned(),
            key: "max_usd".to_owned(),
        };
        let apply = Origin::SettingsApply {
            scope: "budgets".to_owned(),
            key: "max_usd".to_owned(),
        };
        let mut queue = VecDeque::from([Origin::SettingsLoad, dry.clone(), apply.clone()]);
        assert_eq!(pair_origin(&mut queue, &done()), Some(Origin::SettingsLoad));
        assert_eq!(pair_origin(&mut queue, &done()), Some(dry));
        assert_eq!(pair_origin(&mut queue, &done()), Some(apply));
        assert_eq!(pair_origin(&mut queue, &done()), None);
    }

    const ROWS: &str = r#"{"sections":[{"id":"budgets","rows":[{"section":"budgets","scope":"budgets","key":"max_usd","value":"20","file":"f.toml","tracked":true,"pat_only":false}]}]}"#;

    fn key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Runs the app's pending call through a `FakeRunner` that answers `code` and `output`, pairs
    /// the result with its origin and applies it. Returns the argvs the runner saw.
    fn run_pending(app: &mut App, code: i32, output: &str) -> Vec<Vec<String>> {
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runner: Arc<dyn CmdRunner> = Arc::new(FakeRunner {
            result: ExecResult {
                argv: vec!["cox".to_owned()],
                code: Some(code),
                output: output.to_owned(),
            },
            calls: Arc::clone(&calls),
        });
        let (tx, rx) = mpsc::channel::<ExecResult>();
        let mut origins = VecDeque::new();

        let pending = app.take_pending().expect("a call is pending");
        origins.push_back(pending.origin);
        exec::start(runner, pending.argv, tx);
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the runner should deliver a result");
        let origin = pair_origin(&mut origins, &result).expect("the origin is queued");
        app.apply_exec_result(result, origin);

        calls.lock().unwrap().clone()
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    /// An app whose settings screen holds `ROWS` and has `budgets max_usd` staged at 40, with the
    /// dry-run call still pending.
    fn app_with_staged_edit() -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.open_settings();
        run_pending(&mut app, 0, ROWS);
        for code in [
            KeyCode::Tab,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Char('4'),
            KeyCode::Char('0'),
            KeyCode::Enter,
        ] {
            app.settings_key(key(code));
        }
        app
    }

    #[test]
    fn opening_settings_runs_the_get_call_and_a_json_result_loads_the_rows() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.open_settings();
        let calls = run_pending(&mut app, 0, ROWS);
        assert_eq!(calls, vec![strings(&["cox", "settings", "get", "--json"])]);
        assert_eq!(app.settings().map(|s| s.current_rows().len()), Some(1));
    }

    #[test]
    fn a_refused_dry_run_leaves_the_edit_staged_with_a_refusal() {
        let mut app = app_with_staged_edit();
        let calls = run_pending(&mut app, 1, "max_usd must be below 30\n");
        assert_eq!(
            calls,
            vec![strings(&[
                "cox",
                "settings",
                "set",
                "budgets",
                "max_usd",
                "40",
                "--dry-run"
            ])]
        );
        let staged = app.settings().map(|s| s.staged().clone());
        assert_eq!(staged.as_ref().map(|s| s.edits().len()), Some(1));
        assert_eq!(
            staged
                .as_ref()
                .and_then(|s| s.refusal_for("budgets", "max_usd")),
            Some("max_usd must be below 30")
        );
    }

    #[test]
    fn a_confirmed_apply_runs_the_set_call_removes_the_edit_and_queues_the_reload() {
        let mut app = app_with_staged_edit();
        app.take_pending();
        app.settings_key(key(KeyCode::Tab));
        app.settings_key(key(KeyCode::Char('a')));
        app.modal_key(key(KeyCode::Char('y')));
        let calls = run_pending(&mut app, 0, "set\n");
        assert_eq!(
            calls,
            vec![strings(&[
                "cox", "settings", "set", "budgets", "max_usd", "40"
            ])]
        );
        assert_eq!(app.settings().map(|s| s.staged().edits().len()), Some(0));
        assert_eq!(
            app.take_pending().map(|p| (p.argv, p.origin)),
            Some((
                strings(&["cox", "settings", "get", "--json"]),
                Origin::SettingsLoad
            ))
        );
    }

    #[test]
    fn editor_argv_runs_a_plain_program_on_the_path() {
        assert_eq!(editor_argv(Some("vim"), "/t/f"), strings(&["vim", "/t/f"]));
    }

    #[test]
    fn editor_argv_splits_arguments_before_the_path() {
        assert_eq!(
            editor_argv(Some("code -w"), "/t/f"),
            strings(&["code", "-w", "/t/f"])
        );
    }

    #[test]
    fn editor_argv_falls_back_to_vi_when_unset() {
        assert_eq!(editor_argv(None, "/t/f"), strings(&["vi", "/t/f"]));
    }

    #[test]
    fn editor_argv_falls_back_to_vi_when_blank() {
        assert_eq!(editor_argv(Some("  "), "/t/f"), strings(&["vi", "/t/f"]));
    }

    #[test]
    fn kind_arg_names_each_detail_kind() {
        assert_eq!(kind_arg(DetailKind::Watch), "watch");
        assert_eq!(kind_arg(DetailKind::Spend), "spend");
        assert_eq!(kind_arg(DetailKind::Health), "health");
        assert_eq!(kind_arg(DetailKind::Run), "run");
        assert_eq!(kind_arg(DetailKind::Initiative), "initiative");
        assert_eq!(kind_arg(DetailKind::Machine), "machine");
    }

    #[test]
    fn spend_and_health_ask_for_no_id_and_watch_runs_no_child() {
        assert_eq!(child_id(DetailKind::Spend, "spend"), "");
        assert_eq!(child_id(DetailKind::Health, "health"), "");
        assert_eq!(child_id(DetailKind::Run, "r1"), "r1");
        assert!(has_detail_child(DetailKind::Spend));
        assert!(has_detail_child(DetailKind::Health));
        assert!(!has_detail_child(DetailKind::Watch));
    }
}
