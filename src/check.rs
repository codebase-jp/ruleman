//! The check engine: runs each rule against the working tree and collects
//! failures as diagnostics. How those reach the user is `output`'s job.

use crate::checksum::file_checksum;
use crate::config::{Config, load_config, resolve_config_path};
use crate::output::{Diagnostic, OutputFormat, render};
use crate::paths::{Target, resolve};
use crate::rule::{ContentFormat, FileState, MatchState, Rule, Severity};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Walks a dot-separated path into a parsed document. A segment of digits
/// indexes into an array, so `dependencies.0` and `a.b.1.c` resolve.
pub(crate) fn get_value_by_dotted_key<'a>(root: &'a Value, dotted_key: &str) -> Option<&'a Value> {
    dotted_key.split('.').try_fold(root, |current, segment| {
        match (current, segment.parse::<usize>()) {
            (Value::Array(items), Ok(index)) => items.get(index),
            _ => current.get(segment),
        }
    })
}

pub(crate) fn json_key_matches(root: &Value, key: &str, expected: &Value) -> bool {
    get_value_by_dotted_key(root, key).is_some_and(|actual| actual == expected)
}

/// Checks a directory's emptiness once it's already confirmed to exist and
/// be a directory. Returns `None` if `empty` is unset or the check passes.
pub(crate) fn check_directory_emptiness(
    path: &Path,
    display: &str,
    empty: Option<bool>,
) -> Option<String> {
    let want_empty = empty?;
    let is_empty = match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => {
            return Some(format!("cannot read directory '{}': {}", display, e));
        }
    };
    match (want_empty, is_empty) {
        (true, false) => Some(format!(
            "directory '{}' must be empty, but it is not",
            display
        )),
        (false, true) => Some(format!(
            "directory '{}' must not be empty, but it is",
            display
        )),
        _ => None,
    }
}

/// Applies a per-path check to whatever an entry resolved to.
///
/// A literal path is checked as written, existing or not. A pattern is checked
/// once per match — and when it matches nothing, `present` fails (it asserts
/// "there is at least one") while `absent` passes (nothing is there to forbid).
fn check_paths(
    target: Target,
    state: FileState,
    kind: &str,
    check: impl Fn(&Path, &str) -> Option<String>,
) -> Vec<(Option<String>, String)> {
    match target {
        Target::Literal(path) => check(Path::new(&path), &path)
            .map(|message| vec![(Some(path), message)])
            .unwrap_or_default(),
        Target::Matched { pattern, paths } if paths.is_empty() => match state {
            FileState::Present => vec![(
                None,
                format!("no {} matches the pattern '{}'", kind, pattern),
            )],
            FileState::Absent => Vec::new(),
        },
        Target::Matched { paths, .. } => paths
            .into_iter()
            .filter_map(|path| check(Path::new(&path), &path).map(|message| (Some(path), message)))
            .collect(),
    }
}

fn check_one_file(path: &Path, display: &str, state: FileState) -> Option<String> {
    match state {
        FileState::Absent if path.exists() => {
            Some(format!("file '{}' must not exist, but it does", display))
        }
        FileState::Present if !path.exists() => {
            Some(format!("required file '{}' is missing", display))
        }
        FileState::Present if !path.is_file() => Some(format!(
            "'{}' must be a regular file, but a directory exists there",
            display
        )),
        _ => None,
    }
}

fn check_one_directory(
    path: &Path,
    display: &str,
    state: FileState,
    empty: Option<bool>,
) -> Option<String> {
    match state {
        FileState::Absent if path.exists() => Some(format!(
            "directory '{}' must not exist, but it does",
            display
        )),
        FileState::Present if !path.exists() => {
            Some(format!("required directory '{}' is missing", display))
        }
        FileState::Present if !path.is_dir() => Some(format!(
            "'{}' must be a directory, but a file exists there",
            display
        )),
        FileState::Present => check_directory_emptiness(path, display, empty),
        FileState::Absent => None,
    }
}

