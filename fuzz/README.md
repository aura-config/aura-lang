# Fuzzing Aura

Coverage-guided fuzzing of the parser/interpreter with
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer). This is a
standalone crate (its own `[workspace]`) kept out of the stable workspace,
because `cargo-fuzz` requires **nightly Rust** and libFuzzer (Linux/macOS;
Windows/MSVC is not supported).

## Targets

- **`fuzz_lexer`** — `Lexer::tokenize` on arbitrary bytes must not panic/hang.
- **`fuzz_parser`** — lex + parse arbitrary UTF-8; must not panic, hang, or
  overflow the stack (deep nesting is capped by `MAX_PARSE_DEPTH` → E0208).
- **`fuzz_pipeline`** — lex + parse + eval with all capabilities denied and no
  resolver, so it is deterministic and does no I/O; must not panic/hang/OOM.

Seed corpora (`corpus/<target>/`) are the `examples/*.aura` files and the
reference manifest, so the fuzzer starts from valid inputs and mutates them.

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

CI runs each target for a bounded time on Linux nightly (a non-blocking job).
