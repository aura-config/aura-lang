//! Phase 3 hot-path benchmark: the full lex + parse + eval pipeline on the reference manifest.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use indexmap::IndexMap;
use std::collections::HashMap;

use aura_lang::eval::value::Value;
use aura_lang::eval::{EnvCap, Interpreter, MemFs, Options};

const MANIFEST: &str = include_str!("../tests/fixtures/production_deploy.aura");

fn interp<'a>() -> Interpreter<'a> {
    let mut it = Interpreter::new(Options::default());
    it.fs = Box::new(MemFs(HashMap::from([(
        "./Cargo.toml".to_string(),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\n".to_string(),
    )])));
    it.env_cap = EnvCap::AllowAll;
    it.env_overrides
        .insert("APP_ENV".to_string(), "production".to_string());
    it.provide_module("rust", Value::object(IndexMap::new()));
    let mut labels = IndexMap::new();
    labels.insert("team".to_string(), Value::str("core"));
    let mut defaults = IndexMap::new();
    defaults.insert("global_labels".to_string(), Value::object(labels));
    it.provide_module("defaults", Value::object(defaults));
    it
}

fn bench_eval(c: &mut Criterion) {
    c.bench_function("eval/full_pipeline_manifest", |b| {
        b.iter(|| {
            let toks = aura_lang::lexer::Lexer::new(black_box(MANIFEST), 0)
                .tokenize()
                .unwrap();
            let module = aura_lang::parser::Parser::new(toks).parse_module().unwrap();
            black_box(interp().eval_module(&module).unwrap());
        })
    });
}

criterion_group!(benches, bench_eval);
criterion_main!(benches);
