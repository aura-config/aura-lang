//! A checkout service whose risk rules live in Aura (D33, D35, D36).
//!
//! An online store decides for every order: approve, hold for review, or
//! decline. The decision belongs to the risk team, who change it several
//! times a day; the service belongs to engineers, who release it once a week.
//! So the rules live in `checkout.aura` and the service runs them:
//!
//! - it **calls** them per order (D33) - parsed and checked once, then cheap;
//! - it **gives** them what only it has (D35): a GeoIP table (`pure`), the
//!   dispute database and exchange rates (`effect`) - and gives the rates to
//!   the shared `money.aura` module by name, nothing more;
//! - it **reloads** them when the file changes (D36) - and refuses an edit
//!   that breaks the contract or reaches for something it was never given,
//!   while the old rules keep serving.
//!
//! Run it with `cargo run --example checkout`. It works on a copy of the rule
//! files in a temporary directory, so it can edit them as the risk team would.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use aura_lang::{Error, Function, Script, ScriptVersion};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Line {
    sku: &'static str,
    qty: i64,
    price: f64,
}

#[derive(Serialize)]
struct Order {
    customer: &'static str,
    ip: &'static str,
    currency: &'static str,
    lines: Vec<Line>,
}

#[derive(Deserialize, Debug)]
struct Decision {
    action: String,
    reasons: Vec<String>,
    total_eur: Option<f64>,
}

#[derive(Deserialize)]
struct Decided {
    decision: Decision,
}

fn order(customer: &'static str, ip: &'static str, currency: &'static str, price: f64) -> Order {
    Order {
        customer,
        ip,
        currency,
        lines: vec![
            Line {
                sku: "KB-01",
                qty: 2,
                price,
            },
            Line {
                sku: "CBL-3",
                qty: 1,
                price: 9.5,
            },
        ],
    }
}

/// What the service owns and the rules may ask for.
struct Services {
    geoip: HashMap<&'static str, &'static str>,
    rates: RwLock<HashMap<String, f64>>,
    disputes: HashMap<&'static str, i64>,
}

impl Services {
    fn chargebacks(&self, customer: &str) -> Result<i64, String> {
        // One customer's record lives on a shard that is down: the rules see
        // an error, not a zero, and the service decides what that means.
        if customer == "mallory" {
            return Err("risk database unavailable".into());
        }
        Ok(self.disputes.get(customer).copied().unwrap_or(0))
    }
}

/// Copies the rules where they can be edited, as a deploy would put them.
fn stage_rules() -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/checkout");
    let dir = std::env::temp_dir().join(format!("aura-checkout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    for f in ["checkout.aura", "money.aura"] {
        std::fs::copy(src.join(f), dir.join(f)).expect("copy rules");
    }
    dir.join("checkout.aura")
}

/// How the risk team publishes an edit: write a new file, rename it over the
/// old one. A reader never sees half a file (D36 §6).
fn publish(path: &Path, edit: impl FnOnce(String) -> String) {
    let text = edit(std::fs::read_to_string(path).expect("read rules"));
    let tmp = path.with_extension("aura.new");
    std::fs::write(&tmp, text).expect("write rules");
    std::fs::rename(&tmp, path).expect("publish rules");
}

/// The service's own policy for rules that could not run: hold the order.
fn run<'o>(decide: &Function<'_, (&'o Order,), Decided>, o: &'o Order) -> (String, Vec<String>) {
    match decide.call((o,)) {
        Ok(d) => (d.decision.action, d.decision.reasons),
        Err(e) => ("review".into(), vec![format!("rules failed: {}", e.code())]),
    }
}

