//! D38: an object on one line, and an object bound with `=`.
//!
//! Written before the implementation, one test per clause of the decision.
//! A block object and its one-line form are the same object: everything that
//! holds for one - the schema's fields, its enums, its invariants - holds for
//! the other, so most tests here compare the two rather than restate a rule.

use std::collections::HashMap;

use aura_lang::facade::{eval_source, EvalOptions, Report};
use aura_lang::fmt::format_source;
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

const ENDPOINT: &str = "\
enum Method
  \"GET\"
  \"POST\"
end

type Endpoint
  path:   String
  method: Method = \"GET\"

  assert path.starts_with(\"/\"), \"a path starts with a slash\"
end
";

fn with_schema(body: &str) -> String {
    format!("{ENDPOINT}\n{body}")
}

// ---- Clause 1: `new` on one line ----

#[test]
fn a_one_line_new_is_the_same_object_as_a_block() {
    let one_line = ok(&with_schema(
        "routes: [
  new Endpoint path: \"/health\" end
  new Endpoint path: \"/orders\", method: \"POST\" end
]
",
    ));
    let block = ok(&with_schema(
        "routes: [
  new Endpoint
    path: \"/health\"
  end
  new Endpoint
    path:   \"/orders\"
    method: \"POST\"
  end
]
",
    ));
    assert_eq!(one_line, block);
    assert_eq!(
        one_line["routes"][1],
        json!({"path": "/orders", "method": "POST"})
    );
}

