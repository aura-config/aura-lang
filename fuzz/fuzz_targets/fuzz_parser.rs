#![no_main]
//! Fuzz lex + parse: arbitrary UTF-8 must produce Ok/Err, never panic, hang, or
//! overflow the stack (deep nesting is capped by MAX_PARSE_DEPTH — E0208).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        if let Ok(tokens) = aura_core::lexer::Lexer::new(src, 0).tokenize() {
            let _ = aura_core::parser::Parser::new(tokens).parse_module();
        }
    }
});
