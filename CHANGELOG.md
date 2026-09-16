# Changelog

All notable changes to Aura are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — while `0.x`, minor releases may
still change the language.

## [Unreleased]

### Added

- **Digit separators in numbers (D29).** `max_bytes: 10_000_000`, and in the
  fractional part too, where groups run away from the point: `0.000_001`.

  Configuration is where long numbers actually live — byte limits, quotas,
  retention windows. Miscounting a zero is the kind of mistake nothing else
  catches: the value is valid, the type is right, and the service quietly gets
  ten times the memory it should.

  It is optional and carries no meaning: `2371933` and `2_371_933` are the same
  token, and nothing asks you to group anything.

  Where used, it must **group by threes, counted away from the decimal point** —
  so the short group sits at the front of an integer and the end of a fraction:
  `8_080`, `1.123_45`. This is stricter than Go, Ruby and Rust, where
  `2_3_7_1_9_3_3` is legal. A separator free to sit anywhere is a second way to
  write one number, and it misleads: `1_0000_000` is ten million, but a reader
  scanning for threes sees something far larger. The cost is Indian grouping
  (`1_00_00_000`), which has to be written without separators.

  `100_`, `1__0` and `1_.5` are the new `E0108` as well. A leading `_` is still
  an identifier, as it always was.

  Numbers written plainly stay zero-copy; only a literal that actually contains a
  separator allocates.

- **A schema can state rules about its own values (D28).** `assert` inside a
  `type` body, checked on every `new`:

  ```aura
  type Plan
    price_monthly: Int
    price_yearly:  Int
    assert price_yearly <= price_monthly * 12, "yearly must not exceed twelve months"
  end
  ```

  Cross-field rules are the reason this is worth having — no per-field constraint
  can relate two values. They need no ordering or cycle analysis either, because
  every field is filled and type-checked before any invariant runs. That is what
  makes this feasible where the dependent defaults of D15 were not.

  **It travels with the schema.** An importer instantiating `new pkg.Port` gets
  the package's rules, in a manifest whose author never read them. Written beside
  the `new` instead, the rule is not part of the type: it does not move, tooling
  cannot see it, and the next person does not write it. That is the argument D18
  made for `enum`, applied to conditions.

  An invariant sees **its own fields and nothing else** — not the declaring
  module, not the instantiation site. A rule that could read a module variable
  would mean something different in each manifest that imported the schema, which
  is the problem being solved; it also puts everything effectful out of reach, so
  an instance is valid or invalid identically for everyone. An undefined name
  there says so and lists the fields available.

  A failure is the new `E0515`, naming the schema and carrying its message.

- **A field can say that it may have no value (D27).** `quota: Int?` admits
  `null`; every other field still rejects it.

  This exists to kill the sentinel. Without it, "unlimited" gets written as `-1`
  or `2147483647`, and everyone has to carry in their head which number means
  what and where. A sentinel is a *valid* value of its type, so nothing catches a
  wrong one — which makes it the worst kind of the implicit rule this language
  keeps removing.

  `?` answers exactly one question: may the value be absent. Whether the field
  may be left out is the separate job of `= default`, and the two compose:

  | Declaration | Omitting it | Writing `null` |
  | --- | --- | --- |
  | `quota: Int` | `E0511` | `E0512` |
  | `quota: Int?` | `E0511` | allowed |
  | `quota: Int = 100` | gives `100` | `E0512` |
  | `quota: Int? = null` | gives `null` | allowed |

  A nullable field is still required to be written, and its non-null values are
  still type-checked. Nullability belongs to the field rather than to the type,
  which makes `[Int?]` unrepresentable — so there is never a question of whether
  a list or its elements may be empty — while `[Int]?` still means one thing.

  `aura types` emits `Option<T>`, `T | null` and `*T`. That is a breaking change
  for a host already reading such a field as a plain value, and deliberately so:
  the break happens when the author writes the `?`, not silently afterwards.

  **One grammar change came with it, and it is breaking.** A newline is no
  longer suppressed after `?`. It had to stop being suppressed: with `?` now
  also the nullable marker, `quota: Int?` at the end of a line swallowed its own
  separator — the exact defect that ruled `List<T>` out of D26, arriving through
  the back door. The parser tolerated it and the formatter did not, which is how
  it surfaced.

  A ternary broken across lines therefore needs parentheses:

  ```aura
  tier: (replicas > 3 ?
    "large" : "small")
  ```

  That was already the rule for breaking after the `:` half, so the two halves
  now behave alike instead of one carrying an exception.

  **`W0513` guards the one way this could cost determinism.** `?` widens what a
  field accepts, which matters when the value comes from outside. Without the
  marker, `quota: data.get("quota")` on a file missing that key is `E0512` and
  the run stops, so two people with different inputs find out. With the marker,
  the same difference passes as `null` and they ship different configurations.

  A nullable field filled from `get(key)` or `env(name)` with no fallback is now
  a warning, and an error under `--strict`. It asks only that the value standing
  for absence be named: `data.get("quota", 0)` puts it in the manifest instead of
  in someone's head. A field without `?` is never warned about — there the
  missing value already stops the run.

  Writing `null` into a field without the marker now names the fix:

  ```
  [E0512] Error: field 'q' of schema P expects Int, got Null
            Help: to allow an absent value, declare the field nullable: q: Int?
  ```

