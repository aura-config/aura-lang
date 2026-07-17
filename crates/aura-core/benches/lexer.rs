//! Бенчмарк hot path Фазы 1: пропускная способность лексера (МБ/с) на эталонном манифесте.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const MANIFEST: &str = include_str!("../tests/fixtures/production_deploy.aura");

fn bench_lexer(c: &mut Criterion) {
    // x64 — типичный конфиг среднего размера (~100 КБ)
    let big: String = MANIFEST.repeat(64);
    let mut group = c.benchmark_group("lexer");
    group.throughput(Throughput::Bytes(big.len() as u64));
    group.bench_function("tokenize_manifest_x64", |b| {
        b.iter(|| aura_core::lexer::Lexer::new(black_box(&big), 0).tokenize().unwrap())
    });
    group.finish();
}

criterion_group!(benches, bench_lexer);
criterion_main!(benches);
