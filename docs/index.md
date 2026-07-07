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
npx ruleman init   # scaffolds ruleman.json
npx ruleman        # runs the checks
```

```jsonc
// ruleman.json
{
  "$schema": "https://codebase-jp.github.io/ruleman/schema.json",
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

Add `"$schema": "https://codebase-jp.github.io/ruleman/schema.json"` to any
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
case.

```jsonc
{ "type": "file", "state": "present", "files": ["README.md"] }
{ "type": "file", "state": "absent", "files": ["yarn.lock"] }
```

| Field   | Type                      | Required | Description                                                        |
| ------- | ------------------------- | -------- | ------------------------------------------------------------------ |
| `files` | `string[]`                | yes      | Paths to check (repo-relative).                                    |
| `state` | `"present"` \| `"absent"` | no       | `"present"` (default) fails if missing; `"absent"` fails if found. |

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

### Comments and trailing commas

Config files are parsed as JSONC, so comments (`//`, `/* */`) and trailing
commas are allowed.

## CLI reference

```text
ruleman [--config <path>]     # run checks (default command)
ruleman init [--force]        # scaffold a starter ruleman.json
ruleman --version
ruleman --help
```

## Building from source

```sh
cargo build --release
./target/release/ruleman --version
```

## License

[MIT](https://github.com/codebase-jp/ruleman/blob/main/LICENSE) © Codebase Inc.
