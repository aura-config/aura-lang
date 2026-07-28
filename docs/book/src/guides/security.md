# The security model

Aura assumes configuration gets assembled out of other people's code — packages,
a colleague's templates, snippets copied from somewhere — and builds its defences
**by construction** rather than by agreement.

## The capability model

By default a manifest can do **nothing**: it can read neither files nor
environment variables. Rights are granted by flags at the point of running it, and
they do not leak.

| Flag | Permits |
| --- | --- |
| `--allow-read=<dir>` | `read_file()` inside that directory; paths are canonicalised, and `..` cannot escape (`E0311`) |
| `--allow-env[=A,B]` | `env()` for the named variables — with no list, all of them |
| `--allow-imports-io` | extend the root's rights to imported modules |

A call without the right is `E0310`, with a hint naming the flag to add.

## Import isolation

Rights belong to the **root manifest**. An imported module:

- cannot call `env()` or `read_file()` — not even when the root holds those rights;
- does not acquire rights through its own exported functions: the body of a
  `pub def` runs with the capabilities of the module it *came from*, not the
  caller's;
- is flagged statically with warning `W0512` if it contains effectful calls.

```console
$ aura eval main.aura --allow-read=.
[E0310] Error: imported module has no capability to call read_file()
   ╭─[ evil_dependency.aura:2:7 ]
```

There is a runnable version of this in `examples/security_demo/`.

## Supply chain

- A package's version is mandatory in the import itself:
  `import github/acme/pkg@v1.2 as p`.
- `aura.lock` pins the exact version and an integrity hash of the module's **token
  stream**; a substitution is `E0402`. Hashing tokens rather than bytes means a
  reformat or an edited comment does not fire the check, while any change that can
  alter behaviour does. A lock written by an older version holds a `sha256-` byte
  hash; it still verifies, and is upgraded in place on the next run that is not
  `--frozen`.
- `--frozen` (the CI mode) forbids resolving outside the lock (`E0403`).
- The network exists only in `aura add`; evaluation is always offline.

## Determinism

- `now()` does not exist (`E0533`) — the host passes time in through `env`.
- Output key order is declaration order; two runs are byte-identical.
- `--dry-run` performs the whole evaluation but writes neither JSON nor the lock,
  and reports every read: `[dry-run] read: ./Cargo.toml`.
