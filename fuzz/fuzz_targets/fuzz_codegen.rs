#![no_main]
//! Fuzz `aura types`: emitting host-language types from arbitrary input must
//! never panic — a malformed manifest is a Diagnostic, not a crash.

use libfuzzer_sys::fuzz_target;

use aura_lang::codegen::{generate, Lang};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    for lang in [Lang::Rust, Lang::TypeScript, Lang::Go] {
        let _ = generate(src, lang);
    }
});
