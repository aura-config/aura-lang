//! D41: a field typed by a schema holds what `new` of that schema would accept.
//!
//! Before, `[Port]` accepted any object - one without `number`, or one whose
//! `number` broke Port's own invariant - and so did a field typed `Port`. A
//! value that arrived as data, from `parse_json` or a host, skipped every rule
//! the schema states. Written before the implementation, one test per clause.

use std::collections::HashMap;

use aura_lang::facade::{eval_source, EvalOptions, Report};
use serde_json::{json, Value};

fn eval_files(files: &[(&str, &str)], opts: &EvalOptions) -> Result<Value, Vec<Report>> {
    let files: HashMap<String, String> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    eval_source(files, "main.aura", opts).map(|e| e.json)
}

fn ok(src: &str) -> Value {
    eval_files(&[("main.aura", src)], &EvalOptions::default())
        .unwrap_or_else(|r| panic!("expected success, got {r:#?}\n--- source ---\n{src}"))
}

fn fail(src: &str) -> Report {
    match eval_files(&[("main.aura", src)], &EvalOptions::default()) {
        Ok(v) => panic!("expected an error, got {v}\n--- source ---\n{src}"),
        Err(mut r) => r.remove(0),
    }
}

const PORT: &str = "type Port
  number:   Int
  protocol: String = \"tcp\"

  assert number > 0, \"port must be positive\"
end

type Svc
  ports: [Port]
end
";

fn with(body: &str) -> String {
    format!("{PORT}\n{body}")
}

// ---- Clause 1: an element is checked as `new` would check it ----

#[test]
fn an_element_missing_a_field_is_refused_with_its_index() {
    let r = fail(&with(
        "s: new Svc ports: [new Port number: 80 end, \"{}\".parse_json()] end\n",
    ));
    assert_eq!(r.code, "E0511", "{r:#?}");
    assert!(r.message.contains("ports[1]"), "{r:#?}");
}

#[test]
fn an_element_breaking_an_invariant_is_refused() {
    let r = fail(&with(
        "raw = \"[{\\\"number\\\": -5}]\".parse_json()\ns: new Svc ports: raw end\n",
    ));
    assert_eq!(r.code, "E0515", "{r:#?}");
    assert!(r.message.contains("ports[0]"), "{r:#?}");
}

#[test]
fn an_element_of_the_wrong_type_names_the_field_and_the_index() {
    let r = fail(&with(
        "raw = \"[{\\\"number\\\": \\\"80\\\"}]\".parse_json()\ns: new Svc ports: raw end\n",
    ));
    assert_eq!(r.code, "E0512", "{r:#?}");
    assert!(r.message.contains("ports[0]"), "{r:#?}");
}

// ---- Clause 2: defaults are filled, as by `new` ----

#[test]
fn elements_from_data_get_the_schemas_defaults_and_order() {
    let v = ok(&with(
        "raw = \"[{\\\"number\\\": 80}]\".parse_json()\ns: new Svc ports: raw end\n",
    ));
    assert_eq!(v["s"]["ports"], json!([{"number": 80, "protocol": "tcp"}]));
    let keys: Vec<_> = v["s"]["ports"][0]
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, ["number", "protocol"]);
}

#[test]
fn an_instance_made_by_new_passes_unchanged() {
    let made = ok(&with(
        "s: new Svc ports: [new Port number: 80, protocol: \"udp\" end] end\n",
    ));
    assert_eq!(
        made["s"]["ports"],
        json!([{"number": 80, "protocol": "udp"}])
    );
}

// ---- Clause 3: extra fields, as by `new` ----

#[test]
fn an_extra_field_is_kept_and_refused_only_under_strict() {
    let src = with("raw = \"[{\\\"number\\\": 80, \\\"note\\\": \\\"x\\\"}]\".parse_json()\ns: new Svc ports: raw end\n");
    assert_eq!(ok(&src)["s"]["ports"][0]["note"], json!("x"));
    let strict = EvalOptions {
        strict: true,
        ..EvalOptions::default()
    };
    let r = eval_files(&[("main.aura", &src)], &strict).expect_err("strict refuses");
    assert_eq!(r[0].code, "E0513", "{r:#?}");
}

// ---- Clause 4: a single field, nested lists, nullable fields, enums ----

