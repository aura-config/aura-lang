Aura is a configuration language. A manifest evaluates to a value, which is
exported as JSON, YAML or TOML. It is not general-purpose: there are no loops, no
mutation, no I/O beyond two capability-gated calls, and no way to observe the
clock.

This document is the whole language. If something is not here, it does not exist —
do not infer syntax from Python, Ruby, YAML or HCL, which Aura resembles in
places and diverges from in others.

## The one rule to internalise

`:` exports, `=` does not.

```aura
port: 8080          # a property — appears in the output
internal = "secret" # a binding — usable in this file, never exported
```

Every other question about what ends up in the output reduces to this. A `def` or
`type` is private unless marked `pub`, and `pub` makes it importable — not
exported into the JSON.

## Blocks, not braces

Indentation is not significant. Blocks open with a name and close with `end`.

```aura
api:
  port: 8080
  tls:
    enabled: true
  end
end
```

There is no `{}` object literal. An object comes from a block, a `def`, or `new`.
Lists use `[]`.

## Values

The built-in type names a schema field may use are listed under **Keywords and
types** below — take them from there rather than guessing, because the error for
an invented one is `E0504: use of undefined variable`, which does not say "unknown
type" and is easy to misread.

A schema can state rules about its own values, checked on every `new` and
carried with the schema into any manifest that imports it:

```aura
type Plan
  price_monthly: Int
  price_yearly:  Int
  assert price_yearly <= price_monthly * 12, "yearly exceeds twelve months"
end
```

An invariant sees only that schema's fields — not the surrounding module — so it
means the same thing everywhere. A failure is `E0515`.

A field takes `null` only if declared `Int?`. Use it for a genuinely absent
value instead of a sentinel like `-1`:

```aura
type Plan
  quota: Int?
end
```

`?` does not make the field optional — omitting it is still an error. That is
what `= default` is for, and the two combine: `quota: Int? = null`.

A list field says what it holds: `tags: [String]`, `endpoints: [Endpoint]`,
nesting as `[[Int]]`, and an `enum` works as the element type. A wrong element is
`E0512` reported with its index. Bare `List` still accepts anything.

`slice(start, end)` takes a substring by **character** index, `end` exclusive:
`topic.slice(0, topic.len() - 1)` drops a trailing character. `Int.to_float()`
widens a number; there is no `Float.to_int()`.

A long number may be grouped with `_`, between digits only:

```aura
max_bytes: 10_000_000
port:      8_080
```

It is optional and means nothing — `10_000` and `10000` are the same value.
Where used it must group by threes, counted away from the decimal point, so the
short group is at the front of an integer and the end of a fraction: `8_080`,
`1.123_45`. `2_3_7_1_9_3_3`, `1_0000_000`, `100_` and `1__0` are all errors.

`Int` and `Float` are distinct: passing `1` where `1.0` is required is `E0512`.

Strings are single-line and double-quoted, with `#{...}` interpolation:

```aura
name = "checkout"
label: "svc-#{name}"
```

Multi-line text uses a block string, which ends with `end` at the opener's
indentation and interpolates the same way:

```aura
config: text
  server {
    server_name #{name}.example.com;
  }
end
```

Lists fold and answer questions. `any` and `all` are what an `assert` over a
collection should say; `reduce` needs an explicit initial value:

```aura
ports = [8080, 8081, 9090]
assert ports.all (p, i) -> p > 1024 end, "ports must be unprivileged"
total: ports.reduce(0) (acc, p) -> acc + p end
```

`find(default)` and `index_of(value, default)` both require a fallback. There is
no `-1` and no `Null`: absent is an ordinary outcome, and it has to be named.

An object is traversed by turning it into pairs and back. `entries()` yields
`{ key, value }` objects in declaration order, and `to_object()` is the inverse:

```aura
def limits()
  cpu: 2
  mem: 8
end
doubled: limits().entries().map (e, i) ->
  key:   e.key
  value: e.value * 2
end.to_object()
```

`to_object()` is also the only way to build an object whose keys are computed,
because a property key is a literal. A repeated key is `E0323`, not an overwrite.

