# Modules and packages

## Imports

```ruby
import "templates/defaults.aura" as defaults # a file, relative to this one
import github/acme/aura-k8s@v1.2 as k8s      # a registry package; the version is mandatory
```

Cyclic imports are reported with the whole chain:
`E0401: cyclic import: a.aura -> b.aura -> a.aura`.

## What a module exports

A module hands the importer an object: its properties, its blocks, and its **pub**
items.

```ruby
# validators.aura
pub type Service # the schema is part of the API
  name: String
  port: Int
end

def valid_port(p) # a private helper: invisible to importers
  ok: p > 0 && p < 65536
end

pub def service(name, port) # the function is part of the API
  name: name
  port: valid_port(port).ok ? port : fail("invalid port #{port}")
end
```

```ruby
# using it
import "validators.aura" as v

api:    v.service("api", 8080)
worker: new v.Service
  name: "worker"
  port: 9000
end
```

The key safety guarantee: **an exported function runs with its own module's
rights, not the caller's**. A package cannot borrow your permission to read files
by getting you to call its function.

## Installing packages: aura add

```console
aura add github/acme/aura-k8s@v1.2.3           # from the network (an exact version)
aura add pkg/internal@v1.0.0 --from ./pkg.aura # from a local file
```

`aura add` is **the only place Aura touches the network**: the package is
downloaded, validated, placed in the local cache (`~/.aura/registry`) and recorded
in `aura.lock` with a SHA-256 hash. `eval` always works offline.

## aura.lock and --frozen

- the lock stores the exact version and integrity hash of every package;
- if a package's contents change underneath it, that is `E0402 integrity mismatch`;
- in CI, run with `--frozen`: resolution goes strictly through the lock, a missing
  entry is `E0403`, and the lock is never rewritten.

Version ranges (`@v1`, `@v1.2`) resolve against the local cache to the highest
match — `v1.2` selects `1.2.*`.
