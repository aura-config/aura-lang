//! The embedding example shown in both READMEs, kept here so it compiles and
//! runs: `tests/readme_example.rs` checks that the READMEs show exactly the
//! lines between the markers, and calls `run`.
//!
//! Run it with `cargo run --example readme_embed`.

// README: start
use aura_lang::Script;
use serde::Serialize;

#[derive(Serialize)]
struct Order {
    plan: String,
    seats: i64,
}

// The rule lives in Aura: change the price without rebuilding the service.
const PRICING: &str = r#"
pub def price(order)
  base = order.plan == "pro" ? 20.0 : 8.0
  base * order.seats.to_float()
end
"#;

pub fn run() -> Result<f64, aura_lang::Error> {
    // Loaded once; it can read no file and no variable - none was granted.
    let rules = Script::builder().source("pricing.aura", PRICING).load()?;
    let order = Order {
        plan: "pro".into(),
        seats: 3,
    };
    rules.call("price", (order,))
}
// README: end

pub fn main() {
    println!("{}", run().expect("the rule runs"));
}
