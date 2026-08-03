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
| `extends` | `string[]` | Other ruleman config files to inherit rules from, resolved relative to this file. Cycles are detected and rejected. |
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
{ "type": "file", "state": "absent", "files": ["yarn.lock"] }
```

| Field   | Type                      | Required | Description                                                                                              |
| ------- | ------------------------- | -------- | -------------------------------------------------------------------------------------------------------- |
| `files` | `string[]`                | yes      | Paths to check (repo-relative).                                                                          |
| `state` | `"present"` \| `"absent"` | no       | `"present"` (default) fails if missing or not a regular file; `"absent"` fails if anything exists there. |

### `directory`

The same idea as `file`, for directories — kept as a separate rule type
rather than folded into `file` because file/directory are genuinely
different things to check (not a superficial present/absent-style
variation), and future directory-specific attributes (like `empty` below)
shouldn't leak into `file`'s schema.

```jsonc
{ "type": "directory", "state": "present", "directories": [".github/workflows"] }
{ "type": "directory", "directories": ["dist"], "empty": false }
```

| Field         | Type                      | Required | Description                                                                                                            |
| ------------- | ------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------- |
| `directories` | `string[]`                | yes      | Paths to check (repo-relative).                                                                                        |
| `state`       | `"present"` \| `"absent"` | no       | `"present"` (default) fails if missing or not a directory; `"absent"` fails if anything exists there.                  |
| `empty`       | `boolean`                 | no       | If set, additionally requires zero (`true`) or at least one (`false`) entries. Only checked when `state` is `present`. |

### `content`

Checks a value inside a structured file. Rather than a `json-match` rule
type today and a `yaml-match`/`toml-match` type each time a new format is
supported, `format` selects the parser and the rule type itself stays
`content`. Named to pair with `file`: `file` checks whether a file exists,
`content` checks what's inside it.

`state: "match"` (default) fails unless `key` (a dot-separated path) in
`file` equals `expected`; `state: "mismatch"` fails when it does.

```jsonc
{
  "type": "content",
  "format": "json",
  "file": "package.json",
  "key": "engines.node",
  "expected": ">=18"
}
```

| Field      | Type                      | Required | Description                                                              |
| ---------- | ------------------------- | -------- | ------------------------------------------------------------------------ |
| `format`   | `"json"`                  | no       | Parser to use. `"json"` (default); `yaml`/`toml` planned.                |
| `file`     | `string`                  | yes      | Path to the file.                                                        |
| `key`      | `string`                  | yes      | Dot-separated path into the parsed document.                             |
| `expected` | any                       | yes      | The value `key` is compared against.                                     |
| `state`    | `"match"` \| `"mismatch"` | no       | `"match"` (default) requires equality; `"mismatch"` requires inequality. |

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

Share rules across repos or config files:

```jsonc
// ruleman.json
{
  "extends": ["./base.ruleman.json"],
  "rules": [{ "type": "file", "files": ["CHANGELOG.md"] }]
}
```

Rules from extended files run first, in the order listed, followed by the
file's own rules. `extends` paths are resolved relative to the file that
declares them, and circular references are rejected with an error.

### Path resolution

`file`'s `files`, `content`'s `file`, and `extends` paths are all resolved
relative to the config file that declares them — not the directory
`ruleman` is invoked from. This keeps results consistent whether you run
`ruleman` from the repo root, from a subdirectory (via upward config
discovery), or via `extends` pulling in rules defined in another directory.

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
ruleman [--config <path>]     # run checks (default command)
ruleman init [--force]        # scaffold a starter ruleman.json
ruleman add <path>...         # add existing paths as rules
ruleman --version
ruleman --help
```

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

## License

[MIT](https://github.com/codebase-jp/ruleman/blob/main/LICENSE) © Codebase Inc.
