//! Dispatches rendering to the current page's module. No layout logic lives here; each page
//! module owns its own `render`.

mod chair_card;
pub mod confirm_dialog;
pub mod form_frame;
pub mod health_drill;
pub mod initiative_drill;
pub mod machine_drill;
pub mod palette_frame;
mod regatta;
mod run_cost;
pub mod run_drill;
pub mod settings_frame;
mod slipstream;
pub mod spend_drill;
pub mod status_line;
pub mod watch_drill;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::actions::{Target, bindings};
use crate::app::{App, AppPage, DetailKind, Modal};
use crate::chair_panel;
use crate::detail::DetailSnapshot;
use crate::exec::StatusLevel;
use crate::feed::{FeedSnapshot, Run, RunEnd};
use crate::theme::Theme;

pub use regatta::frame_rects as regatta_frame_rects;

/// Drawn in every frame but the chair panel while the feed has failed before its first snapshot.
const NO_FEED: &str = "no feed yet";

/// The page or detail view, then the status line over the footer row, then the open modal.
pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    render_base(f, app, theme);
    render_form(f, app, theme);
    render_status(f, app, theme);
    render_modal(f, app, theme);
}

/// The open form over the page, under the status line and any modal, so a confirm drawn after
/// it shows its command above the form.
fn render_form(f: &mut Frame, app: &App, theme: &Theme) {
    if let Some(form) = app.form() {
        form_frame::render(f, f.area(), theme, form);
    }
}

/// While a status shows it replaces the footer row; the page body is never resized. With no
/// status, a focused chair panel's release hint takes only the page's share of that row, so the
/// panel keeps its bottom border; a full-width panel leaves no share and its title carries it.
fn render_status(f: &mut Frame, app: &App, theme: &Theme) {
    let hint = chair_panel::release_hint(app.chair_focused())
        .map(|text| (StatusLevel::Ok, text.to_string()));
    let shown = match (app.status(), hint.as_ref()) {
        (Some(status), _) => Some((last_row(f.area()), status)),
        (None, Some(hint)) => Some((last_row(split_page(f.area(), app).0), hint)),
        (None, None) => None,
    };
    if let Some((row, status)) = shown.filter(|(row, _)| row.width > 0) {
        f.render_widget(Clear, row);
        status_line::render(f, row, theme, Some(status));
    }
}

fn render_modal(f: &mut Frame, app: &App, theme: &Theme) {
    match app.modal() {
        Some(Modal::Confirm(state)) => confirm_dialog::render(f, f.area(), theme, state),
        Some(Modal::Palette(state)) => palette_frame::render(f, f.area(), theme, state),
        None => {}
    }
}

/// An open settings screen takes the page body in place of the page or detail view.
fn render_base(f: &mut Frame, app: &App, theme: &Theme) {
    if let Some(screen) = app.settings() {
        return settings_frame::render(f, f.area(), theme, screen);
    }
    if app.detail().is_none() {
        return render_page(f, app, theme);
    }
    // A detail view takes the page's share of the width, so the chair panel stays up beside it.
    let (body, side) = split_page(f.area(), app);
    if body.width > 0 {
        render_detail(f, body, app, theme);
    }
    if let Some(side) = side {
        render_panel(f, side, app, theme);
    }
}

/// The open detail view, its error, or its loading paragraph, drawn in `area`.
fn render_detail(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    match (app.detail(), app.detail_error()) {
        (Some((kind, id, _)), Some(err)) => render_detail_error(f, area, *kind, id, err, theme),
        // Watch holds no snapshot: it reads the feed's list, empty before the first snapshot.
        (Some((DetailKind::Watch, _, _)), None) => {
            let items = app.snapshot().map_or(&[][..], |s| s.watch.as_slice());
            watch_drill::render(f, area, items, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Spend(detail)))), None) => {
            spend_drill::render(f, area, detail, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Health(detail)))), None) => {
            health_drill::render(f, area, detail, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Run(detail)))), None) => {
            let project = app.snapshot().and_then(|s| run_project(s, &detail.run));
            run_drill::render(f, area, detail, project, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Initiative(detail)))), None) => {
            initiative_drill::render(f, area, detail, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Machine(detail)))), None) => {
            machine_drill::render(f, area, detail, theme, app.utc_offset())
        }
        (Some((kind, id, None)), None) => render_loading(f, area, *kind, id, theme),
        (None, _) => {}
    }
}

/// The page and, when the chair panel is open, the panel on its right. The page gets the width
/// the panel leaves; a panel at full width leaves it none.
fn render_page(f: &mut Frame, app: &App, theme: &Theme) {
    let (page, side) = split_page(f.area(), app);
    if page.width > 0 {
        render_page_in(f, page, app, theme);
    }
    if let Some(side) = side {
        render_panel(f, side, app, theme);
    }
}

/// The page's area on a screen of `area`, then the chair panel's while it is shown. A shown
/// panel is drawn even with no session, so its prompt (Enter starts a session) or the last line
/// of an ended session is visible. Drawing and mouse hit-testing both split here.
pub fn split_page(area: Rect, app: &App) -> (Rect, Option<Rect>) {
    let panel = app.chair_panel();
    let percent = (panel.is_open() || app.chair_panel_shown()).then(|| panel.width().percent());
    split_panel(area, percent)
}

/// The page's area, then the panel's when `percent` is the panel's share of the width. Without
/// a panel the page keeps `area` whole.
fn split_panel(area: Rect, percent: Option<u16>) -> (Rect, Option<Rect>) {
    let Some(percent) = percent else {
        return (area, None);
    };
    let width = u16::try_from(u32::from(area.width) * u32::from(percent.min(100)) / 100)
        .unwrap_or(area.width);
    let page = Rect {
        width: area.width - width,
        ..area
    };
    let side = Rect {
        x: area.x + page.width,
        width,
        ..area
    };
    (page, Some(side))
}