`+` joins two strings or two lists, and never converts between types:

```aura
base = ["auth", "billing"]
all: base + ["frontend"]
tag: "svc-" + name
```

`"port " + 8080` is `E0306` — call `.to_str()` on the number, or interpolate.
There is no `concat` method; `+` is the only way to join lists.

Braces inside a block string are literal — only `#{` starts interpolation. This is
what makes Aura usable for generating nginx configs, Dockerfiles and the like.

## Conditionals

`cond` is an expression and every arm produces a value. The `else` arm is
mandatory (E0207) — there is no path that yields nothing.

```aura
replicas: cond
  env == "production" -> 6
  env == "staging"    -> 2
  else -> 1
end
```

There is no truthiness. A condition must be `Bool`, or it is E0306.

## Functions

```aura
def service(name, port)
  kind: "Service"
  name: name
  port: port
end

api: service("checkout", 8080)
```

A `def` body is an object. Lambdas are `(a, b) -> expr`, used with list methods.
Functions are private unless `pub`.

## Schemas and enums

```aura
enum Tier
  "frontend"
  "backend"
end

type Service
  name: String
  port: Int
  tier: Tier
  replicas: Int = 1 # optional: a default makes it so
end

api: new Service
  name: "checkout"
  port: 8080
  tier: "backend"
end
```

A missing required field is E0511, a type mismatch E0512, a value outside an
`enum` E0514. An unknown field is a warning, or E0513 under `--strict`. A default
is evaluated at instantiation and cannot reference sibling fields — derive those
in a `def` instead.

## Assertions

```aura
assert replicas > 0, "replicas must be positive"
```

A failed assertion is E0530 and stops the build. `fail("...")` (E0531) aborts
unconditionally, which is how you reject an unreachable branch.

## Modules

```aura
import "shared.aura" as shared
import github/acme/pkg@v1.2 as pkg

port: shared.default_port
```

A registry import must carry a version. `aura.lock` pins the exact version and a
hash of the module's token stream; a mismatch is E0402.

## Capabilities — the part most likely to surprise you

A manifest can do nothing by default. Two calls touch the outside world, and both
are granted per run on the command line:

- `env("NAME", "default")` needs `--allow-env` or `--allow-env=NAME,OTHER`
- `read_file("path")` needs `--allow-read=<dir>`

Without the grant the call is E0310. With a grant that does not cover the path,
E0311.

**Grants belong to the root manifest and do not reach imports.** A module you
import cannot call `env()` or `read_file()` even when the root holds those rights,
and an exported function runs with the capabilities of the module it came from,
not the caller's. Assume any code you did not write cannot perform I/O.

`--hermetic` goes further: both calls become E0505 everywhere, and because that is
an analysis error, `aura check --hermetic` proves a manifest performs no I/O
without evaluating it.

## Determinism

`now()` and `timestamp()` do not exist and cannot be added (E0533). The same
manifest with the same inputs always produces byte-identical output. A build
timestamp is an input: pass it as `env("BUILD_TIME", ...)`.

Key order in the output is declaration order, not sorted.

## Common mistakes

- Using `=` for something that should appear in the output. It will silently not
  be there. This is the single most common error.
- Expecting `{}` to build an object, or `if`/`elif` to exist. Neither does.
- Omitting the `else` arm of a `cond`.
- Assuming an imported package can read a file because the root was granted it.
- Reaching for a loop. Use `.map`, `.filter` and the other list methods.
- Writing `1` where a `Float` is required — the types are distinct.

## Command line

```
aura eval <file>   [--strict] [--frozen] [--hermetic] [--dry-run]
                   [--allow-read=<dir>]... [--allow-env[=A,B]] [--allow-imports-io]
                   [--format json|json-flat|yaml|toml] [-o <file>]
aura check <file>  [--strict] [--hermetic]
aura fmt <files>   [--check]
aura types <file>  --lang rust|ts|go
aura add <path>@vX.Y.Z
```

Exit codes: `0` success, `1` diagnostics, `2` an I/O or argument error.
