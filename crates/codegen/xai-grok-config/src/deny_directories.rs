//! Startup deny list from `[cli] deny_directories` in the user `config.toml`.
//!
//! ```toml
//! [cli]
//! deny_directories = [
//!   "$HOME/secret-project",
//!   "$HOME/secret-project/*",
//!   "$HOME/secret-project/**",
//! ]
//! ```
//!
//! `$VAR` and `${VAR}` expand when the process starts. A variable that is not set expands to an
//! empty string. `$$` is a literal `$`. After expansion each entry must be an absolute path.
//! A plain path matches only that directory. `*` matches one path component, so `path/*` matches
//! only the directories directly inside `path`. `**` matches one or more components, so `path/**`
//! matches every directory inside `path` at any depth and does not match `path` itself.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use crate::loader::{USER_CONFIG_FILENAME, toml_error_detail};

/// `[cli]` key. Paths are absolute after `$VAR` / `${VAR}` expansion.
pub const DENY_DIRECTORIES_KEY: &str = "deny_directories";

/// Why startup was refused while applying [`DENY_DIRECTORIES_KEY`].
#[derive(Debug, thiserror::Error)]
pub enum DenyDirectoriesError {
    /// The user config exists but could not be read, so the deny list could not be applied.
    #[error("grok: cannot apply deny_directories from {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The user config is not valid TOML, so the deny list could not be applied.
    #[error("grok: cannot apply deny_directories from {path}: {detail}")]
    Parse { path: PathBuf, detail: String },
    /// `[cli] deny_directories` is present but is not an array of strings.
    #[error("grok: cannot apply deny_directories from {path}: {detail}")]
    Shape { path: PathBuf, detail: String },
    /// An entry is not absolute after environment expansion.
    #[error(
        "grok: [cli] deny_directories entry in {path} must be an absolute path\n  \
         entry: {entry:?}\n  \
         expanded: {expanded:?}\n  \
         Missing environment variables expand to an empty string."
    )]
    NotAbsolute {
        path: PathBuf,
        entry: String,
        expanded: String,
    },
    /// The checked directory matched one `[cli] deny_directories` pattern.
    #[error(
        "grok: refusing to start in {cwd}\n  \
         The current directory is denied in {config_path} under [cli] deny_directories\n  \
         entry: {entry:?}\n  \
         expanded: {expanded:?}\n  \
         path: {denied}"
    )]
    Denied {
        cwd: PathBuf,
        config_path: PathBuf,
        entry: String,
        expanded: String,
        denied: PathBuf,
    },
    /// The process working directory could not be read, so the deny list could not be applied.
    #[error(
        "grok: cannot apply deny_directories because the current directory is unavailable: {source}"
    )]
    CurrentDir {
        #[source]
        source: std::io::Error,
    },
}

/// Refuse startup when `directory` matches `[cli] deny_directories` in the user `config.toml`
/// (`$GROK_HOME/config.toml`, or `<home>/.grok/config.toml`).
///
/// The pager calls this for the launch directory and again for `--cwd`, so a denied directory
/// cannot be selected from somewhere else.
///
/// Missing config, or a config with no `deny_directories` key, allows startup.
/// An unreadable or unparseable config refuses startup: the deny list cannot be skipped
/// because the file could not be read. A list entry that is not absolute after expansion
/// also refuses startup, including when `directory` is outside the other entries.
///
/// # Errors
///
/// [`DenyDirectoriesError`] when the config cannot be applied or `directory` is denied.
pub fn enforce_startup_deny_directories_for(directory: &Path) -> Result<(), DenyDirectoriesError> {
    let Some(home) = xai_dirs::resolve_grok_home() else {
        return Ok(());
    };
    let config_path = home.join(USER_CONFIG_FILENAME);
    evaluate_deny_directories(&config_path, directory, process_env_lookup)
}

