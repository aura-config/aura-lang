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

/// A snippet must not name something the language does not have.
///
/// Formatting was the only thing checked here, and formatting never looks up a
/// name. So the agent preamble — the file whose entire job is to teach the
/// language to a coding assistant — taught `cond` with this:
///
/// ```text
/// replicas: cond
///   env == "production" -> 6
/// ```
///
/// `env` is the built-in that reads an environment variable, and using it as a
/// binding is `E0504: use of undefined variable 'env'`. An assistant copying
/// the shape it was given produces code that does not compile, and the file
/// that taught it is the one document we promise is generated from the compiler.
///
/// The rule is deliberately narrow. Snippets legitimately fail for reasons that
/// are not defects: a capability that was not granted (`E0310`), a package that
/// is not installed (`E0404`), or a deliberate error example — the enum typo
/// `"backand"` is in three pages on purpose. What no snippet may do is refer to
/// a variable or a method that does not exist, because that is the document
/// describing a language other than the one that ships.
/// The global functions. A binding of one of these names shadows it, so a
/// snippet using one as a variable is teaching something that will not run.
const BUILTINS: &[&str] = &["env", "read_file", "fail", "range"];

#[test]
fn no_snippet_names_something_that_does_not_exist() {
    use aura_lang::facade::{eval_source, EvalOptions};

    let root = repo_root();
    let mut files = vec![
        root.join("README.md"),
        root.join("README.ru.md"),
        root.join("SPEC.md"),
        root.join("SPEC.ru.md"),
        root.join("crates/aura-lang/agent-preamble.md"),
    ];
    markdown_files(&root.join("docs"), &mut files);

    let mut offenders = Vec::new();
    for file in files.iter().filter(|f| f.exists()) {
        let src = std::fs::read_to_string(file).expect("read markdown");
        for (code, line) in snippets(&src) {
            let Ok(tokens) = Lexer::new(&code, 0).tokenize() else {
                continue;
            };
            if Parser::new(tokens).parse_module().is_err() {
                continue;
            }
            let files = std::collections::HashMap::from([("snippet.aura".to_string(), code)]);
            let Err(reports) = eval_source(files, "snippet.aura", &EvalOptions::default()) else {
                continue;
            };
            // A plain undefined name is ordinary in a fragment: README shows a
            // piece of a manifest whose `base` and `region` are established in
            // the prose around it. What is never ordinary is a *built-in* name
            // used as a variable — that is the document teaching a shape the
            // language rejects — or a method that does not exist.
            let relevant = |r: &aura_lang::facade::Report| match r.code {
                "E0309" => true,
                "E0504" => BUILTINS
                    .iter()
                    .any(|b| r.message.contains(&format!("'{b}'"))),
                _ => false,
            };
            for r in reports.iter().filter(|r| relevant(r)) {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                offenders.push(format!("{name}:{line} {} — {}", r.code, r.message));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these snippets name something the language does not have:\n  {}",
        offenders.join("\n  ")
    );
}
