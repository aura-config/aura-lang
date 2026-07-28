# Blocks and scopes

Configuration is rarely flat. Aura has two forms of nesting, and they differ not
in what they output but in **what you are allowed to write inside**.

| Form | What goes inside | In JSON |
| --- | --- | --- |
| `key:` … `end` | properties only — data | a nested object |
| `domain "name"` … `end` | properties **and computations** (`=`, `shadow`, `assert`, `def`) | an object under the label as key |

## An object literal: just data

```ruby
security:
  tls:         true
  min_version: "1.3"
end
```

```json
{ "security": { "tls": true, "min_version": "1.3" } }
```

Only properties are allowed inside. If you need computations, use `domain`.

## `domain` — a section with its own scope

The label becomes the key, and the body is a full scope: `=`, `shadow` and
`assert` all work.

```ruby
domain "prod"
  replicas: 3
  region:   "eu-central"
end
```

```json
{ "prod": { "replicas": 3, "region": "eu-central" } }
```

The label can be any expression, not only a string literal.

## Scope and `shadow`

Outer variables are visible, but overriding one requires an explicit `shadow`
(D7):

```ruby
port = 80

domain "prod"
  shadow port = 443 # without `shadow` this is E0302
  listen: port      # 443
end

fallback: port # 80 — nothing outside changed
```

This is the direct answer to "why is the port different in production": the
override is visible in the code.

## Lists of objects: `map`, with no extra keywords

A lambda body is a scope too (D17), so list elements are described directly,
private computations included:

```ruby
services: ["api", "web"].map (n, i) ->
  base = 8000 # private: never reaches the JSON
  name: n
  port: base + i
end
```

```json
{
  "services": [
    { "name": "api", "port": 8000 },
    { "name": "web", "port": 8001 }
  ]
}
```

The `name` field here is an ordinary property that you write yourself. A `component`
keyword used to insert that line implicitly; D17 removed it, because the language
should not add fields behind your back.

## Which to use

- **Data only** → `key:` … `end`.
- **Data plus computation** → `domain`.
- **A list of objects** → `map` with a lambda body; no block needed.

The general rule: **code bodies** (`def`, a lambda, `domain`) are scopes; an
**object literal** is data.
