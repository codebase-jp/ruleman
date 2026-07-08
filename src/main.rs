use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_CANDIDATES: &[&str] = &["ruleman.json", "ruleman.jsonc", ".ruleman.json"];

const INIT_TEMPLATE: &str = r#"{
  "$schema": "https://ruleman.dev/schema.json",
  "rules": [
    {
      "type": "file",
      "severity": "error",
      "state": "present",
      "files": ["README.md", "LICENSE"]
    }
  ]
}
"#;

#[derive(Parser, Debug)]
#[command(
    name = "ruleman",
    version,
    about = "Repository static analysis by declarative rules"
)]
struct Cli {
    /// Path to the config file. When omitted, ruleman.json / ruleman.jsonc / .ruleman.json
    /// is discovered starting from the current directory and walking up.
    #[arg(long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a starter ruleman.json in the current directory.
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum Severity {
    #[default]
    Error,
    Warn,
    Off,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum FileState {
    #[default]
    Present,
    Absent,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum ContentFormat {
    #[default]
    Json,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum MatchState {
    #[default]
    Match,
    Mismatch,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Rule {
    #[serde(rename = "file")]
    File {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        state: FileState,
        files: Vec<String>,
    },
    #[serde(rename = "content")]
    Content {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        format: ContentFormat,
        #[serde(default)]
        state: MatchState,
        file: String,
        key: String,
        expected: Value,
    },
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default, rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    #[serde(default)]
    extends: Vec<String>,
    #[serde(default)]
    rules: Vec<Rule>,
}

struct Config {
    rules: Vec<Rule>,
}

fn parse_config_text(raw: &str) -> Result<RawConfig, String> {
    jsonc_parser::parse_to_serde_value(raw, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())
        .and_then(|value| {
            let value = value.unwrap_or(Value::Object(Default::default()));
            serde_json::from_value(value).map_err(|e| e.to_string())
        })
}

fn load_raw_config(path: &Path) -> Result<RawConfig, String> {
    if !path.exists() {
        return Err(format!(
            "::error::[ruleman] 設定ファイル '{}' が見つかりません。",
            path.display()
        ));
    }

    let raw = fs::read_to_string(path).map_err(|e| {
        format!(
            "::error::[ruleman] 設定ファイル '{}' の読み込みに失敗しました: {}",
            path.display(),
            e
        )
    })?;

    parse_config_text(&raw).map_err(|e| {
        format!(
            "::error::[ruleman] 設定ファイル '{}' の解析に失敗しました: {}",
            path.display(),
            e
        )
    })
}

/// Joins `file` onto `base_dir`, unless `base_dir` is empty (a config file
/// with no directory component, e.g. plain `ruleman.json` in the cwd), in
/// which case `file` is left untouched to avoid a cosmetic `./` prefix.
fn join_relative(base_dir: &Path, file: &str) -> String {
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
fn resolve_rule_paths(rule: Rule, base_dir: &Path) -> Rule {
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
    }
}

/// Resolves `extends` recursively (relative to each config file's own directory),
/// concatenating rules from extended configs first, followed by the file's own rules.
/// Every rule's file paths are resolved relative to the config file that declared them.
fn load_config(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<Config, String> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Err(format!(
            "::error::[ruleman] 設定ファイルの 'extends' が循環しています: '{}'",
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
fn discover_config() -> Option<PathBuf> {
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

fn get_value_by_dotted_key<'a>(root: &'a Value, dotted_key: &str) -> Option<&'a Value> {
    dotted_key
        .split('.')
        .try_fold(root, |current, segment| current.get(segment))
}

fn json_key_matches(root: &Value, key: &str, expected: &Value) -> bool {
    get_value_by_dotted_key(root, key).is_some_and(|actual| actual == expected)
}

fn report(severity: Severity, message: &str) -> bool {
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

fn report_at(severity: Severity, file: &str, message: &str) -> bool {
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

fn run_config(config: Config) -> i32 {
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
                    let exists = Path::new(&file).exists();
                    let message = match state {
                        FileState::Present if !exists => Some(format!(
                            "[ruleman] 必須ファイル '{}' が見つかりません。",
                            file
                        )),
                        FileState::Absent if exists => Some(format!(
                            "[ruleman] 存在してはいけないファイル '{}' が見つかりました。",
                            file
                        )),
                        _ => None,
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
        }
    }

    if has_errors {
        1
    } else {
        println!("[ruleman] すべての標準チェックに合格しました!");
        0
    }
}

fn run(config_arg: Option<&str>) -> i32 {
    let config_path = match config_arg {
        Some(path) => PathBuf::from(path),
        None => match discover_config() {
            Some(path) => path,
            None => {
                eprintln!(
                    "::error::[ruleman] 設定ファイルが見つかりません。'ruleman init' で作成できます。"
                );
                return 1;
            }
        },
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

fn run_init(force: bool) -> i32 {
    let path = Path::new("ruleman.json");
    if path.exists() && !force {
        eprintln!(
            "::error::[ruleman] '{}' は既に存在します。上書きするには --force を指定してください。",
            path.display()
        );
        return 1;
    }

    match fs::write(path, INIT_TEMPLATE) {
        Ok(()) => {
            println!("[ruleman] '{}' を作成しました。", path.display());
            0
        }
        Err(e) => {
            eprintln!(
                "::error::[ruleman] '{}' の作成に失敗しました: {}",
                path.display(),
                e
            );
            1
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Some(Command::Init { force }) => run_init(force),
        None => run(cli.config.as_deref()),
    };
    std::process::exit(code);
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
    fn join_relative_with_empty_base_dir_is_unchanged() {
        assert_eq!(join_relative(Path::new(""), "README.md"), "README.md");
    }

    #[test]
    fn join_relative_joins_with_nonempty_base_dir() {
        assert_eq!(
            join_relative(Path::new("/tmp/proj"), "README.md"),
            "/tmp/proj/README.md"
        );
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