#[test]
fn a_field_typed_by_a_schema_is_checked_too() {
    let r = fail(&with(
        "type Route\n  target: Port\nend\nr: new Route target: \"{}\".parse_json() end\n",
    ));
    assert_eq!(r.code, "E0511", "{r:#?}");
    assert!(r.message.contains("target"), "{r:#?}");
}

#[test]
fn nested_lists_are_checked_at_every_depth() {
    let r = fail(&with(
        "type Grid\n  rows: [[Port]]\nend\ng: new Grid rows: [[new Port number: 1 end], [\"{}\".parse_json()]] end\n",
    ));
    assert_eq!(r.code, "E0511", "{r:#?}");
    assert!(r.message.contains("rows[1][0]"), "{r:#?}");
}

#[test]
fn a_nullable_schema_field_takes_null_and_checks_anything_else() {
    let src = "type Route\n  target: Port? = null\nend\n";
    assert_eq!(
        ok(&with(&format!("{src}r: new Route\nend\n")))["r"],
        json!({"target": null})
    );
    let r = fail(&with(&format!(
        "{src}r: new Route target: \"{{}}\".parse_json() end\n"
    )));
    assert_eq!(r.code, "E0511", "{r:#?}");
}

#[test]
fn an_enum_inside_an_element_is_checked() {
    let r = fail("enum Proto\n  \"tcp\"\n  \"udp\"\nend\ntype P\n  proto: Proto\nend\ntype S\n  ps: [P]\nend\n\
                  s: new S ps: \"[{\\\"proto\\\": \\\"icmp\\\"}]\".parse_json() end\n");
    assert_eq!(r.code, "E0514", "{r:#?}");
    assert!(r.message.contains("ps[0]"), "{r:#?}");
}

// ---- Clause 5: the path reaches the field that is wrong ----

#[test]
fn the_path_names_every_step_down() {
    let r = fail(&with(
        "type Route\n  target: Port\nend\ntype Table\n  routes: [Route]\nend\n\
         t: new Table routes: \"[{\\\"target\\\": {\\\"number\\\": 1}}, {\\\"target\\\": {}}]\".parse_json() end\n",
    ));
    assert_eq!(r.code, "E0511", "{r:#?}");
    assert!(r.message.contains("routes[1].target"), "{r:#?}");
}

// ---- Clause 6: recursive schemas, and a depth that cannot overflow ----

#[test]
fn a_recursive_schema_checks_the_whole_tree() {
    let tree = "type Node\n  name:     String\n  children: [Node] = []\nend\n";
    let v = ok(&format!(
        "{tree}t: new Node name: \"a\", children: \"[{{\\\"name\\\": \\\"b\\\"}}]\".parse_json() end\n"
    ));
    assert_eq!(v["t"]["children"][0], json!({"name": "b", "children": []}));
    let r = fail(&format!(
        "{tree}t: new Node name: \"a\", children: \"[{{\\\"name\\\": \\\"b\\\", \\\"children\\\": [{{}}]}}]\".parse_json() end\n"
    ));
    assert_eq!(r.code, "E0511", "{r:#?}");
    assert!(r.message.contains("children[0].children[0]"), "{r:#?}");
}

#[test]
fn a_tree_too_deep_is_an_error_not_a_crash() {
    // Built in the language, so no parser limit stands in front of the check.
    let src = "type Node\n  name:     String\n  children: [Node] = []\nend\n\
               wrap = (n) ->\n  name:     \"n\"\n  children: [n]\nend\n\
               deep = range(100).reduce(\"{\\\"name\\\": \\\"leaf\\\"}\".parse_json()) (acc, i) -> wrap(acc) end\n\
               t: new Node name: \"root\", children: [deep] end\n";
    let r = fail(src);
    assert_eq!(r.code, "E0208", "{r:#?}");
}

// ---- Clause 4, imports: the schema is the one in scope where the type was declared ----

#[test]
fn an_imported_schema_checks_its_fields_by_its_own_schemas() {
    let lib = "pub type Port\n  number: Int\n  assert number > 0, \"positive\"\nend\n\
               pub type Svc\n  ports: [Port]\nend\n";
    let main = "import \"lib.aura\" as lib\n\
                s: new lib.Svc ports: \"[{\\\"number\\\": 0}]\".parse_json() end\n";
    let r = eval_files(
        &[("main.aura", main), ("lib.aura", lib)],
        &EvalOptions::default(),
    )
    .expect_err("the imported invariant applies");
    assert_eq!(r[0].code, "E0515", "{r:#?}");
}
