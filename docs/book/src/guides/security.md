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
| `--allow-read=<dir>` | `read_file()` inside that directory; paths are canonicalised, and `..` cannot escape (`E0311`). A relative path is read from the manifest's own folder (D37) |
| `--allow-env[=A,B]` | `env()` for the named variables — with no list, all of them |
| `--hermetic` | no I/O at all: `env()` and `read_file()` are `E0505` in every module; refused together with `--allow-*` |
| `--allow-imports-io` | extend the root's rights to imported modules |

A call without the right is `E0310`, with a hint naming the flag to add.

## Hermetic mode

`--hermetic` is the opposite end of the same dial: instead of granting rights, it
declares that none exist. `env()` and `read_file()` become `E0505` wherever they
appear — root or import — and the flag cannot be combined with any `--allow-*`.

The distinction from simply omitting the grants is *when* you find out. A denied
call is `E0310` at runtime, on the branch that happened to execute; `E0505` is an
analysis error, so `aura check --hermetic` proves the property for the whole
manifest without evaluating it. That makes it usable as a gate:

```console
$ aura check --hermetic manifests/*.aura
```

CUE, KCL and Starlark are hermetic by construction. Aura makes it a mode, so one
repository can hold both the manifests that must read the environment and the
manifests that must not, and CI can require the strict kind per directory.

## Bounded evaluation

Hermetic proves a manifest cannot *take* anything. It does not prove the manifest
will *finish*: three nested `range(1000).map` fit in 95 bytes, pass
`check --hermetic`, and would run for over a minute. Every evaluation therefore
has a budget — the CLI, `eval_file`, the playground and `Script` alike:

| Budget | Exhausted | Default | Flag |
| --- | --- | --- | --- |
| steps — expressions evaluated | `E0325` | 10 million | `--max-steps=N` |
| bytes — every value and frame created | `E0326` | 256 MiB | `--max-memory=MB` |

Both are counted, never timed, so the same manifest stops at the same point on
every machine. The defaults are about thirty thousand times what the heaviest
example in the repository spends; `--stats` prints what a manifest spends, so a
budget can be set from a measurement.

Depth has a limit of its own, because steps do not bound it: a function calling
itself a few hundred times spends few steps and a great deal of stack. A single
expression nests at most 256 levels - a long chain such as `a + b + c + ...`
counts its length - and evaluation nests at most 256 levels at run time, a
recursive call counting the expressions it passes through. Past either is an
error, `E0208` or `E0399`, never a stack overflow (D42). Like the budget, the
limit is a count, so it stops a manifest at the same point everywhere. A release
build fits it in 512 KiB of stack, so a host may call a `Script` from any thread
with at least that much - an iOS secondary thread has exactly 512 KiB.

Values have the same limit (D43). A loop can build a deep value without any
nesting of its own - `reduce` wrapping its accumulator adds a level a turn - and
every walk over a value, from writing it out to freeing it, recurses once per
level. So a list or object deeper than 256 is `E0328` where it is built, and a
value from a host deeper than that is refused at the boundary. Parsed JSON and
YAML already stop at 128.

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

## Host functions and reload

An application that embeds Aura can give a script its own functions (D35) and
reload it while running (D36). Both are held to the model above:

- A host function is a grant, named by the application, one function at a time.
  `pure` ones - the host's promise of determinism - keep a script hermetic;
  `effect` ones are grants like `--allow-env`.
- They reach the root module only. An imported package cannot call one unless
  the application grants that function to imports by name (`E0310`).
- Every use is checked at load, before anything runs: a function not provided is
  `E0007`. A host function cannot call back into the script (`E0010`).
- A reload is a full load with the first load's grants and its `aura.lock`,
  frozen: an edited script cannot widen its rights, pull in an unpinned package,
  or drop an export the application bound (`E0011`). A file still being written
  is not loaded until it has been left alone for the settle period.

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
