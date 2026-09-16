//! Both books' method reference must list every method the language has.
//!
//! Neither did. The Russian reference was missing twenty-nine of the fifty-eight
//! names in the stdlib manifest — half the library — including `trim`, `split`,
//! `sort`, `sum`, `min`, `max`, `entries`, `to_object` and every fold. Nothing
//! failed when a method was added and only one translation was updated, so the
//! gap grew one change at a time and was invisible to anyone reading either book
//! on its own.
//!
//! A reader cannot tell an undocumented method from a nonexistent one. Someone
//! working from the Russian book would conclude Aura has no way to trim a string
//! and write `replace(" ", "")` instead — which is not the same thing and is
//! wrong at the edges. That is worse than an error message, because nothing
//! reports it.
//!
//! The manifest is the single source: the LSP's completion and `aura docs
//! --agent` are already generated from it, and `manifest_matches_registry`
//! checks it against the real registry. This adds the books to the same chain,
//! so a new method cannot be half-documented.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Method names as the manifest declares them: a bare `name:` opening a block,
/// indented two spaces, inside a type's section.
fn names_in_manifest() -> BTreeSet<String> {
    let text = std::fs::read_to_string(repo_root().join("crates/aura-lang/stdlib.aura"))
        .expect("the stdlib manifest");
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("  ")?;
            let name = rest.strip_suffix(':')?;
            let ok = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            ok.then(|| name.to_string())
        })
        .collect()
}

/// Every name a book spells as a call — `` `trim(` `` — or as a method taking a
/// lambda, which is written `` `map (x, i) -> … end` `` with a space and so does
/// not end in a parenthesis.
fn names_in_book(rel: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(repo_root().join(rel)).expect("a book page");
    let mut out = BTreeSet::new();
    for (i, _) in text.match_indices('`') {
        let rest = &text[i + 1..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let after = rest[name.len()..].chars().next();
        if matches!(after, Some('(') | Some(' ')) {
            out.insert(name);
        }
    }
    out
}

const BOOKS: &[(&str, &str)] = &[
    ("English", "docs/book/src/reference/methods.md"),
    ("Russian", "docs/book-ru/src/reference/methods.md"),
];

#[test]
fn every_book_documents_every_method() {
    for (language, page) in BOOKS {
        let documented = names_in_book(page);
        let missing: Vec<_> = names_in_manifest()
            .into_iter()
            .filter(|n| !documented.contains(n))
            .collect();
        assert!(
            missing.is_empty(),
            "the {language} method reference does not mention: {missing:?}\n\
             Add them to {page}, or remove them from stdlib.aura if they are gone."
        );
    }
}

/// The two books must describe the same language. A method documented in one and
/// not the other is the shape the drift actually took.
#[test]
fn the_books_document_the_same_methods() {
    let english = names_in_book(BOOKS[0].1);
    let russian = names_in_book(BOOKS[1].1);
    let manifest = names_in_manifest();

    // Compared through the manifest, so prose mentioning an unrelated word in
    // backticks in one translation does not count as a difference.
    let only_english: Vec<_> = manifest
        .iter()
        .filter(|n| english.contains(*n) && !russian.contains(*n))
        .collect();
    let only_russian: Vec<_> = manifest
        .iter()
        .filter(|n| russian.contains(*n) && !english.contains(*n))
        .collect();

    assert!(
        only_english.is_empty() && only_russian.is_empty(),
        "the translations disagree — only in English: {only_english:?}, only in Russian: {only_russian:?}"
    );
}
