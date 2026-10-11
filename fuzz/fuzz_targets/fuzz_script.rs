#![no_main]
//! Fuzz the embedding API (D33) the way a host uses it: load a script from
//! untrusted text and call into it.
//!
//! The property is the one D33 exists for. Whatever the text, loading and
//! calling end — with a result or with an error — and never with a panic or a
//! hang. The budget is set tight so a runaway is caught in milliseconds; a
//! call that outlives libFuzzer's timeout is a budget that failed to hold.
//!
//! The harness calls `f` with zero, one and two arguments. The fuzzer finds
//! `pub def f(` quickly from the seed corpus, and every arity that does not
//! match is itself a path worth exercising: it must be E0004, not a crash.

use libfuzzer_sys::fuzz_target;
use serde::de::IgnoredAny;

use aura_lang::{Budget, Script};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(script) = Script::builder()
        .source("fuzz.aura", src)
        .budget(Budget::per_call().steps(100_000).memory_mb(16))
        .load()
    else {
        return;
    };

    let _ = script.call::<(), IgnoredAny>("f", ());
    let _ = script.call::<(i64,), IgnoredAny>("f", (7,));
    let _ = script.call::<(&str, f64), IgnoredAny>("f", ("x", 0.5));
    let _ = script.get::<IgnoredAny>("f");
});
