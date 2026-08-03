use clap::{Parser, Subcommand, ValueEnum};
use jsonc_parser::cst::{CstArray, CstInputValue, CstObject, CstRootNode};
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
    /// Add existing files/directories to the config as existence rules.
    ///
    /// Each path must exist; whether it becomes a `file` or a `directory`
    /// rule is decided by what's on disk. Paths are stored relative to the
    /// config file, and comments/formatting in the config are preserved.
    Add {
        /// Paths to add, relative to the current directory.
        #[arg(required = true)]
        paths: Vec<String>,

        /// Severity of the rule the paths are added to.
        #[arg(long, value_enum, default_value_t = Severity::Error)]
        severity: Severity,
    },
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum Severity {
    #[default]
    Error,
    Warn,
    Off,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Off => "off",
        }
    }
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
    #[serde(rename = "directory")]
    Directory {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        state: FileState,
        directories: Vec<String>,
        /// Only checked when `state` is `present` and the path is confirmed
        /// to be a directory: `true` requires zero entries, `false` requires
        /// at least one. Unset skips the check.
        #[serde(default)]
        empty: Option<bool>,
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

/// Checks a directory's emptiness once it's already confirmed to exist and
/// be a directory. Returns `None` if `empty` is unset or the check passes.
fn check_directory_emptiness(path: &Path, display: &str, empty: Option<bool>) -> Option<String> {
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
        }
    }

    if has_errors {
        1
    } else {
        println!("[ruleman] すべての標準チェックに合格しました!");
        0
    }
}

fn resolve_config_path(config_arg: Option<&str>) -> Result<PathBuf, String> {
    match config_arg {
        Some(path) => Ok(PathBuf::from(path)),
        None => discover_config().ok_or_else(|| {
            "::error::[ruleman] 設定ファイルが見つかりません。'ruleman init' で作成できます。"
                .to_string()
        }),
    }
}

fn run(config_arg: Option<&str>) -> i32 {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKind {
    File,
    Directory,
}

impl PathKind {
    /// The `type` discriminant of the rule this kind is added to.
    fn rule_type(self) -> &'static str {
        match self {
            PathKind::File => "file",
            PathKind::Directory => "directory",
        }
    }

    /// The rule field holding the paths (`file`'s `files`, `directory`'s `directories`).
    fn paths_field(self) -> &'static str {
        match self {
            PathKind::File => "files",
            PathKind::Directory => "directories",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PathKind::File => "ファイル",
            PathKind::Directory => "ディレクトリ",
        }
    }
}

/// Rewrites a cwd-relative path into one relative to the config file's own
/// directory, matching how rule paths are resolved at check time (see
/// `resolve_rule_paths`). Always emits `/` separators so configs stay
/// portable across platforms.
fn path_relative_to_config(config_path: &Path, input: &str) -> Result<String, String> {
    let config_dir = match config_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = fs::canonicalize(&config_dir).map_err(|e| {
        format!(
            "[ruleman] 設定ファイルのディレクトリ '{}' を解決できません: {}",
            config_dir.display(),
            e
        )
    })?;
    let target = fs::canonicalize(input)
        .map_err(|e| format!("[ruleman] '{}' を解決できません: {}", input, e))?;

    let relative = target.strip_prefix(&base).map_err(|_| {
        format!(
            "[ruleman] '{}' は設定ファイル '{}' のディレクトリの外にあります。",
            input,
            config_path.display()
        )
    })?;

    let joined = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    if joined.is_empty() {
        Err(format!(
            "[ruleman] '{}' は設定ファイルのディレクトリそのものです。",
            input
        ))
    } else {
        Ok(joined)
    }
}

/// Reads a string-valued property off a rule object, if present and a string.
fn cst_string_prop(object: &CstObject, name: &str) -> Option<String> {
    object
        .get(name)?
        .value()?
        .as_string_lit()?
        .decoded_value()
        .ok()
}

/// Reads the string elements of an array-valued property, ignoring any
/// element that isn't a string.
fn cst_string_array(array: &CstArray) -> Vec<String> {
    array
        .elements()
        .iter()
        .filter_map(|element| element.as_string_lit())
        .filter_map(|literal| literal.decoded_value().ok())
        .collect()
}

struct AddOutcome {
    text: String,
    added: Vec<(PathKind, String)>,
    skipped: Vec<String>,
}

