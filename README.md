# Aura

**A configuration language that reads its own inputs — and never lets an import do the same.**

Aura compiles readable manifests into JSON, YAML or TOML — with schemas, enums,
assertions that fail the build instead of the deploy, and a capability model in
which `env()` and `read_file()` are granted per run and the grant does not reach
imported modules. One static binary, 16 keywords, a 1.7 MB download.

[![CI](https://github.com/aura-config/aura-lang/actions/workflows/ci.yml/badge.svg)](https://github.com/aura-config/aura-lang/actions/workflows/ci.yml)
![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Status](https://img.shields.io/badge/status-v0.1%20preview-blue)

---

## Why Aura

| Problem | Aura's answer |
| --- | --- |
| YAML breaks from one stray space | No significant indentation: structure comes from line breaks and an explicit `end` |
| Configs "work on my machine" | Deterministic by construction: without explicit flags a manifest has **no** access to files or environment variables |
| A module pulled from the internet reads `/etc/passwd` | Imported modules are isolated from I/O (Deno-style capabilities) |
| "Why is prod on a different port?" | Values are immutable; shadowing requires the explicit `shadow` keyword |
| Dependency version drift in CI | Versioned imports + `aura.lock` with a token-stream integrity hash and `--frozen` mode |
| Dead config fragments live for years | Static analysis: unused variables and imports, `--strict` for CI |

## Example

```ruby
import "templates/k8s_defaults.aura" as defaults

base_port = 8000 # private computation
is_prod   = env("APP_ENV", "production") == "production"

type ServiceMeta
  name: String
  port: Int
end

def build_labels(app_name, tier)
  name:       app_name
  tier:       tier
  managed_by: "aura-engine"
end

domain "production-eu"
  replicas: is_prod ? 3 : 1 # a property - ends up in JSON

  cargo_data  = read_file("./Cargo.toml").parse_toml()
  app_version = cargo_data.package.version

  services = ["auth", "billing", "frontend", null]
  active   = services.compact().uniq()

  meta: new ServiceMeta
    name: "auth".upper()
    port: base_port + 1
  end

  apps: active.map (name, index) ->
    name:   name
    image:  "company/#{name}:#{app_version}"
    labels: build_labels(name, "backend").merge(defaults.global_labels)
  end

  assert active.len() >= 1, "Domain must have at least 1 service"
end
```

```console
aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV
```

```json
{
  "production-eu": {
    "replicas": 3,
    "meta": { "name": "AUTH", "port": 8001 },
    "apps": [
      {
        "name": "auth",
        "image": "company/auth:1.2.3",
        "labels": { "name": "auth", "tier": "backend", "managed_by": "aura-engine", "team": "core" }
      }
    ]
  }
}
```

## Quick start

```console
git clone https://github.com/aura-config/aura-lang && cd aura-lang
cargo build --release
cd examples
../target/release/aura eval production_deploy.aura \
    --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```

Check without evaluating (lex + parse + static analysis):

```console
aura check production_deploy.aura --strict
```

## Tour of the language

### Computation and output are different things

Aura's central rule (the same idea as locals vs. outputs in Terraform):

```ruby
tmp = base * 2 #  =  a private variable: does NOT end up in JSON
port: tmp + 1  #  :  a property: ends up in JSON
```

This is what makes dead-code analysis precise: an unused `=` variable is
always genuine cruft (`W0501`), never "maybe someone needs this output."

### Immutability and explicit shadowing

```ruby
path = "/etc/global.config"

domain "prod"
  path        = "/var/log" # E0302: shadowing requires the keyword
  shadow path = "/var/log" # OK - the intent is explicit
end
```

Reassigning a name within the same scope is always an error (`E0301`).

### Schemas

```ruby
type ServiceMeta
  name: String
  port: Int
end

meta: new ServiceMeta
  name: "auth"
  port: "8001" # E0512: expected Int
end
```

A missing field is `E0511`; an extra field is `E0513` under `--strict`.
`Int` and `Float` are separate types: byte limits and 64-bit IDs never lose precision.

A field with `= default` is optional — omit it and the default applies (it may
reference module vars). No nullable fields: optionality never introduces a `null`.

```ruby
type Service
  name: String
  port: Int    = 8080 # optional: omitted -> 8080
  tier: String = "backend"
end

api: new Service
  name: "api" # port and tier take their defaults
end
```

### Closed sets with `enum`

A `String` field accepts any string, so `"backand"` used to reach production. An
`enum` makes the set closed — and a member is still an ordinary string, so the
JSON output does not change:

```ruby
enum Tier
  "frontend"
  "backend"
  "cache"
end

type Service
  name: String
  tier: Tier
end

svc: new Service
  name: "api"
  tier: "backand" # E0514: did you mean "backend"? members: ...
end
```

`pub enum` crosses the module boundary (D12), and members are resolved where the
schema is declared — an imported schema validates against its own module's enum.

### Types for your service: `aura types`

The same schema that validates the config can type the service that consumes its
JSON — no hand-written structs to keep in sync:

```console
aura types config.aura --lang rust   # or ts | go
```

```ruby
enum Scheme
  "https"
  "http"
end

type Endpoint
  host: String
  port: Int      = 443
  scheme: Scheme = "https"
end
```

becomes, for TypeScript:

```ts
export type Scheme = "https" | "http";

export interface Endpoint {
  host: string;
  port: number;
  scheme: Scheme;
}
```

Rust gets a `serde` struct plus an enum with `#[serde(rename)]`; Go gets a struct
with `json:` tags plus a named string type and typed constants. Parsing only —
the manifest is never evaluated, so no capabilities are involved.

### Functions, lambdas, methods

```ruby
def labels(app) # def returns an object
  app: app
end

up = (s) -> s.upper() end # a lambda expression

xs.compact().uniq().map (item, index) ->
  "#{index}: #{item}"
end
```

Built-in methods: `upper` `lower` `len` `trim` `split` `replace` `starts_with`
`ends_with` `to_int` `to_float` `to_str` `compact` `uniq` `map` `filter` `sort`
`reverse` `sum` `min` `max` `flatten` `slice` `first` `last` `get` `contains`
`join` `merge` `keys` `values` `abs` `parse_toml` `parse_json` `parse_yaml`
`to_json` `to_yaml` `to_toml` `parse_duration` `format_duration` `parse_datetime`
`format_datetime` `sha256` `base64` `base64_decode`. The registry grows without
touching the parser.

The global `range(n)` generates `[0, 1, ..., n-1]` — handy for producing N items
(shards, replicas) instead of listing them by hand:

```ruby
shards: range(3).map (i, _) ->
  name:       "shard-#{i}"
  replica_of: "primary"
end
```

### Multi-way choice with `cond`

For 3+ branches where nested ternaries get unreadable, `cond` picks the first
true arm; `else` is mandatory (a missing one is a parse error):

```ruby
tier = cond
  region == "eu" -> "frankfurt"
  region == "us" -> "virginia"
  else -> "singapore"
end
```

The left of each `->` must be `Bool`; the right is any expression. No pattern
destructuring - deliberately simpler than a `match`.

### Multi-line values with `text … end`

A `text … end` block is an ordinary string, just multi-line - so any property
takes either a `"one-liner"` or a block. Interpolation `#{}` and escapes work as
usual; the common indentation is stripped and lines join with `\n`:

```ruby
domain "worker"
  entrypoint: text
    #!/bin/sh
    echo "starting #{app_name}"
    exec ./server --port #{port}
  end
end
```

The closing `end` sits at the `entrypoint:` indentation; content is indented
deeper, so an embedded `end` (a shell/Ruby block) is just text. For many or large
scripts prefer `read_file(...)` over an inline block.

### Time - deterministic only

`now()` does not exist in Aura and never will (D13) - an unreproducible config
cannot be written by construction. Durations and dates, on the other hand, are
first-class:

```ruby
ttl = "1h30m".parse_duration()             # -> 5400 (seconds)
refresh:    (ttl / 3).format_duration()    # -> "30m"
window_end: ("2026-07-18T22:00:00Z".parse_datetime()
+ "4h".parse_duration()).format_datetime() # -> "2026-07-19T02:00:00Z"
```

If a build timestamp is needed, the host supplies it: `env("BUILD_TIME", ...)` under `--allow-env`.

### Data access

Dot for fields, brackets only for list indices - one operator per operation:

```ruby
loaded = read_file("./data.json").parse_json()

version:  loaded.package.version          # ordinary keys
port:     loaded.servers."eu west".port   # a key with a space/dot - a string
dynamic:  loaded.envs."#{region}".url     # a dynamic key
first:    loaded.apps[0].name             # list index (out of bounds - E0317)
optional: loaded.get("maybe", "fallback") # safe access, no error
```

A typo in a key is an `E0308` error with a position, not a silent `null`.

### Aura as a format converter

Since the language reads TOML/JSON/YAML and writes all three, conversion is a one-liner:

```ruby
# convert.aura
config: read_file("./legacy.toml").parse_toml()
```

```console
aura eval convert.aura --allow-read=. --format yaml   # TOML -> YAML
```

Unlike `yq`/`jq`, you can validate against a schema, merge multiple sources,
and add `assert` checks along the way - conversion with guarantees.

### Modules

```ruby
import github/actions/rust-cache@v1.2 as rust # a version is mandatory
import "templates/k8s_defaults.aura" as defaults
```

- Cyclic imports are detected with the full chain: `E0401: cyclic import: a.aura -> b.aura -> a.aura`.
- Every module is loaded, parsed, and evaluated exactly once.
- Exact versions and integrity hashes are pinned in `aura.lock`; use `--frozen` in CI.

### Validation

```ruby
assert active.len() >= 1, "Domain must have at least 1 service"
value: broken ? fail("unreachable config") : 42
```

## Security model

By default a manifest **can do nothing**: no reading files, no environment
variables. Capabilities are granted via CLI flags and do not propagate to
imported modules.

| Flag | What it allows |
| --- | --- |
| `--allow-read=<dir>` | `read_file()` inside the directory (repeatable; paths are canonicalized, `..` cannot escape) |
| `--allow-env[=A,B]` | `env()` for the listed variables (no list - all of them) |
| `--allow-imports-io` | grant imported modules the root's capabilities |
| `--hermetic` | the opposite: no I/O at all. `env()` and `read_file()` are `E0505` in every module, and the flag excludes the `--allow-*` ones |

A call without a grant is an `E0310` error with a hint about which flag to
add. An effectful call inside an imported module is additionally caught
statically (`W0512`).

`--hermetic` inverts the question: rather than granting rights, it requires that
none are needed. Because `E0505` is an analysis error, `aura check --hermetic
deploy.aura` proves a manifest touches nothing at all - no evaluation, no
branch-dependent answer - which makes it a CI gate.

## CLI

```text
aura eval <file.aura>  [--strict] [--dry-run] [--frozen]
                       [--allow-read=<dir>] [--allow-env[=A,B]] [--allow-imports-io]
                       [--hermetic]
                       [--format json|json-flat|yaml|toml] [-o out.json] [--registry-dir=<dir>]
aura check <file.aura> [--strict] [--hermetic]
aura fmt <files...> [--check]
aura add <path>@vX.Y.Z [--from <file>] [--registry-dir=<dir>]
```

`aura add` is the only place Aura ever touches the network: a package is
downloaded (by convention `github/<owner>/<repo>` -> `package.aura` at tag
`vX.Y.Z`), validated, stored in the local cache, and pinned in `aura.lock`
with an integrity hash. **`eval` always runs offline** - the result never depends
on the network, by construction.

| Mode | Behavior |
| --- | --- |
| `--strict` | analysis warnings become errors; extra schema fields are forbidden |
| `--dry-run` | full evaluation, but neither JSON nor `aura.lock` is written - only a `[dry-run] would write ...` report |
| `--frozen` | dependencies resolve strictly via `aura.lock` (a mismatch is an error), the lock is never rewritten |
| `--format json-flat` | flattened output: `production-eu.metrics.port = 9090` |
| `--format yaml` / `toml` | the same result in YAML or TOML (TOML requires an object at the top level) |

Exit codes: `0` - success, `1` - diagnostics, `2` - I/O or argument errors.

## Using Aura from other languages

Aura follows the terraform/jq/pandoc pattern: a stable CLI contract is the
API. JSON on stdout, diagnostics with stable codes (`E0xxx`) on stderr,
exit codes `0`/`1`/`2`. From any language:

```python
# Python
import json, subprocess
r = subprocess.run(["aura", "eval", "app.aura", "--frozen"], capture_output=True, text=True)
if r.returncode != 0:
    raise RuntimeError(r.stderr)
config = json.loads(r.stdout)
```

```javascript
// Node.js
const { execFileSync } = require("node:child_process");
const config = JSON.parse(execFileSync("aura", ["eval", "app.aura", "--frozen"]));
```

```go
// Go
out, err := exec.Command("aura", "eval", "app.aura", "--frozen").Output()
if err != nil { log.Fatal(err) }
var config map[string]any
json.Unmarshal(out, &config)
```

Recommendations for production: `--frozen` (a lock file is mandatory),
capabilities only explicit, `--format yaml|toml` if the consumer prefers
another format. Rust projects can embed Aura directly, without a subprocess,
via `aura_lang::facade::eval_file()`. Mobile apps are consumers of the
*result*: a server/CI evaluates the `.aura` file, the client reads the
resulting JSON. Native bindings (WASM/npm, PyO3) are on the roadmap.

## Diagnostics

Errors point to the file, line, and column, highlight the code, and suggest a fix:

```text
[E0302] Error: 'global_file_path' shadows an outer variable
    ╭─[ production_deploy.aura:24:3 ]
 24 │   global_file_path = "/var/log/aura.log"
    │   ─────────┬────────
    │            ╰── add `shadow`
    │
    │   Help: write `shadow global_file_path = ...` to make the shadowing explicit
────╯
```

Every error has a stable code (`E0xxx` / `W0xxx`) - convenient for grepping CI logs.

## Architecture

```text
source ──▶ lexer ──▶ parser ──▶ static analysis ──▶ evaluation ──▶ JSON
 &'a str    Vec<Token>    AST        Vec<Diagnostic>        Value
            (zero-copy: tokens and the AST borrow the source's memory)
```

```text
crates/
├── aura-lang          # the library + the `aura` CLI (one publishable crate)
│   ├── lexer/         # zero-copy DFA, newline normalization
│   ├── parser/        # recursive descent + Pratt expressions
│   ├── analysis/      # dead code, undefined vars, shadow rules
│   ├── eval/          # tree-walking interpreter, Environment, method registry
│   ├── vfs/           # FileResolver, cycle detection, aura.lock
│   ├── serialize/     # Value -> serde_json (Int with no precision loss)
│   └── main.rs        # the `aura` binary (clap + ariadne); the library stays
│                      #   clap/ariadne-free and WASM/LSP-ready
└── aura-lsp           # language server (ships in the VS Code extension)
```

Key invariants:

- **Zero-copy**: neither the lexer nor the parser copies strings - only `&'a str` slices.
- **Determinism**: JSON key order = declaration order (`IndexMap`); two runs produce byte-identical output.
- **Immutability**: containers live in `Arc`, cloning a value is O(1).

## Performance

Criterion benchmarks on the reference manifest (`cargo bench -p aura-lang`):

| Stage | Result |
| --- | --- |
| Lexer | ~200 MB/s |
| Lexer + parser | ~120 MB/s |
| Full pipeline (lex + parse + eval) | ~37 µs per manifest |

## Development

```console
cargo test    # units, conformance suite, property tests, golden snapshots
cargo bench   # lexer, parser, resolver, full-pipeline benchmarks
```

Coverage-guided fuzzing (lexer, parser, full pipeline) lives in
[fuzz/](fuzz/README.md) — nightly + `cargo-fuzz`, run in a non-blocking CI job.
The recursive-descent parser is DoS-hardened: deeply-nested input yields `E0208`
instead of a stack overflow.

The full language and architecture specification is [SPEC.md](SPEC.md). The
reference manifest, which must pass the entire pipeline, is
[examples/production_deploy.aura](examples/production_deploy.aura).
Nine themed examples (k8s, a CI matrix, feature flags, a service catalog,
i18n, a Telegram bot, a validators package, a capability-model demo) live in
[examples/](examples/README.md).

## Status and roadmap

Aura is in working-preview status (v0.1): all six specification phases are implemented.

- [x] Zero-copy lexer and Pratt parser
- [x] Runtime with a capability model and schemas
- [x] Modules, cycle detection, `aura.lock`
- [x] Static analysis and `--strict` / `--dry-run`
- [x] JSON export and a CLI with ariadne diagnostics
- [x] Reading and writing JSON/YAML/TOML (`parse_*` / `to_*`, `--format yaml|toml`)
- [x] Indexing and access to arbitrary keys (`xs[0]`, `obj."eu west"`, `.get`)
- [x] `aura fmt` - indentation canonicalization with a token-stream-preservation guarantee
- [x] Deterministic time: `parse_duration`/`format_duration`,
      `parse_datetime`/`format_datetime`; `now()` is forbidden by construction (D13)
- [x] Extended standard-library methods (`sort`, `split`, `trim`, `join`,
      `slice`, `flatten`, `reverse`, ...) - [the method reference](docs/book/src/reference/methods.md)
- [x] LSP server: completion, hover, go-to-definition, references, document
      symbols, rename (with `prepare`), signature help, and format-on-save -
      [crates/aura-lsp/](crates/aura-lsp/)

**Ecosystem and distribution**:

- [x] A release pipeline: a `v*` tag builds binaries for six targets (Linux
      gnu/musl, aarch64, macOS Intel and Apple silicon, Windows), each with a
      `.sha256`, after checking the tag against `Cargo.toml` -
      [release.yml](.github/workflows/release.yml)
- [x] A `setup-aura` GitHub Action: resolves the version, verifies the checksum
      and puts `aura` on `PATH` -
      [aura-config/setup-aura](https://github.com/aura-config/setup-aura). The
      release workflow's `self-test` job installs from a real release through
      that published action and runs the result on Linux, macOS and Windows
- [ ] Publication: crates.io, and `setup-aura` extracted into its own repository
      for the Marketplace - both are done when this repository becomes public
- [x] Syntax highlighting: VS Code (TextMate + auto-indent), Vim/Neovim,
      nano - [editors/](editors/README.md)
- [ ] A tree-sitter grammar (Helix, Zed, Neovim, GitHub Linguist)
- [x] A documentation book ([docs/book/](docs/book/), with a full Russian
      translation at [docs/book-ru/](docs/book-ru/)): a tutorial (6 chapters),
      guides (security, formats, embedding), a CLI/method reference,
      and a full error-code catalog; built in CI, deployed to Pages - after
      the repository is published
- [x] A browser playground - [playground/](playground/): the real compiler as
      WebAssembly, multi-file, with `aura fmt` and diagnostics; nothing is
      installed and nothing leaves the page. Deployed to Pages together with the
      book, after the repository is published
- [x] Usage from other languages: the subprocess pattern (Python/Node/Go) -
      see the section above. The WebAssembly module the playground runs on is
      built and CI-tested ([crates/aura-wasm/](crates/aura-wasm/)); publishing it
      to npm, then PyO3 and a C ABI, is on demand
- [x] Cross-platform coverage: a `cargo check` matrix (freebsd, aarch64-linux,
      musl, wasm32) in CI; macOS in the test matrix

**v1.3 - the package ecosystem**:

- [x] D12: `pub def` / `pub type` - exporting functions and schemas from
      modules (`pkg.fn(...)`, `new pkg.Schema ... end`); exported functions
      run with their own module's capabilities, not the caller's
- [x] `aura add <pkg>@vX.Y.Z` - installing a package (the network is touched
      only here; `eval` always runs offline), validation before install,
      integrity pinned in `aura.lock`

## Contributing

Issues and PRs are welcome. Before opening a PR: `cargo test --workspace`,
`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
and `aura fmt --check` on any changed `.aura` files - CI runs the same checks.
Code comments are in English (see [SPEC.md](SPEC.md) for the formal language
specification).

## License

Aura is distributed under your choice of [MIT](LICENSE-MIT) or
[Apache License 2.0](LICENSE-APACHE).

Unless you explicitly state otherwise, any contribution (issue/PR)
intentionally submitted for inclusion in this project shall be licensed as
above, without any additional terms or conditions.

---

*[Русская версия / Russian version: README.ru.md](README.ru.md)*
