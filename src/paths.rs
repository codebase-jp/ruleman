//! Turning a rule's path patterns into the concrete paths to check.
//!
//! A pattern without glob metacharacters is a literal path and is never walked
//! for — that keeps the common case a single `stat` and keeps configs written
//! before globs existed behaving exactly as they did.

use globset::GlobBuilder;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

const META: &[char] = &['*', '?', '[', ']', '{', '}'];

pub(crate) fn is_pattern(value: &str) -> bool {
    value.contains(META)
}

/// What a rule's `files` / `directories` entry resolved to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A literal path, checked whether or not it exists.
    Literal(String),
    /// A pattern and the paths it matched, which may be none.
    Matched { pattern: String, paths: Vec<String> },
}

/// Resolves one `files` / `directories` entry into the targets to check.
///
/// With `for_each` set, the entry is resolved inside every directory that
/// `for_each` matches, yielding one target per directory — "for all", so no
/// matching directory means nothing to check. Without it, exactly one target
/// comes back.
pub(crate) fn resolve(entry: &str, for_each: Option<&str>) -> Result<Vec<Target>, String> {
    let Some(for_each) = for_each else {
        return Ok(vec![target(entry)?]);
    };

    let mut targets = Vec::new();
    for dir in expand(for_each)? {
        if Path::new(&dir).is_dir() {
            targets.push(target(&format!("{}/{}", dir, entry))?);
        }
    }
    Ok(targets)
}

fn target(entry: &str) -> Result<Target, String> {
    if is_pattern(entry) {
        Ok(Target::Matched {
            pattern: entry.to_string(),
            paths: expand(entry)?,
        })
    } else {
        Ok(Target::Literal(entry.to_string()))
    }
}

/// Every path in the working tree matching `pattern`, sorted so reports are
/// stable. Dot-prefixed paths are included — `.github/**` has to be matchable
/// — but `.git` and anything gitignored is not: a rule shouldn't fire on a
/// build artifact the repo already ignores.
pub(crate) fn expand(pattern: &str) -> Result<Vec<String>, String> {
    let normalized = normalize_separators(pattern);
    let matcher = compile(&normalized)?;
    let root = walk_root(&normalized);

    let mut matches = Vec::new();
    let walker = WalkBuilder::new(&root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        let candidate = normalize_separators(&entry.path().to_string_lossy());
        let candidate = candidate.strip_prefix("./").unwrap_or(&candidate);
        if matcher.is_match(candidate) {
            matches.push(candidate.to_string());
        }
    }
    matches.sort();
    Ok(matches)
}

/// Compiles a pattern, rejecting the invalid ones. `literal_separator` gives
/// the semantics people expect from a path glob: `*` stops at `/`, and `**` is
/// what crosses directories.
pub(crate) fn compile(pattern: &str) -> Result<globset::GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|e| format!("invalid pattern '{}': {}", pattern, e))
}

/// Walks only what the pattern can actually match: everything up to its first
/// globbed component. `packages/*/README.md` walks `packages`, not the repo.
fn walk_root(pattern: &str) -> PathBuf {
    let mut root = PathBuf::new();
    // Walk components rather than splitting on '/', so a leading separator or
    // `..` survives instead of turning an absolute pattern into a relative one.
    for component in Path::new(pattern).components() {
        if is_pattern(&component.as_os_str().to_string_lossy()) {
            break;
        }
        root.push(component);
    }
    if root.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        root
    }
}