/// Refuse startup when the process working directory is covered by `deny_directories`.
///
/// # Errors
///
/// [`DenyDirectoriesError`] when the config cannot be applied, the working directory cannot be
/// read, or that directory is denied.
pub fn enforce_startup_deny_directories() -> Result<(), DenyDirectoriesError> {
    let cwd =
        std::env::current_dir().map_err(|source| DenyDirectoriesError::CurrentDir { source })?;
    enforce_startup_deny_directories_for(&cwd)
}

fn process_env_lookup(name: &str) -> Option<String> {
    std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
}

/// Expand `$VAR` / `${VAR}` in one deny-list entry.
///
/// A missing variable becomes an empty string. Shellexpand keeps the `$VAR` text when the lookup
/// returns `None`, so this lookup always returns `Some`.
fn expand_deny_path(input: &str, mut lookup: impl FnMut(&str) -> Option<String>) -> String {
    shellexpand::env_with_context_no_errors(input, |name: &str| -> Option<String> {
        Some(lookup(name).unwrap_or_default())
    })
    .into_owned()
}

fn evaluate_deny_directories(
    config_path: &Path,
    cwd: &Path,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Result<(), DenyDirectoriesError> {
    let entries = read_deny_entries(config_path)?;
    let mut prepared = Vec::with_capacity(entries.len());
    for entry in entries {
        let trimmed = entry.trim();
        let expanded = expand_deny_path(trimmed, &mut lookup);
        let pattern = split_pattern(Path::new(&expanded));
        if trimmed.is_empty() || pattern.invalid || !pattern.base.is_absolute() {
            return Err(DenyDirectoriesError::NotAbsolute {
                path: config_path.to_path_buf(),
                entry: trimmed.to_string(),
                expanded,
            });
        }
        let base = resolve_for_compare(&pattern.base);
        if !base.is_absolute() {
            return Err(DenyDirectoriesError::NotAbsolute {
                path: config_path.to_path_buf(),
                entry: trimmed.to_string(),
                expanded,
            });
        }
        prepared.push((trimmed.to_string(), expanded, base, pattern.globs));
    }
    let cwd_resolved = resolve_for_compare(cwd);
    for (entry, expanded, base, globs) in prepared {
        if pattern_covers(&base, &globs, &cwd_resolved) {
            return Err(DenyDirectoriesError::Denied {
                cwd: cwd.to_path_buf(),
                config_path: config_path.to_path_buf(),
                entry,
                expanded,
                denied: base,
            });
        }
    }
    Ok(())
}

fn read_deny_entries(config_path: &Path) -> Result<Vec<String>, DenyDirectoriesError> {
    let source = match std::fs::read_to_string(config_path) {
        Ok(source) => source,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(DenyDirectoriesError::Read {
                path: config_path.to_path_buf(),
                source,
            });
        }
    };
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: toml::Value =
        toml::from_str(&source).map_err(|error| DenyDirectoriesError::Parse {
            path: config_path.to_path_buf(),
            detail: toml_error_detail(&source, &error),
        })?;
    entries_from_value(config_path, &value)
}

fn entries_from_value(
    config_path: &Path,
    value: &toml::Value,
) -> Result<Vec<String>, DenyDirectoriesError> {
    let Some(table) = value.as_table() else {
        return Err(DenyDirectoriesError::Shape {
            path: config_path.to_path_buf(),
            detail: format!(
                "`[cli]` must be a table to read `{DENY_DIRECTORIES_KEY}`, found {}",
                value_kind(value)
            ),
        });
    };
    let Some(cli_value) = table.get("cli") else {
        return Ok(Vec::new());
    };
    let Some(cli) = cli_value.as_table() else {
        return Err(DenyDirectoriesError::Shape {
            path: config_path.to_path_buf(),
            detail: format!(
                "`[cli]` must be a table to read `{DENY_DIRECTORIES_KEY}`, found {}",
                value_kind(cli_value)
            ),
        });
    };
    let Some(raw) = cli.get(DENY_DIRECTORIES_KEY) else {
        return Ok(Vec::new());
    };
    let Some(items) = raw.as_array() else {
        return Err(DenyDirectoriesError::Shape {
            path: config_path.to_path_buf(),
            detail: format!(
                "`[cli] {DENY_DIRECTORIES_KEY}` must be an array of strings, found {}",
                value_kind(raw)
            ),
        });
    };
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(entry) = item.as_str() else {
            let position = index.saturating_add(1);
            return Err(DenyDirectoriesError::Shape {
                path: config_path.to_path_buf(),
                detail: format!(
                    "`[cli] {DENY_DIRECTORIES_KEY}` entry {position} must be a string, found {}",
                    value_kind(item)
                ),
            });
        };
        entries.push(entry.to_string());
    }
    Ok(entries)
}

