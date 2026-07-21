#![no_main]
//! Fuzz the lexer: arbitrary bytes must tokenize to Ok or a Diagnostic, never
//! panic or hang. (The lexer already has a proptest for this; this is the
//! coverage-guided version.)

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        let _ = aura_lang::lexer::Lexer::new(src, 0).tokenize();
    }
});
