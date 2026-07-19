//! Phase 5 acceptance criterion (SPEC §8): the reference manifest is expected to detect
//! `unused_config_version` (and the unused import rust), with no false errors.

use aura_core::analysis::{analyze, has_blocking};
use aura_core::error::Severity;
use aura_core::lexer::Lexer;
use aura_core::parser::Parser;

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

#[test]
fn manifest_dead_code() {
    let toks = Lexer::new(MANIFEST, 0).tokenize().expect("lex ok");
    let module = Parser::new(toks).parse_module().expect("parse ok");
    let diags = analyze(&module, true);

    // Not a single error - only dead-code warnings
    assert!(
        diags.iter().all(|d| d.severity == Severity::Warning),
        "unexpected errors: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == "W0501" && d.message.contains("unused_config_version")),
        "must detect unused_config_version: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == "W0502" && d.message.contains("rust")),
        "must detect unused import rust: {diags:#?}"
    );

    // The normal mode passes, --strict blocks
    assert!(!has_blocking(&diags, false));
    assert!(has_blocking(&diags, true));
}
