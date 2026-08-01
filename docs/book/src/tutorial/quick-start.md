# Quick start

## Installing

The fastest way to try the language is to install nothing: the
[playground](https://aura-config.github.io/aura-lang/playground/) runs the real
compiler in your browser.

For a binary:

```console
cargo install aura-lang
```

Or download a build for your platform from
[Releases](https://github.com/aura-config/aura-lang/releases) — Linux (gnu and
static musl), macOS (Intel and Apple silicon) and Windows, each with a `.sha256`.
On x86_64 Linux prefer the musl build: it is static, so it has no glibc floor.

In GitHub Actions:

```yaml
- uses: aura-config/setup-aura@v1
- run: aura check deploy.aura --strict
```

To build from source instead:

```console
git clone https://github.com/aura-config/aura-lang && cd aura-lang
cargo build --release
# the binary: target/release/aura
```

## Your first manifest

Create `hello.aura`:

```ruby
app_name = "hello"
port     = 8080

service:
  name:     app_name
  url:      "http://localhost:#{port}"
  replicas: 2
end
```

```console
$ aura eval hello.aura
{
  "service": {
    "name": "hello",
    "url": "http://localhost:8080",
    "replicas": 2
  }
}
```

Notice that `app_name` and `port` — declared with `=` — **did not reach** the
JSON: they are private computations. Only properties (`name:`) and blocks are
exported. The next chapter goes into that.

## The three commands you need daily

```console
aura eval app.aura            # evaluate → JSON on stdout
aura check app.aura --strict  # check without evaluating (a linter; for CI)
aura fmt app.aura             # canonicalise the layout
```

## Structure without the pain

Aura has no significant indentation — this is where YAML scar tissue heals.
Structure comes from line breaks and an explicit `end`:

```ruby
server:
  http:
    port: 8080
    timeouts:
      read: "30s".parse_duration()
    end
  end
end
```

Indentation is for humans only. `aura fmt` canonicalises it automatically and is
guaranteed not to change meaning: the token stream before and after is identical.
