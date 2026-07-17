//! Бенчмарк hot path Фазы 2: полный фронтенд (lex + parse) на эталонном манифесте.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const MANIFEST: &str = include_str!("../tests/fixtures/production_deploy.aura");

fn bench_parser(c: &mut Criterion) {
    // Импорты обязаны идти в начале файла, поэтому конкатенировать манифест нельзя —
    // прогоняем 64 независимых разбора за итерацию.
    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(MANIFEST.len() as u64 * 64));
    group.bench_function("lex_and_parse_manifest_x64", |b| {
        b.iter(|| {
            for _ in 0..64 {
                let toks = aura_core::lexer::Lexer::new(black_box(MANIFEST), 0).tokenize().unwrap();
                black_box(aura_core::parser::Parser::new(toks).parse_module().unwrap());
            }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