#[test]
fn a_one_line_new_goes_wherever_an_expression_does() {
    let v = ok(&with_schema(
        "def at(e)
  where: e.path
end

primary = new Endpoint path: \"/\" end
first:  primary.path
called: at(new Endpoint path: \"/x\" end)
mapped: [\"/a\", \"/b\"].map((p) -> new Endpoint path: p, method: \"POST\" end end)
inline: [new Endpoint path: \"/one\" end, new Endpoint path: \"/two\" end]
",
    ));
    assert_eq!(v["first"], json!("/"));
    assert_eq!(v["called"], json!({"where": "/x"}));
    assert_eq!(v["mapped"][1], json!({"path": "/b", "method": "POST"}));
    assert_eq!(v["inline"][1]["path"], json!("/two"));
}

#[test]
fn a_one_line_new_may_hold_another() {
    let v = ok(&with_schema(
        "type Route
  name:   String
  target: Endpoint
end

r: new Route name: \"home\", target: new Endpoint path: \"/\" end end
",
    ));
    assert_eq!(v["r"]["target"]["path"], json!("/"));
}

// ---- Clause 4: the schema's rules hold on one line exactly as in a block ----

#[test]
fn the_schema_rules_are_the_same_on_one_line() {
    for (one_line, block) in [
        // An invariant (D28).
        (
            "e: new Endpoint path: \"x\" end",
            "e: new Endpoint\n  path: \"x\"\nend",
        ),
        // An enum member (D18).
        (
            "e: new Endpoint path: \"/\", method: \"PUT\" end",
            "e: new Endpoint\n  path:   \"/\"\n  method: \"PUT\"\nend",
        ),
        // A required field left out.
        (
            "e: new Endpoint method: \"GET\" end",
            "e: new Endpoint\n  method: \"GET\"\nend",
        ),
    ] {
        let (a, b) = (fail(&with_schema(one_line)), fail(&with_schema(block)));
        assert_eq!(
            (a.code, &a.message),
            (b.code, &b.message),
            "{one_line} and its block form disagree"
        );
    }
}

// ---- Clauses 2 and 6: commas between fields, none before `end` ----

#[test]
fn fields_on_one_line_need_a_comma_and_the_help_says_so() {
    let r = fail(&with_schema(
        "e: new Endpoint path: \"/\" method: \"POST\" end",
    ));
    assert_eq!(r.code, "E0205");
    assert!(help(&r).contains("comma"), "{r:#?}");
    // The remedy the help names works.
    ok(&with_schema(
        "e: new Endpoint path: \"/\", method: \"POST\" end",
    ));
}

#[test]
fn a_comma_before_end_is_refused_and_the_help_says_so() {
    let r = fail(&with_schema("e: new Endpoint path: \"/\", end"));
    assert_eq!(r.code, "E0205");
    assert!(help(&r).contains("comma"), "{r:#?}");
    ok(&with_schema("e: new Endpoint path: \"/\" end"));
}

// ---- Clause 8: an object bound with `=` ----

#[test]
fn an_object_block_bound_with_equals_is_not_output() {
    let v = ok("defaults =
  port:    8080
  timeout: \"30s\"
end

server:
  port:    defaults.port
  timeout: defaults.timeout
end
");
    // D10: `:` is output, `=` is not - for an object as for anything else.
    assert_eq!(v, json!({"server": {"port": 8080, "timeout": "30s"}}));
}

#[test]
fn an_equals_block_holds_what_a_colon_block_holds() {
    // The two differ in whether they are output, and in nothing else: a
    // binding inside is refused by both, with the same diagnostic.
    let (a, b) = (fail("d =\n  x = 1\nend\n"), fail("d:\n  x = 1\nend\n"));
    assert_eq!((a.code, &a.message), (b.code, &b.message));
}

#[test]
fn an_unclosed_equals_block_points_at_the_line_that_opened_it() {
    // Before D38 `x =` with nothing after it was "expected expression" on the
    // spot. Now it opens a block, and a forgotten value would surface as a
    // missing `end` at the bottom of the file - so the error names the opener.
    let r = fail("x = 1\nd =\n  port: 1\n\ny: 2\n");
    assert_eq!(r.code, "E0203");
    assert_eq!(r.line, 2, "{r:#?}");
}

#[test]
fn an_expression_continued_after_equals_is_unchanged() {
    // `=` at the end of a line still continues the expression onto the next
    // one when that line is not a property.
    assert_eq!(ok("x =\n  1 + 2\ny: x\n"), json!({"y": 3}));
}

// ---- Clause 9: an anonymous object on one line ----

#[test]
fn an_anonymous_one_line_object_after_equals_and_after_a_key() {
    let one_line = ok("limits = cpu: \"500m\", memory: \"1Gi\" end
resources:
  limits:   limits
  requests: cpu: \"250m\", memory: \"512Mi\" end
  nested:   a: b: 1 end end
end
");
    let block = ok("limits =
  cpu:    \"500m\"
  memory: \"1Gi\"
end
resources:
  limits: limits
  requests:
    cpu:    \"250m\"
    memory: \"512Mi\"
  end
  nested:
    a:
      b: 1
    end
  end
end
");
    assert_eq!(one_line, block);
}

#[test]
fn an_unclosed_one_line_object_names_its_end() {
    let r = fail("l = cpu: 1, memory: 2\ny: 3\n");
    assert_eq!(r.code, "E0203");
    assert_eq!(r.line, 1, "{r:#?}");
    assert!(help(&r).contains("end"), "{r:#?}");
}

#[test]
fn an_anonymous_one_line_object_is_refused_in_a_list() {
    // In a list the comma would separate both fields and elements.
    let r = fail("ports: [name: \"http\", port: 80 end]\n");
    assert_eq!(r.code, "E0205");
    assert!(help(&r).contains("new"), "{r:#?}");
    // The remedy the help names works.
    ok("type Port
  name: String
  port: Int
end

ports: [new Port name: \"http\", port: 80 end]
");
}

// ---- Clause 5: `aura fmt` keeps the author's choice ----

#[test]
fn fmt_keeps_one_line_and_block_forms_and_is_idempotent() {
    let src = with_schema(
        "defaults =
port:8080
end
limits = cpu: \"500m\", memory: \"1Gi\" end
resources:
requests: cpu: \"250m\", memory: \"512Mi\" end
limits: cpu: \"500m\", memory: \"1Gi\" end
end
routes: [
new Endpoint   path: \"/\" end
]
after: 1
",
    );
    let once = format_source(&src).expect("formats");
    let twice = format_source(&once).expect("formats");
    assert_eq!(once, twice, "not idempotent");
    let tail = &once[once.find("defaults").expect("kept")..];
    assert_eq!(
        tail,
        "defaults =
  port: 8080
end
limits = cpu: \"500m\", memory: \"1Gi\" end
resources:
  requests: cpu: \"250m\", memory: \"512Mi\" end
  limits:   cpu: \"500m\", memory: \"1Gi\" end
end
routes: [
  new Endpoint path: \"/\" end
]
after: 1
"
    );
}
