//! Lexer property tests (SPEC §8, Phase 1): on ANY input the lexer
//! never panics, and on a successful parse the span invariants hold.

use aura_core::lexer::{Lexer, TokenKind};
use proptest::prelude::*;

proptest! {
    /// Fuzzing: any Unicode input -> Ok(tokens) or Err(diagnostic), never a panic.
    #[test]
    fn never_panics_on_arbitrary_input(src in "\\PC*") {
        let _ = Lexer::new(&src, 0).tokenize();
    }

    /// Span invariants: within the source's bounds, never inverted, monotonic,
    /// and every span lies on UTF-8 character boundaries.
    #[test]
    fn spans_are_sane(src in "\\PC{0,200}") {
        if let Ok(tokens) = Lexer::new(&src, 0).tokenize() {
            let mut prev_start = 0u32;
            for t in &tokens {
                prop_assert!(t.span.start <= t.span.end);
                prop_assert!((t.span.end as usize) <= src.len());
                prop_assert!(t.span.start >= prev_start, "spans must be ordered");
                prop_assert!(src.is_char_boundary(t.span.start as usize));
                prop_assert!(src.is_char_boundary(t.span.end as usize));
                prev_start = t.span.start;
            }
            prop_assert_eq!(&tokens.last().unwrap().kind, &TokenKind::Eof);
        }
    }

    /// Lexeme idempotence: slicing the source by an identifier's span yields the identifier itself.
    #[test]
    fn ident_lexemes_match_spans(name in "[a-zA-Z_][a-zA-Z0-9_]{0,20}") {
        let src = format!("{name} = 1");
        let tokens = Lexer::new(&src, 0).tokenize().unwrap();
        // the name may have matched a keyword - then there is nothing to check
        if let TokenKind::Ident(s) = &tokens[0].kind {
            prop_assert_eq!(*s, &src[tokens[0].span.start as usize..tokens[0].span.end as usize]);
        }
    }
}
