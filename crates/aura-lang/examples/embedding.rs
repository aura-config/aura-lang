//! Aura inside a Rust service: telemetry rules that change without a redeploy.
//!
//! The service receives events from devices, hands them to a rule file in
//! chunks, and keeps the running totals itself. Editing `telemetry.aura`
//! changes what counts as an alert; the binary is not rebuilt.
//!
//! Run it with `cargo run --example embedding`.
//!
//! What this example is written to show, in order of importance:
//!
//! 1. The script cannot touch anything. No grant was given, so `Script` proved
//!    at load that `telemetry.aura` reads no file, no variable and no clock —
//!    not that it happened not to, on this run.
//! 2. The script cannot run away. Each call has a budget in steps and bytes;
//!    a rule that loops by accident, or on purpose, ends that call with an
//!    error instead of stalling the service.
//! 3. Memory does not grow with traffic. A call releases everything it made.
//! 4. The types are the host's own. Events go in as `&[Event]` and the summary
//!    comes back as `Summary`, through `serde`; nothing in this file names a
//!    lexer, a span or a lifetime.

use aura_lang::{Budget, Script};
use serde::{Deserialize, Serialize};

/// The rule file. In a service it would be a file next to the binary, or a
/// config map; inline here so the example runs with no setup.
const TELEMETRY: &str = r#"
# Which readings are alerts. Edit this file to change the rule; the service
# picks it up without being rebuilt.
pub def summarize(events)
  hot = events.filter (e, i) -> e.temp_c > 80.0 end

  total:  events.len()
  alerts: hot.len()
  worst:  hot.len() > 0 ? hot.map (e, i) -> e.temp_c end.max() : null
end

version: "telemetry-v4"
"#;

#[derive(Serialize)]
struct Event {
    device: String,
    temp_c: f64,
}

#[derive(Deserialize, Debug, PartialEq)]
struct Summary {
    total: u64,
    alerts: u64,
    worst: Option<f64>,
}

#[derive(Default, Debug)]
struct Totals {
    events: u64,
    alerts: u64,
    worst: Option<f64>,
}

impl Totals {
    fn merge(&mut self, s: Summary) {
        self.events += s.total;
        self.alerts += s.alerts;
        self.worst = match (self.worst, s.worst) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
}

fn main() -> Result<(), aura_lang::Error> {
    // Load once: lexing, parsing, analysis and the hermetic proof all happen
    // here. A wrong rule file fails now, when the service starts, with the same
    // diagnostic `aura check` would print.
    let rules = Script::builder()
        .source("telemetry.aura", TELEMETRY)
        .budget(Budget::per_call().steps(5_000_000).memory_mb(32))
        .load()?;

    // Plain data comes out by name, into any `Deserialize` type.
    let version: String = rules.get("version")?;
    println!("rules: {version}");

    // Bind the function once. Its existence, that it is a function, and that it
    // takes one argument are all checked here — not on the first event.
    let summarize = rules.function::<(&[Event],), Summary>("summarize")?;

    let mut totals = Totals::default();
    for chunk in incoming_events().chunks(10_000) {
        // One call per chunk: one budget, one conversion of each event, and
        // nothing left behind when it returns.
        totals.merge(summarize.call((chunk,))?);
    }

    println!(
        "{} events, {} alerts, worst {:?} °C",
        totals.events, totals.alerts, totals.worst
    );
    Ok(())
}

/// Stands in for a socket or a queue.
fn incoming_events() -> Vec<Event> {
    (0..50_000)
        .map(|i| Event {
            device: format!("sensor-{}", i % 64),
            temp_c: 20.0 + (i % 90) as f64,
        })
        .collect()
}