/// Rows the decision card takes above the terminal: its text, its borders and room for a
/// wrapped line, never more than half the panel.
fn card_height(options: usize, panel_height: u16) -> u16 {
    let wanted = u16::try_from(options)
        .unwrap_or(u16::MAX)
        .saturating_add(10);
    wanted.min(panel_height / 2)
}

/// The chair panel in `area`, with the decision card above its terminal while the card shows.
fn render_panel(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let card = app.decision_card();
    let Some(decision) = card.decision().filter(|_| app.card_visible()) else {
        return app.chair_panel().render(f, area, theme);
    };
    let top = Rect {
        height: card_height(decision.options.len(), area.height),
        ..area
    };
    let rest = Rect {
        y: area.y + top.height,
        height: area.height - top.height,
        ..area
    };
    card.render(f, top, theme, app.utc_offset());
    app.chair_panel().render(f, rest, theme);
}

/// The page behind any detail view. A feed error with no snapshot yet replaces the waiting
/// text; an error beside a held snapshot is drawn over the finished page.
fn render_page_in(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    match (app.snapshot(), app.feed_error()) {
        (None, Some(err)) => render_feed_failed(f, area, app, err, theme),
        (_, err) => match app.page() {
            AppPage::Regatta => {
                regatta::render(f, area, app, theme);
                if let Some(err) = err {
                    render_error_line(f, last_row(chair_inner(area, app)), err, theme);
                }
            }
            // Slipstream draws its own error line, in its own palette.
            AppPage::Slipstream => slipstream::render(f, area, app),
        },
    }
}

/// `text` cut to `width` chars, so a line drawn into a row of that width never wraps.
fn fit_line(text: &str, width: u16) -> String {
    text.chars().take(usize::from(width)).collect()
}

fn feed_error_text(err: &str) -> String {
    format!("feed error: {err}")
}

fn bordered_inner(rect: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(rect)
}

fn first_row(rect: Rect) -> Rect {
    Rect {
        height: rect.height.min(1),
        ..rect
    }
}

fn last_row(rect: Rect) -> Rect {
    let height = rect.height.min(1);
    Rect {
        y: rect.y + rect.height - height,
        height,
        ..rect
    }
}

/// Regatta's chair frame, or the first visible frame when frame 1 is hidden. `area` is the page.
fn regatta_chair(area: Rect, app: &App) -> Option<Rect> {
    let rects = regatta::page_frame_rects(area, app);
    rects[0].or_else(|| rects.into_iter().flatten().next())
}

/// The inside of Regatta's chair frame (the first visible frame when frame 1 is hidden).
fn chair_inner(area: Rect, app: &App) -> Rect {
    bordered_inner(regatta_chair(area, app).unwrap_or(area))
}

/// Draws `feed error: <err>` on one row, in the error status color, clearing what was there.
fn render_error_line(f: &mut Frame, row: Rect, err: &str, theme: &Theme) {
    let text = fit_line(&feed_error_text(err), row.width);
    f.render_widget(Clear, row);
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.status_failed).bg(theme.bg)),
        row,
    );
}

/// The feed failed before any snapshot: the Regatta frame layout, the chair frame carrying the
/// error line and every other frame saying there is no feed. The word "waiting" is never drawn.
fn render_feed_failed(f: &mut Frame, area: Rect, app: &App, err: &str, theme: &Theme) {
    let style = Style::default().fg(theme.fg).bg(theme.bg);
    let chair = regatta_chair(area, app);
    f.render_widget(Block::default().style(style), area);
    for rect in regatta::page_frame_rects(area, app).into_iter().flatten() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(style);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if Some(rect) != chair {
            f.render_widget(Paragraph::new(NO_FEED).style(style), first_row(inner));
        }
    }
    let chair_row = first_row(bordered_inner(chair.unwrap_or(area)));
    render_error_line(f, chair_row, err, theme);
}

/// Drawn in place of a drill body when its detail child failed: the block, and one error row.
fn render_detail_error(
    f: &mut Frame,
    area: Rect,
    kind: DetailKind,
    id: &str,
    err: &str,
    theme: &Theme,
) {
    let block = Block::default()
        .title(kind_and_id(kind, id))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    render_error_line(f, first_row(inner), err, theme);
}

/// The lowercase noun a loading paragraph names a [`DetailKind`] with. `src/main.rs`'s
/// `kind_arg` picks the same strings for the `cox dash --detail` argument.
fn kind_label(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Run => "run",
        DetailKind::Initiative => "initiative",
        DetailKind::Machine => "machine",
        DetailKind::Watch => "watch",
        DetailKind::Spend => "spend",
        DetailKind::Health => "health",
    }
}

/// The kind's noun and its id. The watch, spend and health views use the noun as their id, so
/// it is not said twice.
fn kind_and_id(kind: DetailKind, id: &str) -> String {
    match (kind_label(kind), id) {
        (label, id) if label == id => label.to_string(),
        (label, id) => format!("{label} {id}"),
    }
}

/// Drawn in place of a drill board while its detail snapshot is still in flight.
fn render_loading(f: &mut Frame, area: Rect, kind: DetailKind, id: &str, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    let paragraph = Paragraph::new(format!("loading {}", kind_and_id(kind, id)))
        .block(block)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(paragraph, area);
}

/// The action keys of a target as `p pause  k kill`, one `key label` pair per binding.
fn footer_text(target: &Target) -> String {
    bindings(target)
        .iter()
        .map(|b| format!("{} {}", b.key, b.label))
        .collect::<Vec<_>>()
        .join("  ")
}

/// A drill board's footer: its action keys, drawn dim on the bottom border so no row moves.
fn footer_line(target: &Target, theme: &Theme) -> Line<'static> {
    Line::styled(
        footer_text(target),
        Style::default().fg(theme.dim).bg(theme.bg),
    )
}

