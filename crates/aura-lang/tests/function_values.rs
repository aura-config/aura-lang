//! D40: a function's value is its last expression, and no value is thrown away.
//!
//! Before, a bare expression anywhere - the end of a `def`, the middle of a
//! `domain`, the top of a module - was evaluated and dropped without a word:
//! `def pair(n)` ending in `[n, n]` returned `{}`, and `aura check --strict`
//! said `ok`. Written before the implementation, one test per clause.

use std::collections::HashMap;
use std::process::Command;

use aura_lang::facade::{eval_source, EvalOptions, Report};
use serde_json::{json, Value};

fn eval(src: &str) -> Result<Value, Vec<Report>> {
    let files = HashMap::from([("main.aura".to_string(), src.to_string())]);
    eval_source(files, "main.aura", &EvalOptions::default()).map(|e| e.json)
}

fn ok(src: &str) -> Value {
    eval(src).unwrap_or_else(|r| panic!("expected success, got {r:#?}\n--- source ---\n{src}"))
}

fn fail(src: &str) -> Report {
    match eval(src) {
        Ok(v) => panic!("expected an error, got {v}\n--- source ---\n{src}"),
        Err(mut r) => r.remove(0),
    }
}

fn help(r: &Report) -> &str {
    r.help.as_deref().unwrap_or("")
}

// ---- Clause 1: the last expression is the function's value ----

#[test]
fn a_function_returns_its_last_expression() {
    let v = ok("def ports_of(xs)
  all = xs.map (x, i) -> x * 10 end
  assert all.len() > 0, \"no ports\"
  all.reverse()
end

ports: ports_of([1, 2, 3])
twice: ports_of([4]).len()
");
    assert_eq!(v, json!({"ports": [30, 20, 10], "twice": 1}));
}

#[test]
fn a_function_of_properties_still_returns_its_object() {
    let v = ok("def labels(team)
  t = team.upper()
  team: t
  tier: \"backend\"
end

l: labels(\"core\")
");
    assert_eq!(v, json!({"l": {"team": "CORE", "tier": "backend"}}));
}

// ---- Clause 2: properties and a final value together are refused ----

#[test]
fn properties_and_a_final_value_are_refused_with_both_remedies() {
    let src = "def f(n)
  label: \"x\"
  [n, n]
end

v: f(1)
";
    let r = fail(src);
    assert_eq!(r.code, "E0215");
    assert_eq!(r.line, 3, "{r:#?}");
    assert!(help(&r).contains("value:"), "{r:#?}");
    // The first remedy: the value becomes a property.
    let v = ok(&src.replace("  [n, n]", "  value: [n, n]"));
    assert_eq!(v["v"], json!({"label": "x", "value": [1, 1]}));
}

// ---- Clause 3: a value thrown away is an error, wherever it is ----

#[test]
fn a_discarded_value_is_refused_at_the_top_of_a_module() {
    let src = "port = 8080\nport + 1\n";
    let r = fail(src);
    assert_eq!(r.code, "E0214");
    assert_eq!(r.line, 2, "{r:#?}");
    // Each remedy the help names works.
    for (remedy, line) in [
        ("`key: ", "next: port + 1"),
        ("`name = ", "next = port + 1"),
        ("`assert`", "assert port + 1 > 0"),
    ] {
        assert!(
            help(&r).contains(remedy),
            "help does not name {remedy}: {r:#?}"
        );
        ok(&src.replace("port + 1", line));
    }
}

#[test]
fn a_discarded_value_is_refused_in_a_domain() {
    let r = fail("domain \"eu\"\n  port: 8080\n  port + 1\nend\n");
    assert_eq!(r.code, "E0214");
    assert_eq!(r.line, 3, "{r:#?}");
}

#[test]
fn a_discarded_value_is_refused_before_the_end_of_a_function() {
    let r = fail("def f(n)\n  n * 2\n  n * 3\nend\nv: f(1)\n");
    assert_eq!(r.code, "E0214");
    assert_eq!(r.line, 2, "{r:#?}");
}

// ---- Clause 4: a multi-line lambda follows the same rule ----

#[test]
fn a_multi_line_lambda_returns_its_last_expression() {
    let v = ok("g = (n) ->
  y = n * 2
  [y, y]
end

v: g(1)
");
    assert_eq!(v["v"], json!([2, 2]));
}

#[test]
fn a_multi_line_lambda_refuses_what_a_function_refuses() {
    let r = fail("g = (n) ->\n  y: n\n  [n]\nend\nv: g(1)\n");
    assert_eq!(r.code, "E0215");
}

// ---- Clause 5: a body of bindings alone is still the empty object ----

#[test]
fn a_body_of_bindings_alone_is_an_empty_object() {
    let v = ok("def f(n)\n  y = n\n  assert y > 0\nend\nv: f(1)\n");
    assert_eq!(v["v"], json!({}));
}

// ---- Clause 7: `aura check` finds both without running anything ----

#[test]
fn check_reports_both_without_evaluating() {
    let dir = std::env::temp_dir().join(format!("aura-d40-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    for (name, src, code) in [
        ("discarded.aura", "x: 1\nx + 1\n", "E0214"),
        ("mixed.aura", "def f()\n  a: 1\n  2\nend\n", "E0215"),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, src).expect("write");
        let out = Command::new(env!("CARGO_BIN_EXE_aura"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("run aura check");
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{name}: check passed");
        assert!(text.contains(code), "{name}: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
