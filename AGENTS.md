# Working on Aura

Notes for a coding assistant contributing to the compiler. If you are **writing
`.aura` manifests** rather than changing Rust, you want a different document: run
`aura docs --agent`, or read [llms.txt](llms.txt).

## The gate

Every commit must pass all four. CI runs exactly these commands.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p aura-lang && find examples -name '*.aura' -print0 | xargs -0 ./target/debug/aura fmt --check
```

The last one is the language formatting its own examples. It is easy to forget
because it is not a `cargo` command, and CI's Lint job fails on it all the same.

**Run the gate on every commit, not on the first one in a branch.** A second
commit that "only adds a test" is exactly the one that skips `cargo fmt` and
fails in CI.

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
| Standard library surface | `crates/aura-lang/stdlib.aura` | `manifest_matches_registry` (in `crates/aura-lsp/src/stdlib.rs`) compares it with the real method registry |
| The method reference in both books | the same manifest | `tests/method_reference.rs` — every book documents every method, and the translations agree |
| Diagnostic codes | the `"E0xxx"` literals in `src/` | `tests/diagnostic_catalogue.rs` compares them with both books |
| Design decisions | the decision log in `SPEC.md` and `SPEC.ru.md` | `tests/decision_log.rs` — every `Dn` cited anywhere has a row, in both translations |
| The version | the workspace `Cargo.toml` | `tests/version_consistency.rs` — the wasm crate, both lockfiles and the changelog agree |
| The agent reference | `agent-preamble.md` + generated sections | `tests/agent_reference.rs` checks `llms.txt` is current and its examples parse and are formatted |
| Snippets in the books | the Markdown itself | `tests/docs_snippets.rs` — formatted, and naming nothing the language lacks |
| Playground examples | `playground/app.js` | `tests/playground_examples.rs` runs each one |
| Stdlib coverage in examples | `examples/` | `tests/showcase_coverage.rs` — the showcases together use every stdlib name |
| Reading foreign formats | the three readers in `eval/methods.rs` | `tests/format_bridge.rs` and the `fuzz_bridge` target |
| Line-ending independence | — | `tests/line_endings.rs` writes CRLF at run time, because `.gitattributes` hides it from every checkout CI makes |

If you are about to write down a fact that already exists somewhere, do not.
Generate it, or assert it. **That includes counts:** "six fuzz targets" and
"decisions D1–D18" were both in this file, and both were wrong within weeks.

## What tends to go wrong

The recurring defect in this repository is **a file asserting something that is no
longer true**, and it is never found by reading. A partial list, all found by
measuring: the roadmap marked finished work as pending; the published crates
declared a licence whose text they did not contain; the integrity hash's tag table
was almost entirely unexercised, which is where a collision would hide; `json-flat`
silently dropped a key; the release workflow created two draft releases for one
tag; the error catalogue documented a code that could never be emitted while
omitting seven that could; the Russian method reference lacked half the library;
the agent preamble taught a `cond` example that did not compile; three adopted
decisions were cited everywhere and logged nowhere; and the published crate and
the repository carried the same version number while being different languages.

So: when a change touches something a document claims, check the document. When
you add a claim, add the test that would notice it becoming false.

A **help line in a diagnostic is a claim about code**. Run the code it suggests in
a test. `E0516`'s first remedy advised `q: Int?` for a null default, which does
not work — the marker alone still requires the field at `new` — and only
executing it showed that.

## Conventions

- **Comments and commit messages in English.** The books exist in English
  (`docs/book/`) and Russian (`docs/book-ru/`) and must stay in step — tests check
  that the diagnostic tables, the method references and the decision logs agree.
- Tests go on critical paths, not everywhere. A test that cannot fail is worse
  than no test, because it reads as coverage. **Before trusting a new test, break
  the thing it guards and watch it fail.**
- Benchmarks (criterion) for anything on a hot path: lexer, parser, evaluator,
  resolver.
- Never commit without the maintainer's approval of the commit message.

## Layout

```
crates/aura-lang/   the language: lexer, parser, resolve, analysis, eval,
                    serialize, fmt, codegen, vfs, and the host-facing facade
crates/aura-lsp/    the language server, built on aura-lang
crates/aura-wasm/   the browser build (own workspace, size-tuned profile)
packaging/e2e.sh    end-to-end checks run against a built binary, in containers
examples/           themed manifests; conformance tests evaluate them
fuzz/               cargo-fuzz targets, one per stage (run under WSL on Windows)
```

`SPEC.md` is the formal specification. Its decision log — the numbered `Dn`
entries — is what the code, the diagnostics and the changelog refer to by name.
