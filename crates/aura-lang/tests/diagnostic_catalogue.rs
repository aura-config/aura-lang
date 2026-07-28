//! The documented list of diagnostic codes must be the list the compiler can
//! actually emit.
//!
//! It was not. The catalogue in the book described `E0311` — a path escaping the
//! directories granted by `--allow-read` — and no such code existed: the check
//! fired, but reported `E0310`, which tells the user to pass the very flag they
//! had already passed. Meanwhile seven codes the compiler does emit appeared in
//! no catalogue at all, including `E0208`, which the README cites by name as the
//! reason deeply-nested input is an error rather than a stack overflow.
//!
//! Both directions matter. An undocumented code leaves someone grepping for a
//! string with no explanation. A documented code that cannot occur is worse: it
//! is a promise about behaviour, and here it was hiding a diagnostic that named
//! the wrong remedy.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `"E0xxx"` / `"W0xxx"` literal under `src/`, which is how diagnostics are
/// constructed throughout — there is no central registry to read instead.
///
/// Test modules are skipped: a test asserting `E0107` is expected must not be
/// what makes `E0107` count as emittable, or a code could survive the deletion of
/// the only line that raises it.
fn codes_in_source() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect(&repo_root().join("crates/aura-lang/src"), &mut out);
    out
}

fn collect(dir: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("readable file");
            let code_part = text
                .split_once("\n#[cfg(test)]")
                .map_or(text.as_str(), |(before, _)| before);
            out.extend(scan(code_part));
        }
    }
}

/// `"E0123"` or `"W0123"` — quoted, four digits, nothing else.
fn scan(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for w in bytes.windows(7) {
        if w[0] == b'"'
            && (w[1] == b'E' || w[1] == b'W')
            && w[2..6].iter().all(u8::is_ascii_digit)
            && w[6] == b'"'
        {
            out.push(String::from_utf8_lossy(&w[1..6]).to_string());
        }
    }
    out
}

/// The leading `| E0123 |` cell of a table row.
fn codes_in_book(rel: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(repo_root().join(rel)).expect("the error-code reference");
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("| ")?;
            let (code, _) = rest.split_once(" |")?;
            let ok = code.len() == 5
                && (code.starts_with('E') || code.starts_with('W'))
                && code[1..].chars().all(|c| c.is_ascii_digit());
            ok.then(|| code.to_string())
        })
        .collect()
}

#[test]
fn the_english_catalogue_matches_the_compiler_exactly() {
    let source = codes_in_source();
    let book = codes_in_book("docs/book/src/reference/error-codes.md");

    assert!(
        source.len() > 50,
        "only {} codes found in the source — the scanner is probably broken, and \
         an empty comparison would pass for the wrong reason",
        source.len()
    );

    let undocumented: Vec<_> = source.difference(&book).collect();
    let unreachable: Vec<_> = book.difference(&source).collect();

    assert!(
        undocumented.is_empty(),
        "these codes can be emitted but appear in no catalogue: {undocumented:?}"
    );
    assert!(
        unreachable.is_empty(),
        "these codes are documented but no longer exist in the source: {unreachable:?}. \
         Either the code was renamed and the book was not, or the behaviour it \
         describes now reports something else — which is how E0311 came to promise \
         a diagnostic nobody could ever see."
    );
}

/// The translation must list the same codes. Wording is free; the set is not.
#[test]
fn the_russian_catalogue_lists_the_same_codes() {
    let english = codes_in_book("docs/book/src/reference/error-codes.md");
    let russian = codes_in_book("docs/book-ru/src/reference/error-codes.md");
    let missing: Vec<_> = english.difference(&russian).collect();
    let extra: Vec<_> = russian.difference(&english).collect();
    assert!(
        missing.is_empty(),
        "missing from the Russian book: {missing:?}"
    );
    assert!(extra.is_empty(), "only in the Russian book: {extra:?}");
}