- **A list field can say what it holds (D26, closing D21).** `endpoints: [Endpoint]`,
  `tags: [String]`, nesting as `[[Int]]`, and an `enum` as the element type.

  This was the worst-served shape in the language. On 0.1.1 a field declared
  `List` accepted `["not an endpoint", 42, true]` under `--strict`, and the only
  thing the compiler said was that `Endpoint` was **unused** — calling the schema
  dead code while rubbish sailed into the field it described. Both halves are now
  fixed: the element is checked, and a schema used as an element type counts as
  used.

  A wrong element is `E0512` carrying its index, because "one of these is wrong"
  is no help in a list of forty:

  ```
  [E0512] Error: endpoints[0] of schema Service expects Endpoint, got String
  ```

  It reaches the host too: `aura types` emits `Vec<Endpoint>`, `Endpoint[]` and
  `[]Endpoint` where it used to emit an untyped array.

  **Bare `List` is unchanged**, so every manifest written before this evaluates
  exactly as it did.

  Syntax note: `[T]` mirrors the value, so no new keyword was added and the
  parser needs no lookahead. `List<T>` was rejected on evidence — `>` suppresses
  the following newline, so a field ending in `>` swallows its own separator.

- **Folds and predicates on lists (D25).** `reduce`, `any`, `all`, `find` and
  `index_of`. The one that changes how manifests read is `all`:
  `assert ports.all (p, i) -> p > 1024 end` states what it checks, where the old
  spelling `assert ports.filter(...).len() == 0` hid the intent behind a count.
  A manifest's assertions are the part a reader most needs to follow.

  `reduce` requires an explicit initial value, so an empty list has an answer
  rather than an error. `any` and `all` short-circuit, read `false` and `true`
  respectively on an empty list, and reject a predicate that returns anything
  but `Bool`.

  `find(default)` and `index_of(value, default)` both take an explicit fallback.
  Neither returns `Null` nor `-1`: "not found" is an ordinary outcome and the
  caller has to name what it means. A `-1` sentinel is worse than it looks,
  because passing it on silently indexes from the end.

- **`Object.entries()` and `List.to_object()` (D24).** An object was a dead end
  for traversal: `keys()` and `values()` split a map into two lists that nothing
  could rejoin. `entries()` yields `{ key, value }` objects in declaration order
  and `to_object()` is its exact inverse, so a map can be filtered or rewritten
  and put back.

  `to_object()` also closes a gap that had no workaround: a property key is a
  literal in the grammar, so an object keyed by a computed value — services by
  name, the commonest map shape in configuration — could not be built at all.

  The pair is an object rather than a two-element list so a lambda reads `e.key`
  instead of `e[0]`. A repeated key is the new `E0323` rather than an overwrite,
  because a result that silently depends on element order is the class of
  surprise D7 exists to remove.

- **`+` joins strings and lists (D23).** `"svc-" + name` and `base + ["frontend"]`
  now evaluate. The second had no expression at all before this: there was no
  operator and no `concat` method, so a base list plus an environment-specific
  tail — the commonest shape in real configuration — could not be written.

  `+` does not convert. `"port " + 8080` stays `E0306`, and the diagnostic now
  names which side to call `.to_str()` on. Coercion is refused for the reason D4
  and D7 refuse implicitness: a configuration language that guesses is the
  failure being designed away.

  Joining is bounded at 1,000,000 list elements and 16 MiB of string, reported as
  the new `E0322`. `+` is the first operator whose result can outgrow its
  operands, so `x = y + y` repeated once per line doubles, and sixty lines that
  fit on a screen reach 2^60 elements. Evaluation runs over third-party packages,
  so that has to be a diagnostic rather than an exhausted heap.

