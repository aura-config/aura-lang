//! Dogfooding: every Aura snippet in the repository's Markdown must be exactly
//! what `aura fmt` produces. CI already checks `examples/**.aura`, but the code
//! blocks inside README/SPEC/the book were drifting from the canonical style —
//! which is confusing when a reader copies one and formats it.
//!
//! Snippets that do not parse (deliberate fragments) are skipped; snippets that
//! parse but fail analysis (deliberate error examples) are still checked, since
//! formatting only needs a token stream.

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

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            markdown_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// `(code, line-number)` for every ```ruby / ```aura block in `src`.
fn snippets(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut line_no = 1;
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let opener = line.trim() == "```ruby" || line.trim() == "```aura";
        line_no += 1;
        if !opener {
            continue;
        }
        let start = line_no;
        let mut code = String::new();
        for body in lines.by_ref() {
            line_no += 1;
            if body.trim() == "```" {
                break;
            }
            code.push_str(body);
            code.push('\n');
        }
        out.push((code, start));
    }
    out
}

#[test]
fn markdown_snippets_are_canonically_formatted() {
    let root = repo_root();
    let mut files = vec![
        root.join("README.md"),
        root.join("README.ru.md"),
        root.join("SPEC.md"),
        root.join("SPEC.ru.md"),
    ];
    markdown_files(&root.join("docs"), &mut files);

    let mut offenders = Vec::new();
    let mut checked = 0;
    for file in files.iter().filter(|f| f.exists()) {
        let src = std::fs::read_to_string(file).expect("read markdown");
        for (code, line) in snippets(&src) {
            // Skip fragments that do not tokenize/parse on their own.
            let Ok(tokens) = Lexer::new(&code, 0).tokenize() else {
                continue;
            };
            if Parser::new(tokens).parse_module().is_err() {
                continue;
            }
            checked += 1;
            let formatted = format_source(&code).expect("lexes, so it formats");
            if formatted != code {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                offenders.push(format!("{name}:{line}"));
            }
        }
    }
    assert!(checked > 20, "expected to find snippets, found {checked}");
    assert!(
        offenders.is_empty(),
        "these Markdown snippets are not `aura fmt` output: {offenders:?}"
    );
}
