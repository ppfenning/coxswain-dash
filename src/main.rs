//! coxtop, the Coxswain fleet dashboard. Streams `cox dash --feed` into the `App`, dispatches
//! rendering by page, and turns Tab/BackTab/t/digit keys into `App` mutations. `q` and Ctrl-C
//! save the page and theme and exit.

mod app;
mod config;
mod detail;
mod feed;
mod input;
// A later task wires the pty session into the chair panel.
#[allow(dead_code)]
mod pty;
mod theme;
mod ui;

use std::io::{self, BufRead, BufReader};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use app::{App, AppPage, DetailKind, ThemeId};

fn version_line() -> String {
    format!("coxtop {}", env!("CARGO_PKG_VERSION"))
}

/// The `cox dash --detail` argument naming `kind`. The `src/ui/mod.rs` twin, `kind_label`,
/// picks the same three strings for the loading paragraph.
fn kind_arg(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Run => "run",
        DetailKind::Initiative => "initiative",
        DetailKind::Machine => "machine",
    }
}

/// Spawns `cox dash --detail <kind_arg(kind)> <id>` and reads its stdout on a thread into a
/// fresh channel, the same way the feed's child is read.
fn spawn_detail_reader(
    kind: DetailKind,
    id: &str,
) -> (std::process::Child, mpsc::Receiver<String>) {
    let mut child = detail::spawn_detail(kind_arg(kind), id);
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

fn main() {
    eprintln!("{}", version_line());

    let (page, theme_id) =
        config::load_state(&config::state_path()).unwrap_or((AppPage::Regatta, ThemeId::Regatta));
    let mut app = App::new(page, theme_id).with_utc_offset(*chrono::Local::now().offset());

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

    enable_raw_mode().expect("failed to enable raw mode");
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)
        .expect("failed to enter the alternate screen");
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).expect("failed to build the terminal");

    let mut detail_child: Option<std::process::Child> = None;
    let mut detail_rx: Option<mpsc::Receiver<String>> = None;
    let mut detail_key: Option<(DetailKind, String)> = None;

    let mut changed = true;
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
            if let Some((kind, id)) = wanted {
                let (child, rx) = spawn_detail_reader(kind, &id);
                detail_child = Some(child);
                detail_rx = Some(rx);
            }
            changed = true;
        }

        if let Some(rx) = &detail_rx {
            while let Ok(line) = rx.try_recv() {
                if let Ok(snapshot) = detail::parse_detail(&line) {
                    app.apply_detail_snapshot(snapshot);
                    changed = true;
                }
            }
        }

        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    let is_quit = key.code == KeyCode::Char('q')
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL));
                    if is_quit {
                        break;
                    }
                    input::handle_key(&mut app, key.code);
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

        if changed {
            let theme = theme::resolve(app.theme());
            terminal
                .draw(|f| ui::render(f, &app, &theme))
                .expect("failed to draw the frame");
            changed = false;
        }
    }

    config::save_state(&config::state_path(), app.page(), app.theme());

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
    fn kind_arg_names_the_three_detail_kinds() {
        assert_eq!(kind_arg(DetailKind::Run), "run");
        assert_eq!(kind_arg(DetailKind::Initiative), "initiative");
        assert_eq!(kind_arg(DetailKind::Machine), "machine");
    }
}
