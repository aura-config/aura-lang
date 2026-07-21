#![no_main]
//! Fuzz the full pipeline lex + parse + eval. Capabilities are fully denied and
//! no resolver is wired, so evaluation is deterministic and performs no I/O:
//! imports/effects simply return diagnostics. Must never panic, hang, or OOM.

use libfuzzer_sys::fuzz_target;

use aura_lang::eval::{Interpreter, Options};
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(tokens) = Lexer::new(src, 0).tokenize() else {
        return;
    };
    let Ok(module) = Parser::new(tokens).parse_module() else {
        return;
    };
    // Default Interpreter: DenyFs + EnvCap::Deny, no modules provided.
    let mut interp = Interpreter::new(Options::default());
    let _ = interp.eval_module(&module);
});
