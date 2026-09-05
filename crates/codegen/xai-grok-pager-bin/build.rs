use std::path::Path;
use std::process::Command;

fn git_stdout(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=GROK_VERSION");

    // Watch the git files that change on commit/checkout so the version stamp refreshes
    // Never emit a missing path: cargo treats it as always dirty and rebuilds this crate every build
    let mut watch_paths = Vec::new();
    watch_paths.extend(git_stdout(&["rev-parse", "--git-path", "HEAD"]));
    watch_paths.extend(git_stdout(&["rev-parse", "--git-path", "logs/HEAD"]));
    if let Some(head_ref) = git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        watch_paths.extend(git_stdout(&["rev-parse", "--git-path", &head_ref]));
    }
    for path in watch_paths.iter().filter(|p| Path::new(p).exists()) {
        println!("cargo:rerun-if-changed={path}");
    }

    // Upstream stamp is `"<version> (<12-char-sha>)"`. This fork keeps that
    // (and any existing `+build` metadata) and appends `tbhaxor.<short-sha>`.
    // `1.0.16`            → `1.0.16+tbhaxor.abc1234 (deadbeef0123)`
    // `1.0.16+ci.1`       → `1.0.16+ci.1.tbhaxor.abc1234 (deadbeef0123)`
    let version = std::env::var("GROK_VERSION")
        .or_else(|_| std::env::var("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| "0.0.0".to_string());

    let commit = git_stdout(&["rev-parse", "HEAD"])
        .map(|s| s.chars().take(12).collect::<String>())
        .filter(|s| s.len() == 12)
        .unwrap_or_else(|| "unknown".to_string());

    let short_sha = git_stdout(&["rev-parse", "--short", "HEAD"])
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".to_string());

    let stamped = append_fork_build_meta(&version, "tbhaxor", &short_sha);
    println!("cargo:rustc-env=VERSION_WITH_COMMIT={stamped} ({commit})");
}

/// Append `fork_id.short_sha` to SemVer build metadata without replacing any
/// existing `+…` identifiers. A previous `tbhaxor[.sha]` pair is refreshed
/// in place so rebuilds do not stack `tbhaxor.old.tbhaxor.new`.
fn append_fork_build_meta(version: &str, fork_id: &str, short_sha: &str) -> String {
    let (core, build) = match version.split_once('+') {
        Some((core, build)) => (core, Some(build)),
        None => (version, None),
    };
    let mut parts: Vec<&str> = build
        .map(|b| b.split('.').filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    if let Some(i) = parts.iter().position(|p| *p == fork_id) {
        parts.truncate(i);
    }
    parts.push(fork_id);
    parts.push(short_sha);
    format!("{core}+{}", parts.join("."))
}
