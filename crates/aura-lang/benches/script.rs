//! The embedding API's costs (D33 §7–§9): what one call costs, and whether a
//! batch of events in one call really is cheaper than the same events one by one.
//!
//! The batch question is the one D33 makes a promise about. A telemetry host
//! at a million events a second either calls the rule per event or hands it
//! chunks; these two benchmarks process the same 1,000 events both ways, so
//! their times are directly comparable.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use serde::{Deserialize, Serialize};

use aura_lang::{Budget, Script};

const RULES: &str = r#"
pub def classify(e)
  alert: e.temp_c > 80.0
end

pub def summarize(events)
  total:  events.len()
  alerts: events.filter (e, i) -> e.temp_c > 80.0 end.len()
end

pub def route(path)
  backend: path.starts_with("/api") ? "api" : "static"
end
"#;

#[derive(Serialize)]
struct Event {
    device: &'static str,
    temp_c: f64,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Alert {
    alert: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Summary {
    total: u64,
    alerts: u64,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Route {
    backend: String,
}

fn events(n: usize) -> Vec<Event> {
    (0..n)
        .map(|i| Event {
            device: "sensor",
            temp_c: 20.0 + (i % 90) as f64,
        })
        .collect()
}

fn bench_script(c: &mut Criterion) {
    let rules = Script::builder()
        .source("rules.aura", RULES)
        .budget(Budget::per_call().steps(u64::MAX).memory_bytes(u64::MAX))
        .load()
        .expect("loads");

    // The smallest useful call: one string in, one object out.
    let route = rules.function::<(&str,), Route>("route").expect("binds");
    c.bench_function("script/call/route_one_string", |b| {
        b.iter(|| route.call(("/api/orders",)).expect("call"))
    });

    // D35: the cost a host call adds - arguments out as JSON, the result back
    // through the exact serializer. Compare with the plain call above.
    let hosted = Script::builder()
        .source(
            "hosted.aura",
            "import \"host:geo\" as geo
pub def route(ip)
  region: geo.region(ip)
end
",
        )
        .pure("geo.region", |ip: String| -> Option<String> {
            (ip == "203.0.113.7").then(|| "eu".to_string())
        })
        .budget(Budget::per_call().steps(u64::MAX).memory_bytes(u64::MAX))
        .load()
        .expect("loads");
    let hosted_route = hosted
        .function::<(&str,), serde_json::Value>("route")
        .expect("binds");
    c.bench_function("script/call/one_host_call", |b| {
        b.iter(|| hosted_route.call(("203.0.113.7",)).expect("call"))
    });

    // Loading: lex, parse, analyse, prove hermetic, evaluate the module.
    c.bench_function("script/load", |b| {
        b.iter(|| {
            Script::builder()
                .source("rules.aura", RULES)
                .load()
                .expect("loads")
        })
    });

    let batch = events(1_000);
    let mut g = c.benchmark_group("script/1000_events");
    g.throughput(Throughput::Elements(1_000));

    let classify = rules
        .function::<(&Event,), Alert>("classify")
        .expect("binds");
    g.bench_function("one_call_per_event", |b| {
        b.iter(|| {
            for e in &batch {
                classify.call((e,)).expect("call");
            }
        })
    });

    let summarize = rules
        .function::<(&[Event],), Summary>("summarize")
        .expect("binds");
    g.bench_function("one_call_per_batch", |b| {
        b.iter(|| summarize.call((&batch[..],)).expect("call"))
    });
    g.finish();
}

criterion_group!(benches, bench_script);
criterion_main!(benches);
