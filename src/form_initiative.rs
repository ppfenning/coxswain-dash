//! The initiative forms: new, edit and remove. Each builds one `cox route` step.

use crate::form::{Field, Form, Plan};

const EDIT_PREFIX: &str = "Edit initiative ";
const REMOVE_PREFIX: &str = "Remove initiative ";

/// What an existing initiative holds, as plain strings.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefill {
    pub repo: String,
    pub title: String,
    pub body: String,
}

pub fn new_form(repos: Vec<String>) -> Form {
    let options: Vec<&str> = repos.iter().map(String::as_str).collect();
    let first = options.first().copied().unwrap_or("");
    Form::new(
        "New initiative",
        vec![
            Field::choice("repo", &options, first),
            Field::text("title", ""),
            Field::body("body", ""),
        ],
        build_new,
    )
}

/// `build` is a bare fn pointer, so the id rides in the form title.
pub fn edit_form(id: &str, prefill: Prefill) -> Form {
    Form::new(
        &format!("{EDIT_PREFIX}{id}"),
        vec![
            Field::choice("repo", &[prefill.repo.as_str()], &prefill.repo),
            Field::text("title", &prefill.title),
            Field::body("body", &prefill.body),
        ],
        build_edit,
    )
}

/// `build` is a bare fn pointer, so the id rides in the form title.
pub fn remove_form(id: &str) -> Form {
    Form::new(
        &format!("{REMOVE_PREFIX}{id}"),
        vec![Field::text("reason", "")],
        build_remove,
    )
}

fn id_of<'a>(form: &'a Form, prefix: &str) -> Result<&'a str, String> {
    form.title
        .strip_prefix(prefix)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "the form has no initiative id".to_string())
}

fn argv(tokens: Vec<&str>) -> Vec<String> {
    tokens.into_iter().map(str::to_string).collect()
}

fn field<'a>(form: &'a Form, label: &str) -> Option<&'a Field> {
    form.fields.iter().find(|f| f.label == label)
}

/// The body's path as a string, when the editor produced one.
fn body_path(form: &Form) -> Option<String> {
    field(form, "body")
        .and_then(|f| f.path.as_ref())
        .map(|p| p.display().to_string())
}

fn build_new(form: &Form) -> Result<Plan, String> {
    let repo = form.value_of("repo");
    if repo.is_empty() {
        return Err("repo must not be empty".into());
    }
    let title = form.value_of("title");
    if title.is_empty() {
        return Err("title must not be empty".into());
    }
    let has_text = field(form, "body").is_some_and(|f| !f.text.trim().is_empty());
    let path = match body_path(form) {
        Some(path) if has_text => path,
        _ => return Err("body must have text".into()),
    };
    let step = argv(vec![
        "cox", "route", "file", "--intake", "--repo", repo, "--title", title, "--body", &path,
    ]);
    Ok(Plan {
        confirm_title: format!("File initiative {title} in {repo}"),
        steps: vec![step],
    })
}

fn build_edit(form: &Form) -> Result<Plan, String> {
    let id = id_of(form, EDIT_PREFIX)?;
    let changed = |label: &str| field(form, label).is_some_and(Field::changed);
    let title = form.value_of("title");
    let repo = form.value_of("repo");
    if (changed("title") && title.is_empty()) || (changed("repo") && repo.is_empty()) {
        return Err("a changed field must not be empty".into());
    }
    let body_file = if changed("body") {
        Some(body_path(form).ok_or_else(|| "the edited body has no file".to_string())?)
    } else {
        None
    };
    let flags: Vec<&str> = [
        ("--title", changed("title").then_some(title)),
        ("--repo", changed("repo").then_some(repo)),
        ("--body-file", body_file.as_deref()),
    ]
    .into_iter()
    .filter_map(|(flag, value)| value.map(|v| [flag, v]))
    .flatten()
    .collect();
    if flags.is_empty() {
        return Err("nothing changed".into());
    }
    let step = argv(
        ["cox", "route", "edit", id]
            .into_iter()
            .chain(flags)
            .collect(),
    );
    Ok(Plan {
        confirm_title: format!("Edit initiative {id} ({title})"),
        steps: vec![step],
    })
}

