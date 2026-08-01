# Embedding, and calling from other languages

## Rust: as a library

```toml
[dependencies]
# `default-features = false` drops the `cli` feature, and with it clap, ariadne
# and ureq — which the library never uses. Measured on an empty project: 38
# packages instead of 96. The feature is on by default only so that
# `cargo install aura-lang` produces the `aura` binary.
aura-lang = { version = "0.1", default-features = false }
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
    log::warn!("{w}");                                       // Display: error[E..]: ... at file:line:col
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

A note on threads: `Interpreter` is not `Send`, and every `eval_file` call is
self-contained, so in an async context wrap the whole call in `spawn_blocking`.

### Sketch: rules the application executes, not just data it reads

`eval_file` returns JSON, and JSON cannot hold a function. But an Aura `pub def`
*is* a value, and the interpreter can call one — so a script can define
`route(path, method)` and the application can call it per request, changing
behaviour by editing a file rather than rebuilding.

`cargo run --example scripting` in this repository demonstrates it end to end.

It is marked a sketch deliberately. The pieces are public API and work today,
but reaching them means dropping below `facade` and keeping the `SourceCache`
alive by hand, and a script still cannot call *back* into the application.
Closing that second gap has to be shaped like the capability model rather than
bolted beside it — arbitrary host functions would give away exactly the property
that makes Aura worth embedding, that `aura check --hermetic` can prove a script
touches nothing before you run it.

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
