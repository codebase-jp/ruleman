# ruleman

[![npm version](https://img.shields.io/npm/v/ruleman.svg)](https://www.npmjs.com/package/ruleman)
[![CI](https://github.com/codebase-jp/ruleman/actions/workflows/ci.yml/badge.svg)](https://github.com/codebase-jp/ruleman/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`ruleman` is a static analysis CLI for repositories. Point it at a declarative
JSON(C) rule file and it checks things like "does this file exist" or "does
this JSON file have the expected value at this key" — useful for enforcing
repo-wide conventions in CI.

Full documentation: **[ruleman.dev](https://ruleman.dev/)**

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

`ruleman.json`:

```jsonc
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

Running `ruleman` (no arguments) auto-discovers `ruleman.json` /
`ruleman.jsonc` / `.ruleman.json`, searching the current directory and
walking up — like `eslint`/`prettier`/`biome`. Pass `--config <path>` to
override discovery.

## Config reference

| Field     | Type       | Description                                                                                                         |
| --------- | ---------- | ------------------------------------------------------------------------------------------------------------------- |
| `$schema` | `string`   | Optional; points editors at the JSON Schema for autocomplete/validation.                                            |
| `extends` | `string[]` | Other ruleman config files to inherit rules from, resolved relative to this file. Cycles are detected and rejected. |
| `rules`   | `Rule[]`   | The checks to run, in order.                                                                                        |

Every rule accepts a `severity`: `"error"` (default, fails the run),
`"warn"` (reported but exit code stays 0), or `"off"` (skipped).

**`file`** — checks whether listed paths exist **as regular files**,
Ansible-`file`-module style: `state: "present"` (default) fails if any is
missing or is actually a directory; `state: "absent"` fails if any exists
(as anything).

```jsonc
{ "type": "file", "state": "present", "files": ["README.md"] }
{ "type": "file", "state": "absent", "files": ["yarn.lock"] }
```

**`directory`** — same idea, for directories. `empty` optionally requires
the directory to have zero (`true`) or at least one (`false`) entries;
omit it to skip the check.

```jsonc
{ "type": "directory", "state": "present", "directories": [".github/workflows"] }
{ "type": "directory", "directories": ["dist"], "empty": false }
```

**`content`** — checks a value inside a structured file. `format` selects
the parser (currently `"json"`; `yaml`/`toml` planned). `state: "match"`
(default) fails unless `key` (dot-separated path) equals `expected`;
`state: "mismatch"` fails when it does.

```jsonc
{
  "type": "content",
  "format": "json",
  "file": "package.json",
  "key": "engines.node",
  "expected": ">=18"
}
```

**`checksum`** — pins a file's exact bytes by hash, for files that should
change only deliberately. `algorithm` selects the digest (currently
`"sha256"`). `state: "match"` (default) fails unless the file's digest
equals `expected`; `state: "mismatch"` fails when it does. Record and
refresh the digest with `ruleman add --checksum <file>` instead of pasting
it by hand.

```jsonc
{
  "type": "checksum",
  "algorithm": "sha256",
  "file": "vendor/lib.js",
  "expected": "3879a5d930ae1999b278a3a498f7de3fd83ba8dae59330fcfa2db31c103ac21d"
}
```

All file paths (`file`'s `files`, `directory`'s `directories`, `content`'s
and `checksum`'s `file`, and `extends`) are resolved relative to the config
file that declares them — not the directory `ruleman` is run from — so
results don't change depending on where you invoke it.

Config files may use comments and trailing commas (JSONC).

The config is validated before any check runs. Unknown attributes are an
error rather than silently ignored — a typo like `"stat": "absent"` would
otherwise leave a rule quietly checking less than intended — as are malformed
values such as a `checksum` `expected` that isn't a bare 64-character hex
digest. Failures name the rule by index: `rules[2]: unknown field ...`.

## CLI

```text
ruleman [--config <path>]     # run checks (default command)
ruleman init [--force]        # scaffold a starter ruleman.json
ruleman add <path>...         # add existing paths as rules
ruleman add --checksum <file>...   # ...pinning their current hash instead
```

`add` registers paths that already exist in the repo. With no options it writes
an existence check (`state: "present"`, `severity: "error"`); pass
`--severity warn|off` to change that. What's on disk decides the rule type —
files go into a `file` rule, directories into a `directory` rule. Paths are
stored relative to the config file, merged into a matching existing rule when
there is one, and the config's comments and formatting are preserved.

`--checksum` writes a `checksum` rule with the file's current digest instead.
Re-running it after an intentional edit rewrites the recorded digest in place,
which is how a pin gets refreshed; on an unchanged file it writes nothing.

## Building from source

```sh
cargo build --release
./target/release/ruleman --version
```

## License

MIT © Codebase Inc.