fn value_kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "an integer",
        toml::Value::Float(_) => "a float",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a datetime",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

struct CompiledPattern {
    base: PathBuf,
    globs: Vec<OsString>,
    invalid: bool,
}

fn is_glob_component(name: &OsStr) -> bool {
    name == OsStr::new("*") || name == OsStr::new("**")
}

/// Split an expanded entry into the literal directory and glob components.
/// `*` and `**` are whole path components. Anything after the first glob is a glob token.
fn split_pattern(expanded: &Path) -> CompiledPattern {
    let mut base = PathBuf::new();
    let mut globs = Vec::new();
    let mut seen_glob = false;
    let mut invalid = false;
    for component in expanded.components() {
        if seen_glob {
            match component {
                Component::Normal(name) => globs.push(name.to_os_string()),
                _ => invalid = true,
            }
            continue;
        }
        match component {
            Component::Normal(name) if is_glob_component(name) => {
                seen_glob = true;
                globs.push(name.to_os_string());
            }
            other => base.push(other.as_os_str()),
        }
    }
    CompiledPattern {
        base,
        globs,
        invalid,
    }
}

/// `**` consumes one or more components. `*` consumes exactly one. Other tokens are literals.
fn components_match(pattern: &[OsString], path: &[OsString]) -> bool {
    let Some((head, pattern_rest)) = pattern.split_first() else {
        return path.is_empty();
    };
    if head.as_os_str() == OsStr::new("**") {
        let mut consumed = 1usize;
        while consumed <= path.len() {
            if let Some(suffix) = path.get(consumed..)
                && components_match(pattern_rest, suffix)
            {
                return true;
            }
            consumed = consumed.saturating_add(1);
        }
        false
    } else if head.as_os_str() == OsStr::new("*") {
        let Some((_, path_rest)) = path.split_first() else {
            return false;
        };
        components_match(pattern_rest, path_rest)
    } else {
        let Some((segment, path_rest)) = path.split_first() else {
            return false;
        };
        segment == head && components_match(pattern_rest, path_rest)
    }
}

fn pattern_covers(base: &Path, globs: &[OsString], cwd_resolved: &Path) -> bool {
    if globs.is_empty() {
        return cwd_resolved == base;
    }
    let Ok(rest) = cwd_resolved.strip_prefix(base) else {
        return false;
    };
    if rest.as_os_str().is_empty() {
        return false;
    }
    let mut parts = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_os_string()),
            Component::CurDir => {}
            _ => return false,
        }
    }
    components_match(globs, &parts)
}

/// Resolve `.` and `..` without touching the filesystem.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match components.last() {
                Some(Component::Normal(_)) => {
                    components.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => components.push(component),
            },
            _ => components.push(component),
        }
    }
    if components.is_empty() {
        PathBuf::from(".")
    } else {
        components.into_iter().collect()
    }
}