/// Adds `entries` (paths already made relative to the config file) to the
/// config text as `state: "present"` rules. Paths are appended to an existing
/// rule of the same type, state and severity when there is one, otherwise a
/// new rule is appended to `rules`. Editing happens on the JSONC concrete
/// syntax tree, so comments, formatting and trailing commas survive.
fn add_entries_to_config_text(
    text: &str,
    entries: &[(PathKind, String)],
    severity: Severity,
) -> Result<AddOutcome, String> {
    let root = CstRootNode::parse(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?;
    let object = root.object_value_or_set();
    let rules = object
        .array_value_or_create("rules")
        .ok_or_else(|| "'rules' が配列ではありません。".to_string())?;

    let mut added = Vec::new();
    let mut skipped = Vec::new();

    for (kind, path) in entries {
        let mut duplicate = false;
        let mut target = None;

        for element in rules.elements() {
            let Some(rule) = element.as_object() else {
                continue;
            };
            if cst_string_prop(&rule, "type").as_deref() != Some(kind.rule_type()) {
                continue;
            }
            // A rule without `state` defaults to `present`, the state this
            // command writes; anything else checks something different.
            if cst_string_prop(&rule, "state").unwrap_or_else(|| "present".to_string()) != "present"
            {
                continue;
            }
            let Some(paths) = rule.array_value(kind.paths_field()) else {
                continue;
            };
            if cst_string_array(&paths).iter().any(|p| p == path) {
                duplicate = true;
                break;
            }
            let rule_severity = cst_string_prop(&rule, "severity")
                .unwrap_or_else(|| Severity::default().as_str().to_string());
            if target.is_none() && rule_severity == severity.as_str() {
                target = Some(paths);
            }
        }

        if duplicate {
            skipped.push(path.clone());
            continue;
        }

        match target {
            Some(paths) => {
                paths.append(CstInputValue::String(path.clone()));
            }
            None => {
                rules.append(CstInputValue::Object(vec![
                    (
                        "type".to_string(),
                        CstInputValue::String(kind.rule_type().to_string()),
                    ),
                    (
                        "severity".to_string(),
                        CstInputValue::String(severity.as_str().to_string()),
                    ),
                    (
                        "state".to_string(),
                        CstInputValue::String("present".to_string()),
                    ),
                    (
                        kind.paths_field().to_string(),
                        CstInputValue::Array(vec![CstInputValue::String(path.clone())]),
                    ),
                ]));
            }
        }
        added.push((*kind, path.clone()));
    }

    Ok(AddOutcome {
        text: root.to_string(),
        added,
        skipped,
    })
}

fn run_add(config_arg: Option<&str>, paths: &[String], severity: Severity) -> i32 {
    let config_path = match resolve_config_path(config_arg) {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{}", message);
            return 1;
        }
    };

    let text = match fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!(
                "::error::[ruleman] 設定ファイル '{}' の読み込みに失敗しました: {}",
                config_path.display(),
                e
            );
            return 1;
        }
    };

    let mut entries = Vec::new();
    for path in paths {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => {
                eprintln!(
                    "::error::[ruleman] '{}' が見つかりません。既存のファイルまたはディレクトリを指定してください。",
                    path
                );
                return 1;
            }
        };
        let kind = if metadata.is_dir() {
            PathKind::Directory
        } else {
            PathKind::File
        };
        match path_relative_to_config(&config_path, path) {
            Ok(relative) => entries.push((kind, relative)),
            Err(message) => {
                eprintln!("::error::{}", message);
                return 1;
            }
        }
    }

    let outcome = match add_entries_to_config_text(&text, &entries, severity) {
        Ok(outcome) => outcome,
        Err(message) => {
            eprintln!(
                "::error::[ruleman] 設定ファイル '{}' の更新に失敗しました: {}",
                config_path.display(),
                message
            );
            return 1;
        }
    };

    for path in &outcome.skipped {
        println!("[ruleman] '{}' は既に登録されています。", path);
    }

    if outcome.added.is_empty() {
        return 0;
    }

    if let Err(e) = fs::write(&config_path, &outcome.text) {
        eprintln!(
            "::error::[ruleman] 設定ファイル '{}' の書き込みに失敗しました: {}",
            config_path.display(),
            e
        );
        return 1;
    }

    for (kind, path) in &outcome.added {
        println!(
            "[ruleman] {} '{}' を '{}' に追加しました。",
            kind.label(),
            path,
            config_path.display()
        );
    }
    0
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Some(Command::Init { force }) => run_init(force),
        Some(Command::Add { paths, severity }) => run_add(cli.config.as_deref(), &paths, severity),
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
        let base = Path::new("some").join("proj");
        let expected = base.join("README.md").to_string_lossy().into_owned();
        assert_eq!(join_relative(&base, "README.md"), expected);
    }

    #[test]
    fn add_appends_to_an_existing_matching_rule_and_keeps_comments() {
        let text = r#"{
  // required files
  "rules": [
    { "type": "file", "files": ["README.md"] }
  ]
}
"#;
        let outcome = add_entries_to_config_text(
            text,
            &[(PathKind::File, "LICENSE".to_string())],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.added, vec![(PathKind::File, "LICENSE".to_string())]);
        assert!(outcome.skipped.is_empty());
        assert!(outcome.text.contains("// required files"));
        assert!(outcome.text.contains(r#"["README.md", "LICENSE"]"#));
    }

    #[test]
    fn add_creates_a_new_rule_when_no_rule_matches() {
        let text = r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#;
        let outcome = add_entries_to_config_text(
            text,
            &[(PathKind::Directory, "src".to_string())],
            Severity::Error,
        )
        .unwrap();

        let config = parse_config_text(&outcome.text).unwrap();
        assert_eq!(config.rules.len(), 2);
        match &config.rules[1] {
            Rule::Directory {
                severity,
                state,
                directories,
                ..
            } => {
                assert_eq!(*severity, Severity::Error);
                assert_eq!(*state, FileState::Present);
                assert_eq!(directories, &["src".to_string()]);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_does_not_merge_into_a_rule_with_a_different_severity_or_state() {
        let text = r#"{
            "rules": [
                { "type": "file", "severity": "warn", "files": ["a.txt"] },
                { "type": "file", "state": "absent", "files": ["b.txt"] }
            ]
        }"#;
        let outcome = add_entries_to_config_text(
            text,
            &[(PathKind::File, "c.txt".to_string())],
            Severity::Error,
        )
        .unwrap();

        let config = parse_config_text(&outcome.text).unwrap();
        assert_eq!(config.rules.len(), 3);
        match &config.rules[2] {
            Rule::File { files, .. } => assert_eq!(files, &["c.txt".to_string()]),
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_skips_paths_already_covered_by_a_present_rule() {
        let text =
            r#"{ "rules": [ { "type": "file", "severity": "warn", "files": ["README.md"] } ] }"#;
        let outcome = add_entries_to_config_text(
            text,
            &[(PathKind::File, "README.md".to_string())],
            Severity::Error,
        )
        .unwrap();

        assert!(outcome.added.is_empty());
        assert_eq!(outcome.skipped, vec!["README.md".to_string()]);
        assert_eq!(parse_config_text(&outcome.text).unwrap().rules.len(), 1);
    }

    #[test]
    fn add_creates_the_rules_array_when_missing() {
        let outcome = add_entries_to_config_text(
            "{}",
            &[(PathKind::File, "README.md".to_string())],
            Severity::Warn,
        )
        .unwrap();

        let config = parse_config_text(&outcome.text).unwrap();
        match &config.rules[0] {
            Rule::File {
                severity, files, ..
            } => {
                assert_eq!(*severity, Severity::Warn);
                assert_eq!(files, &["README.md".to_string()]);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_rejects_a_non_array_rules_property() {
        assert!(
            add_entries_to_config_text(
                r#"{ "rules": {} }"#,
                &[(PathKind::File, "README.md".to_string())],
                Severity::Error,
            )
            .is_err()
        );
    }

    #[test]
    fn path_relative_to_config_uses_slashes_and_rejects_outside_paths() {
        let dir = std::env::temp_dir().join("ruleman_test_add_relative");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        let config_path = dir.join("ruleman.json");
        fs::write(&config_path, "{}").unwrap();
        let nested = dir.join("src").join("main.rs");
        fs::write(&nested, "").unwrap();

        assert_eq!(
            path_relative_to_config(&config_path, &nested.to_string_lossy()).unwrap(),
            "src/main.rs"
        );
        assert!(path_relative_to_config(&config_path, &dir.to_string_lossy()).is_err());
        assert!(
            path_relative_to_config(&config_path, &std::env::temp_dir().to_string_lossy()).is_err()
        );

        fs::remove_dir_all(&dir).unwrap();
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
