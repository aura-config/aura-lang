//! Критерий приёмки Фазы 5 (SPEC §8): в эталонном манифесте детектируется
//! `unused_config_version` (и неиспользуемый импорт rust), без ложных ошибок.

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

    // Ни одной ошибки — только предупреждения о мёртвом коде
    assert!(diags.iter().all(|d| d.severity == Severity::Warning), "unexpected errors: {diags:#?}");
    assert!(
        diags.iter().any(|d| d.code == "W0501" && d.message.contains("unused_config_version")),
        "must detect unused_config_version: {diags:#?}"
    );
    assert!(
        diags.iter().any(|d| d.code == "W0502" && d.message.contains("rust")),
        "must detect unused import rust: {diags:#?}"
    );

    // Обычный режим проходит, --strict блокирует
    assert!(!has_blocking(&diags, false));
    assert!(has_blocking(&diags, true));
}
