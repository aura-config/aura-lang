# Contributing to Aura

Thank you for looking. This file is short and specific, because vague
contribution guides waste the contributor's time first.

## What is most useful

In rough order of how much it helps:

1. **A bug with a manifest that reproduces it.** Two lines of input that behave
   wrong is the most valuable thing you can send. Most Aura bugs are obvious once
   they can be run.
2. **A real configuration that Aura handles badly.** Not "it should have feature
   X" but "here is what I was configuring and here is where it got in the way".
   These often have a better answer than the one either of us had in mind.
3. **Documentation that is wrong or missing.** If something in the book did not
   match what the tool did, that is a bug in the book.
4. **Builtin methods, CLI flags, output formats, editor features, integrations.**
   All genuinely open.

## Language changes

The syntax is frozen as of 0.1. Not because it is perfect — because a
configuration language whose syntax keeps moving is useless for the one thing it
is for, which is files you keep for years.

The design decisions behind it are recorded as the `D1`–`D18` series in
[`SPEC.md`](SPEC.md), each with its reasoning. Several plausible-sounding
additions were considered and **deliberately rejected** there; if your proposal is
one of them, the entry explains why, and that is a faster answer than a
discussion.

Proposals are welcome as issues. But the decision is the maintainer's, and a
syntax pull request that arrives without prior discussion will most likely be
declined regardless of its quality — please do not spend a weekend on one first.

## Working on it

```console
cargo test --workspace                                    # units, conformance, property tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo build -p aura-lang && find examples -name '*.aura' -print0 \
  | xargs -0 ./target/debug/aura fmt --check              # the repo's own .aura files
```

Fuzzing needs nightly and Linux or macOS; on Windows use WSL. See
[`fuzz/README.md`](fuzz/README.md). The WebAssembly module has its own smoke test
(`crates/aura-wasm/smoke.cjs`) because `cargo check --target wasm32` proves only
that it compiles — that gap once hid a real bug for months.

## Two things the review will look for

**The conformance snapshots are the contract.** `examples/*/expected.*` are the
outputs Aura promises for those inputs. If your change moves one, it moves
somebody's generated config. A snapshot may absolutely change — but deliberately,
and said out loud in the pull request, never quietly regenerated.

**Say how you verified it, not that tests pass.** For a bug fix: the test that
fails without your change. For anything touching slicing, spans or the formatter:
a few minutes of the relevant fuzz target. "Looks right" is where this project's
past mistakes came from.

## Style

Match the surrounding code — its comment density, naming and idioms — rather than
a general style guide. Two conventions are worth stating because they are not
obvious:

- **Comments in code are in English**, including in `examples/`.
- **Comments explain why, not what.** A comment that restates the line is noise;
  a comment recording the reason a non-obvious choice was made is why this
  codebase is navigable.

## Licensing

Aura is dual-licensed under [MIT](LICENSE-MIT) or
[Apache 2.0](LICENSE-APACHE), the usual arrangement in the Rust ecosystem.

By opening a pull request you agree that your contribution is licensed under the
same terms. There is no CLA and no sign-off ceremony — inbound equals outbound.

## Security

Do not open a public issue for a capability escape, a read outside a grant, or a
lockfile integrity failure. [`SECURITY.md`](SECURITY.md) says what counts and how
to report it privately.
