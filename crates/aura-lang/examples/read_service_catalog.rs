//! `examples/service_catalog`, read from Rust instead of `aura eval`.
//!
//! The manifest declares no schema - it joins `Cargo.toml` and `team.json`
//! into one record - so there is nothing for `aura types` to write; the struct
//! is described here. That is the common case for a manifest that only
//! gathers data: describe the output once, and let serde check every field.
//!
//! Run it with `cargo run --example read_service_catalog`.

use std::path::Path;

use aura_lang::Script;
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    service: Service,
}

#[derive(Deserialize)]
struct Service {
    name: String,
    version: String,
    owner: String,
    oncall: String,
}

pub fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/service_catalog");
    let catalog: Catalog = Script::builder()
        .file(dir.join("service_catalog.aura"))
        // It reads ./Cargo.toml and ./team.json - beside the manifest, wherever
        // this program runs (D37).
        .allow_read(&dir)
        .load()
        .expect("service_catalog.aura loads")
        .output()
        .expect("the output is a catalog entry");

    let s = &catalog.service;
    println!(
        "{} {} - owner {}, on call {}",
        s.name, s.version, s.owner, s.oncall
    );

    assert_eq!(s.name, "payments-service");
    assert_eq!(s.version, "3.1.4");
    assert_eq!(
        (s.owner.as_str(), s.oncall.as_str()),
        ("alice", "platform-team")
    );
}
