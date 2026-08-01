//! **A sketch, not a supported API.** Aura as a scripting layer inside a Rust
//! application: rules live in a `.aura` file, the application executes them, and
//! changing behaviour means editing that file rather than rebuilding.
//!
//! Run it with `cargo run --example scripting`.
//!
//! # Why this is a sketch
//!
//! Everything below uses public API and works today. What does not exist is a
//! comfortable way to reach it. `facade::eval_file` returns `serde_json::Value`,
//! and JSON cannot hold a function — so anything that wants to *call* into the
//! script has to drop to the layer used here and manage lifetimes by hand.
//!
//! A real feature would be a `Script` type owning the `SourceCache` and exposing
//! something like `script.call("route", &[json!("/api"), json!("GET")])`. Until
//! that exists, treat this file as a demonstration that the pieces fit, not as
//! the shape the API will end up having.
//!
//! # What is missing beyond ergonomics
//!
//! The script cannot call back into the application. Methods can be registered
//! on the interpreter's registry, but `MethodFn` is a plain `fn` pointer with no
//! captured state, so a host method cannot see your `AppState`; and the global
//! builtins (`env`, `read_file`) are not extensible at all.
//!
//! That gap is deliberate for now. Handing a script arbitrary host functions
//! gives away the property that makes Aura worth embedding — that a manifest can
//! do nothing you did not grant, and that `aura check --hermetic` can *prove* it
//! before you run it. Whatever fills the gap should be shaped like the existing
//! capability model rather than bolted beside it.
//!
//! # What Aura is not
//!
//! There are no loops, no mutation, no I/O and no clock. Rules that are a pure
//! function of their inputs — routing, feature flags, pricing tiers, validation —
//! fit well. A job that mutates state or queries a database does not, and should
//! not be written here.

use aura_lang::eval::value::Value;
use aura_lang::eval::{Interpreter, Options};
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;
use aura_lang::source::SourceCache;
use aura_lang::span::Span;

/// Loaded at runtime in a real application — from a file, a config map, a
/// database row. Inline here so the example has no setup.
const RULES: &str = r#"
# Routing rules. Editing this changes behaviour with no rebuild.
pub def route(path, method)
  backend: cond
    path.starts_with("/api") -> "api-cluster"
    method == "POST"         -> "write-pool"
    else -> "static"
  end
  timeout_ms: cond
    path.starts_with("/api") -> 5000
    else -> 1000
  end
end

version: "rules-v3"
"#;

fn main() {
    // The cache owns every source text, and `Value<'a>` borrows from it. It must
    // therefore outlive every value taken out of the script — which is the main
    // thing a `Script` wrapper would hide. On reload: build a new cache, drop the
    // old values first, then drop the old cache.
    let cache = SourceCache::new();
    let (id, src) = cache.add("rules.aura".into(), RULES.to_string());

    let tokens = Lexer::new(src, id).tokenize().expect("rules.aura lexes");
    let module = Parser::new(tokens)
        .parse_module()
        .expect("rules.aura parses");

    let mut interp = Interpreter::new(Options {
        strict: false,
        dry_run: false,
    });
    // No capabilities are granted, so the script cannot read files or the
    // environment even if it tries. This is the default, not a precaution.
    let exported = interp.eval_module(&module).expect("rules.aura evaluates");

    let Value::Object(top) = &exported else {
        panic!("a module evaluates to an object");
    };

    // Ordinary data, exactly as `facade::eval_file` would give it.
    println!("rules version: {:?}", top.get("version").expect("version"));

    // And the part JSON cannot carry: a function, called from Rust, repeatedly,
    // with different arguments.
    let route = top.get("route").expect("`pub def route`").clone();
    let span = Span::new(id, 0, 0);

    for (path, method) in [
        ("/api/orders", "GET"),
        ("/img/logo.png", "GET"),
        ("/submit", "POST"),
    ] {
        let decision = interp
            .call_value(&route, &[Value::str(path), Value::str(method)], span)
            .expect("route() is callable");

        let Value::Object(fields) = &decision else {
            panic!("route() returns an object");
        };
        let backend = fields.get("backend").expect("backend");
        let timeout = fields.get("timeout_ms").expect("timeout_ms");
        println!("{path:16} {method:5} -> {backend:?}, timeout {timeout:?}");

        // Assertions rather than eyeballing, so running this checks itself.
        let expected = match (path, method) {
            ("/api/orders", _) => ("api-cluster", 5000),
            (_, "POST") => ("write-pool", 1000),
            _ => ("static", 1000),
        };
        assert_eq!(*backend, Value::str(expected.0), "backend for {path}");
        assert_eq!(*timeout, Value::Int(expected.1), "timeout for {path}");
    }

    println!("\nall decisions matched — the script drove them, not this binary");
}
