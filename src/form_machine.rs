//! The add machine form: `cox host add`, then `cox host doctor` on the new host.

use crate::form::{Field, Form, Plan};

pub fn form() -> Form {
    Form::new(
        "Add machine",
        vec![
            Field::text("name", ""),
            Field::text("ssh", ""),
            Field::text("capacity", "1"),
            Field::text("capabilities", ""),
        ],
        build,
    )
}

fn build(form: &Form) -> Result<Plan, String> {
    let name = form.value_of("name");
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err("name must be non-empty with no whitespace".into());
    }
    let ssh = form.value_of("ssh");
    if ssh.is_empty() {
        return Err("ssh must not be empty".into());
    }
    let capacity = match form.value_of("capacity").parse::<u32>() {
        Ok(n) if n >= 1 => n,
        _ => return Err("capacity must be a whole number of at least 1".into()),
    };
    let capabilities: Vec<&str> = form
        .value_of("capabilities")
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    let add = ["cox", "host", "add", name, "--ssh", ssh, "--capacity"]
        .into_iter()
        .map(str::to_string)
        .chain([capacity.to_string()])
        .chain(if capabilities.is_empty() {
            vec![]
        } else {
            vec!["--capabilities".to_string(), capabilities.join(",")]
        })
        .collect();
    let doctor = ["cox", "host", "doctor", name]
        .into_iter()
        .map(str::to_string)
        .collect();
    Ok(Plan {
        confirm_title: format!("Add machine {name}, then run cox host doctor {name}"),
        steps: vec![add, doctor],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::shell_join;

    fn filled(name: &str, ssh: &str, capacity: &str, capabilities: &str) -> Form {
        let mut form = form();
        for (field, value) in form
            .fields
            .iter_mut()
            .zip([name, ssh, capacity, capabilities])
        {
            field.value = value.to_string();
        }
        form
    }

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn the_fields_become_an_add_and_a_doctor_step() {
        let plan = filled("edge-1", "pat@edge-1", "4", "gpu, arm")
            .submit()
            .unwrap();
        assert_eq!(
            plan.steps,
            vec![
                argv(&[
                    "cox",
                    "host",
                    "add",
                    "edge-1",
                    "--ssh",
                    "pat@edge-1",
                    "--capacity",
                    "4",
                    "--capabilities",
                    "gpu,arm"
                ]),
                argv(&["cox", "host", "doctor", "edge-1"]),
            ]
        );
        assert_eq!(
            plan.confirm_title,
            "Add machine edge-1, then run cox host doctor edge-1"
        );
    }

    #[test]
    fn empty_capabilities_drop_the_flag() {
        let plan = filled("edge-1", "pat@edge-1", "2", " , ").submit().unwrap();
        assert_eq!(
            plan.steps[0],
            argv(&[
                "cox",
                "host",
                "add",
                "edge-1",
                "--ssh",
                "pat@edge-1",
                "--capacity",
                "2"
            ])
        );
    }

    #[test]
    fn an_ssh_target_with_a_space_and_a_quote_displays_quoted_and_runs_unquoted() {
        let plan = filled("e", "-o 'X' h", "1", "").submit().unwrap();
        assert_eq!(plan.steps[0][5], "-o 'X' h");
        assert_eq!(
            shell_join(&plan.steps[0]),
            "cox host add e --ssh '-o '\\''X'\\'' h' --capacity 1"
        );
    }

    #[test]
    fn an_invalid_capacity_is_an_error() {
        for bad in ["0", "abc", "-1", ""] {
            let mut form = filled("e", "h", bad, "");
            assert_eq!(form.submit(), None, "capacity {bad:?}");
            assert!(form.error.unwrap().contains("capacity"));
        }
    }

    #[test]
    fn an_empty_name_is_an_error() {
        let mut form = filled("  ", "h", "1", "");
        assert_eq!(form.submit(), None);
        assert!(form.error.unwrap().contains("name"));
    }

    #[test]
    fn a_name_with_whitespace_or_an_empty_ssh_is_an_error() {
        assert_eq!(filled("a b", "h", "1", "").submit(), None);
        assert_eq!(filled("a", "", "1", "").submit(), None);
    }
}
