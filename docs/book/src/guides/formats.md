# Formats: TOML, JSON, YAML

## Reading

The three parsers are string methods, and they return ordinary Aura objects and
lists:

```ruby
cargo = read_file("./Cargo.toml").parse_toml()
team  = read_file("./team.json").parse_json()
lint  = read_file("./.rules.yaml").parse_yaml()

service:
  name:  cargo.package.name
  owner: team.teams[0].lead
end
```

Integers from every format arrive as `Int`, with no loss of precision. A parse
failure is `E0314`, carrying the underlying library's message.

## Writing

From the CLI, with a flag:

```console
aura eval app.aura --format json       # the default, pretty-printed
aura eval app.aura --format json-flat  # flattened keys: a.b.c = 1
aura eval app.aura --format yaml
aura eval app.aura --format toml       # requires an object at the top level
```

From inside the language, with methods — useful for configs nested as strings:

```ruby
configmap:
  "app-config.yaml": settings.to_yaml()
end
```

TOML's limitations become honest `E0603` errors: it has no `null`, and it requires
an object at the top level.

## Aura as a converter

Because the language reads and writes all three formats, a migration is one line:

```ruby
# convert.aura
config: read_file("./legacy.toml").parse_toml()
```

```console
aura eval convert.aura --allow-read=. --format yaml
```

Unlike `yq` or `jq`, schemas, `assert` and merging several sources are available
along the way — conversion with guarantees.
