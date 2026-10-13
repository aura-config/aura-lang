# Embedding, and calling from other languages

## Rust: as a library

```toml
[dependencies]
# `default-features = false` drops the `cli` feature, and with it clap, ariadne
# and ureq — which the library never uses. Measured on an empty project: 38
# packages instead of 96. The feature is on by default only so that
# `cargo install aura-lang` produces the `aura` binary.
aura-lang = { version = "0.3", default-features = false }
```

```rust
use aura_lang::facade::{eval_file, EvalOptions};

let opts = EvalOptions {
    strict: true,
    allow_read: vec!["config/".into()],
    ..Default::default()
};
let out = eval_file("config/app.aura".as_ref(), &opts)?;
let cfg: MyConfig = serde_json::from_value(out.json)?;      // straight into your own structs
for w in &out.warnings {
    log::warn!("{w}");                                       // Display: warning[W..]: ... at file:line:col
}
if let Some(lock) = out.updated_lockfile {
    std::fs::write("config/aura.lock", lock)?;               // writing it is the host's decision
}
```

Rights are set by the host application — the config never gets more than you
granted. Diagnostics arrive structured (`code`, `severity`, `file`, `line`,
`column`, `help`), so you can render them into your own log however you like.

A manifest that exists only in memory — a test harness, a browser, an editor
holding unsaved buffers — goes through `eval_source` instead, which takes a map of
name to text and resolves imports inside it.

In an async context, wrap `eval_file` in `spawn_blocking`: it reads files and
evaluates synchronously.

### Reading a whole manifest into your types

When the manifest *is* the configuration, read all of it at once, straight into
your own structs - no JSON in between, and the same result `aura eval` prints:

```rust
#[derive(serde::Deserialize)]
struct Device { board: Board, sensors: Vec<Sensor>, /* ... */ }

let device: Device = Script::builder()
    .file("device.aura")
    .allow_read("config")                      // the files it reads, beside it
    .env_values([("APP_ENV", "production")])   // exactly these, and no others
    .load()?
    .output()?;
```

`env_values` grants the named variables with the given values; `env()` reads
them from there and never from the process, so the service's own environment
cannot leak into the manifest, and a test can give it any environment without
touching the process's. `aura types --lang rust` writes the structs for the
schemas a manifest declares. Every example in the repository is read this way
in a test and compared with the output the CLI is pinned to.

### Calling into a script: `Script`

`eval_file` returns JSON, and JSON cannot hold a function. But an Aura `pub def`
*is* a value, so rules can live in a `.aura` file and the application can call
them — per request, per event, per batch — changing behaviour by editing the file
rather than rebuilding. `Script` is the type for that (D33).

```rust
use aura_lang::{Budget, Script};

#[derive(serde::Serialize)]
struct Event { device: String, temp_c: f64 }

#[derive(serde::Deserialize)]
struct Summary { total: u64, alerts: u64 }

let rules = Script::builder()
    .file("telemetry.aura")
    .budget(Budget::per_call().steps(5_000_000).memory_mb(32))
    .load()?;                                                // parse, analyse, prove hermetic — once

let summarize = rules.function::<(&[Event],), Summary>("summarize")?;  // name, kind, arity — checked now

for chunk in events.chunks(10_000) {
    let s = summarize.call((chunk,))?;                       // one call, one budget, nothing left behind
}
```

What it guarantees, in the order the decision ranks them:

- **Hermetic by default.** With no grants, `load` proves statically that the
  script reads no file, variable or clock, and fails with `E0505` if it could —
  on a branch that never runs, too. `allow_env` and `allow_read` grant exactly
  what `--allow-env` and `--allow-read` do, and reach the root module only.
- **Every call has a budget**, counted in steps and bytes — never in time, which
  would make the outcome depend on the machine. A runaway ends with `E0325` or
  `E0326` instead of stalling the host. The default is finite.
- **Memory per call is constant.** Each call runs in its own interpreter and
  releases what it made when it returns.
- **The boundary is checked.** Arguments cross by `serde`; an integer past `i64`
  or a NaN is `E0005` rather than a silently different input, and a result that
  does not fit your type is `E0006`, naming the path inside it.
- **`Script` is `Send + Sync`.** One instance serves every thread.

Measured on one core: a call costs about 1.2 µs, and a rule over events processes
roughly 0.9 million events a second called per event, 1.3 million in batches —
`cargo bench --bench script`. `cargo run --example embedding` runs the example
above end to end.

### Reloading rules while the service runs

Rules change more often than the service that runs them. A script loaded from a
file can be loaded again without a restart (D36):

```rust
use std::time::Duration;

let rules = Script::builder()
    .file("routing.aura")
    .watch(Duration::from_secs(2), |result| match result {
        Ok(v) => log::info!("rules at generation {} ({})", v.generation, v.hash),
        Err(e) => log::error!("rules not reloaded, the old ones keep serving: {e}"),
    })
    .load()?;

rules.reload()?;     // or reload now, with or without watch
```

- **A reload is a load.** It runs every check the first load did, with the same
  grants: it can never make a script less hermetic, reach a host function you
  did not provide, or pull in a package `aura.lock` did not pin - the lock is
  read once, and a reload resolves as `--frozen` does.
