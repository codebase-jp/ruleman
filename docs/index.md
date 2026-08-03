# ruleman

A static analysis CLI for repositories. Point it at a declarative JSON(C)
rule file and it checks things like "does this file exist" or "does this
JSON file have the expected value at this key" — useful for enforcing
repo-wide conventions in CI.

[View on GitHub](https://github.com/codebase-jp/ruleman) ·
[View on npm](https://www.npmjs.com/package/ruleman)

## Install

```sh
npm install --save-dev ruleman
# or run once without installing
npx ruleman
```

Prebuilt native binaries are published for Linux (x64/arm64), macOS
(x64/arm64), and Windows (x64) — no Rust toolchain required on install.

## Quick start

```sh
npx ruleman init             # scaffolds ruleman.json
npx ruleman add README.md    # adds an existing file as a rule
npx ruleman                  # runs the checks
```

```jsonc
// ruleman.json
{
  "$schema": "https://ruleman.dev/schema.json",
  "rules": [
    {
      "type": "file",
      "severity": "error",
      "state": "present",
      "files": ["README.md", "LICENSE"]
    },
    {
      "type": "content",
      "severity": "warn",
      "format": "json",
      "file": "tsconfig.json",
      "key": "compilerOptions.strict",
      "expected": true
    }
  ]
}
```

Running `ruleman` with no arguments auto-discovers `ruleman.json` /
`ruleman.jsonc` / `.ruleman.json`, searching the current directory and
walking up — the same pattern used by eslint, prettier, and biome. Pass
`--config <path>` to point at a specific file instead.

## Config reference

Add `"$schema": "https://ruleman.dev/schema.json"` to any
config file to get autocomplete and validation in editors that support the
`$schema` convention (VS Code, JetBrains IDEs, etc.).

| Field     | Type       | Description                                                                                                         |
| --------- | ---------- | --------------------------------------------------------------------------------------------------------------------- |
| `$schema` | `string`   | Optional; points editors at the JSON Schema.                                                                        |
| `extends` | `string[]` | Other ruleman configs to inherit rules from: a path, or an installed npm package. Cycles are detected and rejected. |
| `rules`   | `Rule[]`   | The checks to run, in order.                                                                                         |

Every rule accepts a `severity`:

- `"error"` (default) — failure exits non-zero.
- `"warn"` — reported, but the run still exits `0`.
- `"off"` — the rule is skipped entirely.

### `file`

Named and shaped after Ansible's `file` module: checks file presence via a
`state` attribute rather than inventing a mirror rule type for the negated
case. `state: "present"` requires the path to exist **and be a regular
file** — a directory with the same name does not satisfy it.

```jsonc
{ "type": "file", "state": "present", "files": ["README.md"] }
{ "type": "file", "state": "absent", "files": ["yarn.lock", "**/*.log"] }
{ "type": "file", "for_each": "packages/*", "files": ["README.md"] }
```

| Field      | Type                      | Required | Description                                                                                              |
| ---------- | ------------------------- | -------- | -------------------------------------------------------------------------------------------------------- |
| `files`    | `string[]`                | yes      | Paths to check (repo-relative). An entry containing `*`, `?`, `[` or `{` is a [glob pattern](#globs).    |
| `state`    | `"present"` \| `"absent"` | no       | `"present"` (default) fails if missing or not a regular file; `"absent"` fails if anything exists there. |
| `for_each` | `string`                  | no       | A glob matching directories; each `files` entry is checked inside every match. See [globs](#globs).      |

### `directory`

The same idea as `file`, for directories — kept as a separate rule type
rather than folded into `file` because file/directory are genuinely
different things to check (not a superficial present/absent-style
variation), and future directory-specific attributes (like `empty` below)
shouldn't leak into `file`'s schema.

```jsonc
{ "type": "directory", "state": "present", "directories": [".github/workflows"] }
{ "type": "directory", "directories": ["dist"], "empty": false }
{ "type": "directory", "state": "absent", "for_each": "packages/*", "directories": ["node_modules"] }
```

| Field         | Type                      | Required | Description                                                                                                            |
| ------------- | ------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------- |
| `directories` | `string[]`                | yes      | Paths to check (repo-relative). An entry with `*`, `?`, `[` or `{` is a [glob pattern](#globs).                        |
| `state`       | `"present"` \| `"absent"` | no       | `"present"` (default) fails if missing or not a directory; `"absent"` fails if anything exists there.                  |
| `empty`       | `boolean`                 | no       | If set, additionally requires zero (`true`) or at least one (`false`) entries. Only checked when `state` is `present`. |
| `for_each`    | `string`                  | no       | A glob matching directories; each `directories` entry is checked inside every match. See [globs](#globs).              |

### `content`

Checks a value inside a structured file. Rather than a `json-match` rule
type today and a `yaml-match`/`toml-match` type each time a new format is
supported, `format` selects the parser and the rule type itself stays
`content`. Named to pair with `file`: `file` checks whether a file exists,
`content` checks what's inside it.

`comparison` decides *how* the value at `key` is compared with `expected`, and
`state` decides *whether that comparison has to hold* — two independent axes, so
every comparison also works negated:

```jsonc
// engines.node is exactly ">=18"
{ "type": "content", "file": "package.json", "key": "engines.node", "expected": ">=18" }

// engines.node mentions 18 somewhere
{ "type": "content", "comparison": "contains", "file": "package.json",
  "key": "engines.node", "expected": "18" }

// workspaces lists packages/*
{ "type": "content", "comparison": "contains", "file": "package.json",
  "key": "workspaces", "expected": "packages/*" }

// the package name is scoped
{ "type": "content", "comparison": "regex", "file": "package.json",
  "key": "name", "expected": "^@acme/" }

// ...and must not be published as UNLICENSED
{ "type": "content", "comparison": "regex", "state": "mismatch",
  "file": "package.json", "key": "license", "expected": "^UNLICENSED$" }
```

`key` is a dot-separated path, and a numeric segment indexes into an array:
`workspaces.0`, `contributors.1.name`. A key that isn't there fails every
comparison.

| Field        | Type                                      | Required | Description                                                                                       |
| ------------ | ----------------------------------------- | -------- | ------------------------------------------------------------------------------------------------- |
| `format`     | `"json"`                                  | no       | Parser to use. `"json"` (default); `yaml`/`toml` planned.                                          |
| `file`       | `string`                                  | yes      | Path to the file.                                                                                 |
| `key`        | `string`                                  | yes      | Dot-separated path into the parsed document; numeric segments index arrays.                        |
| `expected`   | any                                       | yes      | The value `key` is compared against. Must be a string when `comparison` is `"regex"`.             |
| `comparison` | `"equals"` \| `"contains"` \| `"regex"`   | no       | `"equals"` (default) deep equality; `"contains"` substring of a string or element of an array; `"regex"` the value must match. |
| `state`      | `"match"` \| `"mismatch"`                 | no       | `"match"` (default) requires the comparison to hold; `"mismatch"` requires it to fail.            |

### `checksum`

Pins a file's exact bytes by hash — for files that are supposed to change only
deliberately (vendored bundles, generated output, a CI workflow you don't want
edited casually). Separate from `content` because it asserts a different thing:
`content` reads a value out of a parsed document, `checksum` compares the whole
file's digest and doesn't care about its format.

`state: "match"` (default) fails unless the file's digest equals `expected`;
`state: "mismatch"` fails when it does (pinning a digest the file must *not*
have, e.g. a known-bad revision). A missing or unreadable file fails too.

```jsonc
{
  "type": "checksum",
  "algorithm": "sha256",
  "file": "vendor/lib.js",
  "expected": "3879a5d930ae1999b278a3a498f7de3fd83ba8dae59330fcfa2db31c103ac21d"
}
```

Record the digest with [`ruleman add --checksum`](#add) rather than by hand,
and re-run the same command to refresh it after an intentional change.

| Field       | Type                      | Required | Description                                                                                   |
| ----------- | ------------------------- | -------- | --------------------------------------------------------------------------------------------- |
| `algorithm` | `"sha256"`                | no       | Hash algorithm. `"sha256"` (default); more can be added later.                                |
| `file`      | `string`                  | yes      | Path to the file to hash.                                                                     |
| `expected`  | `string`                  | yes      | Hex digest, compared case-insensitively.                                                      |
| `state`     | `"match"` \| `"mismatch"` | no       | `"match"` (default) requires the digest to equal `expected`; `"mismatch"` requires it not to. |

### `extends`

Share rules across config files, or across repos:

```jsonc
// ruleman.json
{
  "extends": [
    "./base.ruleman.json",        // a file in this repo
    "@acme/ruleman-config",       // an installed npm package
    "@acme/ruleman-config/strict.jsonc"  // a specific file inside one
  ],
  "rules": [{ "type": "file", "files": ["CHANGELOG.md"] }]
}
```

Rules from extended files run first, in the order listed, followed by the
file's own rules. Circular references are rejected with an error.

An entry starting with `.` or `/` is a path, resolved relative to the file that
declares it. Anything else is an npm package name, looked up in `node_modules`
walking up from the config file — the same shape as eslint's shareable configs
and tsconfig's `extends`. A package name alone uses that package's
`ruleman.json` / `ruleman.jsonc` / `.ruleman.json`; add a subpath to name a
specific file.

Package resolution is **offline**: the package has to be installed, so what a
run checks against is pinned by your lockfile rather than fetched over the
network mid-check. If it isn't installed, the run says so rather than reporting
a confusing missing-file error.

Rules from a package check the **consuming** repo. A shared
`files: ["LICENSE"]` means your LICENSE, not the one inside `node_modules` —
so a package's rules resolve against the config that extended it, all the way
down the package's own `extends` chain.

### Path resolution

`file`'s `files`, `directory`'s `directories`, `for_each`, `content`'s and
`checksum`'s `file`, and path-shaped `extends` entries are all resolved
relative to the config file that declares them — not the directory `ruleman` is
invoked from. This keeps results consistent whether you run `ruleman` from the
repo root, from a subdirectory (via upward config discovery), or via `extends`
pulling in rules defined elsewhere. The exception is entries under `for_each`,
which are relative to each directory it matches, and rules from an extended
package, which resolve against the consuming repo.

### Globs

An entry in `files` / `directories` containing `*`, `?`, `[` or `{` is a glob
pattern matched against the working tree; anything else is a literal path,
which is checked with a single `stat` and never triggers a directory walk.

- `*` matches within one path segment; `**` crosses segments.
  `*.log` matches `debug.log` but not `logs/debug.log`; `**/*.log` matches both.
- Dot-prefixed paths are searched, so `.github/workflows/*.yml` works.
- `.git` and anything matched by `.gitignore` are skipped — a rule shouldn't
  fire on a build artifact the repo already ignores.
- A malformed pattern is a config error, reported with its rule index before
  any check runs.

Patterns and `for_each` express the two different quantifiers, which is why
they are separate attributes rather than one overloaded pattern:

```jsonc
// "there is at least one": passes as soon as any package has a README
{ "type": "file", "files": ["packages/*/README.md"] }

// "for all": fails once per package that is missing one
{ "type": "file", "for_each": "packages/*", "files": ["README.md"] }
```

A pattern that matches nothing fails `state: "present"` (there is no match to
satisfy it) and satisfies `state: "absent"` (there is nothing to forbid). A
`for_each` that matches no directory is vacuously true — nothing to check — so
pair it with a `directory` rule if the directory itself is required.

### Comments and trailing commas

Config files are parsed as JSONC, so comments (`//`, `/* */`) and trailing
commas are allowed.

### Config validation

The config is validated before any check runs, and anything it can't make
sense of is an error rather than something quietly ignored:

- **Unknown attributes are rejected.** A misspelled attribute would otherwise
  be dropped, leaving a rule that passes while checking less than intended —
  `{ "type": "file", "files": ["a"], "stat": "absent" }` looks like it
  forbids the file but would silently require it. This mirrors
  `additionalProperties: false` in the JSON Schema, so editors and the CLI
  agree.
- **Malformed values are rejected**, including `checksum`'s `expected`, which
  must be a bare hex digest of the algorithm's width (64 characters for
  sha256 — no `sha256:` prefix, surrounding whitespace and uppercase are
  fine). A typo'd digest would otherwise surface much later as a confusing
  hash mismatch.
- Errors name the offending rule by index:

```text
::error::[ruleman] 設定ファイル 'ruleman.json' の解析に失敗しました: rules[2]: unknown field `stat`, expected one of `severity`, `state`, `files`
```

Attributes that fill the same slot are mutually exclusive rather than
combined — specifying both is an error, not an AND.

## CLI reference

```text
ruleman [--config <path>] [--format <fmt>]   # run checks (default command)
ruleman init [--force]                       # scaffold a starter ruleman.json
ruleman add <path>...                        # add existing paths as rules
ruleman --version
ruleman --help
```

### `--format`

How results are reported. Defaults to `auto`.

| Value    | Output                                                                                       |
| -------- | -------------------------------------------------------------------------------------------- |
| `auto`   | `github` when `GITHUB_ACTIONS=true`, `text` otherwise.                                       |
| `github` | GitHub Actions workflow commands (`::error file=...::`), surfaced as annotations on the run. |
| `text`   | One `error:` / `warning:` line per failure, plus a count. Readable in any terminal or CI.    |
| `json`   | A single JSON document: `{ "diagnostics": [...], "summary": { "errors": n, "warnings": n } }`. |

Each diagnostic carries `severity`, `rule` (the rule type that produced it),
`file` (the path it's about, or `null`) and `message`. Config-level failures are
reported the same way, so `--format json` always emits one parseable document:

```sh
ruleman --format json | jq '.diagnostics[] | select(.severity == "error") | .file'
```

The exit code is `1` if any error was reported and `0` otherwise, in every
format — `warn` never fails the run.

### `add`

Registers paths that already exist in the repo, so you don't have to hand-edit
the config to lock in a file that's there today:

```sh
ruleman add README.md .github/workflows   # one file rule, one directory rule
ruleman add --severity warn CHANGELOG.md
ruleman add --checksum vendor/lib.js      # pin the file's current hash
```

With no options it writes an existence check — `state: "present"` at
`severity: "error"`. Each path must exist, and what's on disk decides the rule
type: a regular file goes into a `file` rule's `files`, a directory into a
`directory` rule's `directories`.

Paths are appended to an existing rule with the same type, `state` and
`severity` when there is one, otherwise a new rule is appended to `rules`.
Paths already covered by a matching `present` rule are reported and skipped.

#### `--checksum`

Hashes each file as it is right now and writes a [`checksum`](#checksum) rule
instead of an existence rule — the recording half of hash pinning, so the
digest never has to be pasted in by hand:

```sh
ruleman add --checksum vendor/lib.js schema.graphql
ruleman add --checksum --algorithm sha256 vendor/lib.js   # sha256 is the default
```

Directories are rejected. When a `match` rule for the same file and algorithm
already exists, its `expected` is rewritten in place rather than duplicated —
so after an intentional edit, re-running the same command is how you refresh
the pin. Re-running it on an unchanged file reports that and writes nothing.
`mismatch` rules are never rewritten, since they pin a digest the file must
*not* have.

Arguments are interpreted relative to the current directory but stored relative
to the config file (with `/` separators), matching how rule paths resolve at
check time — so `ruleman add main.rs` from `src/` writes `src/main.rs` into a
config at the repo root. Paths outside the config file's directory are
rejected.

Edits are applied to the config's syntax tree rather than reserialized, so
comments, indentation and trailing commas are preserved.

## Building from source

```sh
cargo build --release
./target/release/ruleman --version
```

## Changelog

[CHANGELOG.md](https://github.com/codebase-jp/ruleman/blob/main/CHANGELOG.md)
records what changed in each release, including the breaking changes to watch
for when upgrading.

## License

[MIT](https://github.com/codebase-jp/ruleman/blob/main/LICENSE) © Codebase Inc.
