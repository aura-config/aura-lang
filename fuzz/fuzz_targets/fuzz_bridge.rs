#![no_main]
//! Fuzz the three format readers.
//!
//! These are the only places Aura accepts structured input it did not lex
//! itself: the bytes come from a vendor's file, not from a manifest, so they
//! are the part of the language most exposed to whatever someone else wrote.
//! The readers are also hand-written rather than routed through serde — the
//! type mapping is deliberate, and YAML's merge key is expanded here — so the
//! usual argument that the dependency has already been fuzzed does not cover
//! the code that turns a document into a value.
//!
//! Two properties, both about the bridge rather than about the parsers:
//!
//! 1. Malformed input is a diagnostic, never a panic. A configuration language
//!    that aborts on a bad input file cannot report where the problem is.
//!
//! 2. Whatever is read must be emittable and re-readable without changing. If
//!    a value survives a round trip through JSON, the bridge is lossless for
//!    that value; if it does not, some document out there converts to something
//!    that is not what it said.

use libfuzzer_sys::fuzz_target;

use aura_lang::eval::methods::{read_json, read_toml, read_yaml};
use aura_lang::serialize::to_json;
use aura_lang::span::Span;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    // A document large enough to recurse deeply is a stack question, not a
    // correctness one, and libFuzzer's default corpus stays well under this.
    if src.len() > 64 * 1024 {
        return;
    }
    let sp = Span::new(0, 0, 0);

    for value in [
        read_yaml(src, sp),
        read_toml(src, sp),
        read_json(src, sp),
    ]
    .into_iter()
    .flatten()
    {
        // Emitting is allowed to refuse — a function value or a null under TOML
        // has no representation — but it must refuse rather than panic.
        let Ok(json) = to_json(&value) else {
            continue;
        };
        let text = json.to_string();

        // Read back what was just written: the bridge's own output is the one
        // input it is obliged to handle.
        let again = read_json(&text, sp).expect("Aura must be able to read its own JSON");
        let again = to_json(&again).expect("a value that emitted once emits twice");
        assert_eq!(
            json,
            again,
            "a round trip through JSON changed the value: {text}"
        );
    }
});
