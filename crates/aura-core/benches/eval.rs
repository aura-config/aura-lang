//! Бенчмарк hot path Фазы 3: полный пайплайн lex + parse + eval эталонного манифеста.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use indexmap::IndexMap;
use std::collections::HashMap;

use aura_core::eval::value::Value;
use aura_core::eval::{EnvCap, Interpreter, MemFs, Options};

const MANIFEST: &str = include_str!("../tests/fixtures/production_deploy.aura");

fn interp<'a>() -> Interpreter<'a> {
    let mut it = Interpreter::new(Options::default());
    it.fs = Box::new(MemFs(HashMap::from([(
        "./Cargo.toml".to_string(),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\n".to_string(),
    )])));
    it.env_cap = EnvCap::AllowAll;
    it.env_overrides.insert("APP_ENV".to_string(), "production".to_string());
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
            let toks = aura_core::lexer::Lexer::new(black_box(MANIFEST), 0).tokenize().unwrap();
            let module = aura_core::parser::Parser::new(toks).parse_module().unwrap();
            black_box(interp().eval_module(&module).unwrap());
        })
    });
}

criterion_group!(benches, bench_eval);
criterion_main!(benches);
