# Working on Aura

Notes for a coding assistant contributing to the compiler. If you are **writing
`.aura` manifests** rather than changing Rust, you want a different document: run
`aura docs --agent`, or read [llms.txt](llms.txt).

## The gate

Every change must pass these three. CI runs exactly the same commands.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Check the exit code, not the output.** `cargo clippy … | grep -c "^error"`
always reports zero, because clippy colours its output and the line begins with an
escape sequence rather than the word `error`. That mistake let a commit reach CI
and fail there.

`crates/aura-wasm` has **its own workspace** — `cargo test --workspace` from the
root does not cover it. Run its checks from its own directory.

## Where the truth lives

Several things exist in exactly one place, deliberately, and are checked by tests
that fail if a second copy appears and disagrees.

| Fact | Source | Kept honest by |
| --- | --- | --- |
| Standard library surface | `crates/aura-lang/stdlib.aura` | `stdlib::tests::manifest_matches_registry` compares it with the real method registry |
| Diagnostic codes | the `"E0xxx"` literals in `src/` | `tests/diagnostic_catalogue.rs` compares them with both books |
| The agent reference | `agent-preamble.md` + generated sections | `tests/agent_reference.rs` checks `llms.txt` is current and its examples parse |
| Playground examples | `playground/app.js` | `tests/playground_examples.rs` runs each one |
| Snippets in the books | the Markdown itself | `tests/docs_snippets.rs` |

If you are about to write down a fact that already exists somewhere, do not.
Generate it, or assert it.

## What tends to go wrong

The recurring defect in this repository is **a file asserting something that is no
longer true**, and it is never found by reading. A partial list, all found by
measuring: the roadmap marked finished work as pending; the published crates
declared a licence whose text they did not contain; the integrity hash's tag table
was almost entirely unexercised, which is where a collision would hide; `json-flat`
silently dropped a key; the release workflow created two draft releases for one
tag; the error catalogue documented a code that could never be emitted while
omitting seven that could.

So: when a change touches something a document claims, check the document. When
you add a claim, add the test that would notice it becoming false.

## Conventions

- **Comments and commit messages in English.** The books exist in English
  (`docs/book/`) and Russian (`docs/book-ru/`) and must stay in step — a test
  checks the diagnostic tables match.
- Tests go on critical paths, not everywhere. A test that cannot fail is worse
  than no test, because it reads as coverage.
- Benchmarks (criterion) for anything on a hot path: lexer, parser, evaluator,
  resolver.
- Never commit without the maintainer's approval of the commit message.

## Layout

```
crates/aura-lang/   the language: lexer, parser, eval, analysis, fmt, codegen, vfs
crates/aura-lsp/    the language server, built on aura-lang
crates/aura-wasm/   the browser build (own workspace, size-tuned profile)
packaging/e2e.sh    end-to-end checks run against a built binary, in containers
examples/           themed manifests; conformance tests evaluate them
fuzz/               six cargo-fuzz targets (run under WSL on Windows)
```

`SPEC.md` is the formal specification, including the numbered design decisions
(D1–D18) that the code refers to by name.
