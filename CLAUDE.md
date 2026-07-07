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
  (`docs/schema.json`), served at `https://codebase-jp.github.io/ruleman/`.
- `.github/workflows/ci.yml` — fmt/clippy/test on push and PR.
- `.github/workflows/release.yml` — on `vX.Y.Z` tag push: builds all
  platforms natively (no cross-compilation toolchains needed — each matrix
  entry runs on a native runner for its target), then `npm publish`s every
  package, then creates a GitHub Release.

## Rule design convention

When a rule needs a negated/inverse check, don't add a mirror rule type
(e.g. no `file-not-existence`). Follow the Ansible `file` module pattern:
add an attribute that expresses direction/state on the existing rule type
(`state: "present" | "absent"` on `file`, `negate: boolean` on `json-match`).
Keep the attribute name meaningful per rule type rather than forcing one
generic flag across all rule types.

## Local dev

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets -- -D warnings   # CI fails on any warning
```

## Commit messages

Write commit messages in English, even though conversation with the user
may be in Japanese.
