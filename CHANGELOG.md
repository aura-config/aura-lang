# Changelog

All notable changes to Aura are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — while `0.x`, minor releases may
still change the language.

## [0.1.1] — 2026-07-31

A documentation and diagnostics release. No behaviour of the language itself
changed, and no manifest that worked on 0.1.0 needs touching.

### Fixed

- **The agent reference taught a type that does not exist.** `aura docs --agent`
  and the published `llms.txt` listed `Str` among the built-in types and used it
  in a schema example. `Str` is the Rust variant name; the language accepts only
  `String`. An assistant following the reference wrote a field the parser rejects.

  Fixed at the class rather than the instance: `BUILTIN_TYPE_NAMES` is now the
  one list, the parser matches through it, the reference prints from it, and a
  test fails if `Str` reappears.

- **An unknown field type said "undefined variable".** `a: Str` reported
  `use of undefined variable 'Str'` and pointed at the whole `type` block —
  wording that never mentions types, sending the reader to look for a missing
  binding. It now says `unknown type 'Str'`, points at the type name, lists the
  six built-ins, and suggests the likely one.

  The suggestion needed more than edit distance: `Str` against `String` is three
  insertions, past the typo threshold, so the most probable mistake — an
  abbreviation of the real name — got no hint at all. A prefix relation is tried
  first now.

- **Referring to a property by name explained nothing.** `assert limits.max <= 1`
  where `limits:` is a property reported an undefined variable. True, and
  useless: `:` exports without creating a name, and that is the language's
  central rule, so the diagnostic now teaches it instead of restating the
  symptom. It fires whichever order the reference and the property appear in.

### Documentation

- **How to install Aura was documented nowhere.** The quick start told the reader
  to clone the repository and build from source — the instruction from before
  crates.io, release binaries and `setup-aura` existed. Both READMEs and the book
  now lead with the playground, which needs no install at all.
- The README is rebuilt around what a reader needs first, in both languages:
  installation up front, a sixty-second example, the language tour and the
  verification table folded away, and badges for crates.io and docs.rs.
- The type mapping used by `aura types` is documented for all three targets,
  including the part that only showed up when generating: a schema's `List` and
  `Object` carry no element type, so neither can the generated code.
- Embedding now says to turn default features off — `cargo add aura-lang` locks
  96 packages, and 38 without the `cli` feature the library never uses.
- `SPEC` says `String` where it describes something a user sees, notably the
  `E0512` wording. The lexer's `TokenKind::Str` keeps its name, which is correct.

### Added

- **D19, open and not adopted**: a host can already call a manifest's `pub def`,
  so rules can live in a `.aura` file and be executed by an application per
  request, with no intermediate JSON and no rebuild.
  `cargo run --example scripting` demonstrates it and asserts its own results.
  What is missing is ergonomics, and what is deliberately unresolved is the
  other direction — a script cannot call back into the host, and opening that has
  to be shaped like the capability model rather than bolted beside it.
- The crate documentation now states which surface is supported: `facade` is
  meant to stay stable; the layers beneath are public for tools and may change
  while `0.x`.

## [0.1.0] — 2026-07-29

The first release. Aura compiles readable manifests to JSON, YAML or TOML,
with schemas, assertions that fail the build rather than the deploy, and a
capability model in which `env()` and `read_file()` are granted per run and the
grant does not reach imported modules.

Nothing shipped before this, so there is nothing to migrate from and no
compatibility section. While `0.x`, minor releases may still change the
language.

### Language

- Deterministic evaluation. `now()` and randomness do not exist (D13); durations
  and dates are epoch integers, via `parse_duration`/`parse_datetime` and their
  formatters. The same manifest with the same inputs produces byte-identical
  output.
- `key:` properties are exported, `x =` bindings are private (D10) — which is
  also what makes dead-code analysis sound.
- Schemas with type checking, and optional fields through `= default` (D15).
- **`enum` (D18)**: a closed set of allowed strings, usable as a schema field
  type. A member is an ordinary `String`, so output is unchanged; a non-member is
  `E0514`, with a did-you-mean suggestion and the full member list. `pub enum` is
  exported, and members resolve where the schema is declared, so an imported
  schema validates against its own module's enum.
- Immutability with an explicit `shadow` (D7). `Int` and `Float` are distinct
  types (D6).
- Packages: `pub def` / `pub type`, versioned imports, and `aura.lock` (D8/D12).
  An exported function runs with its *origin* module's capabilities, so
  isolation cannot be borrowed by asking the caller to invoke it.
- Multi-way `cond` (D14), `range(n)`, and `text … end` block strings (D16), whose
  interiors interpolate with `#{}` while leaving braces alone — which is what
  makes generating nginx configs and Dockerfiles practical.
- Every code body is a scope (D17): a `def` or lambda body takes statements
  (`=`, `shadow`, `assert`) exactly as a module does.
