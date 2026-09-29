//! coxtop, the Coxswain fleet dashboard. The view reads `cox dash --feed` and runs actions as `cox` commands; this
//! scaffold only names itself until those land (coxswain 0.25).

mod app;
mod config;
mod feed;

fn version_line() -> String {
    format!("coxtop {}", env!("CARGO_PKG_VERSION"))
}

fn main() {
    println!("{}", version_line());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_names_the_binary() {
        assert!(version_line().starts_with("coxtop "));
    }
}