/// Canonicalize `path`, following a symlink in the longest existing ancestor when the full path
/// does not exist yet. A missing suffix stays lexical on top of that canonical ancestor.
fn resolve_for_compare(path: &Path) -> PathBuf {
    let lexical = normalize_lexically(path);
    if let Ok(canonical) = dunce::canonicalize(&lexical) {
        return canonical;
    }
    let mut suffix = Vec::new();
    let mut cursor = lexical;
    while let Some(name) = cursor.file_name() {
        let name = name.to_os_string();
        let Some(parent) = cursor.parent() else {
            break;
        };
        if parent.as_os_str().is_empty() || parent == cursor {
            break;
        }
        let parent = parent.to_path_buf();
        suffix.push(name);
        if let Ok(canonical) = dunce::canonicalize(&parent) {
            let mut resolved = canonical;
            for part in suffix.iter().rev() {
                resolved.push(part);
            }
            return normalize_lexically(&resolved);
        }
        cursor = parent;
    }
    normalize_lexically(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, source: &str) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(&path, source).unwrap();
        path
    }

    fn deny_config(entries: &[&str]) -> String {
        let value = toml::Value::Array(
            entries
                .iter()
                .copied()
                .map(str::to_string)
                .map(toml::Value::String)
                .collect(),
        );
        let mut cli = toml::map::Map::new();
        cli.insert(DENY_DIRECTORIES_KEY.to_string(), value);
        let mut table = toml::map::Map::new();
        table.insert("cli".to_string(), toml::Value::Table(cli));
        toml::to_string(&toml::Value::Table(table)).unwrap()
    }

    fn eval(
        config_path: &Path,
        cwd: &Path,
        vars: &[(&str, &str)],
    ) -> Result<(), DenyDirectoriesError> {
        evaluate_deny_directories(config_path, cwd, |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        })
    }

    fn denied_path(error: DenyDirectoriesError) -> PathBuf {
        match error {
            DenyDirectoriesError::Denied { denied, .. } => denied,
            other => panic!("expected a deny hit, got {other}"),
        }
    }

    #[test]
    fn expands_braced_and_unbraced_variables() {
        let expanded = expand_deny_path("$HOME/${USER}/box", |name| match name {
            "HOME" => Some("/home/ada".to_string()),
            "USER" => Some("ada".to_string()),
            _ => None,
        });
        assert_eq!(expanded, "/home/ada/ada/box");
    }

    #[test]
    fn missing_variable_expands_to_an_empty_string() {
        assert_eq!(expand_deny_path("${MISSING}", |_| None), "");
        assert_eq!(expand_deny_path("$MISSING/child", |_| None), "/child");
        assert_eq!(expand_deny_path("rel/$MISSING", |_| None), "rel/");
    }

    #[test]
    fn missing_process_environment_variable_expands_to_an_empty_string() {
        let name = format!("GROK_DENY_DIR_UNSET_{}", std::process::id());
        assert!(std::env::var_os(&name).is_none(), "{name} is set");
        assert_eq!(
            expand_deny_path(&format!("${{{name}}}"), process_env_lookup),
            ""
        );
        assert_eq!(
            expand_deny_path(&format!("${name}/child"), process_env_lookup),
            "/child"
        );
    }

    #[test]
    fn doubled_dollar_is_a_literal_dollar() {
        assert_eq!(expand_deny_path("$$HOME", |_| None), "$HOME");
    }

    #[test]
    fn missing_config_allows_startup() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        eval(&config_path, dir.path(), &[]).unwrap();
    }

    #[test]
    fn empty_config_and_other_keys_allow_startup() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir(&cwd).unwrap();
        let empty = write_config(dir.path(), "\n");
        eval(&empty, &cwd, &[]).unwrap();
        let other = write_config(dir.path(), "[cli]\nauto_update = false\n");
        eval(&other, &cwd, &[]).unwrap();
        let empty_list = write_config(dir.path(), &deny_config(&[]));
        eval(&empty_list, &cwd, &[]).unwrap();
    }

    #[test]
    fn plain_path_matches_only_that_directory() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        let nested = denied.join("nested");
        let sibling = dir.path().join("denied-extra");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let config_path = write_config(dir.path(), &deny_config(&[&denied.display().to_string()]));

        let exact = eval(&config_path, &denied, &[]).unwrap_err();
        let message = exact.to_string();
        assert!(message.contains("denied"), "{message}");
        assert!(message.contains("[cli] deny_directories"), "{message}");
        assert!(
            message.contains(&config_path.display().to_string()),
            "{message}"
        );
        assert_eq!(denied_path(exact), resolve_for_compare(&denied));

        eval(&config_path, &nested, &[]).unwrap();
        eval(&config_path, &sibling, &[]).unwrap();
        eval(&config_path, dir.path(), &[]).unwrap();
    }

    #[test]
    fn star_matches_one_level_and_globstar_matches_every_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("secret-project");
        let area = base.join("area51");
        let sub = area.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let base_text = base.display().to_string();

        let one_level = write_config(dir.path(), &deny_config(&[&format!("{base_text}/*")]));
        eval(&one_level, &base, &[]).unwrap();
        let child = eval(&one_level, &area, &[]).unwrap_err();
        assert_eq!(denied_path(child), resolve_for_compare(&base));
        eval(&one_level, &sub, &[]).unwrap();

        let every_level = write_config(dir.path(), &deny_config(&[&format!("{base_text}/**")]));
        eval(&every_level, &base, &[]).unwrap();
        assert!(eval(&every_level, &area, &[]).is_err());
        assert!(eval(&every_level, &sub, &[]).is_err());
    }

    #[test]
    fn home_star_pattern_expands_before_matching() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let area = home.join("secret-project").join("area51");
        let sub = area.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let config_path = write_config(dir.path(), &deny_config(&["$HOME/secret-project/*"]));
        let home_text = home.display().to_string();
        let vars = [("HOME", home_text.as_str())];
        eval(&config_path, &home.join("secret-project"), &vars).unwrap();
        assert!(eval(&config_path, &area, &vars).is_err());
        eval(&config_path, &sub, &vars).unwrap();
    }

    #[test]
    fn top_level_deny_directories_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        std::fs::create_dir(&denied).unwrap();
        let config_path = write_config(
            dir.path(),
            &format!("deny_directories = [\"{}\"]\n", denied.display()),
        );
        eval(&config_path, &denied, &[]).unwrap();
    }

    #[test]
    fn home_and_user_variables_match_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let secret = home.join("ada").join("secret");
        std::fs::create_dir_all(&secret).unwrap();
        let config_path = write_config(dir.path(), &deny_config(&["$HOME/${USER}/secret"]));
        let error = eval(
            &config_path,
            &secret,
            &[("HOME", &home.display().to_string()), ("USER", "ada")],
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("$HOME/${USER}/secret"), "{message}");
        assert_eq!(denied_path(error), resolve_for_compare(&secret));
        eval(
            &config_path,
            &home,
            &[("HOME", &home.display().to_string()), ("USER", "ada")],
        )
        .unwrap();
    }

    #[test]
    fn unset_variable_can_yield_an_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), &deny_config(&["${GROK_DENY_UNSET}/secret"]));
        let error = eval(&config_path, Path::new("/secret"), &[]).unwrap_err();
        assert_eq!(
            denied_path(error),
            resolve_for_compare(Path::new("/secret"))
        );
    }

    #[test]
    fn relative_and_empty_entries_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir(&cwd).unwrap();

        let relative = write_config(dir.path(), &deny_config(&["relative/$USER"]));
        match eval(&relative, &cwd, &[("USER", "ada")]) {
            Err(DenyDirectoriesError::NotAbsolute {
                entry, expanded, ..
            }) => {
                assert_eq!(entry, "relative/$USER");
                assert_eq!(expanded, "relative/ada");
            }
            other => panic!("expected a non-absolute entry, got {other:?}"),
        }

        let empty = write_config(dir.path(), &deny_config(&["${MISSING}"]));
        match eval(&empty, &cwd, &[]) {
            Err(DenyDirectoriesError::NotAbsolute { expanded, .. }) => assert_eq!(expanded, ""),
            other => panic!("expected an empty expansion to be rejected, got {other:?}"),
        }

        let blank = write_config(dir.path(), &deny_config(&["   "]));
        match eval(&blank, &cwd, &[]) {
            Err(DenyDirectoriesError::NotAbsolute { .. }) => {}
            other => panic!("expected a blank entry to be rejected, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_entry_is_reported_before_a_matching_one() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        std::fs::create_dir(&denied).unwrap();
        let config_path = write_config(
            dir.path(),
            &deny_config(&["relative", &denied.display().to_string()]),
        );
        match eval(&config_path, &denied, &[]) {
            Err(DenyDirectoriesError::NotAbsolute { entry, .. }) => assert_eq!(entry, "relative"),
            other => panic!("expected the relative entry to win, got {other:?}"),
        }
    }

    #[test]
    fn dot_segments_and_surrounding_space_still_match() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        std::fs::create_dir(&denied).unwrap();
        let via_dots = denied.join("..").join("denied");
        let spaced = format!("  {}  ", via_dots.display());
        let config_path = write_config(dir.path(), &deny_config(&[&spaced]));
        let error = eval(&config_path, &denied, &[]).unwrap_err();
        assert_eq!(denied_path(error), resolve_for_compare(&denied));
    }

    #[test]
    fn wrong_shape_names_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let as_string = write_config(dir.path(), "[cli]\ndeny_directories = \"/tmp\"\n");
        match eval(&as_string, cwd, &[]) {
            Err(DenyDirectoriesError::Shape { path, detail }) => {
                assert_eq!(path, as_string);
                assert!(detail.contains("array of strings"), "{detail}");
                assert!(detail.contains("a string"), "{detail}");
            }
            other => panic!("expected a shape error, got {other:?}"),
        }
        let as_int = write_config(dir.path(), "[cli]\ndeny_directories = [1]\n");
        match eval(&as_int, cwd, &[]) {
            Err(DenyDirectoriesError::Shape { detail, .. }) => {
                assert!(detail.contains("entry 1"), "{detail}");
                assert!(detail.contains("an integer"), "{detail}");
            }
            other => panic!("expected an entry shape error, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_and_unreadable_config_refuse_startup() {
        let dir = tempfile::tempdir().unwrap();
        let broken = write_config(dir.path(), "deny_directories = [\n");
        match eval(&broken, dir.path(), &[]) {
            Err(DenyDirectoriesError::Parse { path, detail }) => {
                assert_eq!(path, broken);
                assert!(detail.contains("TOML parse error"), "{detail}");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
        let blocked = dir.path().join("config.toml");
        std::fs::remove_file(&blocked).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        match eval(&blocked, dir.path(), &[]) {
            Err(DenyDirectoriesError::Read { path, .. }) => assert_eq!(path, blocked),
            other => panic!("expected a read error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_ancestors_match_the_canonical_directory() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let nested = real.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let link = dir.path().join("link");
        symlink(&real, &link).unwrap();

        let through_link = write_config(dir.path(), &deny_config(&[&link.display().to_string()]));
        let error = eval(&through_link, &real, &[]).unwrap_err();
        assert_eq!(denied_path(error), resolve_for_compare(&real));
        eval(&through_link, &nested, &[]).unwrap();

        let one_level = write_config(
            dir.path(),
            &deny_config(&[&format!("{}/*", link.display())]),
        );
        assert!(eval(&one_level, &nested, &[]).is_err());
        let deeper = nested.join("subdir");
        std::fs::create_dir(&deeper).unwrap();
        eval(&one_level, &deeper, &[]).unwrap();

        let every_level = write_config(
            dir.path(),
            &deny_config(&[&format!("{}/**", link.display())]),
        );
        assert!(eval(&every_level, &nested, &[]).is_err());
        assert!(eval(&every_level, &deeper, &[]).is_err());
        eval(&every_level, &real, &[]).unwrap();

        let other = dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        eval(&through_link, &other, &[]).unwrap();

        let missing = real.join("missing");
        let configured = link.join("missing");
        let not_yet = write_config(
            dir.path(),
            &deny_config(&[&configured.display().to_string()]),
        );
        let error = eval(&not_yet, &missing, &[]).unwrap_err();
        assert_eq!(denied_path(error), resolve_for_compare(&missing));
    }

    #[test]
    fn lexical_parent_does_not_escape_root() {
        assert_eq!(
            normalize_lexically(Path::new("/foo/../../etc")),
            PathBuf::from("/etc")
        );
    }

    #[test]
    fn resolve_keeps_a_missing_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-child");
        assert_eq!(
            resolve_for_compare(&missing),
            resolve_for_compare(dir.path()).join("no-such-child")
        );
    }
}
