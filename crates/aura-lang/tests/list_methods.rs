//! D39: `group_by`, `sort_by`, `flat_map` and `zip`.
//!
//! Written before the implementation, one test per clause of the decision.
//! Every help line a test meets is a claim about code, so the remedy it names
//! is run here too.

use std::collections::HashMap;

use aura_lang::facade::{eval_source, EvalOptions, Report};
use aura_lang::Budget;
use serde_json::{json, Value};

fn eval_with(src: &str, opts: &EvalOptions) -> Result<Value, Vec<Report>> {
    let files = HashMap::from([("main.aura".to_string(), src.to_string())]);
    eval_source(files, "main.aura", opts).map(|e| e.json)
}

fn ok(src: &str) -> Value {
    eval_with(src, &EvalOptions::default())
        .unwrap_or_else(|r| panic!("expected success, got {r:#?}\n--- source ---\n{src}"))
}

fn fail(src: &str) -> Report {
    match eval_with(src, &EvalOptions::default()) {
        Ok(v) => panic!("expected an error, got {v}\n--- source ---\n{src}"),
        Err(mut r) => r.remove(0),
    }
}

fn help(r: &Report) -> &str {
    r.help.as_deref().unwrap_or("")
}

/// The code between the first pair of backticks in a help line.
fn suggested(r: &Report) -> String {
    let h = help(r);
    let start = h
        .find('`')
        .unwrap_or_else(|| panic!("no code in help: {h}"))
        + 1;
    let len = h[start..].find('`').expect("closing backtick");
    h[start..start + len].to_string()
}

/// Services as data, built with D38 one-line objects - there is no `{}`.
fn services() -> &'static str {
    "type Svc
  name: String
  zone: String
  port: Int
end

services = [
  new Svc name: \"api\",    zone: \"eu\", port: 8080 end
  new Svc name: \"web\",    zone: \"us\", port: 80 end
  new Svc name: \"worker\", zone: \"eu\", port: 9000 end
  new Svc name: \"cron\",   zone: \"ap\", port: 9100 end
]
"
}

// ---- Clause 1: group_by gives an Object, keys in first-appearance order ----

#[test]
fn group_by_gives_an_object_of_lists_in_first_appearance_order() {
    let v = ok(&format!(
        "{}by_zone: services.group_by((s) -> s.zone end).entries().map (e, i) -> e.key end\n\
         names:   services.group_by((s) -> s.zone end).eu.map (s, i) -> s.name end\n",
        services()
    ));
    // `eu` first, though `ap` sorts before it: the order is the data's.
    assert_eq!(v["by_zone"], json!(["eu", "us", "ap"]));
    assert_eq!(v["names"], json!(["api", "worker"]));
}

#[test]
fn group_by_output_is_the_grouped_object() {
    let v =
        ok("xs = [1, 2, 3, 4, 5]\nby: xs.group_by (x, i) -> x % 2 == 0 ? \"even\" : \"odd\" end\n");
    assert_eq!(v["by"], json!({"odd": [1, 3, 5], "even": [2, 4]}));
}

// ---- Clause 2: a key that is not a String is refused, and the help works ----

#[test]
fn group_by_refuses_a_key_that_is_not_a_string() {
    let src = format!("{}by: services.group_by (s) -> s.port end\n", services());
    let r = fail(&src);
    assert_eq!(r.code, "E0306");
    // The remedy is a lambda body; it must run in place of the faulty one.
    // The help writes the lambda's key as `key`.
    let fixed = src.replace(
        "s.port end",
        &format!("{} end", suggested(&r).replace("key", "s.port")),
    );
    assert_ne!(fixed, src, "help offered no replacement: {}", help(&r));
    let v = ok(&fixed);
    assert_eq!(v["by"]["8080"][0]["name"], json!("api"));
}

// ---- Clause 3: sort_by is ascending and stable, keys as for `sort` ----

#[test]
fn sort_by_is_ascending_and_keeps_ties_in_order() {
    let v = ok(&format!(
        "{}by_zone: services.sort_by((s) -> s.zone end).map (s, i) -> s.name end\n",
        services()
    ));
    // `api` and `worker` share `eu`; they keep the order they had.
    assert_eq!(v["by_zone"], json!(["cron", "api", "worker", "web"]));
}

