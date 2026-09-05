//! Installed grok CLI version, kept in sync with the shipping binaries.

use std::sync::OnceLock;

use semver::Version;

pub const TEST_VERSION_ENV: &str = "GROK_TEST_VERSION";

pub const VERSION: &str = match option_env!("GROK_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// The release pipeline always injects `GROK_VERSION`; without it the build is from source.
pub const IS_DEV_BUILD: bool = option_env!("GROK_VERSION").is_none();

/// Runtime-injected user-facing version string.
/// Official builds stamp `"<version> (<shortcommit>)"`. This fork keeps that
/// (and any existing `+build` metadata) and appends `tbhaxor-<short-sha>`,
/// e.g. `"1.0.16+tbhaxor-abc1234 (deadbeef0123)"`. Only the binary injects it
/// at startup so lib crates don't recompile on every commit.
static FULL_VERSION: OnceLock<&'static str> = OnceLock::new();

/// Inject the binary's stamped version string.
/// Idempotent: the first set wins, repeats are ignored.
pub fn set_full_version(v: &'static str) {
    let _ = FULL_VERSION.set(v);
}

/// The injected version string, or plain [`VERSION`] when no binary has called [`set_full_version`] (e.g. lib tests, dev harnesses).
pub fn full_version() -> &'static str {
    FULL_VERSION.get().copied().unwrap_or(VERSION)
}

/// Returns the [`TEST_VERSION_ENV`] override when set, otherwise [`VERSION`].
/// The env value is trimmed so non-semver-aware callers can pass the result straight into parsing.
pub fn installed() -> String {
    std::env::var(TEST_VERSION_ENV)
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| VERSION.to_string())
}

pub fn installed_semver() -> Result<Version, semver::Error> {
    Version::parse(&installed())
}

/// Formats the compiled version with a channel label for user-facing display, e.g. `"0.2.5 [stable]"`.
/// `channel_label` is pre-formatted by `xai_grok_update::channel_label()`: `" [alpha]"`, `" [stable]"`, or `""` when no pointer is cached.
pub fn display_version(channel_label: &str) -> String {
    format!("{}{}", VERSION, channel_label)
}

/// Like [`display_version`], but for the binary-stamped string
/// (`"0.2.5 (abc1234)"` or `"1.0.16+tbhaxor-abc1234 (deadbeef0123)"`).
pub fn display_version_with_commit(version_with_commit: &str, channel_label: &str) -> String {
    format!("{}{}", version_with_commit, channel_label)
}

/// Append `fork_id-short_sha` as one SemVer build identifier (hyphen, not
/// `.`, so it stays a single token). Existing `+…` identifiers are kept.
/// A previous `fork_id-<sha>` token — or the legacy `fork_id.sha` pair —
/// is refreshed in place so rebuilds do not stack.
///
/// The pager-bin `build.rs` stamp must stay in lockstep with this.
pub fn append_fork_build_meta(version: &str, fork_id: &str, short_sha: &str) -> String {
    let token = format!("{fork_id}-{short_sha}");
    let prefix = format!("{fork_id}-");
    let (core, build) = match version.split_once('+') {
        Some((core, build)) => (core, Some(build)),
        None => (version, None),
    };
    let mut parts: Vec<String> = build
        .map(|b| {
            b.split('.')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Some(i) = parts
        .iter()
        .position(|p| p == fork_id || p.starts_with(&prefix))
    {
        // Legacy `tbhaxor.<sha>` used two identifiers; drop both.
        let end = if parts[i] == fork_id && i + 1 < parts.len() {
            i + 1
        } else {
            i
        };
        parts.drain(i..=end);
    }
    parts.push(token);
    format!("{core}+{}", parts.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks that the channel label is appended for alpha, stable, and empty labels.
    #[test]
    fn test_display_version_formatting_matrix() {
        let cases: &[(&str, &str, &str)] = &[
            // (version_with_commit,    label,        expected_suffix)
            ("0.2.5 (abc1234)", " [alpha]", "0.2.5 (abc1234) [alpha]"),
            ("0.2.5 (abc1234)", " [stable]", "0.2.5 (abc1234) [stable]"),
            ("0.2.5 (abc1234)", "", "0.2.5 (abc1234)"),
            (
                "0.1.220-alpha.2 (def0)",
                " [alpha]",
                "0.1.220-alpha.2 (def0) [alpha]",
            ),
            (
                "1.0.16+tbhaxor-abc1234 (deadbeef0123)",
                " [stable]",
                "1.0.16+tbhaxor-abc1234 (deadbeef0123) [stable]",
            ),
            (
                "1.0.16+ci.1.tbhaxor-abc1234 (deadbeef0123)",
                "",
                "1.0.16+ci.1.tbhaxor-abc1234 (deadbeef0123)",
            ),
        ];
        for (vwc, label, expected) in cases {
            assert_eq!(
                display_version_with_commit(vwc, label),
                *expected,
                "display_version_with_commit({:?}, {:?})",
                vwc,
                label,
            );
        }
        // display_version uses compiled VERSION, so verify only that the label appends
        assert_eq!(display_version(""), VERSION);
        assert!(display_version(" [stable]").ends_with("[stable]"));
    }

    #[test]
    fn append_fork_build_meta_adds_or_extends_without_replacing() {
        assert_eq!(
            append_fork_build_meta("1.0.16", "tbhaxor", "abc1234"),
            "1.0.16+tbhaxor-abc1234"
        );
        assert_eq!(
            append_fork_build_meta("1.0.16+ci.1", "tbhaxor", "abc1234"),
            "1.0.16+ci.1.tbhaxor-abc1234"
        );
        assert_eq!(
            append_fork_build_meta("1.0.16+ci.1.tbhaxor-oldsha", "tbhaxor", "abc1234"),
            "1.0.16+ci.1.tbhaxor-abc1234"
        );
        assert_eq!(
            append_fork_build_meta("1.0.16+tbhaxor.oldsha", "tbhaxor", "abc1234"),
            "1.0.16+tbhaxor-abc1234"
        );
    }

    #[test]
    fn full_version_falls_back_then_first_set_wins() {
        assert_eq!(full_version(), VERSION);
        set_full_version("first (aaaaaaa)");
        assert_eq!(full_version(), "first (aaaaaaa)");
        set_full_version("second (bbbbbbb)");
        assert_eq!(full_version(), "first (aaaaaaa)");
    }
}
