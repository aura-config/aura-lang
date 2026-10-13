//! The Rust readers of the examples are documentation that runs, so they are
//! run here: each asserts what it reads. And their types, written by
//! `aura types`, must be what the generator writes today - a schema changed in
//! a manifest without regenerating them fails here, not in a reader's build.

#[path = "../examples/read_device.rs"]
mod read_device;
#[path = "../examples/read_k8s_deploy.rs"]
mod read_k8s_deploy;
#[path = "../examples/read_service_catalog.rs"]
mod read_service_catalog;

use std::path::Path;
use std::process::Command;

#[test]
fn the_device_reader_runs() {
    read_device::main();
}

#[test]
fn the_k8s_reader_runs() {
    read_k8s_deploy::main();
}

#[test]
fn the_service_catalog_reader_runs() {
    read_service_catalog::main();
}

#[test]
fn the_generated_types_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (manifest, committed) in [
        (
            "../../examples/device/device.aura",
            "examples/readers/device_types.rs",
        ),
        (
            "../../examples/k8s_deploy/k8s_deploy.aura",
            "examples/readers/k8s_types.rs",
        ),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_aura"))
            .args(["types"])
            .arg(root.join(manifest))
            .args(["--lang", "rust"])
            .output()
            .expect("run aura types");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let now = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
        let have = std::fs::read_to_string(root.join(committed))
            .expect("committed types")
            .replace("\r\n", "\n");
        assert_eq!(
            have, now,
            "{committed} is out of date: regenerate it with `aura types {manifest} --lang rust`"
        );
    }
}