### Fixed

- **SPEC described an integrity hash the compiler stopped using.** §5.2 still
  said `integrity = "sha256-..."` and "content hash", while the code has hashed
  the **token stream** under an `aura1-` prefix for some time — and the decision
  table in the same document already said so. One document, two answers.

  §5.2 now states what actually happens, in both languages: SHA-256 over the
  token stream, a byte fallback under the same prefix for input that does not
  lex, and legacy `sha256-` entries still verified their own way so an existing
  lock keeps working. Every one of those claims is pinned by a named test.

  The `.gitattributes` comment carried the same stale reason for its `eol=lf`
  pin. The pin stays — conformance fixtures compare bytes, legacy entries still
  hash bytes, and `tests/line_endings.rs` builds its CRLF variant from an LF
  source — but it is not the language that needs it. The language is required to
  behave identically under either line ending, and that is proved separately.

- **Two instances of one schema could serialise with different key orders.**
  Defaults were appended after whatever the author happened to write, so the
  same schema produced different bytes depending on whether an optional field
  was supplied:

  ```json
  "a": { "id": "free", "fallback": 0, "quota": 100 }
  "b": { "id": "pro",  "quota": 500, "fallback": 0 }
  ```

  Key order now comes from the schema's declaration. Two instances always
  serialise with the same keys in the same places, and the order fields are
  written at the construction site stops mattering entirely.

  This matters more than it looks for a configuration language: a diff between
  two environments that shows reordered lines with no change of meaning teaches
  the reader to stop reading diffs, which is the opposite of the point.

  Undeclared fields — an error under `--strict`, kept with a warning otherwise —
  follow the declared ones in the order they were written. No example's expected
  output changed.

- **A manifest with a block string did not compile on a CRLF checkout.** Not
  "produced different output" — failed outright:

  ```
  [E0105] Error: unexpected character
   2 │   listen 80;
     │            ┬── not a valid Aura token
  ```

  The block-string opener lookahead skipped spaces and tabs before the newline
  but not a carriage return, so `text` was never recognised as an opener and the
  block's contents were lexed as code. Ordinary code was unaffected, which is why
  this went unnoticed: the lexer collapses `
` everywhere else, and the
  scanner already stripped the carriage return from each captured line. Only the
  opener check and the newline skip were missing it.

  On Windows, `core.autocrlf=true` is the default, so this was the first thing a
  Windows user hit on any manifest using the construct Aura advertises for
  generating nginx configs and Dockerfiles.

  **The repository could not see it.** Its own `.gitattributes` pins `eol=lf`, so
  every file CI reads is LF on all three platforms. That pin is correct here —
  the conformance fixtures depend on it — but it made the defect invisible to the
  whole suite. `tests/line_endings.rs` therefore writes both variants at run
  time and requires byte-identical output; verified to fail without the fix.

- **`E0317` knew the cure and never told anyone.** The diagnostic catalogue has
  said "use `.get(i, default)`" since the code existed, but the runtime message
  read only `first() on an empty list`. Advice that lives in the reference is
  advice nobody reads at the moment they need it.

  All four `E0317` sites — `first()`, `last()`, `min()`/`max()` and an
  out-of-range index — now print a remedy containing the code to type. `last()`
  and `min()`/`max()` suggest a length guard rather than `.get`, because `.get`
  takes no negative index and a hint that does not run is worse than none: it
  costs the reader a second failure to learn the advice was wrong. A test
  evaluates every suggested form.

- **`index 5 out of bounds (list has 1 elements)`** now agrees in number.

- **A `null` in a list reported a symptom and hid the cause.** `sum()` said
  `expects numbers, got Null`, and `sort()`/`min()`/`max()` said they could not
  compare two types. All true, and all pointing the reader at their types when
  the real problem is almost always a `null` that arrived from parsed YAML or
  JSON, where an empty value is an ordinary shape.

  These now name the cure, `xs.compact().sum()`, but **only when a `null` is
  actually involved**. A genuine mix of scalar types gets different advice,
  because `compact()` removes nulls and nothing else, and recommending it for a
  `String` among `Int`s would send the reader down a path that cannot work. Both
  branches are tested, and so is the suggested call.

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
