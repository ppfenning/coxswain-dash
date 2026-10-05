//! Will own staged edits and their `cox settings set` commands.

/// One pending change to a setting, with the last dry-run result attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedEdit {
    pub scope: String,
    pub key: String,
    pub original: String,
    pub value: String,
    pub diff: Option<String>,
    pub refusal: Option<String>,
}

impl StagedEdit {
    fn is_field(&self, scope: &str, key: &str) -> bool {
        self.scope == scope && self.key == key
    }
}

/// The ordered list of staged edits, at most one per scope and key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Staged {
    edits: Vec<StagedEdit>,
}

impl Staged {
    /// Adds or replaces the edit for the field. A value equal to the original removes it.
    pub fn stage(&mut self, scope: &str, key: &str, original: &str, value: &str) {
        if value == original {
            self.edits.retain(|e| !e.is_field(scope, key));
            return;
        }
        let edit = StagedEdit {
            scope: scope.to_string(),
            key: key.to_string(),
            original: original.to_string(),
            value: value.to_string(),
            diff: None,
            refusal: None,
        };
        match self.edits.iter().position(|e| e.is_field(scope, key)) {
            Some(i) => self.edits[i] = edit,
            None => self.edits.push(edit),
        }
    }

    /// Removes the edit at `index`; an index past the end changes nothing.
    pub fn unstage(&mut self, index: usize) {
        if index < self.edits.len() {
            self.edits.remove(index);
        }
    }

    pub fn edits(&self) -> &[StagedEdit] {
        &self.edits
    }

    /// Code 0 keeps the output as the diff; any other code keeps its last non-empty line as the refusal.
    pub fn record_dry_run(&mut self, scope: &str, key: &str, code: Option<i32>, output: &str) {
        let last_line = output
            .lines()
            .map(str::trim)
            .rev()
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string();
        self.edits
            .iter_mut()
            .filter(|e| e.is_field(scope, key))
            .for_each(|e| match code {
                Some(0) => {
                    e.diff = Some(output.to_string());
                    e.refusal = None;
                }
                _ => e.refusal = Some(last_line.clone()),
            });
    }

    pub fn record_applied(&mut self, scope: &str, key: &str) {
        self.edits.retain(|e| !e.is_field(scope, key));
    }

    pub fn refusal_for(&self, scope: &str, key: &str) -> Option<&str> {
        self.edits
            .iter()
            .find(|e| e.is_field(scope, key))
            .and_then(|e| e.refusal.as_deref())
    }

    pub fn diff_for(&self, scope: &str, key: &str) -> Option<&str> {
        self.edits
            .iter()
            .find(|e| e.is_field(scope, key))
            .and_then(|e| e.diff.as_deref())
    }
}

/// The `cox settings set` argv for an edit, with `--dry-run` last when asked.
pub fn argv(edit: &StagedEdit, dry_run: bool) -> Vec<String> {
    [
        "cox",
        "settings",
        "set",
        &edit.scope,
        &edit.key,
        &edit.value,
    ]
    .into_iter()
    .chain(dry_run.then_some("--dry-run"))
    .map(str::to_string)
    .collect()
}

/// Joins argv with spaces, single-quoting an argument that is empty or holds whitespace or a quote.
pub fn command_string(argv: &[String]) -> String {
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn quote(arg: &str) -> String {
    let needs_quotes = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || c == '\'' || c == '"');
    if needs_quotes {
        // An embedded single quote is closed, escaped and reopened, as a POSIX shell reads it.
        format!("'{}'", arg.replace('\'', "'\\''"))
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged_budget() -> Staged {
        let mut s = Staged::default();
        s.stage("budgets", "max_usd", "25", "40");
        s
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn a_staged_edit_becomes_the_exact_argv_and_command_string() {
        let s = staged_budget();
        let args = argv(&s.edits()[0], false);
        assert_eq!(
            args,
            strings(&["cox", "settings", "set", "budgets", "max_usd", "40"])
        );
        assert_eq!(command_string(&args), "cox settings set budgets max_usd 40");
    }

    #[test]
    fn the_dry_run_form_ends_with_dry_run() {
        let s = staged_budget();
        let args = argv(&s.edits()[0], true);
        assert_eq!(
            args,
            strings(&[
                "cox",
                "settings",
                "set",
                "budgets",
                "max_usd",
                "40",
                "--dry-run"
            ])
        );
    }

    #[test]
    fn a_value_with_a_space_is_quoted_in_command_string() {
        let args = strings(&["cox", "settings", "set", "chair", "name", "big boat"]);
        assert_eq!(
            command_string(&args),
            "cox settings set chair name 'big boat'"
        );
    }

    #[test]
    fn restaging_the_same_field_replaces_it_and_clears_the_old_refusal() {
        let mut s = staged_budget();
        s.record_dry_run(
            "budgets",
            "max_usd",
            Some(1),
            "checking\nmax_usd above cap\n",
        );
        assert_eq!(
            s.refusal_for("budgets", "max_usd"),
            Some("max_usd above cap")
        );
        s.stage("budgets", "max_usd", "25", "30");
        assert_eq!(s.edits().len(), 1);
        assert_eq!(s.edits()[0].value, "30");
        assert_eq!(s.refusal_for("budgets", "max_usd"), None);
    }

    #[test]
    fn staging_the_original_value_removes_the_edit() {
        let mut s = staged_budget();
        s.stage("budgets", "max_usd", "25", "25");
        assert_eq!(s.edits(), &[]);
    }

    #[test]
    fn a_dry_run_with_code_0_stores_a_diff() {
        let mut s = staged_budget();
        s.record_dry_run(
            "budgets",
            "max_usd",
            Some(0),
            "- max_usd = 25\n+ max_usd = 40\n",
        );
        assert_eq!(
            s.diff_for("budgets", "max_usd"),
            Some("- max_usd = 25\n+ max_usd = 40\n")
        );
        assert_eq!(s.refusal_for("budgets", "max_usd"), None);
    }

    #[test]
    fn a_dry_run_with_code_1_stores_a_refusal_and_the_edit_stays_staged() {
        let mut s = staged_budget();
        s.record_dry_run(
            "budgets",
            "max_usd",
            Some(1),
            "checking\nmax_usd above cap\n\n",
        );
        assert_eq!(
            s.refusal_for("budgets", "max_usd"),
            Some("max_usd above cap")
        );
        assert_eq!(s.edits().len(), 1);
    }

    #[test]
    fn record_applied_removes_the_edit() {
        let mut s = staged_budget();
        s.record_applied("budgets", "max_usd");
        assert_eq!(s.edits(), &[]);
    }
}