pub(crate) fn check_config(config: Config) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for rule in config.rules {
        match rule {
            Rule::File {
                severity,
                state,
                files,
                for_each,
            } => {
                if severity == Severity::Off {
                    continue;
                }
                for entry in files {
                    let targets = match resolve(&entry, for_each.as_deref()) {
                        Ok(targets) => targets,
                        Err(message) => {
                            diagnostics.push(Diagnostic::new(severity, "file", None, message));
                            continue;
                        }
                    };
                    for target in targets {
                        for (path, message) in
                            check_paths(target, state, "file", |path, display| {
                                check_one_file(path, display, state)
                            })
                        {
                            diagnostics.push(Diagnostic::new(severity, "file", path, message));
                        }
                    }
                }
            }
            Rule::Directory {
                severity,
                state,
                directories,
                empty,
                for_each,
            } => {
                if severity == Severity::Off {
                    continue;
                }
                for entry in directories {
                    let targets = match resolve(&entry, for_each.as_deref()) {
                        Ok(targets) => targets,
                        Err(message) => {
                            diagnostics.push(Diagnostic::new(severity, "directory", None, message));
                            continue;
                        }
                    };
                    for target in targets {
                        for (path, message) in
                            check_paths(target, state, "directory", |path, display| {
                                check_one_directory(path, display, state, empty)
                            })
                        {
                            diagnostics.push(Diagnostic::new(severity, "directory", path, message));
                        }
                    }
                }
            }
            Rule::Content {
                severity,
                format,
                state,
                file,
                key,
                expected,
            } => {
                if severity == Severity::Off {
                    continue;
                }
                if let Some(message) = check_content(format, state, &file, &key, &expected) {
                    diagnostics.push(Diagnostic::new(severity, "content", Some(file), message));
                }
            }
            Rule::Checksum {
                severity,
                algorithm,
                state,
                file,
                expected,
            } => {
                if severity == Severity::Off {
                    continue;
                }

                let actual = match file_checksum(Path::new(&file), algorithm) {
                    Ok(actual) => actual,
                    Err(e) => {
                        let message = format!("cannot hash '{}': {}", file, e);
                        diagnostics.push(Diagnostic::new(
                            severity,
                            "checksum",
                            Some(file),
                            message,
                        ));
                        continue;
                    }
                };

                let expected = expected.trim();
                let matches = actual.eq_ignore_ascii_case(expected);
                let message = match state {
                    MatchState::Match if !matches => Some(format!(
                        "{} checksum of '{}' does not match the recorded digest \
                         (expected {}, actual {}). If the change was intentional, \
                         re-record it with 'ruleman add --checksum {}'",
                        algorithm.as_str(),
                        file,
                        expected,
                        actual,
                        file
                    )),
                    MatchState::Mismatch if matches => Some(format!(
                        "{} checksum of '{}' matches '{}', which it must not",
                        algorithm.as_str(),
                        file,
                        expected
                    )),
                    _ => None,
                };
                if let Some(message) = message {
                    diagnostics.push(Diagnostic::new(severity, "checksum", Some(file), message));
                }
            }
        }
    }

    diagnostics
}

/// Returns the failure message for a `content` rule, or `None` when it passes.
fn check_content(
    format: ContentFormat,
    state: MatchState,
    file: &str,
    key: &str,
    expected: &Value,
) -> Option<String> {
    let path = Path::new(file);
    if !path.exists() {
        return Some(format!(
            "cannot check '{}': file '{}' is missing",
            key, file
        ));
    }

    let raw = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => return Some(format!("cannot read '{}': {}", file, e)),
    };

    let parsed = match format {
        ContentFormat::Json => serde_json::from_str::<Value>(&raw),
    };
    let document = match parsed {
        Ok(value) => value,
        Err(e) => {
            return Some(format!(
                "cannot parse '{}' as {}: {}",
                file,
                format.as_str(),
                e
            ));
        }
    };

    let matches = json_key_matches(&document, key, expected);
    match state {
        MatchState::Match if !matches => {
            let actual = match get_value_by_dotted_key(&document, key) {
                Some(value) => value.to_string(),
                None => "not set".to_string(),
            };
            Some(format!(
                "'{}' in '{}' must be {}, but it is {}",
                key, file, expected, actual
            ))
        }
        MatchState::Mismatch if matches => Some(format!(
            "'{}' in '{}' must not be {}, but it is",
            key, file, expected
        )),
        _ => None,
    }
}

