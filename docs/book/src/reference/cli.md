# CLI

```text
aura eval <file.aura>  [--strict] [--dry-run] [--frozen]
                       [--allow-read=<dir>]... [--allow-env[=A,B]] [--allow-imports-io]
                       [--format json|json-flat|yaml|toml] [-o out] [--registry-dir=<dir>]
aura check <file.aura> [--strict]
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
| `--format` | `json` (default), `json-flat`, `yaml` or `toml` |
| `-o, --output <file>` | write to a file instead of stdout |
| `--registry-dir=<dir>` | the package cache directory (default `~/.aura/registry`) |

## check

Lex, parse and static analysis only — a fast gate for pre-commit hooks and CI.

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
cache and recording it in `./aura.lock` with a SHA-256. `--from <file>` installs
from a local file instead. This is the only command that uses the network.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | diagnostics (language errors, a failed assert, a blocking strict warning) |
| 2 | I/O and argument errors |
