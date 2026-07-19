//! The embedding facade: the success path and structured errors.

use std::path::{Path, PathBuf};

use aura_core::error::Severity;
use aura_core::facade::{eval_file, EvalOptions};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap()
}

#[test]
fn eval_file_returns_json() {
    let path = examples().join("environments/environments.aura");
    let out = eval_file(&path, &EvalOptions::default()).expect("eval ok");
    assert_eq!(
        out.json["environments"]["prod"]["replicas"],
        serde_json::json!(6)
    );
    assert!(
        out.warnings.is_empty(),
        "clean example must have no warnings: {:?}",
        out.warnings
    );
    assert!(out.updated_lockfile.is_none());
}

#[test]
fn errors_are_structured_reports() {
    let path = examples().join("security_demo/main.aura");
    let opts = EvalOptions {
        allow_read: vec![examples()],
        ..Default::default()
    };
    let reports = eval_file(&path, &opts).expect_err("import I/O must be denied");
    let first = &reports[0];
    assert_eq!(first.code, "E0310");
    assert_eq!(first.severity, Severity::Error);
    assert!(
        first.file.contains("evil_dependency.aura"),
        "file: {}",
        first.file
    );
    assert_eq!(first.line, 2);
    // The Display form is suitable for the host's logs
    let text = first.to_string();
    assert!(
        text.contains("error[E0310]") && text.contains(":2:"),
        "{text}"
    );
}
