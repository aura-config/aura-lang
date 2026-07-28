<!-- Thank you for this. The checklist below is the same one the maintainer works
     through; it is here so review can be about the change itself rather than
     about the mechanics. -->

## What this changes

<!-- And why. If it fixes an issue, "Fixes #123" links them. -->

## How it was verified

<!-- Not "tests pass" — what you did that would have caught the mistake. For a
     bug fix, the test that fails without the change. For a language change, the
     conformance snapshot. -->

## Checklist

- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `cargo fmt --all --check` is clean
- [ ] `aura fmt --check` is clean on every `.aura` file touched

If it changes how a manifest is **parsed, evaluated or rendered**:

- [ ] The conformance snapshots in `examples/*/expected.*` are unchanged — or the
      change to them is deliberate and explained above. These are the contract:
      a snapshot moving means someone's config output moves.
- [ ] Fuzzed the affected target for a few minutes (`fuzz/README.md`) and no new
      artefact appeared. Slicing, span arithmetic and the formatter especially.

If it adds a builtin method or a CLI flag:

- [ ] Documented in `SPEC.md` and in the book, and the stdlib manifest
      (`crates/aura-lsp/stdlib.aura`) lists it, so completion and hover know
      about it. A test enforces that the manifest and the registry agree.

If it touches diagnostics:

- [ ] The error code is documented in `docs/book/src/reference/error-codes.md`.
      Codes are a contract — people match on them in CI.
