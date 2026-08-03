use clap::{Parser, Subcommand, ValueEnum};
use jsonc_parser::cst::{CstArray, CstInputValue, CstObject, CstRootNode};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
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
    /// Add existing files/directories to the config as rules.
    ///
    /// Each path must exist; whether it becomes a `file` or a `directory`
    /// rule is decided by what's on disk. Paths are stored relative to the
    /// config file, and comments/formatting in the config are preserved.
    ///
    /// With --checksum the file's current hash is recorded as a `checksum`
    /// rule instead; re-running it after an intentional edit refreshes the
    /// recorded hash.
    Add {
        /// Paths to add, relative to the current directory.
        #[arg(required = true)]
        paths: Vec<String>,

        /// Severity of the rule the paths are added to.
        #[arg(long, value_enum, default_value_t = Severity::Error)]
        severity: Severity,

        /// Record each file's current hash as a `checksum` rule instead of an
        /// existence rule.
        #[arg(long)]
        checksum: bool,

        /// Hash algorithm to record with. Defaults to sha256.
        #[arg(long, value_enum, requires = "checksum")]
        algorithm: Option<ChecksumAlgorithm>,
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

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum ChecksumAlgorithm {
    #[default]
    Sha256,
}

impl ChecksumAlgorithm {
    fn as_str(self) -> &'static str {
        match self {
            ChecksumAlgorithm::Sha256 => "sha256",
        }
    }
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
    #[serde(rename = "checksum")]
    Checksum {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        algorithm: ChecksumAlgorithm,
        #[serde(default)]
        state: MatchState,
        file: String,
        /// Hex digest, compared case-insensitively.
        expected: String,
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

/// Streams the file through the hasher rather than reading it into memory, so
/// large tracked artifacts (lockfiles, vendored bundles) don't cost RAM.
fn file_checksum(path: &Path, algorithm: ChecksumAlgorithm) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    match algorithm {
        ChecksumAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok(format!("{:x}", hasher.finalize()))
        }
    }
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

/// One path to register, already made relative to the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AddEntry {
    /// A `file`/`directory` rule asserting the path exists.
    Existence { kind: PathKind, path: String },
    /// A `checksum` rule pinning the file's current digest.
    Checksum {
        path: String,
        algorithm: ChecksumAlgorithm,
        digest: String,
    },
}

impl AddEntry {
    fn path(&self) -> &str {
        match self {
            AddEntry::Existence { path, .. } | AddEntry::Checksum { path, .. } => path,
        }
    }
}

/// What happened to an entry. `Updated` only arises for checksum entries whose
/// rule already existed with a stale digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddResult {
    Added,
    Updated,
    Skipped,
}

struct AddOutcome {
    text: String,
    /// One result per input entry, in the same order.
    results: Vec<AddResult>,
}

impl AddOutcome {
    fn changed(&self) -> bool {
        self.results.iter().any(|r| *r != AddResult::Skipped)
    }
}

