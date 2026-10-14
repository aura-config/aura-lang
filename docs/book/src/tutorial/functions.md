# Functions and methods

## def — constructor functions

`def` returns an object when its body is properties (and a value when it ends
in one - below):

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

## A function that returns a value

A function whose body ends in an expression returns that expression (D40).
Bindings and `assert` may come before it, so a step can be named:

```ruby
def ports_of(services)
  all = services.map (s, i) -> s.port end
  assert all.len() > 0, "no services"
  all.uniq().sort()
end

ports: ports_of(catalogue)
```

A multi-line lambda follows the same rule; a one-line lambda,
`(x) -> x * 2 end`, already was an expression.

### Three cases, and why each is as it is

- **Properties, or a last value - not both.** A body with `key:` properties
  that also ends in an expression is `E0215`. Returning the value would drop the
  properties, returning the properties would drop the value, and either loses
  something without a trace. Make the value a property (`value: ...`) or remove
  the properties.
- **A value nobody uses is an error.** A bare expression anywhere else - at
  the top of a file, in a `domain`, before the last line of a function - is
  `E0214`. Aura has no side effects, so a value that goes nowhere did nothing:
  the author almost always meant `key:` (output it), `name =` (use it later)
  or `assert` (check it), and the help names all three. Before D40 such a line
  was evaluated and silently dropped.
- **Only bindings and checks is still an empty object.** A body of `=` and
  `assert` alone returns `{}`, as before: nothing in it was meant to be the
  result, so nothing is thrown away.

Both errors come from `aura check`, so the editor shows them as you type.

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
