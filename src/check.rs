//! The check engine: runs each rule against the working tree and reports
//! failures in GitHub Actions workflow-command format.

use crate::checksum::file_checksum;
use crate::config::{Config, load_config, resolve_config_path};
use crate::rule::{ContentFormat, FileState, MatchState, Rule, Severity};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub(crate) fn get_value_by_dotted_key<'a>(root: &'a Value, dotted_key: &str) -> Option<&'a Value> {
    dotted_key
        .split('.')
        .try_fold(root, |current, segment| current.get(segment))
}

pub(crate) fn json_key_matches(root: &Value, key: &str, expected: &Value) -> bool {
    get_value_by_dotted_key(root, key).is_some_and(|actual| actual == expected)
}

pub(crate) fn report(severity: Severity, message: &str) -> bool {
    match severity {
        Severity::Off => false,
        Severity::Warn => {
            eprintln!("::warning::{}", message);
            false
        }
        Severity::Error => {
            eprintln!("::error::{}", message);
            true
        }
    }
}

pub(crate) fn report_at(severity: Severity, file: &str, message: &str) -> bool {
    match severity {
        Severity::Off => false,
        Severity::Warn => {
            eprintln!("::warning file={}::{}", file, message);
            false
        }
        Severity::Error => {
            eprintln!("::error file={}::{}", file, message);
            true
        }
    }
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
            return Some(format!(
                "[ruleman] ディレクトリ '{}' の読み取りに失敗しました: {}",
                display, e
            ));
        }
    };
    match (want_empty, is_empty) {
        (true, false) => Some(format!(
            "[ruleman] ディレクトリ '{}' は空である必要がありますが、空ではありません。",
            display
        )),
        (false, true) => Some(format!(
            "[ruleman] ディレクトリ '{}' は空でない必要がありますが、空です。",
            display
        )),
        _ => None,
    }
}

pub(crate) fn run_config(config: Config) -> i32 {
    let mut has_errors = false;

    for rule in config.rules {
        match rule {
            Rule::File {
                severity,
                state,
                files,
            } => {
                if severity == Severity::Off {
                    continue;
                }
                for file in files {
                    let path = Path::new(&file);
                    let message = match state {
                        FileState::Absent if path.exists() => Some(format!(
                            "[ruleman] 存在してはいけないファイル '{}' が見つかりました。",
                            file
                        )),
                        FileState::Present if !path.exists() => Some(format!(
                            "[ruleman] 必須ファイル '{}' が見つかりません。",
                            file
                        )),
                        FileState::Present if !path.is_file() => Some(format!(
                            "[ruleman] '{}' はファイルである必要がありますが、ディレクトリが存在します。",
                            file
                        )),
                        _ => None,
                    };
                    if let Some(message) = message {
                        has_errors |= report(severity, &message);
                    }
                }
            }
            Rule::Directory {
                severity,
                state,
                directories,
                empty,
            } => {
                if severity == Severity::Off {
                    continue;
                }
                for dir in directories {
                    let path = Path::new(&dir);
                    let message = match state {
                        FileState::Absent if path.exists() => Some(format!(
                            "[ruleman] 存在してはいけないディレクトリ '{}' が見つかりました。",
                            dir
                        )),
                        FileState::Present if !path.exists() => Some(format!(
                            "[ruleman] 必須ディレクトリ '{}' が見つかりません。",
                            dir
                        )),
                        FileState::Present if !path.is_dir() => Some(format!(
                            "[ruleman] '{}' はディレクトリである必要がありますが、ファイルが存在します。",
                            dir
                        )),
                        FileState::Present => check_directory_emptiness(path, &dir, empty),
                        FileState::Absent => None,
                    };
                    if let Some(message) = message {
                        has_errors |= report(severity, &message);
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

                let path = Path::new(&file);
                let fail = || format!("[ruleman] ルール不適合: {} の検証に失敗しました。", key);

                if !path.exists() {
                    has_errors |= report_at(severity, &file, &fail());
                    continue;
                }

                let raw = match fs::read_to_string(path) {
                    Ok(content) => content,
                    Err(_) => {
                        has_errors |= report_at(severity, &file, &fail());
                        continue;
                    }
                };

                let parsed = match format {
                    ContentFormat::Json => serde_json::from_str::<Value>(&raw),
                };
                let document = match parsed {
                    Ok(value) => value,
                    Err(_) => {
                        has_errors |= report_at(severity, &file, &fail());
                        continue;
                    }
                };

                let matches = json_key_matches(&document, &key, &expected);
                let fails = match state {
                    MatchState::Match => !matches,
                    MatchState::Mismatch => matches,
                };
                if fails {
                    has_errors |= report_at(severity, &file, &fail());
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
                        let message = format!(
                            "[ruleman] ファイル '{}' のハッシュを計算できません: {}",
                            file, e
                        );
                        has_errors |= report_at(severity, &file, &message);
                        continue;
                    }
                };

                let expected = expected.trim();
                let matches = actual.eq_ignore_ascii_case(expected);
                let message = match state {
                    MatchState::Match if !matches => Some(format!(
                        "[ruleman] ファイル '{}' の {} ハッシュが記録と一致しません (期待: {}, 実際: {})。内容の変更が意図したものなら 'ruleman add --checksum {}' で記録し直してください。",
                        file,
                        algorithm.as_str(),
                        expected,
                        actual,
                        file
                    )),
                    MatchState::Mismatch if matches => Some(format!(
                        "[ruleman] ファイル '{}' の {} ハッシュが '{}' と一致しています。一致してはいけません。",
                        file,
                        algorithm.as_str(),
                        expected
                    )),
                    _ => None,
                };
                if let Some(message) = message {
                    has_errors |= report_at(severity, &file, &message);
                }
            }
        }
    }

    if has_errors {
        1
    } else {
        println!("[ruleman] すべての標準チェックに合格しました!");
        0
    }
}

pub(crate) fn run(config_arg: Option<&str>) -> i32 {
    let config_path = match resolve_config_path(config_arg) {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{}", message);
            return 1;
        }
    };

    let mut visited = HashSet::new();
    match load_config(&config_path, &mut visited) {
        Ok(config) => run_config(config),
        Err(message) => {
            eprintln!("{}", message);
            1
        }
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
        assert_eq!(run_config(config), 1);

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
        assert_eq!(run_config(config), 0);

        fs::write(dir.join("a.txt"), "abcd").unwrap();
        let config = load_config(&config_path, &mut HashSet::new()).unwrap();
        assert_eq!(run_config(config), 1);

        fs::remove_dir_all(&dir).unwrap();
    }
}
