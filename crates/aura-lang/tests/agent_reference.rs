//! `llms.txt` is generated, committed, and therefore able to go stale.
//!
//! It is committed rather than only generated because that is how most agents
//! will meet it: read from the repository or from the published site, without
//! running anything. The cost of that convenience is a file that can drift from
//! the compiler it claims to describe — the failure this repository keeps
//! finding — so the check is the same one `cargo fmt --check` makes.
//!
//! The examples inside it are checked too. A reference exists so an assistant
//! does not guess at syntax; if its own snippets do not parse, it is teaching
//! the guess.

use std::path::{Path, PathBuf};

use aura_lang::fmt::format_source;
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn committed() -> String {
    std::fs::read_to_string(repo_root().join("llms.txt")).expect("llms.txt at the repository root")
}

#[test]
fn the_committed_reference_matches_the_generator() {
    let generated = aura_lang::agentdocs::render();
    // Line endings are the checkout's business, not a difference in content.
    let normalise = |s: &str| s.replace("\r\n", "\n");
    assert_eq!(
        normalise(&committed()),
        normalise(&generated),
        "llms.txt is out of date — regenerate it with `aura docs --agent -o llms.txt`"
    );
}

/// Every ```aura block in the reference must lex and parse. Blocks are checked
/// individually because several are fragments (a `type` declaration, a `cond`
/// arm) that only make sense in context; parsing is the strongest check that
/// applies to all of them uniformly.
#[test]
fn every_example_in_the_reference_parses() {
    let text = committed();
    let blocks = aura_blocks(&text);
    assert!(
        blocks.len() >= 8,
        "found only {} aura blocks — the extractor is probably broken, and finding \
         none would let this pass for the wrong reason",
        blocks.len()
    );

    for (i, block) in blocks.iter().enumerate() {
        let tokens = Lexer::new(block, 0)
            .tokenize()
            .unwrap_or_else(|d| panic!("block {i} does not lex: {}\n{block}", d.message));
        Parser::new(tokens)
            .parse_module()
            .unwrap_or_else(|ds| panic!("block {i} does not parse: {}\n{block}", ds[0].message));
    }
}

/// And is canonical `aura fmt` output, so nobody learns a layout the formatter
/// would immediately undo.
#[test]
fn every_example_in_the_reference_is_canonically_formatted() {
    let text = committed();
    let mut offenders = Vec::new();
    for (i, block) in aura_blocks(&text).iter().enumerate() {
        if let Ok(formatted) = format_source(block) {
            if formatted != *block {
                offenders.push(format!("block {i}:\n{block}--- expected:\n{formatted}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "examples are not `aura fmt` output:\n{}",
        offenders.join("\n")
    );
}

/// The fenced ```aura blocks, with the comment-only fences left out — those
/// illustrate shell usage rather than manifests.
fn aura_blocks(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = md.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "```aura" {
            continue;
        }
        let mut block = String::new();
        for body in lines.by_ref() {
            if body.trim() == "```" {
                break;
            }
            block.push_str(body);
            block.push('\n');
        }
        if !block.trim().is_empty() {
            out.push(block);
        }
    }
    out
}