pub fn main() {
    let services = Arc::new(Services {
        geoip: [
            ("198.51.100.4", "DE"),
            ("203.0.113.7", "US"),
            ("192.0.2.66", "KP"),
        ]
        .into(),
        rates: RwLock::new([("USD".to_string(), 0.92)].into()),
        disputes: [("dave", 5)].into(),
    });
    let rules_path = stage_rules();

    // Every reload, automatic or refused, reaches the service's log. Here the
    // log is a channel, so the example can wait for it.
    let (log, reloads) = mpsc::channel::<Result<ScriptVersion, Error>>();

    let (geo, fx, risk) = (services.clone(), services.clone(), services.clone());
    let rules = Script::builder()
        .file(&rules_path)
        // Deterministic: the table ships with the binary.
        .pure("geo.country", move |ip: String| {
            geo.geoip.get(ip.as_str()).map(|c| c.to_string())
        })
        // Effects: the answer can change between two calls.
        .effect(
            "fx.rate",
            move |currency: String| -> Result<Option<f64>, String> {
                let rates = fx.rates.read().map_err(|_| "rate table poisoned")?;
                Ok(rates.get(&currency).copied())
            },
        )
        .effect("risk.chargebacks", move |customer: String| {
            risk.chargebacks(&customer)
        })
        // money.aura is imported, and imports get nothing by default.
        .grant_to_imports("fx.rate")
        .watch(Duration::from_millis(100), move |r| {
            let _ = log.send(r);
        })
        .load()
        .expect("the rules load");

    // Bound once: name, kind and arity are checked now, and a reload is
    // refused if it would break any of them (D36 §2).
    let decide = rules
        .function::<(&Order,), Decided>("decide")
        .expect("the rules export decide(order)");

    println!("rules v{}", rules.version().generation);
    let orders = [
        order("alice", "198.51.100.4", "EUR", 49.0),
        order("bob", "203.0.113.7", "USD", 640.0),
        order("carol", "192.0.2.66", "EUR", 30.0),
        order("dave", "198.51.100.4", "EUR", 20.0),
        order("erin", "198.51.100.4", "CHF", 15.0),
        order("mallory", "198.51.100.4", "EUR", 15.0),
    ];
    let mut seen = Vec::new();
    for o in &orders {
        let (action, reasons) = run(&decide, o);
        println!("  {:8} {:8} {}", o.customer, action, reasons.join("; "));
        seen.push(action);
    }
    assert_eq!(
        seen,
        ["approve", "review", "decline", "decline", "review", "review"]
    );
    let bob = &orders[1];
    let total = decide.call((bob,)).expect("bob").decision.total_eur;
    assert!(
        total.is_some_and(|t| (t - 1186.34).abs() < 0.01),
        "{total:?}"
    );

    // 1. The risk team raises the review threshold. No release: the watcher
    //    sees the new content and the next order runs on it.
    publish(&rules_path, |t| {
        t.replace("review_over_eur = 1000.0", "review_over_eur = 2000.0")
    });
    let v = reloads
        .recv_timeout(Duration::from_secs(10))
        .expect("reloaded");
    let v = v.expect("the edit is valid");
    println!("rules v{} after raising the threshold", v.generation);
    assert_eq!(run(&decide, bob).0, "approve");

    // 2. An edit renames the entry point. The service bound `decide`, so the
    //    reload is refused and the rules it had keep serving.
    publish(&rules_path, |t| {
        t.replace("pub def decide(", "pub def judge(")
    });
    let refused = reloads
        .recv_timeout(Duration::from_secs(10))
        .expect("reported");
    let e = refused.expect_err("a broken contract is refused");
    println!("refused: {}", e.reports()[0].message);
    assert_eq!(e.code(), "E0011");
    assert_eq!(
        run(&decide, bob).0,
        "approve",
        "still on the last good rules"
    );

    // 3. An edit reaches for a secret. The service never granted `env()`, and a
    //    reload cannot grant itself anything (D36 §3).
    publish(&rules_path, |t| {
        t.replace("pub def judge(", "pub def decide(") + "\nleak = env(\"DB_PASSWORD\", \"\")\n"
    });
    let refused = reloads
        .recv_timeout(Duration::from_secs(10))
        .expect("reported");
    let e = refused.expect_err("an ungranted capability is refused");
    println!("refused: {}", e.reports()[0].message);
    assert_eq!(e.code(), "E0310");

    // 4. Rates are refreshed by the service, not by the rules: CHF gets a
    //    rate, and the next order is priced without touching any file.
    services
        .rates
        .write()
        .expect("rates")
        .insert("CHF".into(), 1.05);
    assert_eq!(run(&decide, &orders[4]).0, "approve");

    println!(
        "serving rules v{} ({}...)",
        rules.version().generation,
        &rules.version().hash[..12]
    );
    assert_eq!(rules.version().generation, 2);
}
