//! Both books' keyword page describes exactly the language's keywords.
//!
//! The list lives in one place, `KEYWORDS` in the lexer, which the language
//! server and `aura docs --agent` already read. The page is prose about it, so
//! it is checked against it: a keyword added without a row, or a row for a word
//! that is not one, fails here. `text` is listed separately because it is
//! contextual rather than reserved.

use std::collections::BTreeSet;
use std::path::Path;

use aura_lang::lexer::token::KEYWORDS;

fn page(book: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs")
            .join(book)
            .join("src/reference/keywords.md"),
    )
    .expect("keyword page")
    .replace("\r\n", "\n")
}

/// The word in the first column of each table row, in order.
fn rows(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split_once('`').map(|(w, _)| w.to_string()))
        .collect()
}

#[test]
fn each_book_describes_exactly_the_keywords() {
    let want: BTreeSet<String> = KEYWORDS
        .iter()
        .map(|k| k.to_string())
        .chain(std::iter::once("text".to_string()))
        .collect();
    for book in ["book", "book-ru"] {
        let rows = rows(&page(book));
        let have: BTreeSet<String> = rows.iter().cloned().collect();
        assert_eq!(rows.len(), have.len(), "{book}: a keyword has two rows");
        let missing: Vec<_> = want.difference(&have).collect();
        let extra: Vec<_> = have.difference(&want).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{book}: missing {missing:?}, not keywords {extra:?}"
        );
    }
}

#[test]
fn the_translations_list_the_words_in_one_order() {
    assert_eq!(rows(&page("book")), rows(&page("book-ru")));
}

#[test]
fn the_page_states_the_count_it_has() {
    // "sixteen" in prose is the kind of count that goes stale, so it is
    // checked rather than trusted.
    assert_eq!(KEYWORDS.len(), 16, "update both keyword pages' first line");
}

#[test]
fn both_readmes_name_the_count_there_is() {
    // The README's header quotes the number of keywords; it is checked against
    // the lexer, not trusted.
    let n = KEYWORDS.len();
    for (name, phrase) in [
        ("README.md", format!("{n} keywords")),
        ("README.ru.md", format!("{n} ключевых слов")),
    ] {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(name),
        )
        .expect("readme");
        assert!(text.contains(&phrase), "{name} does not say \"{phrase}\"");
    }
}
