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
npx ruleman init   # scaffolds ruleman.json
npx ruleman        # runs the checks
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

**`file`** — checks whether listed files exist, Ansible-`file`-module style:
`state: "present"` (default) fails if any file is missing; `state: "absent"`
fails if any file exists.

```jsonc
{ "type": "file", "state": "present", "files": ["README.md"] }
{ "type": "file", "state": "absent", "files": ["yarn.lock"] }
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

Config files may use comments and trailing commas (JSONC).

## CLI

```text
ruleman [--config <path>]     # run checks (default command)
ruleman init [--force]        # scaffold a starter ruleman.json
```

## Building from source

```sh
cargo build --release
./target/release/ruleman --version
```

## License

MIT © Codebase Inc.
