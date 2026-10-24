//! D42: no input can overflow the stack - it ends in an error instead.
//!
//! The nightly fuzzer found two manifests that overflowed it while evaluating,
//! and measuring the cause found more: a one-line `x: 1 + 1 + ... + 1` of 2.4 KB
//! crashed a release build, and `aura check` with it, because the parser built
//! the chain in a loop, past its own depth limit; and a recursive function 200
//! calls deep crashed a release build on a 1 MiB stack, inside the old limit of
//! 256 calls. A crash here is the whole process - the CLI, the language server,
//! or a service that embeds a script.

use std::collections::HashMap;

use aura_lang::eval::{Interpreter, Options};
use aura_lang::facade::{eval_source, EvalOptions, Report};
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;

fn eval(src: &str) -> Result<serde_json::Value, Vec<Report>> {
    let files = HashMap::from([("main.aura".to_string(), src.to_string())]);
    eval_source(files, "main.aura", &EvalOptions::default()).map(|e| e.json)
}

fn code(src: &str) -> &'static str {
    eval(src).expect_err("expected an error")[0].code
}

/// The fuzzer's path: lex, parse, evaluate - no analysis in front of it.
fn evaluate_unchecked(src: &str) -> Result<(), &'static str> {
    let toks = Lexer::new(src, 0).tokenize().map_err(|d| d.code)?;
    let module = Parser::new(toks).parse_module().map_err(|d| d[0].code)?;
    Interpreter::new(Options::default())
        .eval_module(&module)
        .map(|_| ())
        .map_err(|d| d.code)
}

fn recursion(depth: u32) -> String {
    format!("def f(n)\n  n == 0 ? 0 : f(n - 1) + 1\nend\nx: f({depth})\n")
}

// ---- The parser: a chain counts as the nesting it builds ----

#[test]
fn a_long_operator_chain_is_refused_not_overflowed() {
    let chain = format!("x: {}\n", vec!["1"; 20_000].join(" + "));
    let r = eval(&chain).expect_err("too long");
    assert_eq!(r[0].code, "E0208");
    // The help names the remedy, and the remedy works: the same sum, split.
    assert!(
        r[0].help.as_deref().unwrap_or("").contains("part ="),
        "{r:#?}"
    );
    let half = vec!["1"; 150].join(" + ");
    let split = format!("part = {half}\nx: part + {half}\n");
    assert_eq!(eval(&split).expect("split")["x"], 300);
    let calls = format!("x: \" a \"{}\n", ".trim()".repeat(20_000));
    assert_eq!(code(&calls), "E0208");
}

#[test]
fn an_ordinary_chain_is_unaffected() {
    let chain = format!("x: {}\n", vec!["1"; 200].join(" + "));
    assert_eq!(eval(&chain).expect("200 terms")["x"], 200);
}

// ---- Evaluation: one count for calls and expressions together ----

#[test]
fn unbounded_recursion_is_an_error_not_a_crash() {
    assert_eq!(code(&recursion(100_000)), "E0399");
}

#[test]
fn ordinary_recursion_is_unaffected() {
    assert_eq!(eval(&recursion(80)).expect("80 calls")["x"], 80);
}

#[test]
fn the_fuzzers_inputs_end_in_errors() {
    for input in [
        include_str!("fixtures/nesting/fuzz_pipeline_1.aura"),
        include_str!("fixtures/nesting/fuzz_pipeline_2.aura"),
    ] {
        assert!(evaluate_unchecked(input).is_err());
    }
}

#[test]
fn the_limit_is_the_same_through_every_entry() {
    // A count, not a measurement: the CLI's path and the fuzzer's agree on
    // where a manifest stops.
    for depth in [80, 250] {
        let src = recursion(depth);
        let facade = eval(&src).err().map(|r| r[0].code);
        assert_eq!(facade, evaluate_unchecked(&src).err(), "depth {depth}");
    }
}

// ---- The stack the limit was sized for ----

/// A release build fits the whole limit in the 512 KiB of an iOS secondary
/// thread. Debug frames are about five times larger - a debug build evaluates
/// on a thread of its own for that reason - so this is a release-only claim,
/// run with `cargo test --release --test eval_nesting`.
#[test]
#[cfg_attr(debug_assertions, ignore = "a release-build claim")]
fn the_deepest_allowed_evaluation_fits_in_512_kib() {
    let r = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            // Just inside the limit, and past it.
            (eval(&recursion(84)).is_ok(), code(&recursion(250)))
        })
        .expect("spawn")
        .join()
        .expect("no overflow");
    assert_eq!(r, (true, "E0399"));
}