/// Rewrites `\` to `/` on Windows, where `Path::join` produces backslashes but
/// glob syntax reads them as escapes. On Unix a backslash is a legal filename
/// character, so it is left alone.
fn normalize_separators(value: &str) -> String {
    if cfg!(windows) {
        value.replace('\\', "/")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn only_metacharacters_make_a_pattern() {
        assert!(!is_pattern("README.md"));
        assert!(!is_pattern("docs/index.md"));
        assert!(is_pattern("*.log"));
        assert!(is_pattern("packages/*/README.md"));
        assert!(is_pattern("**/*.rs"));
        assert!(is_pattern("file.{js,ts}"));
    }

    #[test]
    fn walk_root_stops_at_the_first_globbed_segment() {
        assert_eq!(walk_root("packages/*/README.md"), PathBuf::from("packages"));
        assert_eq!(walk_root("*.log"), PathBuf::from("."));
        assert_eq!(walk_root("**/*.log"), PathBuf::from("."));
        assert_eq!(walk_root("a/b/c.txt"), PathBuf::from("a/b/c.txt"));
        assert_eq!(walk_root("../pkg/*.log"), PathBuf::from("../pkg"));
    }

    #[test]
    fn star_stops_at_a_separator_but_doublestar_crosses_it() {
        let single = compile("*.log").unwrap();
        assert!(single.is_match("a.log"));
        assert!(!single.is_match("sub/a.log"));

        let recursive = compile("**/*.log").unwrap();
        assert!(recursive.is_match("a.log"));
        assert!(recursive.is_match("sub/a.log"));
        assert!(recursive.is_match("sub/deep/a.log"));

        let scoped = compile("packages/*/README.md").unwrap();
        assert!(scoped.is_match("packages/a/README.md"));
        assert!(!scoped.is_match("packages/a/b/README.md"));
        assert!(!scoped.is_match("README.md"));
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        assert!(compile("a[").is_err());
    }

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expand_finds_matches_and_skips_ignored_paths() {
        let dir = sandbox("ruleman_test_expand");
        fs::create_dir_all(dir.join("packages/a")).unwrap();
        fs::create_dir_all(dir.join("packages/b")).unwrap();
        fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        fs::create_dir_all(dir.join("dist")).unwrap();
        fs::write(dir.join("packages/a/README.md"), "").unwrap();
        fs::write(dir.join("packages/b/README.md"), "").unwrap();
        fs::write(dir.join(".github/workflows/ci.yml"), "").unwrap();
        fs::write(dir.join("dist/debug.log"), "").unwrap();
        fs::write(dir.join("keep.log"), "").unwrap();
        fs::write(dir.join(".gitignore"), "dist/\n").unwrap();

        let base = dir.to_string_lossy().into_owned();
        let matches = expand(&format!("{}/packages/*/README.md", base)).unwrap();
        assert_eq!(matches.len(), 2, "{:?}", matches);
        assert!(matches[0].ends_with("packages/a/README.md"));

        // Dot-prefixed directories are searched...
        let workflows = expand(&format!("{}/.github/workflows/*.yml", base)).unwrap();
        assert_eq!(workflows.len(), 1);

        // ...but gitignored ones are not.
        let logs = expand(&format!("{}/**/*.log", base)).unwrap();
        assert_eq!(logs.len(), 1, "{:?}", logs);
        assert!(logs[0].ends_with("keep.log"));

        // A pattern under a directory that doesn't exist simply matches nothing.
        assert!(expand(&format!("{}/nope/*.txt", base)).unwrap().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn for_each_resolves_the_entry_inside_every_matching_directory() {
        let dir = sandbox("ruleman_test_for_each");
        fs::create_dir_all(dir.join("packages/a")).unwrap();
        fs::create_dir_all(dir.join("packages/b")).unwrap();
        fs::write(dir.join("packages/a/README.md"), "").unwrap();
        let base = dir.to_string_lossy().into_owned();

        let targets = resolve("README.md", Some(&format!("{}/packages/*", base))).unwrap();
        assert_eq!(targets.len(), 2, "{:?}", targets);
        // Both packages are checked, including the one missing the file.
        let literals: Vec<_> = targets
            .iter()
            .map(|t| match t {
                Target::Literal(path) => path.clone(),
                other => panic!("unexpected target {:?}", other),
            })
            .collect();
        assert!(literals.iter().any(|p| p.ends_with("packages/a/README.md")));
        assert!(literals.iter().any(|p| p.ends_with("packages/b/README.md")));

        // "For all" over nothing is vacuously true: no directories, no targets.
        assert!(
            resolve("README.md", Some(&format!("{}/apps/*", base)))
                .unwrap()
                .is_empty()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_literal_entry_resolves_to_itself_without_touching_the_disk() {
        assert_eq!(
            resolve("does/not/exist.md", None).unwrap(),
            vec![Target::Literal("does/not/exist.md".to_string())]
        );
    }
}
