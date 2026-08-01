# CLI

```text
aura eval <file.aura>  [--strict] [--dry-run] [--frozen]
                       [--allow-read=<dir>]... [--allow-env[=A,B]] [--allow-imports-io]
                       [--hermetic]
                       [--format json|json-flat|yaml|toml] [-o out] [--registry-dir=<dir>]
aura check <file.aura> [--strict] [--hermetic]
aura docs --agent [-o <file>]
aura fmt <files...> [--check]
aura types <file.aura> --lang rust|ts|go [--out <file>]
aura add <path>@vX.Y.Z [--from <file>] [--registry-dir=<dir>]
aura --version
```

## eval

Evaluates a manifest with all its imports and prints the result.

| Flag | Effect |
| --- | --- |
| `--strict` | analysis warnings (from every module) become blocking; extra schema fields are error `E0513` |
| `--dry-run` | a full evaluation, but neither the output nor `aura.lock` is written; reports `[dry-run] read/would write` |
| `--frozen` | dependencies strictly per `aura.lock` (`E0403` on a mismatch), and the lock is never rewritten — the CI mode |
| `--allow-read=<dir>` | permits `read_file()` inside that directory (repeatable) |
| `--allow-env[=A,B]` | permits `env()`: either a list of names, or all of them |
| `--allow-imports-io` | extends the root's rights to imported modules |
| `--hermetic` | no I/O at all: `env()` and `read_file()` are `E0505` wherever they appear. Mutually exclusive with the `--allow-*` flags |
| `--format` | `json` (default), `json-flat`, `yaml` or `toml` |
| `-o, --output <file>` | write to a file instead of stdout |
| `--registry-dir=<dir>` | the package cache directory (default `~/.aura/registry`) |

## check

Lex, parse and static analysis only — a fast gate for pre-commit hooks and CI.

`--hermetic` additionally requires that the manifest performs no I/O: `env()` and
`read_file()` anywhere in it, or in anything it imports, is `E0505`. It is decided
statically, so `check` answers it without evaluating — including for branches that
this particular run would not have taken:

```console
$ aura check --hermetic deploy.aura
[E0505] Error: env() is not allowed in hermetic mode
   ╭─[ deploy.aura:4:11 ]
```

Reach for it when a manifest's output must depend on nothing but its own text: a
CI gate on a directory of manifests, or a build whose result you want to be
reproducible on another machine. `eval --hermetic` enforces the same thing, and is
the right flag when the manifest is untrusted.

## docs

`aura docs --agent` prints the complete language reference — syntax, the standard
library, every diagnostic code — assembled from the compiler's own definitions.
It is about four thousand tokens, which is small enough to hand to a coding
assistant whole.

The reason to print it from the binary rather than link to a page is versions: the
output describes the `aura` that produced it, and cannot drift the way a hosted
document can.

The most useful way to use it is to let the assistant fetch it. One line in your
repository's `AGENTS.md`, `CLAUDE.md` or equivalent:

```text
Before writing or editing .aura files, run `aura docs --agent` for the complete
language reference.
```

Or commit it, for assistants that read files but cannot run commands:

```console
$ aura docs --agent -o AURA.md
```

The same text is published at
[/llms.txt](https://aura-config.github.io/aura-lang/llms.txt) for tools that
fetch over the network.

## fmt

The canonical formatter: indentation (two spaces per level, by token depth),
intra-line spacing, and column alignment of runs of `name = value`, `key: value`
and `cond` arms together with their trailing comments. Strings and the contents of
block strings are left alone; the token stream is guaranteed not to change, and
formatting is idempotent. `--check` only checks, exiting 1 if anything differs.

## types

Generates host-language types from the `type` and `enum` declarations:

```console
aura types config.aura --lang ts
```

The six built-in field types map like this:

| Aura | Rust | TypeScript | Go |
| --- | --- | --- | --- |
| `String` | `String` | `string` | `string` |
| `Int` | `i64` | `number` | `int64` |
| `Float` | `f64` | `number` | `float64` |
| `Bool` | `bool` | `boolean` | `bool` |
| `List` | `Vec<serde_json::Value>` | `unknown[]` | `[]any` |
| `Object` | `serde_json::Map<String, serde_json::Value>` | `Record<string, unknown>` | `map[string]any` |

`String` is the only name the language accepts for a text field — there is no
`Str`.

Note the last two rows. A schema's `List` and `Object` carry no element type, so
the generated code cannot either: you get a sequence or a map of untyped values.
Where the shape matters, declare a `type` for the element and let the field hold
that instead.

One schema both validates the config and types the service that reads its JSON.
An enum becomes a string-literal union (TypeScript), an enum with
`#[serde(rename)]` (Rust), or a named string with typed constants (Go). Optional
fields (D15) are emitted as required, because evaluation always substitutes the
default. Parsing only: the manifest is not evaluated and no capabilities are
involved. `--out` writes to a file.

The output is already canonical for the host language's formatter — `rustfmt
--check`, `gofmt -l` and `prettier --check` stay silent on the generated files —
so it can be committed into a service's repository without formatting churn in
diffs.

## add

Installs a package: downloading it (`github/<owner>/<repo>` → `package.aura` at
tag `vX.Y.Z`; an exact version is mandatory), validating it, writing it to the
cache and recording it in `./aura.lock` with an integrity hash. `--from <file>` installs
from a local file instead. This is the only command that uses the network.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | diagnostics (language errors, a failed assert, a blocking strict warning) |
| 2 | I/O and argument errors |
