//! D43: no value is deeper than 256 lists and objects.
//!
//! D42 bounded how deep evaluation nests; a value can still grow deep without
//! any of that, one level per turn of a loop - `reduce` wrapping its
//! accumulator. Every walk over a value recurses once per level, so measured
//! on a release build: writing one out crashed at about 5 000 levels, comparing
//! at 20 000, and merely building one and never using it crashed at 100 000,
//! when it was freed - all inside the default memory budget.

use std::collections::HashMap;

use aura_lang::facade::{eval_source, EvalOptions, Report};
use aura_lang::Script;
use serde_json::{json, Value};

fn eval(src: &str) -> Result<Value, Vec<Report>> {
    let files = HashMap::from([("main.aura".to_string(), src.to_string())]);
    let opts = EvalOptions {
        budget: aura_lang::Budget::DEFAULT.steps(100_000_000),
        ..EvalOptions::default()
    };
    eval_source(files, "main.aura", &opts).map(|e| e.json)
}

/// A value `depth` objects deep, built by a loop with no nesting of its own.
fn wrapped(depth: u32, then: &str) -> String {
    format!(
        "wrap = (n) ->\n  child: n\nend\n\
         deep = range({depth}).reduce(\"leaf\") (acc, i) -> wrap(acc) end\n{then}\n"
    )
}

#[test]
fn a_value_within_the_limit_is_untouched() {
    let v = eval(&wrapped(200, "x: deep")).expect("200 levels");
    let mut node = &v["x"];
    let mut levels = 0;
    while let Some(child) = node.get("child") {
        node = child;
        levels += 1;
    }
    assert_eq!((levels, node), (200, &json!("leaf")));
}

#[test]
fn a_value_past_the_limit_is_refused_where_it_is_built() {
    for then in ["x: deep", "x: 1", "x: deep == deep"] {
        let r = eval(&wrapped(300, then)).expect_err(then);
        assert_eq!(r[0].code, "E0328", "{then}: {r:#?}");
    }
}

#[test]
fn a_value_far_past_the_limit_is_an_error_not_a_crash() {
    // Built and never used: before, freeing it overflowed the stack.
    let r = eval(&wrapped(100_000, "x: 1")).expect_err("too deep");
    assert_eq!(r[0].code, "E0328");
}

#[test]
fn the_remedy_the_help_names_works() {
    // "side by side": the same 300 levels as a list of items.
    let r = eval(&wrapped(300, "x: 1")).expect_err("too deep");
    assert!(r[0].help.as_deref().unwrap_or("").contains("side by side"));
    let v = eval("x: range(300).map (i, j) -> child: i end.len()\n").expect("flat");
    assert_eq!(v["x"], 300);
}

#[test]
fn a_refused_value_does_not_poison_the_next_evaluation() {
    assert!(eval(&wrapped(300, "x: 1")).is_err());
    assert_eq!(eval("x: 1\n").expect("unrelated")["x"], 1);
}

#[test]
fn an_interpreter_with_no_budget_is_not_poisoned_either() {
    // Driven directly, as the fuzzer does: no budget is installed, and the
    // stop must not leave the thread believing memory ran out.
    use aura_lang::eval::{Interpreter, Options};
    use aura_lang::lexer::Lexer;
    use aura_lang::parser::Parser;
    let run = |src: &str| {
        let toks = Lexer::new(src, 0).tokenize().expect("lexes");
        let module = Parser::new(toks).parse_module().expect("parses");
        Interpreter::new(Options::default())
            .eval_module(&module)
            .map(|_| ())
            .map_err(|d| d.code)
    };
    assert_eq!(run(&wrapped(300, "x: 1")), Err("E0328"));
    assert_eq!(run("x: [1, 2, 3]\n"), Ok(()));
}

#[test]
fn a_host_value_past_the_limit_is_refused_at_the_boundary() {
    let script = Script::builder()
        .source("t.aura", "pub def echo(v)\n  got: v\nend\n")
        .load()
        .expect("loads");
    let mut deep = json!("leaf");
    for _ in 0..300 {
        deep = json!({ "child": deep });
    }
    let r: Result<Value, _> = script.call("echo", (deep,));
    assert!(r.is_err(), "a 300-deep argument was accepted");
    let mut shallow = json!("leaf");
    for _ in 0..100 {
        shallow = json!({ "child": shallow });
    }
    let ok: Value = script.call("echo", (shallow,)).expect("100 levels");
    assert!(ok["got"]["child"].is_object());
}
