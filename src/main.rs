//! coxtop, the Coxswain fleet dashboard. Streams `cox dash --feed` into the `App`, dispatches
//! rendering by page, and turns Tab/BackTab/t/digit keys into `App` mutations. `q` and Ctrl-C
//! save the page and theme and exit.

mod app;
mod config;
mod feed;
mod input;
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

use app::{App, AppPage, ThemeId};

fn version_line() -> String {
    format!("coxtop {}", env!("CARGO_PKG_VERSION"))
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

    let mut changed = true;
    loop {
        while let Ok(line) = rx.try_recv() {
            if let Ok(snapshot) = feed::parse_snapshot(&line) {
                app.apply_snapshot(snapshot);
                changed = true;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_names_the_binary() {
        assert!(version_line().starts_with("coxtop "));
    }
}