- **The exports are a contract.** Each export keeps its kind and number of
  arguments, or the reload is `E0011` and the old version keeps serving. New
  exports are fine. A bound `Function` follows its name across versions.
- **Never half a version.** A call finishes on the version it started on; the
  old version is freed when the last such call returns.
- **Content decides.** A `touch` loads nothing. While watching, a change is
  loaded only once the files have been left alone for the settle period - two
  intervals and at least a second, or `.settle(d)` - so a file still being
  written is not loaded half done. A writer that pauses mid-write for longer
  than that can still be caught: deploy by writing a new file and renaming it
  over the old one, as editors do, and raise `.settle` for a slow network
  share.
- **No silent failure.** The handler `watch` requires hears of every automatic
  reload - the new version, or the error - and of the same broken content once.
- `version()` names the generation and a hash of every file, for your logs.

`cargo run --example checkout` puts it all together: an online store's risk
rules, given a GeoIP table, exchange rates and a dispute database by the
service, reloaded on an edit - and refusing an edit that renames what the
service calls, or that reaches for a secret it was never given.

### Advanced: a script that calls its host

Some rules need data only the application has - which region an address is in,
whether a feature is on for a customer. Rather than computing every such value
in advance, the host can give the script named functions to call (D35). This is
an advanced use: read the guarantees below before relying on it.

```aura
import "host:geo" as geo

pub def route(ip)
  region = geo.region(ip)
  backend: region == null ? "global" : "#{region}-cluster"
end
```

```rust
let rules = Script::builder()
    .file("routing.aura")
    .pure("geo.region", move |ip: String| geoip.get(&ip).cloned())   // Option: None is null
    .effect("flags.enabled", move |flag: String, customer: String| {
        flags.enabled(&flag, &customer)                               // Result: Err is E0008
    })
    .load()?;
```

`pure` and `effect` are methods of the Rust API, not words of the language. The
part of the name before the dot is the module the script imports.

- **`pure` is your promise, and the language cannot check it.** A pure function
  must give the same answer for the same arguments - a table that ships with the
  binary, a formula. It leaves the load hermetic. Anything that reads the world -
  a database, a flag service, the clock - is an `effect`: registering it is a
  grant, like `allow_env`, and the load is no longer hermetic.
- **The root module only.** A package the script imports cannot call host
  functions; `grant_to_imports("geo.region")` opens one function to imports.
- **Checked at load.** A module or function the host did not provide is `E0007`,
  a call with the wrong number of arguments is `E0009` - on branches that never
  run, too. Argument types are checked at the call, also as `E0009`.
- **Budgeted.** A host call costs 100 steps (`host_call_steps` changes it). The
  time spent inside your function is invisible to the budget: bound it yourself.
- **No call back.** A host function that calls into the script gets `E0010`.

`aura eval` has no host, so a `host:` import there stops with `E0007`;
`aura check` lists what the script needs from whichever application runs it.
`cargo run --example host_functions` runs a complete example.

## Any language: a subprocess

The CLI contract is stable and is an API: JSON on stdout, diagnostics with
`E0xxx` / `W0xxx` codes on stderr, exit codes `0` (success), `1` (diagnostics),
`2` (I/O or arguments).

```python
import json, subprocess
r = subprocess.run(["aura", "eval", "app.aura", "--frozen"],
                   capture_output=True, text=True)
if r.returncode != 0:
    raise RuntimeError(r.stderr)
config = json.loads(r.stdout)
```

Recommended in production:

- `--frozen` — dependencies strictly as recorded in `aura.lock`;
- minimal, explicit rights (`--allow-read`, `--allow-env=NAMES`);
- `--format yaml|toml` if that suits the consumer better.

## Mobile and browser applications

The right pattern is to evaluate on a server or in CI:

```text
configs.aura ──aura eval──▶ config.json ──▶ CDN / bundle
the device reads finished JSON with its platform's own parser
```

Applications consume the *result* of Aura; validation already happened in CI.

## Why wrappers rather than native reimplementations

Every language has its own YAML parser, so it is fair to ask why Aura does not
work that way. Because YAML is a data format: porting it means writing a parser,
bytes in and data out. Aura is an evaluated language — functions, imports,
schemas, capabilities, determinism — so porting it means writing an interpreter
that is obliged to match this one **byte for byte**. Otherwise the same manifest
yields different values in CI and in the service, which is to say in production.
The divergence is not exotic: a naive implementation on Go's `encoding/json`
emits `1` where Aura emits `1.0`, and `1000000000000000000` where Aura emits
`1e+18`.

So there is one implementation, and other languages reach it through:

1. **Evaluation at build time** — the main path. No Aura runtime is needed inside
   your language at all, and `aura types` will additionally generate types for
   your JSON.
2. **Wrappers around the same core** — on the roadmap, in this order: WASM/npm
   (one artifact covers Node, the browser and a documentation playground), then
   `wazero` for Go (pure Go, no cgo) and wasmtime for Python; PyO3 and UniFFI as
   demand appears.

The core already builds for `wasm32-unknown-unknown`; CI checks that on every
commit, and runs the resulting module under Node so that "it compiles" is not
mistaken for "it works".
