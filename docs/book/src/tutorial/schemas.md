# Schemas and validation

## Schemas

`type` declares a schema; `new` instantiates one with checking:

```ruby
type Container
  name:  String
  image: String
  port:  Int
end

main: new Container
  name:  "gateway"
  image: "company/gateway:2.4.1"
  port:  8080
end
```

What is checked at instantiation:

| Situation | Code | When |
| --- | --- | --- |
| A schema field is missing | `E0511` | always |
| Wrong type (`Int` vs `String`, `Int` vs `Float`) | `E0512` | always |
| An extra field the schema does not declare | `E0513` | `--strict` only |

Field types are `String`, `Int`, `Float`, `Bool`, `List` and `Object`. `Int` and
`Float` are separate types, so memory limits in bytes and 64-bit identifiers keep
their precision — overflow is `E0304`, not a silent wrap.

## assert

Arbitrary invariants are the `assert` statement:

```ruby
admins = [187650342, 244179081]
assert admins.uniq().len() == admins.len(), "Duplicate admin ids"
assert admins.len() >= 1
```

A failure is `E0530`, with your message and an exact position. `assert` also runs
under `--dry-run`: a rehearsal has to be able to find a validation failure.

## fail inside an expression

For validation in the middle of an expression there is `fail`:

```ruby
port: p > 0 && p < 65536 ? p : fail("invalid port #{p}")
```

## Closed sets: `enum`

A `String` field accepts any string, so a typo in a value rides all the way to
production. `enum` declares a closed set of permitted strings:

```ruby
enum Tier
  "frontend"
  "backend"
end

type Service
  tier: Tier
end

svc: new Service
  tier: "backand" # E0514, with "did you mean \"backend\"?"
end
```

An `enum` member is **an ordinary string**: the JSON contains `"backend"` and no
wrapper appears. It constrains the value; it does not introduce a new data type.

`pub enum` is exported to importers (D12). Members resolve where the schema is
declared, so an imported schema is validated against its own module's `enum` —
even if you have one of the same name in scope.

## Static analysis

`aura check` — and automatically before every `eval` — finds problems before
anything runs:

- `E0504` — use of an undeclared variable, including inside `#{...}`;
- `W0501` / `W0502` / `W0503` — unused variables, imports, functions and types;
- `W0512` — an effectful call (`env`, `read_file`) in an imported module.

Under `--strict` warnings become errors. That is the mode for CI.
