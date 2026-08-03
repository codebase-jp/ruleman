//! Rewriting a config file's text to register new paths. Edits are applied to
//! the JSONC concrete syntax tree, so comments, formatting and trailing commas
//! survive; nothing here touches the filesystem, which keeps it unit-testable.

use crate::checksum::ChecksumAlgorithm;
use crate::rule::Severity;
use jsonc_parser::cst::{CstArray, CstInputValue, CstObject, CstRootNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathKind {
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

    pub(crate) fn label(self) -> &'static str {
        match self {
            PathKind::File => "file",
            PathKind::Directory => "directory",
        }
    }
}

/// One path to register, already made relative to the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AddEntry {
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
    pub(crate) fn path(&self) -> &str {
        match self {
            AddEntry::Existence { path, .. } | AddEntry::Checksum { path, .. } => path,
        }
    }
}

/// What happened to an entry. `Updated` only arises for checksum entries whose
/// rule already existed with a stale digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddResult {
    Added,
    Updated,
    Skipped,
}

pub(crate) struct AddOutcome {
    pub(crate) text: String,
    /// One result per input entry, in the same order.
    pub(crate) results: Vec<AddResult>,
}

impl AddOutcome {
    pub(crate) fn changed(&self) -> bool {
        self.results.iter().any(|r| *r != AddResult::Skipped)
    }
}

/// Registers `entries` in the config text. Editing happens on the JSONC
/// concrete syntax tree, so comments, formatting and trailing commas survive.
pub(crate) fn add_entries_to_config_text(
    text: &str,
    entries: &[AddEntry],
    severity: Severity,
) -> Result<AddOutcome, String> {
    let root = CstRootNode::parse(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?;
    let object = root.object_value_or_set();
    let rules = object
        .array_value_or_create("rules")
        .ok_or_else(|| "'rules' is not an array".to_string())?;

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
pub(crate) fn add_existence_entry(
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
pub(crate) fn add_checksum_entry(
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
pub(crate) fn cst_rule_state(rule: &CstObject, default: &'static str) -> String {
    cst_string_prop(rule, "state").unwrap_or_else(|| default.to_string())
}

pub(crate) fn cst_rule_severity(rule: &CstObject) -> String {
    cst_string_prop(rule, "severity").unwrap_or_else(|| Severity::default().as_str().to_string())
}

/// Reads a string-valued property off a rule object, if present and a string.
pub(crate) fn cst_string_prop(object: &CstObject, name: &str) -> Option<String> {
    object
        .get(name)?
        .value()?
        .as_string_lit()?
        .decoded_value()
        .ok()
}

/// Reads the string elements of an array-valued property, ignoring any
/// element that isn't a string.
pub(crate) fn cst_string_array(array: &CstArray) -> Vec<String> {
    array
        .elements()
        .iter()
        .filter_map(|element| element.as_string_lit())
        .filter_map(|literal| literal.decoded_value().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config_text;
    use crate::rule::{FileState, MatchState, Rule};
    use crate::testdata::{DIGEST_ABC, DIGEST_ZEROS};

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
            &[checksum_entry("vendor/lib.js", DIGEST_ABC)],
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
                assert_eq!(expected, DIGEST_ABC);
            }
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_checksum_refreshes_a_stale_digest_in_place() {
        let text = format!(
            r#"{{
  "rules": [
    // pinned by hand
    {{ "type": "checksum", "file": "vendor/lib.js", "expected": "{}" }}
  ]
}}"#,
            DIGEST_ZEROS
        );
        let outcome = add_entries_to_config_text(
            &text,
            &[checksum_entry("vendor/lib.js", DIGEST_ABC)],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Updated]);
        assert!(outcome.text.contains("// pinned by hand"));
        let config = parse_config_text(&outcome.text).unwrap();
        assert_eq!(config.rules.len(), 1);
        match &config.rules[0] {
            Rule::Checksum { expected, .. } => assert_eq!(expected, DIGEST_ABC),
            _ => panic!("unexpected rule"),
        }
    }

    #[test]
    fn add_checksum_skips_an_unchanged_digest() {
        let text = format!(
            r#"{{ "rules": [ {{ "type": "checksum", "file": "a.txt", "expected": "{}" }} ] }}"#,
            DIGEST_ABC.to_uppercase()
        );
        let outcome = add_entries_to_config_text(
            &text,
            &[checksum_entry("a.txt", DIGEST_ABC)],
            Severity::Error,
        )
        .unwrap();

        assert_eq!(outcome.results, vec![AddResult::Skipped]);
        assert!(!outcome.changed());
    }

    #[test]
    fn add_checksum_leaves_a_mismatch_rule_alone() {
        let text = format!(
            r#"{{
            "rules": [
                {{ "type": "checksum", "state": "mismatch", "file": "a.txt", "expected": "{}" }}
            ]
        }}"#,
            DIGEST_ZEROS
        );
        let outcome = add_entries_to_config_text(
            &text,
            &[checksum_entry("a.txt", DIGEST_ABC)],
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
                assert_eq!(expected, DIGEST_ZEROS);
            }
            _ => panic!("unexpected rule"),
        }
    }
}
