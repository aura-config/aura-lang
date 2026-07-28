# Security policy

## Reporting

Use GitHub's **private vulnerability reporting** on this repository
(Security → Report a vulnerability). It creates a private thread, so nothing is
public until there is a fix.

Please do not open a public issue for anything in the first list below.

## What counts as a vulnerability

Aura's security claim is narrow and precise, which makes it testable: **a
manifest gets only the I/O it was granted on the command line, and an imported
module gets none of it.** Anything that breaks that is a vulnerability, even if
it needs an unusual manifest to trigger.

Concretely, please report:

- **A capability escape.** An imported module performing I/O — `read_file()`,
  `env()` — without `--allow-imports-io`. This is the boundary
  `examples/security_demo/` demonstrates, and `E0310` is what should happen.
- **Reading outside the grant.** A manifest reading a path that is not under any
  `--allow-read` directory: symlink traversal, `..`, UNC paths, anything that
  escapes the canonicalised root.
- **Reading an environment variable that was not named.** `--allow-env=A` must
  not make `B` visible.
- **A lockfile or registry integrity failure.** A package whose contents do not
  match the SHA-256 in `aura.lock` being accepted, or `--frozen` resolving to
  something the lockfile does not pin.
- **Non-determinism.** Two runs of the same manifest, with the same grants and
  the same inputs, producing different output. Determinism is a security property
  here: it is what makes a reviewed config the config that ships.
- **Anything that reaches the network** without `aura add`, which is the only
  command that is supposed to.

## What does not count

These are bugs — please still report them as ordinary issues, they are worth
fixing — but they are not vulnerabilities and do not need a private thread:

- **A panic or a hang on malformed input.** `aura` is a local command-line tool
  that you point at your own files; a crash is a bug, not a privilege boundary.
  Deep nesting is bounded by `MAX_PARSE_DEPTH`, and the parser, evaluator,
  formatter and resolver are continuously fuzzed (see `fuzz/`).
- **Memory use on a large manifest.** Same reasoning.
- **A wrong diagnostic, or a diagnostic pointing at the wrong span.** A
  correctness bug.
- **A dependency advisory that Aura cannot reach.** If the affected code path is
  not one Aura uses, say so in a normal issue and we will still update.

## Supported versions

Version 0.1.x, and only the latest. Aura is pre-1.0: there is no long-term
support branch yet, and a fix ships in the next release rather than being
backported.

## Scope

This repository: the `aura` binary, the `aura-lang` library, `aura-lsp`, the
WebAssembly bindings and the playground page. The playground evaluates entirely
in your browser and sends nothing anywhere; if you find that it does, that is
very much a vulnerability.