/// Formats an RFC 3339 timestamp as `%H:%M` in `offset`. A timestamp with no zone is read as UTC.
/// A string that parses neither way comes back unchanged. Pure: the offset is passed in.
pub fn local_time(utc: &str, offset: FixedOffset) -> String {
    match DateTime::parse_from_rfc3339(utc) {
        Ok(at) => at.with_timezone(&offset).format("%H:%M").to_string(),
        Err(_) => match NaiveDateTime::parse_from_str(utc, "%Y-%m-%dT%H:%M:%S%.f") {
            Ok(naive) => naive
                .and_utc()
                .with_timezone(&offset)
                .format("%H:%M")
                .to_string(),
            Err(_) => utc.to_string(),
        },
    }
}

/// Which rule coloured an end chip. `Died` is the only role that may carry `status_failed`
/// when the run has an end; `Fallback` is today's status-word chip for a run with none.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndRole {
    Landed,
    Approved,
    Quarantined,
    Died,
    Idle,
    Stopped,
    Running,
    Fallback,
}

/// A run's end chip. `label` is the bare kind word (the status word for `Fallback`); `spans`
/// are what a page draws: `● label`, then ` cause` when the end carries one.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
#[derive(Debug, Clone, PartialEq)]
pub struct EndChip {
    pub label: String,
    pub role: EndRole,
    pub spans: Vec<Span<'static>>,
}

/// The colour today's runs list gives a status word. Mirrors `regatta::status_color`.
fn status_word_color(theme: &Theme, status: &str) -> Color {
    match status {
        "running" => theme.status_running,
        "approved" => theme.approved,
        "landed" => theme.landed,
        "quarantined" => theme.quarantined,
        "failed" => theme.status_failed,
        _ => theme.status_waiting,
    }
}

fn chip(
    label: &str,
    role: EndRole,
    color: Color,
    cause: Option<(&str, Color)>,
    theme: &Theme,
) -> EndChip {
    let word = Span::styled(
        format!("\u{25cf} {label}"),
        Style::default().fg(color).bg(theme.bg),
    );
    let spans = match cause.filter(|(text, _)| !text.is_empty()) {
        Some((text, cause_color)) => vec![
            word,
            Span::styled(
                format!(" {text}"),
                Style::default().fg(cause_color).bg(theme.bg),
            ),
        ],
        None => vec![word],
    };
    EndChip {
        label: label.to_string(),
        role,
        spans,
    }
}

/// The chip for how a run ended. A run with no end gets today's chip: its status word in the
/// status colour. Only `Died` uses `theme.status_failed`; quarantined has its own role and
/// shows its cause dim.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
pub fn end_chip(end: Option<&RunEnd>, status: &str, theme: &Theme) -> EndChip {
    match end {
        Some(RunEnd::Running) => chip(
            "running",
            EndRole::Running,
            theme.status_running,
            None,
            theme,
        ),
        Some(RunEnd::Landed) => chip("landed", EndRole::Landed, theme.landed, None, theme),
        Some(RunEnd::Approved) => chip("approved", EndRole::Approved, theme.approved, None, theme),
        Some(RunEnd::Quarantined { cause }) => chip(
            "quarantined",
            EndRole::Quarantined,
            theme.quarantined,
            cause.as_deref().map(|c| (c, theme.dim)),
            theme,
        ),
        Some(RunEnd::Idle) => chip("idle", EndRole::Idle, theme.dim, None, theme),
        Some(RunEnd::Died { cause }) => chip(
            "died",
            EndRole::Died,
            theme.status_failed,
            cause.as_deref().map(|c| (c, theme.status_failed)),
            theme,
        ),
        Some(RunEnd::Stopped) => chip("stopped", EndRole::Stopped, theme.dim, None, theme),
        None => chip(
            status,
            EndRole::Fallback,
            status_word_color(theme, status),
            None,
            theme,
        ),
    }
}

/// One initiative's row in a collapsed runs list: its newest run and how many runs it had.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
#[derive(Debug, Clone, Copy)]
pub struct CollapsedRun<'a> {
    pub run: &'a Run,
    pub count: usize,
}

/// A run id is `<initiative>-<n>`; the initiative is the id without a trailing `-<digits>`.
/// An id without that suffix is its own initiative.
fn initiative_of(run_id: &str) -> &str {
    match run_id.rsplit_once('-') {
        Some((initiative, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            initiative
        }
        _ => run_id,
    }
}

/// The run's sequence number, the digits after the last `-`. `None` when the id has none, which
/// sorts older than any number.
fn run_seq(run_id: &str) -> Option<u64> {
    run_id.rsplit_once('-').and_then(|(_, n)| n.parse().ok())
}

/// One row per initiative, in the order each initiative first appears in `runs`. The row holds
/// the initiative's newest run, the one with the highest sequence number (the later row wins a
/// tie), and the number of runs the initiative had. The input is not touched.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
pub fn collapse_runs(runs: &[Run]) -> Vec<CollapsedRun<'_>> {
    runs.iter()
        .enumerate()
        .filter(|(i, r)| {
            let initiative = initiative_of(&r.run);
            !runs[..*i]
                .iter()
                .any(|p| initiative_of(&p.run) == initiative)
        })
        .map(|(_, first)| {
            let initiative = initiative_of(&first.run);
            let group = || runs.iter().filter(|r| initiative_of(&r.run) == initiative);
            CollapsedRun {
                run: group().max_by_key(|r| run_seq(&r.run)).unwrap_or(first),
                count: group().count(),
            }
        })
        .collect()
}

/// The dim `×N` suffix for a collapsed row, drawn only when more than one run was collapsed.
#[allow(dead_code)] // consumed by the regatta and slipstream run lists in later tasks
pub fn count_suffix(count: usize, theme: &Theme) -> Option<Span<'static>> {
    if count > 1 {
        Some(Span::styled(
            format!(" \u{d7}{count}"),
            Style::default().fg(theme.dim).bg(theme.bg),
        ))
    } else {
        None
    }
}

