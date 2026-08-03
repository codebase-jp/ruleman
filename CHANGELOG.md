# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) — while
`0.x`, breaking changes bump the minor version.

The section for a released version is used verbatim as the top of that
release's [GitHub release notes](https://github.com/codebase-jp/ruleman/releases).

## [Unreleased]

## [0.2.0] - 2026-08-03

### Added

- `ruleman add <path>...` registers paths that already exist in the repo, so a
  file that's there today can be locked in without hand-editing the config.
  What's on disk decides the rule type: a regular file goes into a `file`
  rule, a directory into a `directory` rule. Paths are stored relative to the
  config file (always with `/` separators), appended to an existing rule with
  the same type/`state`/`severity` when there is one, and paths already
  covered are reported and skipped. `--severity warn|off` picks the severity.
  Edits are applied to the config's JSONC syntax tree, so comments,
  indentation and trailing commas survive.
- `checksum` rule type pins a file's exact bytes by hash, for files that
  should change only deliberately (vendored bundles, generated output).
  `algorithm` selects the digest (`sha256`), and `state: "match"` (default) /
  `"mismatch"` picks the direction. A missing or unreadable file fails too.
- `ruleman add --checksum <file>...` records each file's current digest as a
  `checksum` rule, so a digest never has to be pasted in by hand. Re-running
  it after an intentional edit rewrites the recorded digest in place — that's
  how a pin gets refreshed — and on an unchanged file it writes nothing.
  `--algorithm` selects the digest. `mismatch` rules are never rewritten,
  since they pin a digest the file must *not* have.

### Changed

- **Breaking:** unknown attributes in a config file are now an error instead
  of being silently ignored. A typo like `"stat": "absent"` previously left a
  rule quietly checking less than its author intended. This matches
  `additionalProperties: false` in the published JSON Schema, so editors and
  the CLI now agree. Configs carrying stray attributes will fail until those
  are removed.
- Parse and validation errors name the offending rule by index
  (`rules[2]: unknown field ...`) instead of leaving it to be hunted down.
- A `checksum` rule's `expected` is validated at load time: it must be a bare
  hex digest of the algorithm's width (64 characters for `sha256`, no
  `sha256:` prefix; surrounding whitespace and uppercase are accepted).
  Previously a typo'd digest surfaced much later as a confusing hash
  mismatch.

### Internal

- `src/main.rs` was split into one module per responsibility (`rule`,
  `config`, `check`, `checksum`, `config_edit`, `add`, `init`), with each
  module's tests alongside it.

## [0.1.3] - 2026-07-08

### Added

- `directory` rule type, with an `empty` attribute that optionally requires
  zero or at least one entry.

### Changed

- **Breaking:** `file` with `state: "present"` now requires the path to be a
  regular file — a directory with the same name no longer satisfies it.

## [0.1.2] - 2026-07-08

### Fixed

- Rule file paths are resolved relative to the config file that declares them
  rather than the directory `ruleman` is invoked from, so results no longer
  change depending on where you run it.

## [0.1.1] - 2026-07-08

### Changed

- **Breaking:** rules were redesigned around Ansible's module conventions.
  `file-existence` became `file` with a `state: "present" | "absent"`
  attribute, and `json-match` became `content` with `format` selecting the
  parser and `state: "match" | "mismatch"` the direction.

### Fixed

- The main npm package publishes from an unambiguous path.

### Documentation

- The docs site and JSON Schema URLs point at <https://ruleman.dev/>.

## [0.1.0] - 2026-07-07

### Added

- First release: a static analysis CLI that checks a repository against a
  declarative JSON(C) rule file, distributed on npm as prebuilt native
  binaries for Linux (x64/arm64), macOS (x64/arm64) and Windows (x64).

[unreleased]: https://github.com/codebase-jp/ruleman/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/codebase-jp/ruleman/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/codebase-jp/ruleman/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/codebase-jp/ruleman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/codebase-jp/ruleman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/codebase-jp/ruleman/releases/tag/v0.1.0
