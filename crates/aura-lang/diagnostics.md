# Error and warning codes

Every Aura diagnostic carries a stable code, which makes it easy to grep for in CI
logs and to refer to in discussion. `E` is an error, `W` a warning — and under
`--strict` warnings block too.

## Host (E00xx)

Raised by the embedding API rather than by a manifest: they carry no source
position, because nothing was parsed yet.

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0001 | the host could not open a file, or `aura.lock` is unreadable | the message carries the underlying I/O error |

## Lexing (E01xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0101 | no digit after the decimal point (`12.`) | write `12.0` or `12` |
| E0102 | a string left unclosed at end of line | Aura strings are single-line; close the `"` |
| E0103 | an integer literal does not fit in i64 | |
| E0104 | a registry import without a version | `import a/b@v1 as x` — the version is mandatory (D8) |
| E0105 | unexpected character | |
| E0106 | an unclosed `#{` interpolation | |
| E0107 | an unterminated block string | close it with `end` at the opener's indentation (D16) |

## Syntax (E02xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0200 | a specific token was expected | the message names which |
| E0201 | a v1.1 inline block (`metrics port: 9090`) | use an object block: `metrics:` … `end` (D3) |
| E0203 | a missing `end` or `]` | |
| E0204 | an expression was expected | |
| E0205 | statements are separated by line breaks | |
| E0206 | `pub` not immediately before `def`, `type` or `enum` | properties export themselves; `=` is always private |
| E0207 | a `cond` with no `else` arm | add `else -> ...`: every branch must produce a value (D14) |
| E0208 | nesting too deep | the recursion limit that makes deeply-nested input an error rather than a stack overflow |
| E0211 | an `enum` member is not a quoted string | members are listed as strings, one per line (D18) |
| E0212 | a duplicate `enum` member | list each member once |
| E0213 | an `enum` with no members | an empty set cannot be satisfied |

## Evaluation (E03xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0301 | assigned twice — values are immutable | use a new name, or a nested scope |
| E0302 | shadowing an outer variable without the marker | `shadow x = ...` (D7) |
| W0303 | `shadow` shadows nothing | drop the `shadow` |
| E0304 | integer arithmetic overflowed | use `Float`, or reconsider the magnitudes |
| E0305 | division by zero | |
| E0306 | an operand of the wrong type | conditions must be `Bool`; there is no truthiness |
| E0307 | a non-scalar in interpolation or `join` | serialise it explicitly with `.to_json()` |
| E0308 | unknown object field | a typo? use `.get(key, default)` for optional ones |
| E0309 | unknown method | see the method reference; a package's function fields are called the same way |
| E0310 | missing capability | the error names the flag you need (D1) |
| E0311 | a path outside the directories allowed by `--allow-read` | |
| E0312 | fewer arguments passed than the function has parameters | |
| E0313 | a file read failed (I/O) | |
| E0314 | malformed TOML, JSON or YAML while parsing | |
| E0315 | a method that needs a lambda (`map`, `filter`) did not get one | |
| E0316 | a malformed expression inside `#{...}` | |
| E0317 | a list index out of range, or `first()` / `last()` on an empty list | use `.get(i, default)` |
| E0318 | `obj["key"]` — brackets are not allowed on objects | use the dotted form: `obj."key"` (D11) |
| E0319 | a malformed duration | the format is `"1h30m"`, units `d/h/m/s` |
| E0320 | a malformed date | RFC 3339: `"2026-07-18T12:00:00Z"` |
| E0321 | malformed base64 | `base64_decode()` on input that is not base64 |
| E0398 | internal: a closure's environment was dropped | a bug in Aura — please report it with the manifest |
| E0399 | call depth exceeded (256) | recursion in a function? |
| E0533 | `now()` and `timestamp()` do not exist | the host passes time in: `env("BUILD_TIME", ...)` (D13) |

## Modules and packages (E04xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0401 | a cyclic import (the chain is in the message) | break the cycle: move shared code into a third module |
| E0402 | a package's integrity hash does not match `aura.lock` | the package's contents changed — find out why |
| E0403 | `--frozen`: no entry in `aura.lock` | run `aura add` locally and commit the lock |
| E0404 | module not found, or does not resolve | check the path, the version, `--registry-dir` |
| E0410 | an import the interpreter was handed without the module loader | only reachable when embedding the evaluator directly; use `facade::eval_file`/`eval_source`, which load modules |

## Analysis and validation (E05xx / W05xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| W0501 | an unused variable | dead code — delete it |
| W0502 | an unused import | |
| W0503 | an unused function or type | if it is a package's API, mark it `pub` |
| E0504 | an undeclared variable | a declaration must precede its use |
| E0505 | `env()` or `read_file()` under `--hermetic` | pass the value in from the host, or drop the flag and grant the capability |
| E0511 | a field the schema requires is missing | |
| E0512 | a field's type does not match the schema | `Int` and `Float` are different types |
| E0513 | an extra field (`--strict` only) | remove the field, or add it to the schema |
| E0514 | a value outside the `enum` | the hint suggests the nearest member and lists the set (D18) |
| W0512 | an effectful call in an imported module | imports have no I/O rights; reconsider the package's design |
| E0530 | an `assert` failed | the message is your own |
| E0531 | `fail(...)` was called | |

## Serialisation (E06xx)

| Code | Meaning | How to fix it |
| --- | --- | --- |
| E0601 | a function or schema inside the exported tree | the key path is in the message; at the top level `pub` items are excluded automatically |
| E0602 | a non-finite Float (NaN or Inf) | |
| E0603 | a format limitation (TOML has no null; its root must be an object) | |
| E0604 | two keys flatten to the same `json-flat` key | a key containing a dot is indistinguishable from a nested path; use `--format json`, or rename the key |