/// Registers `entries` in the config text. Editing happens on the JSONC
/// concrete syntax tree, so comments, formatting and trailing commas survive.
fn add_entries_to_config_text(
    text: &str,
    entries: &[AddEntry],
    severity: Severity,
) -> Result<AddOutcome, String> {
    let root = CstRootNode::parse(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?;
    let object = root.object_value_or_set();
    let rules = object
        .array_value_or_create("rules")
        .ok_or_else(|| "'rules' が配列ではありません。".to_string())?;

    let results = entries
        .iter()
        .map(|entry| match entry {
            AddEntry::Existence { kind, path } => {
                add_existence_entry(&rules, *kind, path, severity)
            }
            AddEntry::Checksum {
                path,
                algorithm,
                digest,
            } => add_checksum_entry(&rules, path, *algorithm, digest, severity),
        })
        .collect();

    Ok(AddOutcome {
        text: root.to_string(),
        results,
    })
}

/// Appends `path` to an existing rule of the same type, state and severity when
/// there is one, otherwise appends a new rule to `rules`.
fn add_existence_entry(
    rules: &CstArray,
    kind: PathKind,
    path: &str,
    severity: Severity,
) -> AddResult {
    let mut target = None;

    for element in rules.elements() {
        let Some(rule) = element.as_object() else {
            continue;
        };
        if cst_string_prop(&rule, "type").as_deref() != Some(kind.rule_type()) {
            continue;
        }
        // A rule without `state` defaults to `present`, the state this command
        // writes; anything else checks something different.
        if cst_rule_state(&rule, "present") != "present" {
            continue;
        }
        let Some(paths) = rule.array_value(kind.paths_field()) else {
            continue;
        };
        if cst_string_array(&paths).iter().any(|p| p == path) {
            return AddResult::Skipped;
        }
        if target.is_none() && cst_rule_severity(&rule) == severity.as_str() {
            target = Some(paths);
        }
    }

    match target {
        Some(paths) => {
            paths.append(CstInputValue::String(path.to_string()));
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
                    CstInputValue::Array(vec![CstInputValue::String(path.to_string())]),
                ),
            ]));
        }
    }
    AddResult::Added
}

/// Rewrites the digest of the existing `match` rule for this file and algorithm
/// if there is one — re-running `add --checksum` after an intentional edit is
/// how a pin gets refreshed — otherwise appends a new rule.
fn add_checksum_entry(
    rules: &CstArray,
    path: &str,
    algorithm: ChecksumAlgorithm,
    digest: &str,
    severity: Severity,
) -> AddResult {
    for element in rules.elements() {
        let Some(rule) = element.as_object() else {
            continue;
        };
        if cst_string_prop(&rule, "type").as_deref() != Some("checksum") {
            continue;
        }
        if cst_string_prop(&rule, "file").as_deref() != Some(path) {
            continue;
        }
        if cst_string_prop(&rule, "algorithm")
            .unwrap_or_else(|| ChecksumAlgorithm::default().as_str().to_string())
            != algorithm.as_str()
        {
            continue;
        }
        // A `mismatch` rule pins a digest the file must *not* have; overwriting
        // it with the current one would invert what it asserts.
        if cst_rule_state(&rule, "match") != "match" {
            continue;
        }

        let recorded = cst_string_prop(&rule, "expected").unwrap_or_default();
        if recorded.trim().eq_ignore_ascii_case(digest) {
            return AddResult::Skipped;
        }
        match rule.get("expected") {
            Some(prop) => prop.set_value(CstInputValue::String(digest.to_string())),
            None => {
                rule.append("expected", CstInputValue::String(digest.to_string()));
            }
        }
        return AddResult::Updated;
    }

    rules.append(CstInputValue::Object(vec![
        ("type".to_string(), CstInputValue::String("checksum".into())),
        (
            "severity".to_string(),
            CstInputValue::String(severity.as_str().to_string()),
        ),
        (
            "algorithm".to_string(),
            CstInputValue::String(algorithm.as_str().to_string()),
        ),
        ("state".to_string(), CstInputValue::String("match".into())),
        ("file".to_string(), CstInputValue::String(path.to_string())),
        (
            "expected".to_string(),
            CstInputValue::String(digest.to_string()),
        ),
    ]));
    AddResult::Added
}

/// A rule's `state`, falling back to the per-rule-type default when unset.
fn cst_rule_state(rule: &CstObject, default: &'static str) -> String {
    cst_string_prop(rule, "state").unwrap_or_else(|| default.to_string())
}

fn cst_rule_severity(rule: &CstObject) -> String {
    cst_string_prop(rule, "severity").unwrap_or_else(|| Severity::default().as_str().to_string())
}

