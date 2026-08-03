//! The rule types a config file can declare, and the validation serde's
//! derives can't express.

use crate::checksum::ChecksumAlgorithm;
use clap::ValueEnum;
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
}

impl ContentFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ContentFormat::Json => "json",
        }
    }
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

/// Cross-attribute checks serde can't express on its own. Mutually exclusive
/// attributes (a future `url` locator vs. `file`, `expected` vs. a URL the
/// reference is fetched from) get rejected here too, rather than silently
/// picking one.
pub(crate) fn validate_rule(rule: &Rule) -> Result<(), String> {
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
