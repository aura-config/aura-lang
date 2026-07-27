# Fuzzing Aura

Coverage-guided fuzzing of the parser/interpreter with
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer). This is a
standalone crate (its own `[workspace]`) kept out of the stable workspace,
because `cargo-fuzz` requires **nightly Rust** and libFuzzer (Linux/macOS;
Windows/MSVC is not supported). On Windows, run it from **WSL** (see below).

## Targets

- **`fuzz_lexer`** — `Lexer::tokenize` on arbitrary bytes must not panic/hang.
- **`fuzz_parser`** — lex + parse arbitrary UTF-8; must not panic, hang, or
  overflow the stack (deep nesting is capped by `MAX_PARSE_DEPTH` → E0208).
- **`fuzz_pipeline`** — lex + parse + eval with all capabilities denied and no
  resolver, so it is deterministic and does no I/O; must not panic/hang/OOM.
- **`fuzz_fmt`** — `aura fmt` must not panic, and must never change the token
  stream (the formatter's own backstop, asserted here on arbitrary input).
- **`fuzz_codegen`** — `aura types` on arbitrary input: a malformed manifest is a
  `Diagnostic`, never a crash.
- **`fuzz_resolve`** — name resolution: every span it reports must be in bounds
  and on a char boundary, or a consumer slicing by it would panic instead.

## Seed corpora

`corpus/<target>/` holds **only reviewable `*.aura` seeds**, which are tracked in
git. Everything libFuzzer writes there itself — hash-named coverage inputs, tens
of thousands of them after a long session — is ignored (see `.gitignore`): those
files are unreadable in review, would dominate the repository's size, and are
cheap for the fuzzer to rediscover. Named regression seeds
(`blockstring_nbsp_regression.aura`) stay forever, so a fixed bug is re-tested on
every run.

Seeds are the `examples/*.aura` manifests plus hand-written files aimed at the
construct a given target is about: scope shapes for `fuzz_resolve` (chained
`shadow`, a lambda parameter shadowing an outer binding, interpolation over a
shadowed name, a parameter named like its own function), declaration shapes for
`fuzz_codegen` (non-identifier enum members, every `TypeName`, forward-referenced
custom types, nothing to emit at all).

Whether a new seed is worth keeping is measurable rather than a matter of taste —
run the target over the seed directory alone and read libFuzzer's counters:

```sh
cargo +nightly fuzz run fuzz_resolve corpus/fuzz_resolve -- -runs=0
#9  INITED cov: 1292 ft: 2744 corp: 8/6345b
```

`cov` is edges reached, `ft` the features (edge counts and value profiles) that
drive mutation. Adding the six scope-shape seeds above moved `fuzz_resolve` from
`cov: 1262 ft: 1778` to `cov: 1292 ft: 2744` — few new edges, since the showcase
manifest already reached most of them, but a much richer starting point to mutate
from. `fuzz_codegen` went from `cov: 746` to `cov: 851`.

## Running

```sh
rustup toolchain install nightly
cargo install cargo-fuzz

# run a target (Ctrl-C to stop); flags cap the resources that caught the
# original recursion DoS — a hang/OOM aborts the run with a repro.
cargo +nightly fuzz run fuzz_parser -- -rss_limit_mb=1024 -timeout=10
cargo +nightly fuzz run fuzz_pipeline -- -rss_limit_mb=1024 -timeout=10
```

A crash writes the offending input to `fuzz/artifacts/<target>/`; reproduce with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`.

## On Windows: fuzz from WSL

libFuzzer needs a Unix toolchain, so on Windows run the fuzzer inside WSL against
the repo on `/mnt/c` (needs `cc`/`gcc`, present on Ubuntu by default):

```sh
rustup toolchain install nightly --component rust-src
cargo install cargo-fuzz --locked
cd /mnt/c/…/aura-lang
cargo +nightly fuzz run fuzz_pipeline -- -max_total_time=90 -rss_limit_mb=2048
```

This is the fastest local reproducer: two char-boundary panics (a block-string
NBSP indent strip and `parse_datetime`'s byte-indexed `split_at`) were confirmed
and fixed this way before relying on the non-blocking CI fuzz job.

CI runs each target for a bounded time on Linux nightly (a non-blocking job).
