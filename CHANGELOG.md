# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) — while
`0.x`, breaking changes bump the minor version.

The section for a released version is used verbatim as the top of that
release's [GitHub release notes](https://github.com/codebase-jp/ruleman/releases).

## [Unreleased]

## [0.4.0] - 2026-08-03

### Added

- **YAML and TOML** for `content`'s `format`, alongside `json`. All three are
  mapped onto the same JSON-shaped tree, so `key`, `comparison` and `state`
  behave identically whichever format a file is written in — which finally
  makes the most common targets checkable: a GitHub workflow's `jobs.*.steps`,
  a `Cargo.toml`'s `package.edition`, a `docker-compose.yml`'s image tags. The
  mapping is part of the documented contract: types are preserved (YAML `3` is
  a number, not `"3"`), anchors and aliases are resolved, TOML date-times and
  YAML `.inf`/`.nan` become the strings they were written as, and a file
  holding several YAML documents is rejected rather than silently checked as
  its first one.
- **Glob patterns in `content`'s and `checksum`'s `file`.** A pattern there
  reads as "every matching file satisfies this", so one rule covers a whole set
  of packages (`packages/*/package.json`). Matching nothing is a failure — a
  rule about a file's contents has nothing to assert without a file. Before
  this, a pattern was taken as a literal path and reported the misleading
  "file 'packages/*/package.json' is missing".
- Integration tests that run the built binary (`tests/cli.rs`), covering exit
  codes, the three output formats, config discovery and `--config`, and what
  `init` and `add` write to disk. That surface was only ever checked by hand.

### Changed

- A `content` or `checksum` `file` containing `*`, `?`, `[` or `{` is now a
  pattern rather than a literal path — including a real filename that happens
  to contain one, which would now be read as a glob (and rejected at load time
  if it isn't a valid one). `file` and `directory` entries have behaved this way
  since 0.3.0.

## [0.3.0] - 2026-08-03

### Added

- **Glob patterns** in `file`'s `files` and `directory`'s `directories`. An
  entry containing `*`, `?`, `[` or `{` is matched against the working tree;
  anything else stays a literal path, checked with a single `stat` and never
  triggering a directory walk. `*` stops at a path separator and `**` crosses
  one. Dot-prefixed paths are searched (`.github/workflows/*.yml` works) while
  `.git` and anything matched by `.gitignore` are skipped — a rule shouldn't
  fire on a build artifact the repo already ignores. Malformed patterns fail at
  config load time with their rule index.

  A pattern asserts "there is at least one match": `state: "absent"` fails once
  per match, which is how a whole class of paths gets forbidden
  (`files: ["**/*.log"]`), and `state: "present"` fails when nothing matches.
  Note that a `present` pattern is satisfied by *any* match —
  `files: ["packages/*/README.md"]` does not require one per package.
- **`comparison`** on `content`: `equals` (default, unchanged), `contains`
  (substring of a string, element of an array) or `regex`. It's a separate axis
  from `state`, so any comparison composes with `mismatch` — a regex the value
  must *not* match is `comparison: "regex"` plus `state: "mismatch"`. Regexes
  are compiled at config load time, so a malformed pattern or a non-string
  `expected` fails with its rule index instead of silently never matching.
- **`extends` from npm packages.** An entry that isn't a path (`./x`, `/x`) is a
  package name resolved from `node_modules`, walking up from the config file —
  the same shape as eslint's shareable configs. A package name alone uses that
  package's `ruleman.json`; add a subpath to name a specific file. Resolution is
  offline: the package has to be installed, so what a run checks against is
  pinned by the lockfile rather than fetched over the network mid-check. Rules
  from a package check the **consuming** repo — a shared `files: ["LICENSE"]`
  means your LICENSE, not the copy inside `node_modules`.
- **`--format`** (`auto` | `github` | `text` | `json`). `auto`, the default,
  uses GitHub Actions workflow commands when `GITHUB_ACTIONS=true` and plain
  text otherwise, so local runs are finally readable and non-GitHub CI is
  usable. `json` emits one document — `{ "diagnostics": [...], "summary": {...} }`,
  each diagnostic carrying `severity`, `rule`, `file` and `message` — for
  editors and scripts. Config-level failures go through the same channel, so
  `--format json` always emits something parseable. The exit code is unchanged
  in every format: `1` if any error was reported.
- Dot-separated `key`s in `content` index into arrays: `workspaces.0`,
  `contributors.1.name`. They silently resolved to nothing before.

### Changed

- **Breaking:** CLI messages are in English. The tool spoke Japanese while its
  docs, schema and history were English; for a package published publicly with
  English documentation, English messages are the coherent half to keep.
- **Breaking:** output outside GitHub Actions is plain text rather than
  `::error::` workflow commands, which were noise in a local terminal. Pass
  `--format github` to keep the old shape everywhere.
- `content` failures report the value they actually found and the comparison
  that was applied, instead of a generic "validation failed".

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

[unreleased]: https://github.com/codebase-jp/ruleman/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/codebase-jp/ruleman/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/codebase-jp/ruleman/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/codebase-jp/ruleman/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/codebase-jp/ruleman/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/codebase-jp/ruleman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/codebase-jp/ruleman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/codebase-jp/ruleman/releases/tag/v0.1.0
