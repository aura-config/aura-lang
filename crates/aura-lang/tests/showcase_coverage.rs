//! The showcase examples, taken together, must exercise the whole language.
//!
//! They claim to. `showcase.aura` opens with "exercises every core construct in
//! one manifest", and by the time this test was written that sentence was false:
//! it used 24 of the 55 methods the stdlib manifest declares, and none of the
//! constructs added in the same week. Nobody had done anything wrong — a
//! sentence in a comment cannot notice that the language moved underneath it.
//!
//! So the claim is checked instead of written. The sources of truth already
//! exist and are the same ones the language server and `aura docs --agent` use:
//! the stdlib manifest for methods and builtins, and the lexer's keyword table.
//! A method nobody demonstrates now fails the build.
//!
//! `PENDING` is the one concession, and it is self-cleaning: an entry that turns
//! out to be covered fails just as loudly as one that is missing. It exists
//! because the showcases land one per change; it must reach empty.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use aura_lang::lexer::{Lexer, TokenKind};

/// Every showcase, by directory. Each has its own subject on purpose — one
/// person's several projects, all configured in Aura — so the language is shown
/// working outside the domain it was first written for.
const SHOWCASES: &[&str] = &["showcase", "trading", "pipeline", "device"];

/// The one name no showcase demonstrates, and the reason is permanent rather
/// than a promise: `fail` aborts the evaluation, so a manifest that called it
/// could not also produce the output a showcase is pinned against. It is
/// covered by `examples/validators` instead, which is checked by error code.
///
/// The list is still self-cleaning in both directions: an entry that turns out
/// to be covered fails as loudly as one that is missing.
const PENDING: &[&str] = &[
    // deliberate: `fail` aborts, so a showcase that used it could not also
    // produce output. It is demonstrated in examples/validators instead.
    "fail",
];

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples directory")
}

/// Every `.aura` source belonging to a showcase, concatenated.
fn showcase_sources() -> String {
    let mut out = String::new();
    for dir in SHOWCASES {
        let path = examples().join(dir);
        for entry in std::fs::read_dir(&path).expect("showcase directory") {
            let p = entry.expect("entry").path();
            if p.extension().is_some_and(|e| e == "aura") {
                out.push_str(&std::fs::read_to_string(&p).expect("readable"));
                out.push('\n');
            }
        }
    }
    out
}

/// Names called as a method (`x.name(`) or as a builtin (`name(`), read from the
/// token stream rather than by pattern-matching the text — a trailing lambda is
/// written `xs.map (x, i) -> … end`, and the space would defeat a regex.
fn called_names(src: &str) -> BTreeSet<String> {
    let tokens = Lexer::new(src, 0).tokenize().expect("showcases must lex");
    let mut out = BTreeSet::new();
    for w in tokens.windows(2) {
        if let (TokenKind::Ident(name), TokenKind::LParen) = (&w[0].kind, &w[1].kind) {
            out.insert((*name).to_string());
        }
    }
    out
}

/// Method and builtin names the stdlib manifest declares: a receiver block whose
/// entries are the names. The manifest is the same file the language server and
/// the agent reference read, so this cannot drift from what the compiler offers.
fn declared_names() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in aura_lang::STDLIB_MANIFEST.lines() {
        // `  name:` at exactly one level of indentation, with nothing after it.
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let Some(name) = rest.strip_suffix(':') else {
            continue;
        };
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            out.insert(name.to_string());
        }
    }
    out
}

#[test]
fn the_showcases_demonstrate_every_stdlib_name() {
    let declared = declared_names();
    assert!(
        declared.len() > 40,
        "only {} names parsed out of the stdlib manifest — the reader is broken, \
         and an empty comparison would pass for the wrong reason",
        declared.len()
    );

    let used = called_names(&showcase_sources());
    let pending: BTreeSet<String> = PENDING.iter().map(|s| (*s).to_string()).collect();

    let uncovered: Vec<&String> = declared
        .iter()
        .filter(|n| !used.contains(*n) && !pending.contains(*n))
        .collect();
    assert!(
        uncovered.is_empty(),
        "these stdlib names are demonstrated by no showcase: {uncovered:?}. \
         Add one to a showcase, or list it in PENDING with the showcase that will."
    );

    // The other direction, which is what stops PENDING from rotting.
    let stale: Vec<&String> = pending.iter().filter(|n| used.contains(*n)).collect();
    assert!(
        stale.is_empty(),
        "these are listed as PENDING but a showcase already uses them: {stale:?}. \
         Remove them from the list."
    );

    // And the list must not name something the language does not have.
    let invented: Vec<&String> = pending.iter().filter(|n| !declared.contains(*n)).collect();
    assert!(
        invented.is_empty(),
        "PENDING names that are not in the stdlib manifest: {invented:?}"
    );
}

#[test]
fn the_showcases_use_every_keyword() {
    // `KEYWORDS` is the lexer's own table, so a new keyword appears here the
    // moment it exists. `text` is contextual and not in it, hence the chain.
    let src = showcase_sources();
    let tokens = Lexer::new(&src, 0).tokenize().expect("showcases must lex");

    // A keyword is present if the lexer produced its token: asking the lexer
    // rather than the text avoids counting the word inside a comment or string.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in &tokens {
        let name = match &t.kind {
            TokenKind::Import => "import",
            TokenKind::As => "as",
            TokenKind::Type => "type",
            TokenKind::Enum => "enum",
            TokenKind::Def => "def",
            TokenKind::End => "end",
            TokenKind::Domain => "domain",
            TokenKind::New => "new",
            TokenKind::Assert => "assert",
            TokenKind::Shadow => "shadow",
            TokenKind::Pub => "pub",
            TokenKind::Cond => "cond",
            TokenKind::Else => "else",
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Null => "null",
            _ => continue,
        };
        seen.insert(name.to_string());
    }

    let missing: Vec<&&str> = aura_lang::lexer::token::KEYWORDS
        .iter()
        .filter(|k| !seen.contains(**k))
        .collect();
    assert!(
        missing.is_empty(),
        "no showcase uses these keywords: {missing:?}"
    );
}
