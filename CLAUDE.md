# CLAUDE.md

Guidance for Claude Code (and other agents) working in this repository.

## What this is

`ruleman` is a static analysis CLI, written in Rust, that checks a repository
against a declarative JSON(C) rule file (`ruleman.json` by default). It's
distributed on npm as prebuilt native binaries (one package per OS/arch,
selected via `optionalDependencies`), the same pattern used by esbuild/Biome.

## Repo layout

- `src/main.rs` — the whole CLI (config parsing, rule engine, subcommands).
- `npm/ruleman/` — the main npm package (`ruleman`); `bin/ruleman.js` resolves
  and spawns the right platform binary.
- `npm/platforms/<os-arch>/` — one npm package per target platform; binaries
  are staged here by CI at release time (not committed).
- `npm/scripts/sync-version.mjs` — syncs one version number across
  `Cargo.toml` and every `npm/**/package.json`.
- `docs/` — GitHub Pages site (`docs/index.md`) and the config JSON Schema
  (`docs/schema.json`), served at `https://ruleman.dev/`.
- `.github/workflows/ci.yml` — fmt/clippy/test on push and PR.
- `.github/workflows/release.yml` — on `vX.Y.Z` tag push: builds all
  platforms natively (no cross-compilation toolchains needed — each matrix
  entry runs on a native runner for its target), then `npm publish`s every
  package, then creates a GitHub Release.

## Rule design convention

Follow Ansible's module conventions rather than inventing a new rule type
for every variation — but only for *superficial* variations of the same
check, not genuinely different checks:

- Don't add a mirror rule type for a negated/inverse check (no
  `file-not-existence`). Add a `state` attribute instead
  (`state: "present" | "absent"` on `file`/`directory`, `state: "match" |
  "mismatch"` on `content`) — Ansible reuses `state` across modules with
  per-module enum values (`file`: present/absent, `service`:
  started/stopped), and this project follows the same convention.
- Don't add a mirror rule type per file format either (no `json-match`,
  `yaml-match`, `toml-match`). One `content` rule type takes a `format`
  attribute (`"json"` today; `yaml`/`toml` planned) that selects the parser;
  the dotted-key comparison logic stays shared regardless of format.
- Do add a separate rule type when the check is genuinely different, not
  just inverted or reparameterized — `file` and `directory` are separate
  rule types (not one `file` type with a `kind: "file" | "directory"`
  attribute) because they check different things and accrue different
  attributes over time (`directory` has `empty`; `file` doesn't). Likewise
  `content` is separate from `file`: existence vs. value-inside-a-file are
  different questions. When two checks share only `state`/`severity`-style
  scaffolding but diverge in what they actually assert, prefer separate
  rule types over a combinatorial attribute on one type.
- Keep axes that can vary independently (e.g. `content`'s `format` vs.
  `state`) as separate attributes rather than cross-producing them into one
  enum (no `state: "json-match" | "json-mismatch" | "yaml-match" | ...`).

## Local dev

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets -- -D warnings   # CI fails on any warning
```

## Commit messages

Write commit messages in English, even though conversation with the user
may be in Japanese. Follow [Conventional Commits](https://www.conventionalcommits.org/):
`<type>: <summary>`, e.g. `feat: add mismatch state to content rule`,
`fix: handle missing config file`, `docs: update rule reference`. Common
types: `feat`, `fix`, `docs`, `refactor`, `test`, `ci`, `chore`.
