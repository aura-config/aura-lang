//! Property-тесты лексера (SPEC §8, Фаза 1): на ПРОИЗВОЛЬНОМ входе лексер
//! не паникует, а на успешном выходе инварианты span'ов держатся.

use aura_core::lexer::{Lexer, TokenKind};
use proptest::prelude::*;

proptest! {
    /// Фаззинг: любой Unicode-вход → Ok(токены) либо Err(диагностика), но не паника.
    #[test]
    fn never_panics_on_arbitrary_input(src in "\\PC*") {
        let _ = Lexer::new(&src, 0).tokenize();
    }

    /// Инварианты span'ов: в границах исходника, не инвертированы, монотонны,
    /// и каждый span лежит на границах UTF-8 символов.
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

    /// Идемпотентность лексем: срез исходника по span идентификатора — это он сам.
    #[test]
    fn ident_lexemes_match_spans(name in "[a-zA-Z_][a-zA-Z0-9_]{0,20}") {
        let src = format!("{name} = 1");
        let tokens = Lexer::new(&src, 0).tokenize().unwrap();
        // имя могло совпасть с ключевым словом — тогда проверять нечего
        if let TokenKind::Ident(s) = &tokens[0].kind {
            prop_assert_eq!(*s, &src[tokens[0].span.start as usize..tokens[0].span.end as usize]);
        }
    }
}
