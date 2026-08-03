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

#[derive(Debug)]
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
            comparison,
            file,
            key,
            expected,
        } => Rule::Content {
            severity,
            format,
            state,
            comparison,
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

/// Resolves `extends` recursively, concatenating rules from extended configs
/// first, followed by the file's own rules.
pub(crate) fn load_config(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<Config, String> {
    load_config_from(path, visited, None)
}

/// `inherited_base` is where this config's rule paths resolve to, when that
/// isn't the config's own directory.
///
/// A config in the repo checks paths relative to itself, so `extends`-ing a
/// sibling file behaves the same wherever `ruleman` runs. A config from an
/// installed package can't: its own directory is inside `node_modules`, and a
/// shared rule saying `files: ["LICENSE"]` means the *consuming* repo's
/// LICENSE, not the package's. So a package-resolved config inherits the base
/// of the config that extended it, and passes that on down its own `extends`
/// chain.
fn load_config_from(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    inherited_base: Option<&Path>,
) -> Result<Config, String> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Err(format!(
            "circular 'extends' in config file '{}'",
            path.display()
        ));
    }

    let raw = load_raw_config(path)?;
    let own_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let rule_base = inherited_base.unwrap_or(own_dir);

    let mut rules = Vec::new();
    for extend in &raw.extends {
        let (extended_path, extended_base) = if is_path_target(extend) {
            (own_dir.join(extend), inherited_base)
        } else {
            (resolve_package_extends(extend, own_dir)?, Some(rule_base))
        };
        let extended = load_config_from(&extended_path, visited, extended_base)?;
        rules.extend(extended.rules);
    }
    rules.extend(
        raw.rules
            .into_iter()
            .map(|rule| resolve_rule_paths(rule, rule_base)),
    );

    Ok(Config { rules })
}

/// Locates an `extends` target that names a package rather than a path: looked
/// up in `node_modules` walking up from the config file, the same shape as
/// eslint's and tsconfig's shareable configs. Resolution is offline — the
/// package has to be installed, so what a run checks against is pinned by the
/// lockfile rather than fetched over the network at check time.
fn resolve_package_extends(target: &str, base_dir: &Path) -> Result<PathBuf, String> {
    let (package, subpath) = split_package_target(target);

    // Canonicalized so the walk reaches the filesystem root even when the
    // config file was found at a relative path like `../ruleman.json`.
    let start = if base_dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base_dir
    };
    let mut dir = fs::canonicalize(start).map_err(|e| {
        format!(
            "cannot resolve '{}' from '{}': {}",
            target,
            start.display(),
            e
        )
    })?;

    loop {
        let package_dir = dir.join("node_modules").join(&package);
        if package_dir.is_dir() {
            return match subpath {
                Some(subpath) => Ok(package_dir.join(subpath)),
                None => package_config(&package_dir).ok_or_else(|| {
                    format!(
                        "package '{}' has no ruleman config; name the file explicitly, \
                         e.g. '{}/ruleman.json'",
                        package, package
                    )
                }),
            };
        }
        if !dir.pop() {
            break;
        }
    }

    Err(format!(
        "cannot resolve 'extends' target '{}': install it, or use a relative path",
        target
    ))
}

/// `./x`, `../x`, `/x` and (on Windows) `C:\x` are paths; a bare `x` or `@a/b`
/// is a package name.
fn is_path_target(target: &str) -> bool {
    target.starts_with('.')
        || target.starts_with('/')
        || Path::new(target).is_absolute()
        || target.starts_with('\\')
}

/// Splits `@scope/pkg/path/to.json` into the package name and the rest.
fn split_package_target(target: &str) -> (String, Option<String>) {
    let segments: Vec<&str> = target.split('/').collect();
    let name_len = if target.starts_with('@') { 2 } else { 1 };
    if segments.len() <= name_len {
        return (target.to_string(), None);
    }
    (
        segments[..name_len].join("/"),
        Some(segments[name_len..].join("/")),
    )
}

/// The config file a package exposes, by the same candidate names used for
/// discovery.
fn package_config(package_dir: &Path) -> Option<PathBuf> {
    CONFIG_CANDIDATES
        .iter()
        .map(|candidate| package_dir.join(candidate))
        .find(|path| path.is_file())
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
    fn extends_targets_are_paths_or_package_names() {
        assert!(is_path_target("./base.json"));
        assert!(is_path_target("../shared/base.json"));
        assert!(is_path_target("/etc/ruleman.json"));
        assert!(!is_path_target("ruleman-config-acme"));
        assert!(!is_path_target("@acme/ruleman-config"));

        assert_eq!(
            split_package_target("@acme/ruleman-config"),
            ("@acme/ruleman-config".to_string(), None)
        );
        assert_eq!(
            split_package_target("@acme/ruleman-config/strict.json"),
            (
                "@acme/ruleman-config".to_string(),
                Some("strict.json".to_string())
            )
        );
        assert_eq!(
            split_package_target("acme-config"),
            ("acme-config".to_string(), None)
        );
        assert_eq!(
            split_package_target("acme-config/nested/strict.json"),
            (
                "acme-config".to_string(),
                Some("nested/strict.json".to_string())
            )
        );
    }

    #[test]
    fn extends_resolves_a_package_from_node_modules() {
        let dir = std::env::temp_dir().join("ruleman_test_extends_pkg");
        let _ = fs::remove_dir_all(&dir);
        // The package lives above the config file, as it would in a monorepo
        // where only the repo root has node_modules.
        let package = dir.join("node_modules/@acme/ruleman-config");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(dir.join("apps/web")).unwrap();
        fs::write(
            package.join("ruleman.json"),
            r#"{ "rules": [ { "type": "file", "files": ["LICENSE"] } ] }"#,
        )
        .unwrap();
        fs::write(
            package.join("strict.jsonc"),
            r#"{ "rules": [ { "type": "file", "files": ["CODEOWNERS"] } ] }"#,
        )
        .unwrap();

        let config_path = dir.join("apps/web/ruleman.json");
        fs::write(
            &config_path,
            r#"{ "extends": ["@acme/ruleman-config"],
                 "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
        )
        .unwrap();

        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        assert_eq!(config.rules.len(), 2);
        // A shared rule checks the consuming repo, not the package it came
        // from: `files: ["LICENSE"]` must not point inside node_modules.
        match &config.rules[0] {
            Rule::File { files, .. } => {
                assert!(!files[0].contains("node_modules"), "{:?}", files);
                assert!(files[0].ends_with("web/LICENSE"), "{:?}", files);
            }
            _ => panic!("unexpected rule"),
        }

        // A subpath names a specific file inside the package.
        fs::write(
            &config_path,
            r#"{ "extends": ["@acme/ruleman-config/strict.jsonc"], "rules": [] }"#,
        )
        .unwrap();
        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        assert_eq!(config.rules.len(), 1);

        // A package that isn't installed says so instead of reporting a
        // confusing missing-file error for a path nobody wrote.
        fs::write(
            &config_path,
            r#"{ "extends": ["@acme/not-installed"], "rules": [] }"#,
        )
        .unwrap();
        let error = load_config(&config_path, &mut HashSet::new()).unwrap_err();
        assert!(
            error.contains("cannot resolve 'extends' target"),
            "{}",
            error
        );

        fs::remove_dir_all(&dir).unwrap();
    }

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