fn build_remove(form: &Form) -> Result<Plan, String> {
    let id = id_of(form, REMOVE_PREFIX)?;
    let reason = form.value_of("reason");
    if reason.is_empty() {
        return Err("reason must not be blank".into());
    }
    Ok(Plan {
        confirm_title: format!("Remove initiative {id}"),
        steps: vec![argv(vec!["cox", "route", "remove", id, "--reason", reason])],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::form::{FieldKind, shell_join};
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    fn tokens(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    fn prefill() -> Prefill {
        Prefill {
            repo: "coxtop".into(),
            title: "Old title".into(),
            body: "old body".into(),
        }
    }

    fn filled_new(repo: &str, title: &str, path: Option<&str>, text: &str) -> Form {
        let mut form = new_form(vec!["coxtop".into(), "pat-skills".into()]);
        form.fields[0].value = repo.to_string();
        form.fields[1].value = title.to_string();
        form.set_body(2, path.map(PathBuf::from), text.to_string());
        form
    }

    fn draw(form: &Form) -> String {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|f| crate::ui::form_frame::render(f, f.area(), &theme, form))
            .expect("draw should not fail");
        terminal.backend().to_string()
    }

    #[test]
    fn the_new_form_has_a_repo_choice_a_title_and_a_body() {
        let form = new_form(vec!["coxtop".into(), "pat-skills".into()]);
        let shape: Vec<(&str, bool)> = form
            .fields
            .iter()
            .map(|f| (f.label.as_str(), matches!(f.kind, FieldKind::Choice(_))))
            .collect();
        assert_eq!(
            shape,
            vec![("repo", true), ("title", false), ("body", false)]
        );
        assert_eq!(form.fields[0].value, "coxtop");
        assert_eq!(form.fields[2].kind, FieldKind::Body);
    }

    #[test]
    fn a_new_initiative_becomes_a_route_file_command() {
        let mut form = filled_new(
            "coxtop",
            "Fix the \"queue\" bug",
            Some("/tmp/body.md"),
            "text",
        );
        let plan = form.submit().unwrap();
        assert_eq!(
            plan.steps,
            vec![tokens(&[
                "cox",
                "route",
                "file",
                "--intake",
                "--repo",
                "coxtop",
                "--title",
                "Fix the \"queue\" bug",
                "--body",
                "/tmp/body.md"
            ])]
        );
        assert_eq!(
            shell_join(&plan.steps[0]),
            "cox route file --intake --repo coxtop --title 'Fix the \"queue\" bug' --body /tmp/body.md"
        );
        assert!(plan.confirm_title.contains("Fix the \"queue\" bug"));
    }

    #[test]
    fn a_new_initiative_without_repo_title_or_body_is_an_error() {
        let bad = [
            filled_new("", "t", Some("/tmp/b.md"), "x"),
            filled_new("coxtop", " ", Some("/tmp/b.md"), "x"),
            filled_new("coxtop", "t", None, "x"),
            filled_new("coxtop", "t", Some("/tmp/b.md"), "  "),
        ];
        for mut form in bad {
            assert_eq!(form.submit(), None);
            assert!(form.error.is_some());
        }
    }

    #[test]
    fn an_edit_with_only_the_title_changed_gives_only_the_title_flag() {
        let mut form = edit_form("p2-x", prefill());
        form.fields[1].value = "New \"title\"".into();
        let plan = form.submit().unwrap();
        assert_eq!(
            plan.steps,
            vec![tokens(&[
                "cox",
                "route",
                "edit",
                "p2-x",
                "--title",
                "New \"title\""
            ])]
        );
        assert!(plan.confirm_title.contains("p2-x"));
    }

    #[test]
    fn an_edit_with_every_field_changed_orders_title_repo_body_file() {
        let mut form = edit_form("p2-x", prefill());
        form.fields[0].value = "pat-skills".into();
        form.fields[1].value = "T".into();
        form.set_body(2, Some(PathBuf::from("/tmp/e.md")), "new body".into());
        let plan = form.submit().unwrap();
        assert_eq!(
            plan.steps[0],
            tokens(&[
                "cox",
                "route",
                "edit",
                "p2-x",
                "--title",
                "T",
                "--repo",
                "pat-skills",
                "--body-file",
                "/tmp/e.md"
            ])
        );
    }

    #[test]
    fn an_edit_with_nothing_changed_is_an_error() {
        let mut form = edit_form("p2-x", prefill());
        assert_eq!(form.submit(), None);
        assert_eq!(form.error.as_deref(), Some("nothing changed"));
    }

    #[test]
    fn the_edit_form_starts_unchanged_and_carries_the_id_in_its_title() {
        let form = edit_form("p2-x", prefill());
        assert_eq!(form.title, "Edit initiative p2-x");
        assert!(form.fields.iter().all(|f| !f.changed()));
    }

    #[test]
    fn a_remove_reason_displays_quoted() {
        let mut form = remove_form("p2-x");
        form.fields[0].value = "done elsewhere".into();
        let plan = form.submit().unwrap();
        assert_eq!(
            plan.steps[0],
            tokens(&[
                "cox",
                "route",
                "remove",
                "p2-x",
                "--reason",
                "done elsewhere"
            ])
        );
        assert_eq!(
            shell_join(&plan.steps[0]),
            "cox route remove p2-x --reason 'done elsewhere'"
        );
        assert!(plan.confirm_title.contains("p2-x"));
    }

    #[test]
    fn a_blank_remove_reason_is_an_error() {
        let mut form = remove_form("p2-x");
        form.fields[0].value = "  ".into();
        assert_eq!(form.submit(), None);
        assert!(form.error.unwrap().contains("reason"));
    }

    #[test]
    fn the_new_initiative_form() {
        let form = new_form(vec!["coxtop".into(), "pat-skills".into()]);
        insta::with_settings!({snapshot_path => "ui/snapshots"}, {
            insta::assert_snapshot!(draw(&form));
        });
    }

    #[test]
    fn the_edit_initiative_form() {
        let form = edit_form("p2-x", prefill());
        insta::with_settings!({snapshot_path => "ui/snapshots"}, {
            insta::assert_snapshot!(draw(&form));
        });
    }

    #[test]
    fn the_remove_initiative_form() {
        let form = remove_form("p2-x");
        insta::with_settings!({snapshot_path => "ui/snapshots"}, {
            insta::assert_snapshot!(draw(&form));
        });
    }
}
