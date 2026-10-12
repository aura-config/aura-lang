//! Host functions: a rule file that asks its host for what it needs (D35).
//!
//! An advanced use of Aura. The routing rules below decide where a request
//! goes, and to decide they need two things only the service has: which region
//! an address is in, and whether a feature is switched on for a customer. The
//! host gives the script exactly those two functions and nothing else.
//!
//! Run it with `cargo run --example host_functions`.
//!
//! What it is written to show:
//!
//! 1. The script declares what it needs – `import "host:geo" as geo` – so the
//!    requirement is visible at the top of the file, and the load fails if the
//!    host did not provide it.
//! 2. The host declares what kind each function is. `geo.region` is `pure`: a
//!    lookup in a table that ships with the binary. `flags.enabled` is an
//!    `effect`: it reads a flag service that changes at run time. That
//!    difference is the host's to state; the language cannot check it.
//! 3. An address the table does not know comes back as `null`, and the rule
//!    decides what that means. A flag service that is down is an error.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use aura_lang::Script;
use serde::Deserialize;

const ROUTING: &str = r##"
import "host:geo" as geo
import "host:flags" as flags

pub def route(ip, customer)
  region = geo.region(ip)

  # An unknown address is not an error: it is a request with no region, and
  # the rule says where those go.
  backend: region == null ? "global" : "#{region}-cluster"

  # Read only when the region is known – a host call costs budget like any
  # other work, so a rule should not make one it does not need.
  canary: region != null && flags.enabled("canary", customer)
end
"##;

#[derive(Deserialize, Debug, PartialEq)]
struct Route {
    backend: String,
    canary: bool,
}

/// The flag service: changes while the process runs, which is what makes
/// reading it an effect rather than a pure lookup.
#[derive(Default)]
struct Flags(RwLock<HashMap<(String, String), bool>>);

impl Flags {
    fn enabled(&self, flag: &str, customer: &str) -> Result<bool, String> {
        let table = self.0.read().map_err(|_| "flag service unavailable")?;
        Ok(table
            .get(&(flag.to_string(), customer.to_string()))
            .copied()
            .unwrap_or(false))
    }
}

fn main() -> Result<(), aura_lang::Error> {
    let geoip: HashMap<&str, &str> = [("203.0.113.7", "eu"), ("198.51.100.4", "us")].into();
    let flags = Arc::new(Flags::default());
    flags
        .0
        .write()
        .expect("lock")
        .insert(("canary".into(), "acme".into()), true);

    let flag_service = flags.clone();
    let rules = Script::builder()
        .source("routing.aura", ROUTING)
        // Deterministic: the table ships with the binary.
        .pure("geo.region", move |ip: String| {
            geoip.get(ip.as_str()).map(|r| r.to_string())
        })
        // Not deterministic: the answer can change between two calls.
        .effect("flags.enabled", move |flag: String, customer: String| {
            flag_service.enabled(&flag, &customer)
        })
        .load()?;

    let route = rules.function::<(&str, &str), Route>("route")?;
    for (ip, customer) in [
        ("203.0.113.7", "acme"),
        ("198.51.100.4", "globex"),
        ("192.0.2.1", "acme"),
    ] {
        let r = route.call((ip, customer))?;
        println!("{ip:14} {customer:7} -> {} canary={}", r.backend, r.canary);
    }
    Ok(())
}