/// Turns a cwd-relative CLI argument into the entry to register, resolving the
/// stored path and — for `--checksum` — hashing the file as it is right now.
fn build_add_entry(
    config_path: &Path,
    input: &str,
    checksum: Option<ChecksumAlgorithm>,
) -> Result<AddEntry, String> {
    let metadata = fs::metadata(input).map_err(|_| {
        format!(
            "[ruleman] '{}' が見つかりません。既存のファイルまたはディレクトリを指定してください。",
            input
        )
    })?;
    let path = path_relative_to_config(config_path, input)?;

    match checksum {
        Some(algorithm) => {
            if !metadata.is_file() {
                return Err(format!(
                    "[ruleman] '{}' はディレクトリです。--checksum はファイルにのみ指定できます。",
                    input
                ));
            }
            let digest = file_checksum(Path::new(input), algorithm)
                .map_err(|e| format!("[ruleman] '{}' のハッシュを計算できません: {}", input, e))?;
            Ok(AddEntry::Checksum {
                path,
                algorithm,
                digest,
            })
        }
        None => {
            let kind = if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::File
            };
            Ok(AddEntry::Existence { kind, path })
        }
    }
}

fn run_add(
    config_arg: Option<&str>,
    paths: &[String],
    severity: Severity,
    checksum: Option<ChecksumAlgorithm>,
) -> i32 {
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
        match build_add_entry(&config_path, path, checksum) {
            Ok(entry) => entries.push(entry),
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

    if outcome.changed()
        && let Err(e) = fs::write(&config_path, &outcome.text)
    {
        eprintln!(
            "::error::[ruleman] 設定ファイル '{}' の書き込みに失敗しました: {}",
            config_path.display(),
            e
        );
        return 1;
    }

    for (entry, result) in entries.iter().zip(&outcome.results) {
        println!("[ruleman] {}", add_message(entry, *result, &config_path));
    }
    0
}

fn add_message(entry: &AddEntry, result: AddResult, config_path: &Path) -> String {
    let path = entry.path();
    let config = config_path.display();
    match (entry, result) {
        (AddEntry::Existence { kind, .. }, AddResult::Added) => {
            format!(
                "{} '{}' を '{}' に追加しました。",
                kind.label(),
                path,
                config
            )
        }
        (AddEntry::Existence { .. }, _) => {
            format!("'{}' は既に登録されています。", path)
        }
        (
            AddEntry::Checksum {
                algorithm, digest, ..
            },
            AddResult::Added,
        ) => format!(
            "'{}' の {} ハッシュを '{}' に記録しました: {}",
            path,
            algorithm.as_str(),
            config,
            digest
        ),
        (
            AddEntry::Checksum {
                algorithm, digest, ..
            },
            AddResult::Updated,
        ) => format!(
            "'{}' の {} ハッシュを更新しました: {}",
            path,
            algorithm.as_str(),
            digest
        ),
        (AddEntry::Checksum { algorithm, .. }, AddResult::Skipped) => format!(
            "'{}' の {} ハッシュは記録済みの値と同じです。",
            path,
            algorithm.as_str()
        ),
    }
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Some(Command::Init { force }) => run_init(force),
        Some(Command::Add {
            paths,
            severity,
            checksum,
            algorithm,
        }) => run_add(
            cli.config.as_deref(),
            &paths,
            severity,
            checksum.then(|| algorithm.unwrap_or_default()),
        ),
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

    fn existence(kind: PathKind, path: &str) -> AddEntry {
        AddEntry::Existence {
            kind,
            path: path.to_string(),
        }
    }

    fn checksum_entry(path: &str, digest: &str) -> AddEntry {
        AddEntry::Checksum {
            path: path.to_string(),
            algorithm: ChecksumAlgorithm::Sha256,
            digest: digest.to_string(),
        }
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
            &[existence(PathKind::File, "LICENSE")],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Added]);
        assert!(outcome.text.contains("// required files"));
        assert!(outcome.text.contains(r#"["README.md", "LICENSE"]"#));
    }

    #[test]
    fn add_creates_a_new_rule_when_no_rule_matches() {
        let text = r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#;
        let outcome = add_entries_to_config_text(
            text,
            &[existence(PathKind::Directory, "src")],
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
            &[existence(PathKind::File, "c.txt")],
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
            &[existence(PathKind::File, "README.md")],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Skipped]);
        assert!(!outcome.changed());
        assert_eq!(parse_config_text(&outcome.text).unwrap().rules.len(), 1);
    }

    #[test]
    fn add_creates_the_rules_array_when_missing() {
        let outcome = add_entries_to_config_text(
            "{}",
            &[existence(PathKind::File, "README.md")],
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
                &[existence(PathKind::File, "README.md")],
                Severity::Error,
            )
            .is_err()
        );
    }

    #[test]
    fn add_checksum_creates_a_rule_with_the_recorded_digest() {
        let outcome = add_entries_to_config_text(
            "{}",
            &[checksum_entry("vendor/lib.js", "abc123")],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Added]);
        let config = parse_config_text(&outcome.text).unwrap();
        match &config.rules[0] {
            Rule::Checksum {
                severity,
                algorithm,
                state,
                file,
                expected,
            } => {
                assert_eq!(*severity, Severity::Error);
                assert_eq!(*algorithm, ChecksumAlgorithm::Sha256);
                assert_eq!(*state, MatchState::Match);
                assert_eq!(file, "vendor/lib.js");
                assert_eq!(expected, "abc123");
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_checksum_refreshes_a_stale_digest_in_place() {
        let text = r#"{
  "rules": [
    // pinned by hand
    { "type": "checksum", "file": "vendor/lib.js", "expected": "old" }
  ]
}"#;
        let outcome = add_entries_to_config_text(
            text,
            &[checksum_entry("vendor/lib.js", "new")],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Updated]);
        assert!(outcome.text.contains("// pinned by hand"));
        let config = parse_config_text(&outcome.text).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0] {
            Rule::Checksum { expected, .. } => assert_eq!(expected, "new"),
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_checksum_skips_an_unchanged_digest() {
        let text = r#"{ "rules": [ { "type": "checksum", "file": "a.txt", "expected": "ABC" } ] }"#;
        let outcome =
            add_entries_to_config_text(text, &[checksum_entry("a.txt", "abc")], Severity::Error)
                .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Skipped]);
        assert!(!outcome.changed());
    }

    #[test]
    fn add_checksum_leaves_a_mismatch_rule_alone() {
        let text = r#"{
            "rules": [
                { "type": "checksum", "state": "mismatch", "file": "a.txt", "expected": "banned" }
            ]
        }"#;
        let outcome = add_entries_to_config_text(
            text,
            &[checksum_entry("a.txt", "current")],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Added]);
        let config = parse_config_text(&outcome.text).unwrap();
        assert_eq!(config.rules.len(), 2);
        match &config.rules[0] {
            Rule::Checksum {
                state, expected, ..
            } => {
                assert_eq!(*state, MatchState::Mismatch);
                assert_eq!(expected, "banned");
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn checksum_rule_defaults_and_hashing() {
        let text = r#"{
            "rules": [ { "type": "checksum", "file": "a.txt", "expected": "x" } ]
        }"#;
        let config = parse_config_text(text).unwrap();
        match &config.rules[0] {
            Rule::Checksum {
                algorithm, state, ..
            } => {
                assert_eq!(*algorithm, ChecksumAlgorithm::Sha256);
                assert_eq!(*state, MatchState::Match);
            }
            _ => panic!("unexpected rule"),
        }

        let dir = std::env::temp_dir().join("ruleman_test_checksum");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "abc").unwrap();

        // Known SHA-256 of "abc".
        assert_eq!(
            file_checksum(&file, ChecksumAlgorithm::Sha256).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(file_checksum(&dir.join("missing.txt"), ChecksumAlgorithm::Sha256).is_err());

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
