//! `examples/device`, read from Rust instead of `aura eval`.
//!
//! The firmware build gets its board and sensor configuration as typed values:
//! `Board` and `Sensor` are written by `aura types --lang rust` from the
//! schemas in `device.aura` (see `readers/device_types.rs`), and the top level
//! is the few fields this program uses. Everything `aura eval` would check -
//! the schemas, their invariants, `--strict` - is checked here too.
//!
//! Run it with `cargo run --example read_device`.

use std::path::Path;

use aura_lang::Script;
use serde::Deserialize;

#[path = "readers/device_types.rs"]
mod types;
use types::{Board, Bus, Sensor};

/// The parts of the manifest's output this program reads. Fields it does not
/// name are ignored, so the manifest can grow without breaking the build.
#[derive(Deserialize)]
struct Device {
    board_info: Board,
    sensor_list: Vec<Sensor>,
    battery_days: String,
    init_script: String,
}

pub fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/device");
    let device: Device = Script::builder()
        .file(dir.join("device.aura"))
        // It reads board.toml and sensors.yaml beside it - and nothing else.
        .allow_read(&dir)
        .strict()
        .load()
        .expect("device.aura loads")
        .output()
        .expect("the output has the shape this program expects");

    let b = &device.board_info;
    println!("{} with {} MB flash", b.model, b.flash_mb);
    for s in &device.sensor_list {
        println!(
            "  {:8} {:?}  every {} ms -> {}",
            s.id, s.bus, s.poll_ms, s.topic
        );
    }
    println!("battery: about {} days", device.battery_days);

    assert_eq!(b.model, "esp32-c6");
    assert_eq!(device.sensor_list.len(), 2);
    assert!(device.sensor_list.iter().all(|s| s.bus == Bus::I2c));
    assert_eq!(device.battery_days, "50.5");
    assert!(device.init_script.starts_with("#!/bin/sh"));
}
