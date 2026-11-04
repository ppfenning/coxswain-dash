use std::process::Command;

/// True when any line of `git tag --points-at HEAD` is `<version>` or `v<version>`.
fn on_release_tag(version: &str, tags_at_head: &str) -> bool {
    let prefixed = format!("v{version}");
    tags_at_head
        .lines()
        .map(str::trim)
        .any(|tag| tag == version || tag == prefixed)
}

/// Trimmed stdout of a git command, or None when git is missing, fails or prints nothing.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (out.status.success() && !text.is_empty()).then(|| text.to_string())
}

fn main() {
    // A worktree keeps HEAD in its own git dir and refs in the shared one, so ask git for the paths.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    // No commit means a source tarball or no git: export neither variable.
    let Some(commit) = git(&["rev-parse", "--short", "HEAD"]) else {
        return;
    };
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let tags = git(&["tag", "--points-at", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=COX_GIT_COMMIT={commit}");
    println!(
        "cargo:rustc-env=COX_HEAD_ON_RELEASE_TAG={}",
        u8::from(on_release_tag(&version, &tags))
    );
}
