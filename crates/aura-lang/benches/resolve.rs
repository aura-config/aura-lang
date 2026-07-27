//! Name resolution is on the editor's hot path: every go-to-definition,
//! find-references and rename request re-derives it from the buffer, and requests
//! arrive as the user types. This measures the whole chain a request pays for
//! (lex + parse + resolve) against parse alone, so the cost of resolution itself
//! is visible rather than assumed.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const MANIFEST: &str = include_str!("../tests/fixtures/production_deploy.aura");

fn parse(src: &str) -> aura_lang::parser::ast::Module<'_> {
    let toks = aura_lang::lexer::Lexer::new(src, 0).tokenize().unwrap();
    aura_lang::parser::Parser::new(toks).parse_module().unwrap()
}

fn bench_resolve(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve");
    group.throughput(Throughput::Bytes(MANIFEST.len() as u64));

    // What an LSP request costs from scratch.
    group.bench_function("lex_parse_resolve_manifest", |b| {
        b.iter(|| {
            let module = parse(black_box(MANIFEST));
            black_box(aura_lang::resolve::resolve(MANIFEST, &module));
        })
    });

    // Resolution alone, over an already-parsed module.
    let module = parse(MANIFEST);
    group.bench_function("resolve_only", |b| {
        b.iter(|| black_box(aura_lang::resolve::resolve(black_box(MANIFEST), &module)))
    });
    group.finish();

    // A large manifest, where anything superlinear becomes obvious. Resolution was
    // once quadratic in file size (a full token scan per declaration, and a full
    // scan of a scope's bindings per lookup): this case took 119 ms before both
    // were indexed. It should stay in the same order of magnitude as the parse.
    let big = big_manifest(5000);
    let big_module = parse(&big);
    let mut group = c.benchmark_group("resolve_large");
    group.throughput(Throughput::Bytes(big.len() as u64));
    group.bench_function("resolve_190kb", |b| {
        b.iter(|| black_box(aura_lang::resolve::resolve(black_box(&big), &big_module)))
    });
    group.finish();
}

/// `n` bindings, each used twice — once directly and once through interpolation —
/// plus a use of an early binding, so lookups do not all hit the newest declaration.
fn big_manifest(n: usize) -> String {
    let mut s = String::from("base = 1\n");
    for i in 0..n {
        s.push_str(&format!("v{i} = base + {i}\n"));
        s.push_str(&format!("k{i}: \"#{{v{i}}}-x\"\n"));
    }
    s
}

criterion_group!(benches, bench_resolve);
criterion_main!(benches);
