//! The rule types a config file can declare, and the validation serde's
//! derives can't express.

use crate::checksum::ChecksumAlgorithm;
use crate::paths::{compile, is_pattern};
use clap::ValueEnum;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    #[default]
    Error,
    Warn,
    Off,
}

impl Severity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Off => "off",
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FileState {
    #[default]
    Present,
    Absent,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ContentFormat {
    #[default]
    Json,
    Yaml,
    Toml,
}

impl ContentFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ContentFormat::Json => "json",
            ContentFormat::Yaml => "yaml",
            ContentFormat::Toml => "toml",
        }
    }
}

/// How `content` compares the value it found against `expected`. A separate
/// axis from `state`, which only decides the direction of the assertion: any
/// comparison can be required to hold or to fail.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Comparison {
    /// Deep equality with `expected`.
    #[default]
    Equals,
    /// `expected` is a substring of a string value, or an element of an array
    /// value. Any other combination of types is a failure.
    Contains,
    /// `expected` is a regular expression that the string value must match.
    Regex,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MatchState {
    #[default]
    Match,
    Mismatch,
}

/// `deny_unknown_fields` makes a misspelled or not-yet-supported attribute an
/// error instead of a silently ignored one — a typo'd attribute would
/// otherwise quietly weaken the check it was meant to tighten. It matches
/// `additionalProperties: false` in `docs/schema.json`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub(crate) enum Rule {
    #[serde(rename = "file")]
    File {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        state: FileState,
        /// Literal paths, or glob patterns when they contain `*`/`?`/`[`/`{`.
        files: Vec<String>,
    },
    #[serde(rename = "directory")]
    Directory {
        #[serde(default)]
        severity: Severity,
        #[serde(default)]
        state: FileState,
        /// Literal paths, or glob patterns when they contain `*`/`?`/`[`/`{`.
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
        /// How the value found at `key` is compared against `expected`.
        #[serde(default)]
        comparison: Comparison,
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

impl Rule {
    /// Every path-shaped attribute of this rule, so a malformed glob is caught
    /// at load time whichever rule type it appears in.
    fn patterns(&self) -> &[String] {
        match self {
            Rule::File { files, .. } => files,
            Rule::Directory { directories, .. } => directories,
            Rule::Content { file, .. } | Rule::Checksum { file, .. } => std::slice::from_ref(file),
        }
    }
}

/// Cross-attribute checks serde can't express on its own. Mutually exclusive
/// attributes (a future `url` locator vs. `file`, `expected` vs. a URL the
/// reference is fetched from) get rejected here too, rather than silently
/// picking one.
pub(crate) fn validate_rule(rule: &Rule) -> Result<(), String> {
    // A malformed glob is a config mistake, so it fails at load time rather
    // than turning into a per-run check failure.
    for pattern in rule.patterns() {
        if is_pattern(pattern) {
            compile(pattern)?;
        }
    }

    if let Rule::Content {
        comparison: Comparison::Regex,
        expected,
        ..
    } = rule
    {
        let Some(pattern) = expected.as_str() else {
            return Err(format!(
                "'expected' must be a string when comparison is 'regex', not {}",
                expected
            ));
        };
        Regex::new(pattern).map_err(|e| format!("invalid regex '{}': {}", pattern, e))?;
    }

    if let Rule::Checksum {
        algorithm,
        expected,
        ..
    } = rule
    {
        let digest = expected.trim();
        if digest.len() != algorithm.hex_width() || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "'expected' must be a bare {} digest of {} hex characters: '{}'",
                algorithm.as_str(),
                algorithm.hex_width(),
                expected
            ));
        }
    }
    Ok(())
}
