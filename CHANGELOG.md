# Changelog

All notable changes to Aura are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — while `0.x`, minor releases may
still change the language.

## [Unreleased]

### Added
- **`enum` (D18)** — a closed set of allowed string values, usable as a schema
  field type. A member stays an ordinary `String`, so output is unchanged; a
  non-member is `E0514` with a did-you-mean suggestion and the member list.
  `pub enum` is exported (D12), and members resolve where the schema is declared,
  so an imported schema validates against its own module's enum.
- **`aura-lang` crate** — the library and the `aura` CLI are now a single
  publishable crate (`cargo add aura-lang` to embed, `cargo install aura-lang`
  for the CLI). Replaces the internal `aura-core` + `aura-cli` split.
- **Language server** (`aura-lsp`, ships in the VS Code extension): live
  diagnostics, context- and type-aware completion, hover, go-to-definition
  (in-file and cross-file to an imported module's member), find-references,
  document symbols, and formatting / format-on-save. Its completion database is
  built by evaluating an Aura manifest with the language itself (dogfooding).

### Changed
- **BREAKING (D17): `component` is removed, and every code body is a scope.**
  A `def` body and a lambda body now accept statements (`=`, `shadow`,
  `assert`), like a module or a `domain` — so a function can finally name a
  subexpression. A list item is written as ordinary properties inside the `map`
  lambda, with `name:` spelled out instead of injected by the keyword:

  ```ruby
  # before
  services: names.map (n, i) ->
    component n
      port: 8000 + i
    end
  end

  # after
  services: names.map (n, i) ->
    name: n
    port: 8000 + i
  end
  ```

  A block in expression position went with it. An object literal
  (`key:` … `end`) stays data-only. Evaluated output is unchanged — the pinned
  conformance snapshots still match byte for byte.
- **`aura fmt` is now a canonical formatter**: besides indentation it
  canonicalizes intra-line spacing and column-aligns runs of `name = value`,
  `key: value`, and `cond` arms together with their trailing comments. Strings
  and block-string interiors are untouched; the token stream is never changed
  and formatting is idempotent (both fuzzed).
- `aura --version` now prints just the crate version (`aura 0.1.0`).

## [0.1.0] — preview

First public preview. Aura compiles readable manifests to JSON/YAML/TOML.

### Language
- Deterministic evaluation; no `now()`/randomness (D13). Durations and dates
  are epoch integers (`parse_duration`/`parse_datetime` and their formatters).
- Capability model (D1): imported modules have no file/env access by default.
- `key:` properties are exported; `x =` bindings are private (D10), enabling
  sound dead-code analysis.
- Schemas with type-checking; optional fields with `= default` (D15).
- Immutability with explicit `shadow` (D7); `Int`/`Float` are distinct (D6).
- `pub def`/`pub type` packages with versioned imports + `aura.lock` (D8/D12).
- Multi-way `cond` (D14), `range(n)`, and `text … end` block strings (D16).
- ~50 stdlib methods across String/Int/Float/Bool/List/Object.

### Tooling
- `aura eval` / `check` / `fmt` / `add`; `--strict`, `--frozen`, `--dry-run`.
- Rich diagnostics (ariadne); JSON/YAML/TOML output.
- Coverage-guided fuzzing (lexer, parser, pipeline, formatter).
- Editor support: TextMate grammar + the `aura-lsp` language server.
