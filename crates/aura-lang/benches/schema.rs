//! What the schema work added in the D23–D28 run actually costs.
//!
//! Four things landed on the evaluation hot path in quick succession, and none
//! of them was measured at the time:
//!
//! - every `new` rebuilds its map so key order comes from the schema (D26 era);
//! - `[T]` checks each element rather than only the container;
//! - a schema's invariants run on every instantiation (D28);
//! - `TypeName` gained a `Box` and stopped being `Copy`, so field types are
//!   cloned rather than copied wherever they are carried.
//!
//! The reference manifest in `eval.rs` exercises none of them, and measuring it
//! before and after the run showed no change — which says nothing about these.
//!
//! Each benchmark here is a **pair**: the same manifest with and without the
//! feature. An absolute number would only say how fast this machine is; the
//! difference between two shapes that do the same work is the cost.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use aura_lang::eval::{Interpreter, Options};

/// Lex, parse and evaluate — the same path the CLI takes.
fn run(src: &str) {
    let toks = aura_lang::lexer::Lexer::new(black_box(src), 0)
        .tokenize()
        .expect("bench manifest must lex");
    let module = aura_lang::parser::Parser::new(toks)
        .parse_module()
        .expect("bench manifest must parse");
    let mut it = Interpreter::new(Options::default());
    black_box(
        it.eval_module(&module)
            .expect("bench manifest must evaluate"),
    );
}

/// N instances of a four-field schema, written in declaration order.
fn instances(n: usize, invariants: bool) -> String {
    let mut s =
        String::from("type Service\n  name: String\n  port: Int\n  tier: String\n  weight: Int\n");
    if invariants {
        s.push_str("  assert port > 1024, \"unprivileged\"\n");
        s.push_str("  assert weight > 0, \"positive weight\"\n");
    }
    s.push_str("end\n");
    for i in 0..n {
        s.push_str(&format!(
            "s{i}: new Service\n  name: \"svc\"\n  port: {}\n  tier: \"backend\"\n  weight: 3\nend\n",
            2000 + i
        ));
    }
    s
}

/// The cost of running a schema's own rules on every instantiation (D28).
fn bench_invariants(c: &mut Criterion) {
    let without = instances(200, false);
    let with = instances(200, true);
    let mut g = c.benchmark_group("schema/invariants");
    g.bench_function("200_instances_without", |b| b.iter(|| run(&without)));
    g.bench_function("200_instances_with_two", |b| b.iter(|| run(&with)));
    g.finish();
}

/// A list field holding `n` elements, declared either bare or with an element
/// type. The difference is the per-element check D26 introduced.
fn list_manifest(n: usize, typed: bool) -> String {
    let ty = if typed { "[Int]" } else { "List" };
    let items: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    format!(
        "type Bag\n  xs: {ty}\nend\n\nb: new Bag\n  xs: [{}]\nend\n",
        items.join(", ")
    )
}

fn bench_element_check(c: &mut Criterion) {
    // 5000, not 500: at the smaller size the per-element check sat below this
    // machine's run-to-run variance and the pair changed sign between runs. A
    // benchmark that cannot detect its own subject reports noise as a finding.
    let bare = list_manifest(5000, false);
    let typed = list_manifest(5000, true);
    let mut g = c.benchmark_group("schema/element_type");
    g.bench_function("5000_elements_bare_list", |b| b.iter(|| run(&bare)));
    g.bench_function("5000_elements_typed_list", |b| b.iter(|| run(&typed)));
    g.finish();
}

/// Key ordering rebuilds the instance map on every `new`. Writing the fields in
/// reverse order does the same work either way — the point is that both cost
/// the same now, and that the rebuild is not a per-field surprise.
fn reordered(n: usize) -> String {
    let mut s = String::from(
        "type Service\n  name: String\n  port: Int\n  tier: String\n  weight: Int\nend\n",
    );
    for i in 0..n {
        s.push_str(&format!(
            "s{i}: new Service\n  weight: 3\n  tier: \"backend\"\n  port: {}\n  name: \"svc\"\nend\n",
            2000 + i
        ));
    }
    s
}

/// What passing through a schema costs at all, against the same data written as
/// a plain object literal. This is validation, defaults and the key-order
/// rebuild together — the rebuild cannot be switched off from the manifest, so
/// it is not separable without changing the compiler.
fn bench_schema_overhead(c: &mut Criterion) {
    let mut plain = String::new();
    for i in 0..200 {
        plain.push_str(&format!(
            "s{i}:\n  name: \"svc\"\n  port: {}\n  tier: \"backend\"\n  weight: 3\nend\n",
            2000 + i
        ));
    }
    let schema = instances(200, false);
    let mut g = c.benchmark_group("schema/overhead");
    g.bench_function("200_plain_objects", |b| b.iter(|| run(&plain)));
    g.bench_function("200_schema_instances", |b| b.iter(|| run(&schema)));
    g.finish();
}

fn bench_key_order(c: &mut Criterion) {
    let in_order = instances(200, false);
    let out_of_order = reordered(200);
    let mut g = c.benchmark_group("schema/key_order");
    g.bench_function("200_instances_in_declaration_order", |b| {
        b.iter(|| run(&in_order))
    });
    g.bench_function("200_instances_written_reversed", |b| {
        b.iter(|| run(&out_of_order))
    });
    g.finish();
}

/// `+` copies both operands into a new list (D23). Repeated joining is the
/// shape a base set plus an environment tail produces, so it is worth knowing
/// what it costs against building the same list in one literal.
fn bench_concat(c: &mut Criterion) {
    const N: usize = 60;
    // The same list, the same length, built two ways. Anything else would be
    // comparing different work and reporting the difference as a cost.
    let literal = {
        let items: Vec<String> = (0..N).map(|i| i.to_string()).collect();
        format!("xs: [{}]\n", items.join(", "))
    };
    let joined = {
        let mut s = String::from("a0 = [0]\n");
        for i in 1..N {
            s.push_str(&format!("a{i} = a{} + [{i}]\n", i - 1));
        }
        s.push_str(&format!("xs: a{}\n", N - 1));
        s
    };
    let mut g = c.benchmark_group("eval/list_building");
    g.bench_function("literal_60", |b| b.iter(|| run(&literal)));
    g.bench_function("concat_60_times", |b| b.iter(|| run(&joined)));
    g.finish();
}

criterion_group!(
    benches,
    bench_invariants,
    bench_element_check,
    bench_schema_overhead,
    bench_key_order,
    bench_concat
);
criterion_main!(benches);
