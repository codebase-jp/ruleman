//! Locating, parsing and resolving config files, including `extends` and the
//! rule paths that are relative to the file that declared them.

use crate::rule::{Rule, validate_rule};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const CONFIG_CANDIDATES: &[&str] = &["ruleman.json", "ruleman.jsonc", ".ruleman.json"];

/// The config file's own fields, with `rules` left unparsed so each rule can
/// be deserialized separately and reported with its index when it's malformed.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawScaffold {
    #[serde(default, rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    #[serde(default)]
    extends: Vec<String>,
    #[serde(default)]
    rules: Vec<Value>,
}

#[derive(Debug, Default)]
pub(crate) struct RawConfig {
    extends: Vec<String>,
    pub(crate) rules: Vec<Rule>,
}

pub(crate) struct Config {
    pub(crate) rules: Vec<Rule>,
}

pub(crate) fn parse_config_text(raw: &str) -> Result<RawConfig, String> {
    let value = jsonc_parser::parse_to_serde_value(raw, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?
        .unwrap_or(Value::Object(Default::default()));

    let scaffold: RawScaffold = serde_json::from_value(value).map_err(|e| e.to_string())?;

    let rules = scaffold
        .rules
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value::<Rule>(value)
                .map_err(|e| e.to_string())
                .and_then(|rule| validate_rule(&rule).map(|()| rule))
                .map_err(|e| format!("rules[{}]: {}", index, e))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RawConfig {
        extends: scaffold.extends,
        rules,
    })
}

pub(crate) fn load_raw_config(path: &Path) -> Result<RawConfig, String> {
    if !path.exists() {
        return Err(format!("config file '{}' not found", path.display()));
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| format!("cannot read config file '{}': {}", path.display(), e))?;

    parse_config_text(&raw)
        .map_err(|e| format!("cannot parse config file '{}': {}", path.display(), e))
}

/// Joins `file` onto `base_dir`, unless `base_dir` is empty (a config file
/// with no directory component, e.g. plain `ruleman.json` in the cwd), in
/// which case `file` is left untouched to avoid a cosmetic `./` prefix.
pub(crate) fn join_relative(base_dir: &Path, file: &str) -> String {
    if base_dir.as_os_str().is_empty() {
        file.to_string()
    } else {
        base_dir.join(file).to_string_lossy().into_owned()
    }
}

/// Rewrites a rule's file-path fields to be relative to the config file that
/// declared it, so checks behave the same regardless of the directory
/// `ruleman` is invoked from (matters once `extends` or upward config
/// discovery puts the config file somewhere other than the cwd).
pub(crate) fn resolve_rule_paths(rule: Rule, base_dir: &Path) -> Rule {
    match rule {
        Rule::File {
            severity,
            state,
            files,
        } => Rule::File {
            severity,
            state,
            files: files
                .into_iter()
                .map(|f| join_relative(base_dir, &f))
                .collect(),
        },
        Rule::Directory {
            severity,
            state,
            directories,
            empty,
        } => Rule::Directory {
            severity,
            state,
            directories: directories
                .into_iter()
                .map(|d| join_relative(base_dir, &d))
                .collect(),
            empty,
        },
        Rule::Content {
            severity,
            format,
            state,
            file,
            key,
            expected,
        } => Rule::Content {
            severity,
            format,
            state,
            file: join_relative(base_dir, &file),
            key,
            expected,
        },
        Rule::Checksum {
            severity,
            algorithm,
            state,
            file,
            expected,
        } => Rule::Checksum {
            severity,
            algorithm,
            state,
            file: join_relative(base_dir, &file),
            expected,
        },
    }
}

/// Resolves `extends` recursively (relative to each config file's own directory),
/// concatenating rules from extended configs first, followed by the file's own rules.
/// Every rule's file paths are resolved relative to the config file that declared them.
pub(crate) fn load_config(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<Config, String> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Err(format!(
            "circular 'extends' in config file '{}'",
            path.display()
        ));
    }

    let raw = load_raw_config(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut rules = Vec::new();
    for extend in &raw.extends {
        let extended_path = base_dir.join(extend);
        let extended = load_config(&extended_path, visited)?;
        rules.extend(extended.rules);
    }
    rules.extend(
        raw.rules
            .into_iter()
            .map(|rule| resolve_rule_paths(rule, base_dir)),
    );

    Ok(Config { rules })
}

/// Searches for a config file in the current directory, then walking up parent
/// directories. Kept relative (`ruleman.json`, `../ruleman.json`, ...) rather
/// than resolved to an absolute path, so rule file paths resolved relative to
/// it (see `join_relative`) stay short in the common case where the config
/// file lives in the cwd.
pub(crate) fn discover_config() -> Option<PathBuf> {
    let mut dir = PathBuf::new();
    loop {
        for candidate in CONFIG_CANDIDATES {
            let path = dir.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        let probe = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir.as_path()
        };
        fs::canonicalize(probe).ok()?.parent()?;
        dir.push("..");
    }
}

