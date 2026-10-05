//! Draws the open form: a centered bordered box with one row per field, the form's error line
//! and a key hint. Rows are clipped to the box and the box is clamped to the area.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};

use crate::form::{Field, FieldKind, Form};
use crate::theme::Theme;

/// Preferred box width in columns. It is wider than the confirm dialog so the form's edges
/// stay visible when the confirm is drawn over it.
const BOX_WIDTH: u16 = 64;
const HINT: &str = "Tab next  Enter confirm  Esc cancel";

/// Centers a `want_width` by `content_rows + 2` box in `area`, clamped to `area` on both axes.
fn box_rect(area: Rect, content_rows: u16, want_width: u16) -> Rect {
    let width = want_width.min(area.width);
    let height = content_rows.saturating_add(2).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// What follows a field's label: the typed value, a body's first line and character count, or
/// a choice's value and its options.
fn value_text(field: &Field) -> String {
    match &field.kind {
        FieldKind::Text => field.value.clone(),
        FieldKind::Choice(options) => format!("{}  [{}]", field.value, options.join("/")),
        FieldKind::Body => format!(
            "{}  ({} chars)",
            field.text.lines().next().unwrap_or(""),
            field.text.chars().count()
        ),
    }
}

/// Draws the form centered in `area`.
pub fn render(f: &mut Frame, area: Rect, theme: &Theme, form: &Form) {
    let style = Style::default().fg(theme.fg).bg(theme.panel);
    let dim = Style::default().fg(theme.dim).bg(theme.panel);
    let accent = Style::default().fg(theme.accent).bg(theme.panel);
    let label_width = form
        .fields
        .iter()
        .map(|field| field.label.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = form
        .fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let (marker, label) = if i == form.focus {
                ("> ", accent.add_modifier(Modifier::BOLD))
            } else {
                ("  ", dim)
            };
            Line::from(vec![
                Span::styled(marker, accent),
                Span::styled(format!("{:<label_width$}  ", field.label), label),
                Span::raw(value_text(field)),
            ])
        })
        .collect();
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            error.clone(),
            Style::default().fg(theme.status_failed).bg(theme.panel),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(HINT, dim));

    let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let rect = box_rect(area, rows, BOX_WIDTH);
    let block = Block::default()
        .title(Span::styled(
            form.title.clone(),
            accent.add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focus).bg(theme.panel))
        .padding(Padding::horizontal(1))
        .style(style);

    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block).style(style), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::form::Plan;
    use ratatui::{Terminal, backend::TestBackend};

    fn draw(form: &Form, width: u16, height: u16) -> String {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &theme, form))
            .expect("draw should not fail");
        terminal.backend().to_string()
    }

    fn never(_: &Form) -> Result<Plan, String> {
        Err("never".into())
    }

    #[test]
    fn the_add_machine_form_empty() {
        insta::assert_snapshot!(draw(&crate::form_machine::form(), 80, 24));
    }

    #[test]
    fn the_add_machine_form_filled_with_an_error_line() {
        let mut form = crate::form_machine::form();
        form.fields[0].value = "bad name".to_string();
        form.fields[1].value = "pat@edge-1".to_string();
        form.focus = 1;
        assert!(form.submit().is_none());
        assert!(form.error.is_some());
        insta::assert_snapshot!(draw(&form, 80, 24));
    }

    #[test]
    fn a_choice_field_and_a_body_field_each_render() {
        let mut form = Form::new(
            "Literal",
            vec![
                Field::choice("kind", &["alpha", "beta", "gamma"], "beta"),
                Field::body("notes", "first line\nsecond line"),
            ],
            never,
        );
        form.focus = 1;
        let screen = draw(&form, 80, 24);
        assert!(screen.contains("beta  [alpha/beta/gamma]"));
        assert!(screen.contains("first line  (22 chars)"));
        insta::assert_snapshot!(screen);
    }

    #[test]
    fn a_small_area_clips_the_box_instead_of_overflowing() {
        let screen = draw(&crate::form_machine::form(), 20, 5);
        assert_eq!(screen.lines().count(), 5);
        assert!(!screen.contains("cancel"));
    }
}