- Sixty standard-library methods across String, Int, Float, Bool, List and
  Object.

### Tooling

- `aura eval` / `check` / `fmt` / `types` / `add` / `docs`, with `--strict`,
  `--frozen`, `--hermetic` and `--dry-run`. Rich diagnostics through ariadne;
  JSON, flattened JSON, YAML and TOML output.
- **`aura fmt` is a canonical formatter**: indentation, intra-line spacing, and
  column-aligned runs of `name = value`, `key: value` and `cond` arms together
  with their trailing comments. Strings and block-string interiors are untouched,
  the token stream never changes, and formatting is idempotent — both fuzzed.
- **`aura types`** generates host-language types from a manifest's `type` and
  `enum` declarations (`--lang rust|ts|go`), so one schema both validates the
  config and types the service consuming its JSON. Parsing only: no evaluation,
  no capabilities, deterministic output, already canonical for `rustfmt`,
  `gofmt` and `prettier`.
- **`aura docs --agent`** prints the complete language reference — syntax,
  standard library, every diagnostic code — assembled from the compiler's own
  definitions, at roughly four thousand tokens. Also published as
  [`llms.txt`](https://aura-config.github.io/aura-lang/llms.txt).
- **Language server** (`aura-lsp`): live diagnostics, context- and type-aware
  completion, hover, go-to-definition in-file and across modules, find
  references, document symbols, rename (F2), signature help, and
  formatting/format-on-save. Its completion database is built by evaluating an
  Aura manifest with Aura itself.
  - Rename rests on scope-precise resolution, so `x` in a lambda and `x` at the
    top level are never conflated and uses inside `#{…}` are found. It refuses,
    with a reason, rather than guessing: on a file with syntax errors, on
    anything that is not a binding, on a malformed name, on a name already bound
    in an overlapping scope, and on `pub` items whose importers it cannot see. A
    test renames every binding in `examples/showcase` and asserts the evaluated
    JSON stays byte-identical.
  - Inside `new lib.Endpoint`, a field typed by an imported `pub enum` offers
    exactly its members. Go-to-definition on a registry import opens the cached
    package through the same resolver evaluation uses, so it cannot land on a
    different file than the one that would be evaluated.
- **A browser playground** — the real compiler as WebAssembly, multi-file, with
  `aura fmt` and diagnostics. Nothing is installed and nothing leaves the page.
- Editor support: TextMate grammar for VS Code, plus Vim/Neovim and nano.
- The library and the CLI are one publishable crate: `cargo add aura-lang` to
  embed, `cargo install aura-lang` for the tool. The `cli` feature is
  detachable, which is what keeps the library free of clap, ariadne and ureq and
  able to build for wasm.

### Security and supply chain

- **`--hermetic`** turns `env()` and `read_file()` into `E0505` in every module.
  Because that is an analysis error, `aura check --hermetic` proves a manifest
  performs no I/O without evaluating it — including for branches a given run
  would not take.
- Capability refusals distinguish their causes: `E0310` when nothing was
  granted, `E0311` when a grant exists but the path resolves outside it, with
  the allowed directories named.
- `aura.lock` pins a package's exact version and a hash of its **token stream**
  rather than its bytes, so reformatting or editing a comment does not fire the
  integrity check while any change that can alter behaviour does. Locks written
  with the older byte hash still verify, and are upgraded in place.
- Six coverage-guided fuzz targets: lexer, parser, pipeline, formatter, codegen
  and resolver.

### Distribution

- Binaries for six targets on a tag — Linux gnu and musl, aarch64 Linux, macOS
  Intel and Apple silicon, Windows — each with a `.sha256`.
- **[`aura-config/setup-aura@v1`](https://github.com/aura-config/setup-aura)**
  installs the CLI in GitHub Actions, verifying the checksum before unpacking.
  On x86_64 Linux it installs the musl build: the gnu build requires
  `GLIBC_2.34` and does not start on Ubuntu 20.04, Debian 11, CentOS 8 or Amazon
  Linux 2, while the static build has no such floor and is measurably faster.
- `packaging/e2e.sh` drives the built binary through the claims this
  documentation makes — exit codes, capability refusals, hermetic mode, output
  formats, and byte-identical output across two runs. It runs on three operating
  systems per push, and on a tag against the real artifacts inside five
  containers, including aarch64 under emulation.

### Performance

Measured on x86_64 Linux; re-run `cargo bench -p aura-lang` on your own machine.

- 258 MiB/s lexing, 178 MiB/s through the parser, **33 µs** for a full
  lex-parse-evaluate of the reference manifest.
- The binary is 3.5 MB, a 1.7 MB download, with link-time optimisation and no
  symbol table. LTO is not a size-against-speed trade here: it made the binary
  17% smaller *and* about 9% faster.
