# Introduction

**Aura** is a configuration language that compiles to JSON, YAML or TOML. It is
built around three promises:

1. **Deterministic by construction.** Two runs produce byte-identical output. The
   language has no `now()`, no randomness and no implicit access to the
   environment — everything external (files, environment variables) arrives only
   through rights granted explicitly at the point of running it.
2. **Supply-chain safety.** An imported package physically cannot read your files
   or environment variables. The capability model is Deno's idea, applied more
   strictly: even calling a package's *exported function* runs with the package's
   rights rather than yours.
3. **Validated before deploy.** Typed schemas, `assert` invariants and static
   dead-code analysis mean configuration mistakes surface at build time instead of
   in production.

## What it looks like

```ruby
type Service
  name: String
  port: Int
end

base_port = 8000 # a private computation
is_prod   = env("APP_ENV", "dev") == "production"

api: new Service # a property — this reaches the output
  name: "api"
  port: base_port + 1
end

replicas: is_prod ? 3 : 1
assert base_port > 1024, "privileged ports are not allowed"
```

```console
aura eval app.aura --allow-env=APP_ENV
```

## Who it is for

- **DevOps and SRE**: Kubernetes manifests, CI matrices, service configs — without
  copy-pasted YAML, and schema-checked before `kubectl apply`.
- **Platform teams**: publishable packages of standards (`pub def` / `pub type`),
  versioned and locked by integrity hash.
- **Application developers**: one `.aura` declaration becomes JSON, YAML or TOML
  for any consumer; in Rust it embeds directly as a library.

## Where things are

- This book is the tutorial and reference for *using the language*.
- [SPEC.md](https://github.com/aura-config/aura-lang/blob/main/SPEC.md) is the
  formal specification, for anyone implementing it.
- [examples/](https://github.com/aura-config/aura-lang/tree/main/examples) holds
  working examples with their expected output.

> This book is also available [in Russian](ru/). (The path is relative to the
> book's root: the Russian translation is published under `/book/ru/`.)