/// The project of run `run_id`, looked up in the live runs and then in the history.
pub fn run_project<'a>(snapshot: &'a FeedSnapshot, run_id: &str) -> Option<&'a str> {
    snapshot
        .runs
        .iter()
        .find(|r| r.run == run_id)
        .map(|r| r.project.as_deref())
        .or_else(|| {
            snapshot
                .history
                .iter()
                .find(|h| h.run == run_id)
                .map(|h| h.project.as_deref())
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_project_looks_in_runs_then_history() {
        let json = r#"{"schema":1,"at":"2026-10-04T20:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-10-04T22:00:00Z","weekly_resets_at":"2026-10-11T04:00:00Z"},"machines":[],"runs":[{"run":"live","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":1.0,"verdict":"ok","status":"running","project":"from-runs"}],"queue":[],"inbox":[],"watch":[],"history":[{"run":"old","machine":"m0","initiative":"i","ended_at":"2026-10-04T08:00:00Z","outcome":"landed","cost_usd":1.0,"project":"from-history"}]}"#;
        let snapshot = crate::feed::parse_snapshot(json).expect("literal feed parses");
        assert_eq!(run_project(&snapshot, "live"), Some("from-runs"));
        assert_eq!(run_project(&snapshot, "old"), Some("from-history"));
        assert_eq!(run_project(&snapshot, "missing"), None);
    }

    fn run(id: &str, end: Option<RunEnd>) -> Run {
        Run {
            run: id.to_string(),
            machine: "omarchy".to_string(),
            phase: "p1".to_string(),
            node: "build".to_string(),
            attempt: 1,
            turns: 0,
            cost: 0.0,
            verdict: "none".to_string(),
            status: "running".to_string(),
            cost_series: vec![],
            end,
            project: None,
        }
    }

    fn theme() -> Theme {
        crate::theme::resolve_for(crate::app::ThemeId::Regatta, Some("truecolor"))
    }

    fn fgs(chip: &EndChip) -> Vec<Option<Color>> {
        chip.spans.iter().map(|s| s.style.fg).collect()
    }

    #[test]
    fn each_end_kind_has_its_label_role_and_colour() {
        let t = theme();
        let cases = [
            (
                RunEnd::Running,
                "running",
                EndRole::Running,
                t.status_running,
            ),
            (RunEnd::Landed, "landed", EndRole::Landed, t.landed),
            (RunEnd::Approved, "approved", EndRole::Approved, t.approved),
            (
                RunEnd::Quarantined { cause: None },
                "quarantined",
                EndRole::Quarantined,
                t.quarantined,
            ),
            (RunEnd::Idle, "idle", EndRole::Idle, t.dim),
            (
                RunEnd::Died { cause: None },
                "died",
                EndRole::Died,
                t.status_failed,
            ),
            (RunEnd::Stopped, "stopped", EndRole::Stopped, t.dim),
        ];
        for (end, label, role, color) in cases {
            let chip = end_chip(Some(&end), "ignored", &t);
            assert_eq!(chip.label, label);
            assert_eq!(chip.role, role);
            assert_eq!(fgs(&chip), vec![Some(color)], "{label}");
            assert_eq!(chip.spans[0].content, format!("\u{25cf} {label}"));
        }
    }

    #[test]
    fn only_died_yields_the_failed_colour() {
        let t = theme();
        let others = [
            RunEnd::Running,
            RunEnd::Landed,
            RunEnd::Approved,
            RunEnd::Idle,
            RunEnd::Stopped,
        ];
        for end in others {
            let chip = end_chip(Some(&end), "", &t);
            assert!(!fgs(&chip).contains(&Some(t.status_failed)), "{end:?}");
        }
        // quarantined shares status_failed's RGB in the Regatta theme, so its role is the proof
        let q = end_chip(Some(&RunEnd::Quarantined { cause: None }), "", &t);
        assert_eq!(q.role, EndRole::Quarantined);
        let died = end_chip(Some(&RunEnd::Died { cause: None }), "", &t);
        assert_eq!(died.role, EndRole::Died);
        assert_eq!(fgs(&died), vec![Some(t.status_failed)]);
    }

    #[test]
    fn quarantined_cause_is_dim_and_died_cause_is_failed() {
        let t = theme();
        let q = end_chip(
            Some(&RunEnd::Quarantined {
                cause: Some("review rejected twice".to_string()),
            }),
            "",
            &t,
        );
        assert_eq!(fgs(&q), vec![Some(t.quarantined), Some(t.dim)]);
        assert_eq!(q.spans[1].content, " review rejected twice");
        let d = end_chip(
            Some(&RunEnd::Died {
                cause: Some("oom".to_string()),
            }),
            "",
            &t,
        );
        assert_eq!(fgs(&d), vec![Some(t.status_failed), Some(t.status_failed)]);
        assert_eq!(d.spans[1].content, " oom");
    }

    #[test]
    fn a_run_with_no_end_keeps_the_status_word_chip() {
        let t = theme();
        let running = end_chip(None, "running", &t);
        assert_eq!(running.label, "running");
        assert_eq!(running.role, EndRole::Fallback);
        assert_eq!(fgs(&running), vec![Some(t.status_running)]);
        assert_eq!(
            fgs(&end_chip(None, "weird", &t)),
            vec![Some(t.status_waiting)]
        );
    }

    #[test]
    fn three_runs_of_one_initiative_collapse_to_the_newest_with_a_count_of_three() {
        let runs = [
            run("dash-feed-1", None),
            run("dash-feed-3", None),
            run("dash-feed-2", None),
        ];
        let rows = collapse_runs(&runs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].run.run, "dash-feed-3");
        assert_eq!(rows[0].count, 3);
    }

    #[test]
    fn collapse_keeps_first_seen_order_and_leaves_the_input_alone() {
        let runs = [
            run("beta-4", None),
            run("alpha-1", None),
            run("beta-5", None),
            run("solo", None),
        ];
        let rows = collapse_runs(&runs);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.run.run.as_str(), r.count)).collect();
        assert_eq!(got, vec![("beta-5", 2), ("alpha-1", 1), ("solo", 1)]);
        let ids: Vec<&str> = runs.iter().map(|r| r.run.as_str()).collect();
        assert_eq!(ids, vec!["beta-4", "alpha-1", "beta-5", "solo"]);
    }

    #[test]
    fn the_count_suffix_is_dim_and_only_for_more_than_one() {
        let t = theme();
        assert!(count_suffix(1, &t).is_none());
        let suffix = count_suffix(3, &t).expect("suffix");
        assert_eq!(suffix.content, " \u{d7}3");
        assert_eq!(suffix.style.fg, Some(t.dim));
    }

    #[test]
    fn local_time_returns_unparseable_input_unchanged() {
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(local_time("not a time", utc), "not a time");
    }

    #[test]
    fn local_time_formats_in_the_given_offset() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(local_time("2026-09-29T18:00:00Z", et), "14:00");
    }

    #[test]
    fn local_time_follows_the_offset_passed_in() {
        let stamp = "2026-11-01T05:30:00Z";
        let at = |secs: i32| local_time(stamp, FixedOffset::east_opt(secs).unwrap());
        assert_eq!(at(3600), "06:30");
        assert_eq!(at(0), "05:30");
        assert_eq!(at(-5 * 3600), "00:30");
    }

    #[test]
    fn local_time_reads_a_naive_timestamp_as_utc() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(local_time("2026-09-29T18:00:00", et), "14:00");
        assert_eq!(
            local_time("2026-09-29T18:00:00", et),
            local_time("2026-09-29T18:00:00Z", et)
        );
        assert_eq!(local_time("2026-09-29T18:00:00.250", et), "14:00");
    }

    use crate::app::ThemeId;
    use crate::detail;
    use ratatui::{Terminal, backend::TestBackend};

    const FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");
    const RUN_DETAIL_FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_run_v1.json");

    /// An app on the Regatta page with the fixture's one run selected and its detail opened
    /// (so `app.detail()` is `Some((DetailKind::Run, "dash-feed-1", _))`).
    fn app_with_open_run_detail() -> App {
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app.open_detail();
        app
    }

    #[test]
    fn renders_a_loading_paragraph_when_the_detail_snapshot_has_not_arrived_yet() {
        let app = app_with_open_run_detail();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_the_run_drill_once_its_detail_snapshot_has_arrived() {
        let mut app = app_with_open_run_detail();
        let snapshot =
            detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(snapshot);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    const WATCH_FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_watch_v1.json");
    const SPEND_FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_spend_v1.json");
    const HEALTH_FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_health_v1.json");

    #[test]
    fn the_watch_view_draws_the_feeds_first_watch_id() {
        let snapshot = crate::feed::parse_snapshot(WATCH_FEED_FIXTURE.trim()).expect("feed parses");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app.open_watch();
        let text = screen(&draw(&app, &crate::theme::resolve(ThemeId::Regatta)));
        assert!(text.contains("coxswain-tools#41"), "{text}");
    }

    #[test]
    fn the_watch_view_opens_and_draws_before_any_feed_snapshot() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.open_watch();
        let text = screen(&draw(&app, &crate::theme::resolve(ThemeId::Regatta)));
        assert!(text.contains("Watch"), "{text}");
    }

    #[test]
    fn the_spend_view_draws_its_board_once_its_snapshot_has_arrived() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.open_spend();
        app.apply_detail_snapshot(detail::parse_detail(SPEND_FIXTURE.trim()).expect("spend"));
        let text = screen(&draw(&app, &crate::theme::resolve(ThemeId::Regatta)));
        assert!(text.contains("2026-10-03"), "{text}");
        assert!(!text.contains("loading"), "{text}");
    }

    #[test]
    fn the_health_view_draws_its_board_once_its_snapshot_has_arrived() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.open_health();
        app.apply_detail_snapshot(detail::parse_detail(HEALTH_FIXTURE.trim()).expect("health"));
        let text = screen(&draw(&app, &crate::theme::resolve(ThemeId::Regatta)));
        assert!(text.contains("chair-loop@omarchy"), "{text}");
        assert!(!text.contains("loading"), "{text}");
    }

    #[test]
    fn spend_and_health_say_loading_once_and_without_an_id_until_their_snapshot_arrives() {
        for (kind, word) in [("spend", "loading spend"), ("health", "loading health")] {
            let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
            match kind {
                "spend" => app.open_spend(),
                _ => app.open_health(),
            }
            let text = screen(&draw(&app, &crate::theme::resolve(ThemeId::Regatta)));
            assert!(text.contains(word), "{text}");
            assert!(!text.contains(&format!("{word} {kind}")), "{text}");
        }
    }

    #[test]
    fn footer_text_lists_each_entitys_bindings() {
        assert_eq!(
            footer_text(&Target::Run("r".into())),
            "p pause  k kill  m move"
        );
        assert_eq!(
            footer_text(&Target::Machine("m".into())),
            "+ lanes up  - lanes down  d drain  a activate"
        );
        assert_eq!(
            footer_text(&Target::Initiative("i".into())),
            "] priority up  [ priority down"
        );
    }

    #[test]
    fn fit_line_cuts_to_the_width_and_leaves_short_text_alone() {
        assert_eq!(fit_line("short", 10), "short");
        assert_eq!(fit_line("abcdefghij", 4), "abcd");
        assert_eq!(fit_line("abc", 0), "");
    }

    #[test]
    fn feed_error_text_prefixes_the_line() {
        assert_eq!(feed_error_text("boom"), "feed error: boom");
    }

    const TRACEBACK: &str = "printf 'Traceback (most recent call last):\\n  File \"x\"\\nValueError: boom\\n' >&2; exit 1";
    const ERROR_LINE: &str = "feed error: ValueError: boom";

    /// An app fed by a stub child run through `read_feed`, as `main` feeds the real one.
    fn app_from_stub(script: &str, page: AppPage) -> App {
        let value: serde_json::Value = serde_json::from_str(FEED_FIXTURE).expect("fixture is json");
        let line = serde_json::to_string(&value).expect("value serializes");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script]).env("LINE", line);
        let (tx, rx) = std::sync::mpsc::channel();
        crate::feed::read_feed(crate::feed::spawn_command(command), tx);
        rx.iter()
            .fold(App::new(page, ThemeId::Regatta), |mut app, message| {
                match message {
                    crate::feed::FeedMessage::Snapshot(snapshot) => app.apply_snapshot(*snapshot),
                    crate::feed::FeedMessage::Error(text) => app.apply_feed_error(text),
                    crate::feed::FeedMessage::ParseError(_) => {}
                }
                app
            })
    }

    fn draw(app: &App, theme: &Theme) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|f| render(f, app, theme))
            .expect("draw should not fail");
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        let area = buffer.area;
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    fn screen(buffer: &ratatui::buffer::Buffer) -> String {
        rows(buffer).join("\n")
    }

    /// The `(x, y)` of the first cell of `needle`. Every cell here holds one char, so a char
    /// index in a row is its column.
    fn find(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
        rows(buffer).iter().enumerate().find_map(|(y, row)| {
            row.find(needle)
                .map(|byte| (row[..byte].chars().count() as u16, y as u16))
        })
    }

    fn assert_drawn_in(buffer: &ratatui::buffer::Buffer, text: &str, color: ratatui::style::Color) {
        let (x, y) = find(buffer, text).unwrap_or_else(|| panic!("{text:?} is not on screen"));
        let len = text.chars().count() as u16;
        assert!((x..x + len).all(|col| buffer[(col, y)].fg == color));
    }

    #[test]
    fn an_error_after_a_good_snapshot_draws_the_page_and_one_error_line() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let app = app_from_stub(&script, AppPage::Regatta);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        assert!(text.contains("chair@omarchy:12345 @ omarchy"));
        assert_drawn_in(&buffer, ERROR_LINE, theme.status_failed);
        assert_eq!(text.matches("feed error").count(), 1);
        for traceback in ["Traceback", "most recent call", "File \"x\""] {
            assert!(
                !text.contains(traceback),
                "{traceback:?} leaked into a cell"
            );
        }
    }

    #[test]
    fn the_error_line_lands_inside_slipstreams_rail_chair_block() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let app = app_from_stub(&script, AppPage::Slipstream);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        assert_drawn_in(
            &buffer,
            ERROR_LINE,
            slipstream::resolved_palette(crate::app::ThemeId::Regatta).failed_chip,
        );
        let (x, _) = find(&buffer, ERROR_LINE).expect("error line is on screen");
        assert!(x >= 120 - slipstream::RAIL_WIDTH);
    }

    #[test]
    fn a_later_good_snapshot_removes_the_error_line() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let mut app = app_from_stub(&script, AppPage::Regatta);
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        app.apply_snapshot(snapshot);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        assert!(!screen(&draw(&app, &theme)).contains("feed error"));
    }

    #[test]
    fn an_error_before_any_snapshot_fills_the_chair_panel_and_never_says_waiting() {
        let app = app_from_stub(TRACEBACK, AppPage::Regatta);
        assert!(app.snapshot().is_none());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        let chair = regatta_frame_rects(Rect::new(0, 0, 120, 40), &app)[0].expect("chair rect");
        let (x, y) = find(&buffer, ERROR_LINE).expect("error line is on screen");
        assert!(x > chair.x && x < chair.x + chair.width);
        assert!(y > chair.y && y < chair.y + chair.height);
        assert_drawn_in(&buffer, ERROR_LINE, theme.status_failed);
        assert_eq!(text.matches(NO_FEED).count(), 5);
        assert!(!text.to_lowercase().contains("waiting"));
        assert!(!text.contains("Traceback"));
    }

    #[test]
    fn a_detail_error_replaces_the_detail_body_with_one_line() {
        let mut app = app_with_open_run_detail();
        let snapshot =
            detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(snapshot);
        app.apply_detail_error("ValueError: detail boom".to_owned());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        assert_drawn_in(
            &buffer,
            "feed error: ValueError: detail boom",
            theme.status_failed,
        );
        assert!(!text.contains("test result: ok"));
        assert!(!text.contains("src/feed.rs"));
    }

    #[test]
    fn a_detail_error_replaces_the_loading_paragraph() {
        let mut app = app_with_open_run_detail();
        app.apply_detail_error("ValueError: detail boom".to_owned());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let text = screen(&draw(&app, &theme));
        assert!(text.contains("feed error: ValueError: detail boom"));
        assert!(!text.contains("loading"));
    }

    use crate::app::Origin;
    use crate::exec::ExecResult;
    use crossterm::event::{KeyCode, KeyEvent};

    fn app_with_feed() -> App {
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app
    }

    fn stop_result(code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: ["cox", "runs", "stop", "dash-feed-1"]
                .map(str::to_owned)
                .to_vec(),
            code,
            output: output.to_owned(),
        }
    }

    /// The given row ranges of the 120x40 screen, so a snapshot holds the rows a modal or the
    /// status line can change. The rest of the page is held by the Regatta page's own snapshots.
    fn excerpt(app: &App, id: ThemeId, ranges: &[std::ops::Range<usize>]) -> String {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let all = rows(&draw(app, &theme));
        ranges
            .iter()
            .flat_map(|range| all[range.clone()].iter().cloned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.modal_key(KeyEvent::from(KeyCode::Char(c)));
        }
    }

    #[test]
    fn the_confirm_dialog_is_drawn_over_the_regatta_page() {
        let mut app = app_with_feed();
        assert!(app.begin_action('k'));
        let text = excerpt(&app, ThemeId::Regatta, &[15..23, 39..40]);
        assert!(text.contains("$ cox runs stop dash-feed-1"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn the_confirm_dialog_is_drawn_in_the_harbor_light_theme() {
        let mut app = app_with_feed();
        assert!(app.begin_action('k'));
        insta::assert_snapshot!(excerpt(&app, ThemeId::HarborLight, &[15..23, 39..40]));
    }

    fn form_type(app: &mut App, keys: &[KeyCode]) {
        for key in keys {
            app.form_key(KeyEvent::from(*key));
        }
    }

    #[test]
    fn the_form_is_drawn_with_its_confirm_over_it() {
        let mut app = app_with_feed();
        app.open_add_machine();
        let mut keys: Vec<KeyCode> = "edge-1".chars().map(KeyCode::Char).collect();
        keys.push(KeyCode::Enter);
        keys.extend("pat@edge-1".chars().map(KeyCode::Char));
        keys.extend([KeyCode::Enter, KeyCode::Backspace, KeyCode::Char('4')]);
        keys.extend([KeyCode::Enter, KeyCode::Enter]);
        form_type(&mut app, &keys);
        let text = excerpt(&app, ThemeId::Regatta, &[14..26, 39..40]);
        assert!(text.contains("$ cox host add edge-1 --ssh pat@edge-1 --capacity 4"));
        assert!(text.contains("Add machine"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn the_palette_prompt_is_drawn_over_the_regatta_page() {
        let mut app = app_with_feed();
        app.open_palette();
        type_text(&mut app, "route list");
        insta::assert_snapshot!(excerpt(&app, ThemeId::Regatta, &[37..39, 39..40]));
    }

    #[test]
    fn the_palette_frame_is_drawn_with_its_output() {
        let mut app = app_with_feed();
        app.open_palette();
        type_text(&mut app, "runs stop dash-feed-1");
        app.modal_key(KeyEvent::from(KeyCode::Enter));
        assert!(app.take_pending().is_some());
        app.apply_exec_result(
            stop_result(Some(0), "stopped dash-feed-1\n"),
            Origin::Palette,
        );
        let text = excerpt(&app, ThemeId::Regatta, &[0..5, 36..40]);
        assert!(text.contains("stopped dash-feed-1"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn an_ok_status_replaces_the_key_bar() {
        let mut app = app_with_feed();
        app.apply_exec_result(
            stop_result(Some(0), "stopped dash-feed-1\n"),
            Origin::Action,
        );
        insta::assert_snapshot!(excerpt(&app, ThemeId::Regatta, &[37..39, 39..40]));
    }

    #[test]
    fn a_failed_status_replaces_the_key_bar() {
        let mut app = app_with_feed();
        app.apply_exec_result(stop_result(Some(2), "no such run\n"), Origin::Action);
        insta::assert_snapshot!(excerpt(&app, ThemeId::Regatta, &[37..39, 39..40]));
    }

    #[test]
    fn a_status_takes_the_footer_row_and_leaves_the_frame_rects_alone() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let area = Rect::new(0, 0, 120, 40);
        let mut app = app_with_feed();
        let bare = rows(&draw(&app, &theme));
        assert!(bare[39].contains("1-9 frames"));
        let rects = regatta_frame_rects(area, &app);

        app.apply_exec_result(
            stop_result(Some(0), "stopped dash-feed-1\n"),
            Origin::Action,
        );
        let with_status = rows(&draw(&app, &theme));
        assert_eq!(regatta_frame_rects(area, &app), rects);
        assert!(with_status[39].starts_with("stopped dash-feed-1"));
        assert!(!with_status[39].contains("frames"));
        assert_eq!(with_status[..39], bare[..39]);
    }

    const SETTINGS_ROWS: &str = r#"{"sections":[{"id":"budgets","rows":[{"section":"budgets","scope":"budgets","key":"max_usd","value":"20","file":"f.toml","tracked":true,"pat_only":false}]}]}"#;

    fn settings_result(code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: Vec::new(),
            code,
            output: output.to_owned(),
        }
    }

    fn settings_keys(app: &mut App, keys: &[KeyCode]) {
        keys.iter()
            .for_each(|key| app.settings_key(KeyEvent::from(*key)));
    }

    /// The feed-loaded app with the settings screen open and `budgets max_usd` staged at 40.
    fn app_with_staged_settings() -> App {
        let mut app = app_with_feed();
        app.open_settings();
        app.take_pending();
        app.apply_exec_result(
            settings_result(Some(0), SETTINGS_ROWS),
            Origin::SettingsLoad,
        );
        settings_keys(
            &mut app,
            &[
                KeyCode::Tab,
                KeyCode::Enter,
                KeyCode::Backspace,
                KeyCode::Backspace,
                KeyCode::Char('4'),
                KeyCode::Char('0'),
                KeyCode::Enter,
            ],
        );
        app
    }

    fn settings_origin(apply: bool) -> Origin {
        let (scope, key) = ("budgets".to_owned(), "max_usd".to_owned());
        match apply {
            true => Origin::SettingsApply { scope, key },
            false => Origin::SettingsDryRun { scope, key },
        }
    }

    fn settings_text(app: &App) -> String {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        rows(&draw(app, &theme))
            .iter()
            .map(|row| row.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_settings_screen_replaces_the_page_over_a_loaded_snapshot() {
        let mut app = app_with_feed();
        app.open_settings();
        app.apply_exec_result(
            settings_result(Some(0), SETTINGS_ROWS),
            Origin::SettingsLoad,
        );
        let text = settings_text(&app);
        assert!(text.contains("budgets"));
        assert!(text.contains("max_usd"));
        assert!(!text.contains("1-9 frames"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn the_confirm_dialog_is_drawn_over_the_settings_screen() {
        let mut app = app_with_staged_settings();
        settings_keys(&mut app, &[KeyCode::Tab, KeyCode::Char('a')]);
        let text = settings_text(&app);
        assert!(text.contains("$ cox settings set budgets max_usd 40"));
        assert!(text.contains("max_usd"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn a_refusal_is_drawn_beside_its_staged_field() {
        let mut app = app_with_staged_settings();
        app.apply_exec_result(
            settings_result(Some(1), "max_usd must be below 30\n"),
            settings_origin(false),
        );
        let text = settings_text(&app);
        assert!(text.contains("max_usd must be below 30"));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn an_apply_status_takes_the_footer_row_under_the_settings_screen() {
        let mut app = app_with_staged_settings();
        app.apply_exec_result(settings_result(Some(1), "locked\n"), settings_origin(true));
        let with_status = settings_text(&app);
        let footer = with_status.lines().last().unwrap_or("");
        assert!(footer.contains("locked"));
        assert!(!footer.contains("Tab next pane"));
    }

    fn rect(x: u16, width: u16) -> Rect {
        Rect::new(x, 0, width, 10)
    }

    #[test]
    fn split_panel_gives_the_panel_its_share_of_the_width_on_the_right() {
        let area = Rect::new(0, 0, 100, 10);
        assert_eq!(split_panel(area, None), (area, None));
        assert_eq!(
            split_panel(area, Some(35)),
            (rect(0, 65), Some(rect(65, 35)))
        );
        assert_eq!(
            split_panel(area, Some(60)),
            (rect(0, 40), Some(rect(40, 60)))
        );
        assert_eq!(
            split_panel(area, Some(100)),
            (rect(0, 0), Some(rect(0, 100)))
        );
    }

    #[test]
    fn card_height_is_its_text_and_never_over_half_the_panel() {
        assert_eq!(card_height(2, 40), 12);
        assert_eq!(card_height(2, 16), 8);
    }

    /// A feed whose chair session is `s1`, with one open decision when `decided`.
    fn panel_snapshot(decided: bool) -> crate::feed::FeedSnapshot {
        let decisions = if decided {
            r#"[{"id":"d1","question":"Ship the preview?","options":["ship","hold"],"context":"Spend is under the stop.","asked_at":"2026-09-29T00:00:00Z"}]"#
        } else {
            "[]"
        };
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"chair@omarchy:1","host":"omarchy","epoch":1,"liveness":"live","beat_age_s":4,"session":"s1"}},"spend":{{"five_hour_fraction":0.1,"five_hour_source":"meter","weekly_fraction":0.2,"weekly_source":"meter","hard_stop_fraction":0.9,"five_hour_resets_at":"2026-09-29T02:00:00Z","weekly_resets_at":"2026-10-04T04:00:00Z"}},"machines":[{{"name":"omarchy","state":"active","lanes_in_use":1,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":12,"checkouts":{{}}}}],"runs":[{{"run":"alpha-1","machine":"omarchy","phase":"p1","node":"build","attempt":1,"turns":3,"cost":0.5,"verdict":"none","status":"running"}}],"queue":[],"inbox":[],"watch":[],"decisions":{decisions}}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    /// An app on `page` whose fake terminal has printed a line; the panel is open when `open`
    /// and the card shows when `decided`.
    fn panel_app(page: AppPage, theme: ThemeId, open: bool, decided: bool) -> App {
        let fake = crate::pty::FakePty::with_output(b"chair> waiting for you\r\n");
        let mut app = App::new(page, theme).with_pty(Box::new(fake));
        app.apply_snapshot(panel_snapshot(false));
        if decided {
            app.apply_snapshot(panel_snapshot(true));
        } else if open {
            app.toggle_chair_panel();
        }
        app.chair_panel_mut().poll();
        app
    }

    fn panel_text(app: &App, theme: ThemeId) -> String {
        let theme = crate::theme::resolve_for(theme, Some("truecolor"));
        screen(&draw(app, &theme))
    }

    const PANEL_THEMES: [(ThemeId, &str); 2] = [
        (ThemeId::Regatta, "regatta"),
        (ThemeId::HarborLight, "harbor_light"),
    ];

    #[test]
    fn renders_panel_closed() {
        for (id, name) in PANEL_THEMES {
            let app = panel_app(AppPage::Regatta, id, false, false);
            assert!(!app.chair_panel().is_open());
            let text = panel_text(&app, id);
            assert!(!text.contains("chair> waiting"));
            insta::assert_snapshot!(format!("renders_panel_closed_{name}"), text);
        }
    }

    #[test]
    fn renders_panel_open() {
        for (id, name) in PANEL_THEMES {
            let app = panel_app(AppPage::Regatta, id, true, false);
            let text = panel_text(&app, id);
            assert!(text.contains("chair> waiting for you"));
            assert!(!text.contains("decision"));
            insta::assert_snapshot!(format!("renders_panel_open_{name}"), text);
        }
    }

    #[test]
    fn renders_panel_card() {
        for (id, name) in PANEL_THEMES {
            let app = panel_app(AppPage::Regatta, id, true, true);
            assert!(app.card_visible());
            let buffer = draw(&app, &crate::theme::resolve_for(id, Some("truecolor")));
            let card = find(&buffer, "Ship the preview?").expect("card on screen");
            let terminal = find(&buffer, "chair> waiting for you").expect("terminal on screen");
            assert!(card.1 < terminal.1);
            insta::assert_snapshot!(format!("renders_panel_card_{name}"), screen(&buffer));
        }
    }

    #[test]
    fn an_open_panel_takes_the_right_of_either_page() {
        for page in [AppPage::Regatta, AppPage::Slipstream] {
            let closed = screen(&draw(
                &panel_app(page, ThemeId::Regatta, false, false),
                &crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor")),
            ));
            let open = panel_app(page, ThemeId::Regatta, true, false);
            let buffer = draw(
                &open,
                &crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor")),
            );
            let (x, _) = find(&buffer, "chair> waiting for you").expect("terminal on screen");
            assert!(x >= 120 * 65 / 100, "terminal starts at column {x}");
            assert!(closed.contains("omarchy"));
            assert!(screen(&buffer).contains("omarchy"));
        }
    }

    #[test]
    fn a_run_drill_draws_beside_the_shown_chair_panel() {
        let mut app = panel_app(AppPage::Regatta, ThemeId::Regatta, true, false);
        app.open_detail();
        let detail = detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(detail);
        assert!(app.detail().is_some());
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let rows = rows(&draw(&app, &theme));
        let title = chair_panel::title(app.chair_focused());
        let side = split_page(Rect::new(0, 0, 120, 40), &app)
            .1
            .expect("the panel is shown");
        let right: String = rows[0].chars().skip(side.x as usize).collect();
        let left: String = rows[0].chars().take(side.x as usize).collect();
        assert!(right.contains(title), "{right}");
        assert!(!left.contains(title), "{left}");
    }
}