#[test]
fn sort_by_refuses_keys_sort_would_refuse() {
    let r = fail("by: [1, 2].sort_by (x, i) -> x == 1 ? \"a\" : 2 end\n");
    assert_eq!(r.code, "E0306");
}

#[test]
fn sort_by_a_null_key_names_a_default_that_works() {
    let src = "type T\n  name: String\n  zone: String? = null\nend\n\
               ts = [new T name: \"a\", zone: \"eu\" end, new T name: \"b\" end]\n\
               by: ts.sort_by((t) -> t.zone end).map (t, i) -> t.name end\n";
    let r = fail(src);
    assert_eq!(r.code, "E0306");
    let fixed = src.replace(
        "(t) -> t.zone end",
        &format!("(t) -> {} end", suggested(&r).replace("key", "t.zone")),
    );
    assert_ne!(fixed, src, "help offered no replacement: {}", help(&r));
    assert_eq!(ok(&fixed)["by"], json!(["b", "a"]));
}

// ---- Clause 4: flat_map flattens one level, and wants a List back ----

#[test]
fn flat_map_flattens_exactly_one_level() {
    let v = ok("by: [1, 2].flat_map (x, i) -> [x, [x * 10]] end\n");
    assert_eq!(v["by"], json!([1, [10], 2, [20]]));
}

#[test]
fn flat_map_refuses_a_lambda_that_returns_no_list() {
    let src = "by: [1, 2].flat_map (x, i) -> x * 10 end\n";
    let r = fail(src);
    assert_eq!(r.code, "E0306");
    let fixed = src.replace(
        "x * 10 end",
        &format!("{} end", suggested(&r).replace("value", "x * 10")),
    );
    assert_ne!(fixed, src, "help offered no replacement: {}", help(&r));
    assert_eq!(ok(&fixed)["by"], json!([10, 20]));
}

// ---- Clauses 5 and 6: zip pairs by name, and refuses unequal lengths ----

#[test]
fn zip_pairs_elements_as_left_and_right() {
    let v = ok("by: [\"a\", \"b\"].zip([1, 2])\n");
    assert_eq!(
        v["by"],
        json!([{"left": "a", "right": 1}, {"left": "b", "right": 2}])
    );
}

#[test]
fn zip_refuses_lists_of_different_lengths_and_names_the_cut() {
    let src = "xs = [\"a\", \"b\", \"c\"]\nys = [1, 2]\nby: xs.zip(ys)\n";
    let r = fail(src);
    assert_eq!(r.code, "E0327");
    let fixed = src.replace("xs.zip(ys)", &suggested(&r));
    assert_ne!(fixed, src, "help offered no replacement: {}", help(&r));
    assert_eq!(ok(&fixed)["by"].as_array().map(Vec::len), Some(2));
}

// ---- Clause 7: the lambdas take the index, as `map` does ----

#[test]
fn every_lambda_sees_the_index() {
    let v = ok("xs = [\"c\", \"a\", \"b\"]\n\
                g: xs.group_by (x, i) -> i < 1 ? \"head\" : \"tail\" end\n\
                s: xs.sort_by (x, i) -> 0 - i end\n\
                f: xs.flat_map (x, i) -> [i] end\n");
    assert_eq!(v["g"], json!({"head": ["c"], "tail": ["a", "b"]}));
    assert_eq!(v["s"], json!(["b", "a", "c"]));
    assert_eq!(v["f"], json!([0, 1, 2]));
}

// ---- Clause 8: each lambda call is a step of the budget (D34) ----

#[test]
fn the_new_methods_spend_the_budget() {
    let opts = EvalOptions {
        budget: Budget::DEFAULT.steps(5_000),
        ..EvalOptions::default()
    };
    // The list alone fits the budget, so what exhausts it is the lambda calls.
    eval_with("by: range(100000).len()\n", &opts).expect("the list alone fits");
    for call in [
        "group_by (x, i) -> \"k\" end",
        "sort_by (x, i) -> x end",
        "flat_map (x, i) -> [x] end",
    ] {
        let src = format!("by: range(100000).{call}.len()\n");
        let r = eval_with(&src, &opts).expect_err(call);
        assert_eq!(r[0].code, "E0325", "{call}: {r:#?}");
    }
}
