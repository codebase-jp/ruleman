//! The `add` subcommand: resolves the paths named on the command line into
//! entries, hands them to `config_edit`, and reports what changed.

use crate::checksum::{ChecksumAlgorithm, file_checksum};
use crate::config::resolve_config_path;
use crate::config_edit::{AddEntry, AddResult, PathKind, add_entries_to_config_text};
use crate::output::{OutputFormat, emit_error, emit_info};
use crate::rule::Severity;
use std::fs;
use std::path::{Path, PathBuf};

/// Rewrites a cwd-relative path into one relative to the config file's own
/// directory, matching how rule paths are resolved at check time (see
/// `resolve_rule_paths`). Always emits `/` separators so configs stay
/// portable across platforms.
pub(crate) fn path_relative_to_config(config_path: &Path, input: &str) -> Result<String, String> {
    let config_dir = match config_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = fs::canonicalize(&config_dir).map_err(|e| {
        format!(
            "cannot resolve the config file's directory '{}': {}",
            config_dir.display(),
            e
        )
    })?;
    let target =
        fs::canonicalize(input).map_err(|e| format!("cannot resolve '{}': {}", input, e))?;

    let relative = target.strip_prefix(&base).map_err(|_| {
        format!(
            "'{}' is outside the directory of config file '{}'",
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
        Err(format!("'{}' is the config file's own directory", input))
    } else {
        Ok(joined)
    }
}

/// Turns a cwd-relative CLI argument into the entry to register, resolving the
/// stored path and — for `--checksum` — hashing the file as it is right now.
pub(crate) fn build_add_entry(
    config_path: &Path,
    input: &str,
    checksum: Option<ChecksumAlgorithm>,
) -> Result<AddEntry, String> {
    let metadata = fs::metadata(input).map_err(|_| {
        format!(
            "'{}' not found; pass a file or directory that exists",
            input
        )
    })?;
    let path = path_relative_to_config(config_path, input)?;

    match checksum {
        Some(algorithm) => {
            if !metadata.is_file() {
                return Err(format!(
                    "'{}' is a directory; --checksum only applies to files",
                    input
                ));
            }
            let digest = file_checksum(Path::new(input), algorithm)
                .map_err(|e| format!("cannot hash '{}': {}", input, e))?;
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

pub(crate) fn run(
    config_arg: Option<&str>,
    paths: &[String],
    severity: Severity,
    checksum: Option<ChecksumAlgorithm>,
    format: OutputFormat,
) -> i32 {
    let config_path = match resolve_config_path(config_arg) {
        Ok(path) => path,
        Err(message) => {
            emit_error(format, &message);
            return 1;
        }
    };

    let text = match fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(e) => {
            emit_error(
                format,
                &format!("cannot read config file '{}': {}", config_path.display(), e),
            );
            return 1;
        }
    };

    let mut entries = Vec::new();
    for path in paths {
        match build_add_entry(&config_path, path, checksum) {
            Ok(entry) => entries.push(entry),
            Err(message) => {
                emit_error(format, &message);
                return 1;
            }
        }
    }

    let outcome = match add_entries_to_config_text(&text, &entries, severity) {
        Ok(outcome) => outcome,
        Err(message) => {
            emit_error(
                format,
                &format!(
                    "cannot update config file '{}': {}",
                    config_path.display(),
                    message
                ),
            );
            return 1;
        }
    };

    if outcome.changed()
        && let Err(e) = fs::write(&config_path, &outcome.text)
    {
        emit_error(
            format,
            &format!(
                "cannot write config file '{}': {}",
                config_path.display(),
                e
            ),
        );
        return 1;
    }

    for (entry, result) in entries.iter().zip(&outcome.results) {
        emit_info(format, &add_message(entry, *result, &config_path));
    }
    0
}

pub(crate) fn add_message(entry: &AddEntry, result: AddResult, config_path: &Path) -> String {
    let path = entry.path();
    let config = config_path.display();
    match (entry, result) {
        (AddEntry::Existence { kind, .. }, AddResult::Added) => {
            format!("added {} '{}' to '{}'", kind.label(), path, config)
        }
        (AddEntry::Existence { .. }, _) => {
            format!("'{}' is already registered", path)
        }
        (
            AddEntry::Checksum {
                algorithm, digest, ..
            },
            AddResult::Added,
        ) => format!(
            "recorded the {1} checksum of '{0}' in '{2}': {3}",
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
            "updated the {1} checksum of '{0}': {2}",
            path,
            algorithm.as_str(),
            digest
        ),
        (AddEntry::Checksum { algorithm, .. }, AddResult::Skipped) => format!(
            "the {1} checksum of '{0}' already matches the recorded digest",
            path,
            algorithm.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