pub(crate) fn resolve_config_path(config_arg: Option<&str>) -> Result<PathBuf, String> {
    match config_arg {
        Some(path) => Ok(PathBuf::from(path)),
        None => discover_config()
            .ok_or_else(|| "no config file found; create one with 'ruleman init'".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::ChecksumAlgorithm;
    use crate::rule::{ContentFormat, FileState, MatchState, Severity};
    use crate::testdata::DIGEST_ABC;

    #[test]
    fn parses_jsonc_with_comments_and_trailing_commas() {
        let text = r#"{
            // a comment
            "rules": [
                { "type": "file", "files": ["README.md"], },
            ],
        }"#;
        let config = parse_config_text(text).unwrap();
        assert_eq!(config.rules.len(), 1);
    }

    #[test]
    fn severity_and_state_default() {
        let text = r#"{ "rules": [ { "type": "file", "files": [] } ] }"#;
        let config = parse_config_text(text).unwrap();
        match &config.rules[0] {
            Rule::File {
                severity, state, ..
            } => {
                assert_eq!(*severity, Severity::Error);
                assert_eq!(*state, FileState::Present);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn directory_state_and_empty_default() {
        let text = r#"{ "rules": [ { "type": "directory", "directories": [] } ] }"#;
        let config = parse_config_text(text).unwrap();
        match &config.rules[0] {
            Rule::Directory { state, empty, .. } => {
                assert_eq!(*state, FileState::Present);
                assert_eq!(*empty, None);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn content_format_and_state_default() {
        let text = r#"{
            "rules": [
                { "type": "content", "file": "x.json", "key": "a", "expected": true }
            ]
        }"#;
        let config = parse_config_text(text).unwrap();
        match &config.rules[0] {
            Rule::Content { format, state, .. } => {
                assert_eq!(*format, ContentFormat::Json);
                assert_eq!(*state, MatchState::Match);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn unknown_attributes_are_rejected() {
        // A typo'd attribute is silently ignored by default, which would let a
        // rule pass while checking less than the author intended.
        let error = parse_config_text(
            r#"{ "rules": [ { "type": "file", "files": ["a"], "stat": "absent" } ] }"#,
        )
        .unwrap_err();
        assert!(error.contains("rules[0]"), "{}", error);
        assert!(error.contains("stat"), "{}", error);

        // Attributes belonging to another rule type don't leak in either.
        assert!(
            parse_config_text(r#"{ "rules": [ { "type": "file", "files": ["a"], "key": "x" } ] }"#)
                .is_err()
        );

        // ...nor unknown top-level fields.
        assert!(parse_config_text(r#"{ "rulez": [] }"#).is_err());

        // The `type` tag itself is not treated as an unknown field.
        assert!(parse_config_text(r#"{ "rules": [ { "type": "file", "files": [] } ] }"#).is_ok());
    }

    #[test]
    fn a_malformed_rule_is_reported_with_its_index() {
        let error = parse_config_text(
            r#"{
                "rules": [
                    { "type": "file", "files": ["a"] },
                    { "type": "directory", "directories": ["b"] },
                    { "type": "content", "file": "c.json", "expected": 1 }
                ]
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("rules[2]"), "{}", error);
        assert!(error.contains("key"), "{}", error);
    }

    #[test]
    fn checksum_expected_must_be_a_well_formed_digest() {
        let cases = [
            "",
            "abc123",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ];
        for expected in cases {
            let text = format!(
                r#"{{ "rules": [ {{ "type": "checksum", "file": "a", "expected": "{}" }} ] }}"#,
                expected
            );
            let error = parse_config_text(&text)
                .err()
                .unwrap_or_else(|| panic!("expected '{}' to be rejected", expected));
            assert!(error.contains("rules[0]"), "{}", error);
        }

        // Surrounding whitespace and uppercase hex are both fine.
        let text = format!(
            r#"{{ "rules": [ {{ "type": "checksum", "file": "a", "expected": "  {}  " }} ] }}"#,
            DIGEST_ABC.to_uppercase()
        );
        assert!(parse_config_text(&text).is_ok());
    }

    #[test]
    fn checksum_rule_defaults() {
        let text = format!(
            r#"{{ "rules": [ {{ "type": "checksum", "file": "a.txt", "expected": "{}" }} ] }}"#,
            DIGEST_ABC
        );
        let config = parse_config_text(&text).unwrap();
        match &config.rules[0] {
            Rule::Checksum {
                algorithm, state, ..
            } => {
                assert_eq!(*algorithm, ChecksumAlgorithm::Sha256);
                assert_eq!(*state, MatchState::Match);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn join_relative_with_empty_base_dir_is_unchanged() {
        assert_eq!(join_relative(Path::new(""), "README.md"), "README.md");
    }

    #[test]
    fn join_relative_joins_with_nonempty_base_dir() {
        let base = Path::new("some").join("proj");
        let expected = base.join("README.md").to_string_lossy().into_owned();
        assert_eq!(join_relative(&base, "README.md"), expected);
    }

    #[test]
    fn file_rule_paths_resolve_relative_to_config_file_location() {
        let dir = std::env::temp_dir().join("ruleman_test_relative_paths");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("ruleman.json");
        fs::write(
            &config_path,
            r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
        )
        .unwrap();

        let mut visited = HashSet::new();
        let config = load_config(&config_path, &mut visited).unwrap();
        match &config.rules[0] {
            Rule::File { files, .. } => {
                assert_eq!(files[0], dir.join("README.md").to_string_lossy());
            }
            _ => panic!("unexpected rule"),
        }

        fs::remove_dir_all(&dir).unwrap();
    }
}
