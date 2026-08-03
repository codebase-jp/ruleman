# CLAUDE.md

Guidance for Claude Code (and other agents) working in this repository.

## What this is

`ruleman` is a static analysis CLI, written in Rust, that checks a repository
against a declarative JSON(C) rule file (`ruleman.json` by default). It's
distributed on npm as prebuilt native binaries (one package per OS/arch,
selected via `optionalDependencies`), the same pattern used by esbuild/Biome.

## Repo layout

- `src/` — one module per responsibility; keep new code in the module that
  owns the concern rather than growing `main.rs` back into the whole CLI:
  - `main.rs` — the clap CLI definition and dispatch, nothing else.
  - `rule.rs` — the rule types a config can declare, plus `validate_rule`.
  - `config.rs` — finding, parsing and resolving config files (`extends`,
    rule paths relative to the declaring file).
  - `check.rs` — the check engine: runs each rule, produces diagnostics.
  - `output.rs` — renders diagnostics as `github`/`text`/`json` and resolves
    `--format auto`. Nothing else may hard-code a CI vendor's syntax, and
    user-facing strings are English (see below).
  - `paths.rs` — expands glob patterns into the paths a rule checks.
  - `checksum.rs` — hashing files and the algorithms `checksum` can name.
  - `document.rs` — parsing JSON/YAML/TOML into the one JSON-shaped tree the
    engine walks. The conversions are deliberate and documented, not delegated
    to a serde bridge: a value's type decides whether `expected` matches it.
  - `config_edit.rs` — rewriting a config file's *text* via the JSONC CST so
    comments and formatting survive. Pure: no filesystem access.
  - `add.rs` / `init.rs` — one module per subcommand, each exposing `run`.
  - `testdata.rs` — `#[cfg(test)]` fixtures shared across modules.
  Tests live in a `#[cfg(test)] mod tests` inside the module they cover.
- `tests/cli.rs` — integration tests that spawn the built binary. Anything
  observable from outside (exit codes, stdout vs stderr, each `--format`, files
  a subcommand writes) belongs here rather than being verified by hand.
- `npm/ruleman/` — the main npm package (`ruleman`); `bin/ruleman.js` resolves
  and spawns the right platform binary.
- `npm/platforms/<os-arch>/` — one npm package per target platform; binaries
  are staged here by CI at release time (not committed).
- `npm/scripts/sync-version.mjs` — syncs one version number across
  `Cargo.toml` and every `npm/**/package.json`.
- `docs/` — GitHub Pages site (`docs/index.md`) and the config JSON Schema
  (`docs/schema.json`), served at `https://ruleman.dev/`.
- `.github/workflows/ci.yml` — fmt/clippy/test on push and PR.
- `CHANGELOG.md` — Keep a Changelog format. The `## [X.Y.Z]` section is
  extracted verbatim as the top of that release's GitHub release notes, so a
  release needs its section written *before* the tag is pushed (the workflow
  fails early, before publishing, if it's missing).
- `.github/workflows/release.yml` — on `vX.Y.Z` tag push: builds all
  platforms natively (no cross-compilation toolchains needed — each matrix
  entry runs on a native runner for its target), then `npm publish`s every
  package, then creates a GitHub Release. The tag is the single source of
  truth for the version — `Cargo.toml` and the `package.json`s stay at
  `0.1.0` in git and are synced from the tag at release time, so don't commit
  version bumps.

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
  attribute (`json`/`yaml`/`toml`) that selects the parser; every format is
  converted to a JSON-shaped tree in `document.rs`, so the dotted-key and
  comparison logic is written once. A new format means a conversion there, not
  a new rule type or a second comparison path.
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
  `comparison` vs. `state`) as separate attributes rather than cross-producing
  them into one enum (no `state: "json-match" | "regex-mismatch" | ...`).
- A glob pattern in `files`/`directories` asserts "there is at least one
  match" — `absent` fails per match, `present` fails only when nothing matches.
  In `content`/`checksum`, where existence isn't the assertion, a pattern reads
  as "every matching file satisfies this" and matching nothing is a failure.
  Asserting "for all" over a *set of directories* ("every package has a
  README") is a different quantifier that no attribute expresses today: a
  `for_each` attribute was built and then removed, because it made `files` mean
  two different things depending on its presence, its no-match case passed
  silently, and enumerating a handful of packages explicitly is clearer. Don't
  re-add it speculatively — wait for a case enumeration can't cover, most
  likely a shared config distributed via `extends` that has to assert something
  about the consuming repo's structure.
- Comparison-style rules decompose into independent axes: *what* is compared
  (the rule type — a key inside a parsed document vs. the whole file's
  digest), *how the reference is supplied* (`checksum`'s `algorithm`,
  `content`'s `format`, and later a URL source), *where the target lives*
  (`file` today; a `url` locator would be a shared attribute, not a new rule
  type), and *direction* (`state`). `state: "match" | "mismatch"` is the
  shared direction axis — no rule type owns the word `match`, and new ways of
  specifying the comparison never extend it.
- Don't add a discriminator attribute naming which of those forms is in use
  (no `source: "literal" | "url"`, no `expected_type: "hash" | "json"`).
  Give each form its own attribute (`expected` vs. `expected_url`) and
  enforce exactly-one-of instead, the way Ansible declares mutually exclusive
  params instead of a mode enum. A discriminator is only needed when one
  attribute has to carry several kinds of value — which is the thing to avoid
  in the first place.
- Specifying both sides of a mutually exclusive pair is an error, never an
  AND: the two attributes fill the same slot, so honouring one silently would
  run a check the author didn't ask for. Enforce it in **both** places —
  `oneOf`/`required` in `docs/schema.json` (editor feedback) and
  `validate_rule` in `src/rule.rs` (the CLI never reads the schema). Anything
  serde's derives can't express — cross-attribute constraints, value formats
  like `checksum`'s hex digest — belongs in `validate_rule`, which runs per
  rule at load time and reports failures as `rules[<index>]: <reason>`.
  Attributes themselves are strict: `Rule` is `deny_unknown_fields`, so a
  typo is an error rather than a silently ignored attribute.

## Conventions

- User-facing strings (CLI messages, `--help`, docs) are English. Only the
  conversation with the user may be Japanese.
- Never `println!`/`eprintln!` a rule failure directly: build a `Diagnostic`
  and let `output::render` decide the shape, so every format stays in sync.
- Nothing resolves anything over the network at check time. `extends` of a
  package reads it from `node_modules`, so a run is reproducible offline and
  pinned by the consuming repo's lockfile.
- Rules from a package-resolved `extends` check the *consuming* repo, not the
  copy in `node_modules` — that's what `load_config_from`'s `inherited_base`
  carries. A shared `files: ["LICENSE"]` has to mean the user's LICENSE.

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
