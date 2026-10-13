//! `examples/k8s_deploy`, read from Rust instead of `aura eval --format yaml`.
//!
//! An operator that applies the Deployment itself reads it as typed values:
//! `Container` is written by `aura types --lang rust` from the schema in
//! `k8s_deploy.aura` (see `readers/k8s_types.rs`); the Deployment around it is
//! Kubernetes' own shape, described here.
//!
//! Run it with `cargo run --example read_k8s_deploy`.

use std::collections::BTreeMap;
use std::path::Path;

use aura_lang::Script;
use serde::Deserialize;

#[path = "readers/k8s_types.rs"]
mod types;
use types::Container;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Deployment {
    api_version: String,
    kind: String,
    metadata: Metadata,
    spec: Spec,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Spec {
    replicas: i64,
    template: Template,
}

#[derive(Deserialize)]
struct Template {
    spec: PodSpec,
}

#[derive(Deserialize)]
struct PodSpec {
    containers: Vec<Container>,
}

pub fn main() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/k8s_deploy/k8s_deploy.aura");
    // Hermetic: the manifest is given no rights, and the load proves it needs none.
    let d: Deployment = Script::load(path)
        .expect("k8s_deploy.aura loads")
        .output()
        .expect("the output is a Deployment");

    println!(
        "{} {} {}: {} replicas",
        d.api_version, d.kind, d.metadata.name, d.spec.replicas
    );
    for c in &d.spec.template.spec.containers {
        println!("  {} {} :{}", c.name, c.image, c.port);
    }
    for (k, v) in &d.metadata.labels {
        println!("  {k} = {v}");
    }

    assert_eq!(
        (d.api_version.as_str(), d.kind.as_str()),
        ("apps/v1", "Deployment")
    );
    assert_eq!(d.spec.replicas, 3);
    let c = &d.spec.template.spec.containers[0];
    assert_eq!((c.image.as_str(), c.port), ("company/gateway:2.4.1", 8080));
    assert_eq!(d.metadata.labels["app.kubernetes.io/version"], "2.4.1");
}
