//! Dogfooding: the editor grammars must cover every reserved keyword and every
//! global builtin. They are hand-maintained regexes in three different formats,
//! so they drift silently — `cond`/`else` and `range` were already missing from
//! nano and vim before this test existed.
//!
//! Comment lines are stripped before checking, so a word merely *mentioned* in a
//! comment cannot satisfy the test (nano's comments do mention `enum`).

use std::path::{Path, PathBuf};

use aura_lang::lexer::token::KEYWORDS;

/// Global builtins (the analyzer's list; this test's failure keeps them in sync).
const BUILTINS: &[&str] = &["env", "read_file", "fail", "range"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// File contents with comment lines removed. The marker differs per format, and
/// JSON has none — one shared heuristic would eat real JSON lines.
fn rules_only(rel: &str, comment: Option<char>) -> String {
    let p = repo_root().join(rel);
    let src =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    src.lines()
        .filter(|l| match comment {
            Some(c) => !l.trim_start().starts_with(c),
            None => true,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn editor_grammars_cover_every_keyword_and_builtin() {
    // `text` is contextual (D16), not reserved, so it is not required here.
    let files = [
        ("editors/vscode/syntaxes/aura.tmLanguage.json", None),
        ("editors/vim/syntax/aura.vim", Some('"')),
        ("editors/nano/aura.nanorc", Some('#')),
    ];
    let mut missing = Vec::new();
    for (rel, comment) in files {
        let rules = rules_only(rel, comment);
        for word in KEYWORDS.iter().chain(BUILTINS.iter()) {
            if !rules.contains(*word) {
                missing.push(format!("{rel}: `{word}`"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "editor grammars are out of date: {missing:?}"
    );
}

#[test]
fn vscode_indent_pattern_covers_block_openers() {
    // The auto-indent rule must know the keywords that open a block.
    let cfg = repo_root().join("editors/vscode/language-configuration.json");
    let src = std::fs::read_to_string(cfg).expect("language-configuration.json");
    let rule = src
        .lines()
        .find(|l| l.contains("increaseIndentPattern"))
        .expect("increaseIndentPattern");
    for kw in ["domain", "def", "type", "enum", "cond", "new"] {
        assert!(rule.contains(kw), "indent rule is missing opener `{kw}`");
    }
}
