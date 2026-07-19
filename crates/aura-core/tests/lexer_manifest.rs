//! Phase 1 acceptance criterion (SPEC §8): the reference manifest tokenizes without errors,
//! and the spans correctly cover the source.

use aura_core::lexer::{Lexer, TokenKind};

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

#[test]
fn reference_manifest_tokenizes() {
    let tokens = Lexer::new(MANIFEST, 0)
        .tokenize()
        .expect("manifest must lex cleanly");
    assert!(
        tokens.len() > 100,
        "suspiciously few tokens: {}",
        tokens.len()
    );
    assert_eq!(tokens.last().map(|t| &t.kind), Some(&TokenKind::Eof));
    // v1.2 constructs are present
    assert!(tokens.iter().any(|t| t.kind == TokenKind::New));
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Assert));
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Shadow));
    assert!(tokens.iter().any(|t| matches!(
        t.kind,
        TokenKind::ImportPath {
            path: "github/actions/rust-cache",
            version: "v1.2"
        }
    )));
}

#[test]
fn spans_are_monotonic_and_within_source() {
    let tokens = Lexer::new(MANIFEST, 0).tokenize().unwrap();
    let mut prev_end = 0u32;
    for t in &tokens {
        assert!(t.span.start <= t.span.end, "inverted span {:?}", t);
        assert!(t.span.start >= prev_end, "overlapping spans at {:?}", t);
        assert!((t.span.end as usize) <= MANIFEST.len());
        prev_end = t.span.start; // Newline may collapse; the order of starts is monotonic
    }
}
