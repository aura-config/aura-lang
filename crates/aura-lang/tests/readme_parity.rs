//! Each README and its Russian translation say the same thing.
//!
//! Before 0.4.0 the English README was brought up to date and the Russian one
//! was not: it still showed the old way to embed, had no word of embedding in
//! its status list, and missed the `--allow-read` line D37 changed. Only the
//! version test noticed, and only because the dependency line happened to sit
//! in the same file.
//!
//! What a translation legitimately changes - prose, comments, the text of
//! string literals, diagram labels - is not compared. What it must not change
//! is compared exactly: the sections, every flag, error code and decision
//! named, the tables' shape, and the code itself.

use std::collections::BTreeSet;
use std::path::Path;

/// Every README with a translation, English first.
const PAIRS: &[(&str, &str)] = &[
    ("README.md", "README.ru.md"),
    ("editors/README.md", "editors/README.ru.md"),
];

fn read(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(name),
    )
    .expect("readable")
    .replace("\r\n", "\n")
}

/// Fenced blocks as `(language, body)`, in order.
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(lang) = line.strip_prefix("```") else {
            continue;
        };
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if l.starts_with("```") {
                break;
            }
            body.push(l);
        }
        out.push((lang.trim().to_string(), body.join("\n")));
    }
    out
}

/// A code block with what a translator may change taken out: comments and
/// the contents of string literals. Identifiers, keywords, numbers and
/// structure stay - those are the code.
fn code_only(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let mut kept = String::new();
        let mut chars = line.chars().peekable();
        let mut in_str = false;
        while let Some(c) = chars.next() {
            if in_str {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '"' => {
                        in_str = false;
                        kept.push('"');
                    }
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    kept.push('"');
                }
                // A comment, in the languages the READMEs show - but not an
                // interpolation, which begins with `#{`.
                '#' if chars.peek() != Some(&'{') => break,
                '/' if chars.peek() == Some(&'/') => break,
                _ => kept.push(c),
            }
        }
        let kept = kept.trim_end();
        if !kept.trim().is_empty() {
            out.push(kept.to_string());
        }
    }
    out.join("\n")
}

/// Every match of a simple token pattern, as a set.
fn tokens(text: &str, keep: impl Fn(&str) -> bool) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| keep(t))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_sections_match() {
    let levels = |t: &str| -> Vec<usize> {
        t.lines()
            .filter(|l| l.starts_with('#') && l.trim_start_matches('#').starts_with(' '))
            .map(|l| l.chars().take_while(|c| *c == '#').count())
            .collect()
    };
    // Headings inside code blocks are comments, not sections.
    let outside = |t: &str| -> String {
        let mut inside = false;
        t.lines()
            .filter(|l| {
                if l.starts_with("```") {
                    inside = !inside;
                    return false;
                }
                !inside
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for (en, ru) in PAIRS {
        assert_eq!(
            levels(&outside(&read(en))),
            levels(&outside(&read(ru))),
            "{en} and {ru} have different sections"
        );
    }
}

#[test]
fn the_code_matches() {
    for (en_name, ru_name) in PAIRS {
        let (en, ru) = (blocks(&read(en_name)), blocks(&read(ru_name)));
        assert_eq!(
            en.len(),
            ru.len(),
            "{en_name}: a different number of code blocks"
        );
        for (i, ((le, be), (lr, br))) in en.iter().zip(&ru).enumerate() {
            assert_eq!(le, lr, "{en_name} block {i}: a different language");
            // Diagrams and plain text are prose that happens to be fenced.
            if matches!(le.as_str(), "mermaid" | "text") {
                continue;
            }
            assert_eq!(
                code_only(be),
                code_only(br),
                "{en_name} block {i} ({le}) differs from {ru_name} beyond comments and string text"
            );
        }
    }
}

#[test]
fn the_same_flags_codes_and_decisions_are_named() {
    let named = |t: &str| {
        tokens(t, |w| {
            (w.starts_with("--") && w.len() > 3)
                || (w.len() == 5
                    && (w.starts_with('E') || w.starts_with('W'))
                    && w[1..].chars().all(|c| c.is_ascii_digit()))
                || (w.len() >= 2
                    && w.len() <= 3
                    && w.starts_with('D')
                    && w[1..].chars().all(|c| c.is_ascii_digit()))
        })
    };
    for (en, ru) in PAIRS {
        let (a, b) = (named(&read(en)), named(&read(ru)));
        let only_en: Vec<_> = a.difference(&b).collect();
        let only_ru: Vec<_> = b.difference(&a).collect();
        assert!(
            only_en.is_empty() && only_ru.is_empty(),
            "named in only one of {en} and {ru} - English: {only_en:?}, Russian: {only_ru:?}"
        );
    }
}

#[test]
fn the_tables_have_the_same_shape() {
    let shape = |t: &str| -> Vec<usize> {
        let mut rows = Vec::new();
        let mut current = 0;
        for line in t.lines() {
            if line.starts_with('|') {
                current += 1;
            } else if current > 0 {
                rows.push(current);
                current = 0;
            }
        }
        rows
    };
    for (en, ru) in PAIRS {
        assert_eq!(
            shape(&read(en)),
            shape(&read(ru)),
            "a table has a different number of rows in {en} and {ru}"
        );
    }
}
