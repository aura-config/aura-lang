//! `Float.round(digits)`: money needs it.
//!
//! Found writing the checkout example: `(640.0 * 2.0 + 9.5) * 0.92` is
//! `1186.3400000000001`, and Aura had no way to show it as `1186.34`. Rounding
//! is defined on the stored value, so the result is the same on every machine:
//! the decimal nearest to the binary number the Float actually holds.
//!
//! The values live in `fixtures/float_cases.json`, which the wasm smoke test
//! reads too: the same expressions must give the same digits natively and in
//! the browser build, and there is one table for both to agree with.

use std::collections::HashMap;

use aura_lang::facade::{eval_source, EvalOptions};

fn eval(src: &str) -> Result<serde_json::Value, String> {
    let files = HashMap::from([("t.aura".to_string(), src.to_string())]);
    eval_source(files, "t.aura", &EvalOptions::default())
        .map(|e| e.json)
        .map_err(|r| r[0].code.to_string())
}

/// Every case in the shared table, digit for digit: rounding, the exact binary
/// ties, `1.005` held just below, no `-0`, and the plain arithmetic that feeds
/// it. Equal shortest round-trip text is equal bits.
#[test]
fn every_shared_case_gives_the_same_digits() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/float_cases.json"
    );
    let table: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
    let cases = table["cases"].as_array().expect("cases");
    assert!(cases.len() > 10, "the table lost its cases");
    for c in cases {
        let expr = c["expr"].as_str().expect("expr");
        let got = eval(&format!("v: ({expr}).to_str()\n")).expect(expr)["v"].clone();
        assert_eq!(got, c["is"], "{expr}");
    }
}

/// The result is a Float, so `round(0)` does not turn money into an Int: Int
/// and Float stay apart (D6).
#[test]
fn the_result_is_a_float() {
    let v = eval("v: 7.6.round(0)\n").expect("evaluates")["v"].clone();
    assert!(v.is_f64(), "{v}");
}

#[test]
fn the_number_of_places_is_an_int_from_0_to_15() {
    assert_eq!(eval("v: 1.5.round(1.0)\n").unwrap_err(), "E0306");
    assert_eq!(eval("v: 1.5.round()\n").unwrap_err(), "E0306");
    assert_eq!(eval("v: 1.5.round(-1)\n").unwrap_err(), "E0306");
    assert_eq!(eval("v: 1.5.round(16)\n").unwrap_err(), "E0306");
}
