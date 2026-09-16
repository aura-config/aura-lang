//! Every design decision the project cites must have an entry in the log.
//!
//! The decision log in `SPEC.md` is how a reader finds out *why*: code comments,
//! diagnostics, the changelog and the books all say "(D31)" and expect the table
//! to answer. D30, D31 and D32 were adopted, implemented, cited in all of those
//! places — and never written into the table, in either translation. A reader
//! following `E0324`'s catalogue entry to D31 found nothing, which is worse than
//! no citation, because it reads as a reference to something that was removed.
//!
//! The rows were added to the prose of §4.4 instead, which is where the feature
//! is described, and nothing noticed that the log — the one place a decision is
//! supposed to be findable by number — did not have them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// `D` followed by digits, not preceded by a letter or digit — so `ID12` and a
/// hex digest do not count, while `(D31)` and `D12's` do.
fn cited(text: &str) -> BTreeSet<u32> {
    let bytes = text.as_bytes();
    let mut out = BTreeSet::new();
    for (i, _) in text.match_indices('D') {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let digits: String = text[i + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() || digits.len() > 3 {
            continue;
        }
        // A following letter makes it a word, not a citation (`D3D`, `D2h`).
        let after = text[i + 1 + digits.len()..].chars().next();
        if after.is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        if let Ok(n) = digits.parse() {
            out.insert(n);
        }
    }
    out
}

/// The numbers that have a row in a decision table: a line beginning `| D`.
fn logged(spec: &str) -> BTreeSet<u32> {
    let text = std::fs::read_to_string(repo_root().join(spec)).expect("a SPEC");
    text.lines()
        .filter_map(|l| l.strip_prefix("| D"))
        .filter_map(|rest| rest.split_whitespace().next()?.parse().ok())
        .collect()
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// Everywhere a decision is cited by number.
fn citing_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files = vec![
        root.join("CHANGELOG.md"),
        root.join("README.md"),
        root.join("AGENTS.md"),
        root.join("SPEC.md"),
        root.join("SPEC.ru.md"),
        root.join("crates/aura-lang/diagnostics.md"),
        root.join("crates/aura-lang/agent-preamble.md"),
    ];
    walk(&root.join("crates/aura-lang/src"), "rs", &mut files);
    walk(&root.join("crates/aura-lsp/src"), "rs", &mut files);
    walk(&root.join("docs/book/src"), "md", &mut files);
    walk(&root.join("docs/book-ru/src"), "md", &mut files);
    files.into_iter().filter(|f| f.exists()).collect()
}

#[test]
fn every_cited_decision_is_in_the_log() {
    for spec in ["SPEC.md", "SPEC.ru.md"] {
        let log = logged(spec);
        assert!(log.len() > 20, "found only {} rows in {spec}", log.len());

        let mut missing = Vec::new();
        for file in citing_files() {
            let text = std::fs::read_to_string(&file).expect("readable");
            for n in cited(&text) {
                if !log.contains(&n) {
                    let rel = file.strip_prefix(repo_root()).unwrap_or(&file);
                    missing.push(format!("D{n} (cited in {})", rel.display()));
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "{spec} has no decision-log row for:\n  {}",
            missing.join("\n  ")
        );
    }
}

/// Both translations must log the same decisions. A row added to one and not the
/// other is the drift this repository has already had twice, in the method
/// reference and in the tutorial.
#[test]
fn the_translations_log_the_same_decisions() {
    assert_eq!(logged("SPEC.md"), logged("SPEC.ru.md"));
}
