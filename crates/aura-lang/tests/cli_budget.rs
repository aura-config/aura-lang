//! D34: every evaluation has a budget — the CLI and the facade, not only
//! `Script`.
//!
//! D33 bounded calls into a script and left the CLI out. But `aura eval` runs
//! third-party packages too, and a 95-byte manifest — three nested
//! `range(1000).map` — passes `check --strict --hermetic` and then ran for over a
//! minute. In CI that is a build that hangs until its job times out, with
//! nothing to say which dependency did it.
//!
//! The default is finite and the same everywhere. It was chosen by measuring
//! every example in the repository: the heaviest spends 302 steps and 17 KB, so
//! ten million steps and 256 MiB is about thirty thousand times that.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const RUNAWAY: &str = concat!(
    "x: range(1000).map (a, i) -> range(1000).map (b, j) -> ",
    "range(1000).map (c, k) -> 1 end end end\n",
);

fn manifest(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write manifest");
    path
}

fn aura(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aura"))
        .args(args)
        .output()
        .expect("run aura")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The case that motivated the budget, stopped by the default and stopped soon.
#[test]
fn the_default_budget_stops_a_runaway_manifest() {
    let path = manifest("runaway.aura", RUNAWAY);
    let started = Instant::now();
    let out = aura(&["eval", path.to_str().unwrap()]);
    let took = started.elapsed();

    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("E0325") || err.contains("E0326"),
        "no budget error in: {err}"
    );
    assert!(
        took < Duration::from_secs(20),
        "the default budget did not stop the evaluation promptly: {took:?}"
    );
}

/// `--max-steps` narrows the budget, and the step counter alone stops it.
#[test]
fn max_steps_is_a_budget_in_steps() {
    let path = manifest(
        "steps.aura",
        "n: range(500).reduce(0) (acc, a) -> range(500).reduce(acc) (acc2, b) -> acc2 + 1 end end\n",
    );
    let out = aura(&["eval", path.to_str().unwrap(), "--max-steps", "1000"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("E0325"), "{}", stderr(&out));

    let ok = aura(&["eval", path.to_str().unwrap(), "--max-steps", "100000000"]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));
}

/// `--max-memory` is in mebibytes and counts every value the run creates.
#[test]
fn max_memory_is_a_budget_in_bytes() {
    let path = manifest(
        "memory.aura",
        "xs: range(200000).map (x, i) -> \"#{x}-#{x}-#{x}\" end\n",
    );
    let out = aura(&["eval", path.to_str().unwrap(), "--max-memory", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("E0326"), "{}", stderr(&out));
}

/// `--stats` reports what was spent, so a budget can be set from a measurement
/// rather than a guess — which is what someone who just hit E0325 needs.
#[test]
fn stats_reports_what_was_spent() {
    let path = manifest("stats.aura", "xs: [1, 2, 3].map (x, i) -> x * 2 end\n");
    let out = aura(&["eval", path.to_str().unwrap(), "--stats"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    let line = err
        .lines()
        .find(|l| l.starts_with("steps:"))
        .unwrap_or_else(|| panic!("no stats line in: {err}"));
    let steps: u64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .expect("a step count");
    assert!(steps > 0, "{line}");

    // And the output itself is untouched: stats go to stderr only.
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    assert_eq!(json["xs"], serde_json::json!([2, 4, 6]));
}

/// The facade gets the same budget, so the browser playground and every host
/// using `eval_file` / `eval_source` are covered without asking.
#[test]
fn the_facade_has_the_same_default() {
    use aura_lang::facade::{eval_source, EvalOptions};
    let files = std::collections::HashMap::from([("r.aura".to_string(), RUNAWAY.to_string())]);
    let err = eval_source(files, "r.aura", &EvalOptions::default()).unwrap_err();
    assert!(
        matches!(err[0].code, "E0325" | "E0326"),
        "expected a budget error, got {}",
        err[0].code
    );

    let files = std::collections::HashMap::from([(
        "ok.aura".to_string(),
        "x: [1, 2].map (v, i) -> v end\n".to_string(),
    )]);
    let ok = eval_source(files, "ok.aura", &EvalOptions::default()).expect("evaluates");
    assert!(ok.usage.steps > 0 && ok.usage.bytes > 0, "{:?}", ok.usage);
}
