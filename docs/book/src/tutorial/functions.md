# Functions and methods

## def — constructor functions

`def` returns an object; its body is properties:

```ruby
def make_env(name, replicas, debug)
  replicas:  replicas
  log_level: debug ? "debug" : "warn"
  db_url:    "postgres://db.#{name}.internal:5432/app"
end

environments:
  dev:  make_env("dev", 1, true)
  prod: make_env("prod", 6, false)
end
```

One `def` instead of three copy-pasted YAML files: this is the language's main
anti-duplication tool.

## Lambdas

```ruby
double = (x) -> x * 2 end
up: ["a", "b"].map (s, i) -> s.upper() end # a trailing lambda
```

A `map` or `filter` callback receives the element and its index; parameters you
do not need can be left out.

## Methods

Called with a dot, and they chain:

```ruby
active: services.compact().uniq().map (s, i) -> s.upper() end
```

The full list is in the [method reference](../reference/methods.md). The ones you
will reach for:

- **lists**: `map` `filter` `compact` (drop `null`) `uniq` `first` `last`
  `join(sep)` `contains(x)` `get(i, default)`
- **objects**: `merge` (the right side wins) `keys` `values` `contains(key)`
  `get(key, default)`
- **strings**: `upper` `lower` `len` `contains(sub)`, plus the format parsers

## The ternary operator

The language's only branching construct:

```ruby
mode: is_prod ? "webhook" : "long_polling"
```

The condition must be a `Bool`. Aura has no notion of truthiness, and anything
else is `E0306`.