pub(crate) fn run(config_arg: Option<&str>, format: OutputFormat) -> i32 {
    let config_path = match resolve_config_path(config_arg) {
        Ok(path) => path,
        Err(message) => return render(format, &[Diagnostic::config(message)]),
    };

    let mut visited = HashSet::new();
    match load_config(&config_path, &mut visited) {
        Ok(config) => render(format, &check_config(config)),
        Err(message) => render(format, &[Diagnostic::config(message)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dotted_key_can_find_nested_value() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        let found = get_value_by_dotted_key(&value, "compilerOptions.strict");
        assert_eq!(found, Some(&json!(true)));
    }

    #[test]
    fn dotted_key_returns_none_when_missing() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        let found = get_value_by_dotted_key(&value, "compilerOptions.noImplicitAny");
        assert!(found.is_none());
    }

    #[test]
    fn dotted_key_indexes_into_arrays() {
        let value = json!({ "workspaces": ["packages/*", "apps/web"] });

        assert_eq!(
            get_value_by_dotted_key(&value, "workspaces.1"),
            Some(&json!("apps/web"))
        );
        assert!(get_value_by_dotted_key(&value, "workspaces.9").is_none());
        // A digit segment against an object still reads the literal key.
        let numeric_keys = json!({ "scripts": { "0": "noop" } });
        assert_eq!(
            get_value_by_dotted_key(&numeric_keys, "scripts.0"),
            Some(&json!("noop"))
        );
    }

    #[test]
    fn json_key_matches_requires_exact_value() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        assert!(json_key_matches(
            &value,
            "compilerOptions.strict",
            &json!(true)
        ));
        assert!(!json_key_matches(
            &value,
            "compilerOptions.strict",
            &json!(false)
        ));
    }

    #[test]
    fn directory_emptiness_check() {
        let dir = std::env::temp_dir().join("ruleman_test_dir_emptiness");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(check_directory_emptiness(&dir, "d", None), None);
        assert_eq!(check_directory_emptiness(&dir, "d", Some(true)), None);
        assert!(check_directory_emptiness(&dir, "d", Some(false)).is_some());

        fs::write(dir.join("file.txt"), "x").unwrap();
        assert_eq!(check_directory_emptiness(&dir, "d", Some(false)), None);
        assert!(check_directory_emptiness(&dir, "d", Some(true)).is_some());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_rule_rejects_directory_when_state_is_present() {
        let dir = std::env::temp_dir().join("ruleman_test_file_vs_dir");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("ruleman.json");
        fs::write(
            &config_path,
            r#"{ "rules": [ { "type": "file", "files": ["a-directory"] } ] }"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("a-directory")).unwrap();

        let mut visited = HashSet::new();
        let config = load_config(&config_path, &mut visited).unwrap();
        let diagnostics = check_config(config);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("must be a regular file"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn content_rule_reports_the_actual_value() {
        let dir = std::env::temp_dir().join("ruleman_test_content_message");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{ "engines": { "node": "18" } }"#,
        )
        .unwrap();
        let config_path = dir.join("ruleman.json");
        fs::write(
            &config_path,
            r#"{ "rules": [ { "type": "content", "file": "package.json",
                 "key": "engines.node", "expected": "20" } ] }"#,
        )
        .unwrap();

        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        let diagnostics = check_config(config);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains(r#"must be "20""#));
        assert!(diagnostics[0].message.contains(r#"but it is "18""#));

        // A key that isn't there at all says so rather than reporting a value.
        fs::write(
            &config_path,
            r#"{ "rules": [ { "type": "content", "file": "package.json",
                 "key": "engines.bun", "expected": "1" } ] }"#,
        )
        .unwrap();
        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        assert!(
            check_config(config)[0]
                .message
                .contains("but it is not set")
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn patterns_quantify_differently_from_for_each() {
        let dir = std::env::temp_dir().join("ruleman_test_patterns");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("packages/a")).unwrap();
        fs::create_dir_all(dir.join("packages/b")).unwrap();
        fs::write(dir.join("packages/a/README.md"), "").unwrap();
        fs::write(dir.join("stray.log"), "").unwrap();
        let config_path = dir.join("ruleman.json");

        let run = |rules: &str| {
            fs::write(&config_path, format!(r#"{{ "rules": [ {} ] }}"#, rules)).unwrap();
            check_config(load_config(&config_path, &mut HashSet::new()).unwrap())
        };

        // A pattern asserts "at least one": one README is enough.
        assert!(run(r#"{ "type": "file", "files": ["packages/*/README.md"] }"#).is_empty());

        // `for_each` asserts "for all": the package without one fails.
        let diagnostics =
            run(r#"{ "type": "file", "for_each": "packages/*", "files": ["README.md"] }"#);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("packages/b/README.md"));

        // A pattern matching nothing fails `present`...
        let diagnostics = run(r#"{ "type": "file", "files": ["docs/*.md"] }"#);
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0]
                .message
                .contains("no file matches the pattern")
        );
        assert_eq!(diagnostics[0].file, None);

        // ...and satisfies `absent`.
        assert!(run(r#"{ "type": "file", "state": "absent", "files": ["docs/*.md"] }"#).is_empty());

        // `absent` reports every match, not just the first.
        let diagnostics = run(r#"{ "type": "file", "state": "absent", "files": ["**/*.log"] }"#);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("stray.log"));

        // `for_each` over nothing is vacuously true.
        assert!(
            run(r#"{ "type": "file", "for_each": "apps/*", "files": ["package.json"] }"#)
                .is_empty()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_rule_passes_and_fails_against_the_real_file() {
        let dir = std::env::temp_dir().join("ruleman_test_checksum_rule");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "abc").unwrap();
        let config_path = dir.join("ruleman.json");

        // Uppercase on purpose: digests compare case-insensitively.
        fs::write(
            &config_path,
            r#"{ "rules": [ { "type": "checksum", "file": "a.txt",
                 "expected": "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD" } ] }"#,
        )
        .unwrap();
        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        assert!(check_config(config).is_empty());

        fs::write(dir.join("a.txt"), "abcd").unwrap();
        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        let diagnostics = check_config(config);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        // The rule path is resolved relative to the config file, which here
        // lives in a temp directory.
        assert!(diagnostics[0].file.as_deref().unwrap().ends_with("a.txt"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
