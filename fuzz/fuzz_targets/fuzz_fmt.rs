#![no_main]
//! Fuzz the formatter: it must never panic on arbitrary input, and formatting
//! an already-formatted document must be a fixpoint (idempotence).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(once) = aura_core::fmt::format_source(src) {
        let twice = aura_core::fmt::format_source(&once).expect("re-formatting must succeed");
        assert_eq!(once, twice, "formatter is not idempotent");
    }
});
