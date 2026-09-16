# Computing and exporting: `=` versus `:`

The one rule that sets Aura apart from YAML and JSON — the same distinction as
locals versus outputs in Terraform:

| Form | Meaning | In the output? |
| --- | --- | --- |
| `x = expr` | a private variable | no |
| `key: expr` | a property | **yes** |
| `domain "name" ... end` | a block | **yes** (the key is the name) |

```ruby
base = 8000     # a computation
port: base + 80 # output: {"port": 8080}
```

This is what makes dead-code analysis exact: an unused `=` variable is always
genuine litter (`W0501`), never "perhaps someone's output".

## Immutability

Assigning twice in the same scope is an error:

```ruby
x = 1
x = 2 # E0301: variable 'x' is immutable
```

## Shadowing is explicit

Redefining an outer variable inside a nested block requires saying so:

```ruby
log_path = "/var/log/app.log"

domain "debug"
  log_path        = "/tmp/debug.log" # E0302: shadow is required
  shadow log_path = "/tmp/debug.log" # fine — the intent shows up in the diff
  path: log_path                     # → "/tmp/debug.log"
end
```

"Why is the path different in production?" stops being a silent surprise:
shadowing is always visible as the word `shadow`.

## Long numbers

Group the digits with `_` where it helps the eye:

```aura
max_bytes: 10_000_000
retention: 2_592_000
port:      8_080
rate:      0.000_001 # in a fraction the groups run away from the point
```

It is entirely optional: `2371933` and `2_371_933` are the same number, and
nothing asks you to group anything.

When you do, the separator has to **group by threes, counted away from the
decimal point**. The short group therefore sits at the front of an integer and
at the end of a fraction: `8_080`, `1.123_45`.

That is stricter than Go, Ruby or Rust, where `2_3_7_1_9_3_3` is legal. Two
reasons. A separator free to sit anywhere is a second way to write one number,
and this language spends its rules removing those. And it misleads: `1_0000_000`
is ten million, but anyone scanning for threes reads something far larger.

`100_`, `1__0` and `1_.5` are errors too — there is nothing to separate.

This exists because configuration is where long numbers actually live, and a
miscounted zero is the mistake nothing else catches — the value is valid, the
type is right, and the service quietly gets ten times what it should.

## Strings and interpolation

```ruby
name = "auth"
image: "company/#{name}:v#{1 + 1}" # → "company/auth:v2"
```

Inside `#{...}` the ordinary expression syntax applies, and quotes in nested
strings need no escaping — `"#{list.join(", ")}"` is valid.

## Reaching into data

A dot is for fields; brackets are only for list indices:

```ruby
version:  cfg.package.version         # ordinary keys
special:  cfg."key with a space"      # any key, as a string
dynamic:  cfg.envs."#{region}"        # a computed key
first:    apps[0].name                # an index (out of range is E0317)
optional: cfg.get("maybe", "default") # a miss returns the default, not an error
```

A typo in a field name is error `E0308` with an exact position, not a silent
`null`.
