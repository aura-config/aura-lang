//! Reading a whole manifest from Rust: `Script::output` and `env_values`.
//!
//! Written before the implementation. The promise is the one the user named:
//! no divergence in behaviour. A manifest read through `Script` gives exactly
//! what `aura eval` prints - checked against the same `expected.*` files the
//! CLI conformance tests use, so there is one set of answers for both.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aura_lang::facade::{eval_source, EvalOptions};
use aura_lang::Script;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples")
}

// ===========================================================================
// The API
// ===========================================================================

/// `output` is the manifest as `aura eval` renders it: properties and blocks,
/// not private bindings, functions, schemas or enums.
#[test]
fn output_is_what_aura_eval_prints() {
    let src = concat!(
        "enum Tier\n  \"web\"\n  \"db\"\nend\n",
        "type Service\n  name: String\n  tier: Tier\nend\n",
        "hidden = 1\n",
        "pub def twice(x)\n  v: x * 2\nend\n",
        "api: new Service\n  name: \"checkout\"\n  tier: \"web\"\nend\n",
        "port: twice(4040).v\n",
    );
    let s = Script::builder()
        .source("m.aura", src)
        .load()
        .expect("loads");
    let from_script: serde_json::Value = s.output().expect("output");

    let files = HashMap::from([("m.aura".to_string(), src.to_string())]);
    let from_cli = eval_source(files, "m.aura", &EvalOptions::default())
        .expect("evaluates")
        .json;
    assert_eq!(from_script, from_cli);
    assert!(from_script.get("hidden").is_none());
    assert!(from_script.get("twice").is_none());
}

/// Into a typed struct, as a host would read its configuration.
#[test]
fn output_reads_into_a_struct() {
    #[derive(serde::Deserialize)]
    struct Config {
        port: i64,
        hosts: Vec<String>,
    }
    let s = Script::builder()
        .source("m.aura", "port: 8080\nhosts: [\"a\", \"b\"]\n")
        .load()
        .expect("loads");
    let c: Config = s.output().expect("output");
    assert_eq!((c.port, c.hosts.len()), (8080, 2));
}

/// `env_values` grants exactly the names it is given, with exactly the values
/// it is given - the process environment is not consulted, so a host's own
/// variables cannot leak into a script that was handed others.
#[test]
fn env_values_grant_those_names_with_those_values() {
    let s = Script::builder()
        .source("m.aura", "path: env(\"PATH\", \"-\")\n")
        .env_values([("PATH", "/given")])
        .load()
        .expect("loads");
    let out: serde_json::Value = s.output().expect("output");
    assert_eq!(out["path"], "/given", "the process's PATH was read instead");

    let err = Script::builder()
        .source("m.aura", "home: env(\"HOME\", \"-\")\n")
        .env_values([("PATH", "/given")])
        .load()
        .unwrap_err();
    assert_eq!(err.code(), "E0310", "{err}");
}

/// The values reach a function the host calls, not only the module as it
/// loads: a call runs in its own interpreter, and it must be handed them too.
#[test]
fn env_values_reach_a_called_function() {
    #[derive(serde::Deserialize)]
    struct Mode {
        v: String,
    }
    let s = Script::builder()
        .source(
            "m.aura",
            "pub def mode()\n  v: env(\"MODE\", \"dev\")\nend\n",
        )
        .env_values([("MODE", "prod")])
        .load()
        .expect("loads");
    let m: Mode = s.call("mode", ()).expect("call");
    assert_eq!(m.v, "prod");
}

/// A grant is a grant: a script given values is not hermetic, as with
/// `allow_env`.
#[test]
fn env_values_are_a_grant() {
    Script::builder()
        .source("m.aura", "mode: env(\"MODE\", \"dev\")\n")
        .env_values([("MODE", "prod")])
        .load()
        .expect("loads: the grant lifts the hermetic proof");
}

// ===========================================================================
// Every example, read from Rust
// ===========================================================================

enum Format {
    Json,
    Yaml,
    Toml,
}

struct Case {
    dir: &'static str,
    entry: &'static str,
    read: bool,
    strict: bool,
    /// What the CLI test leaves unset is given here as the value the manifest
    /// defaults to, so the two runs take the same branch.
    env: &'static [(&'static str, &'static str)],
    expected: &'static str,
    format: Format,
}

const CASES: &[Case] = &[
    Case {
        dir: "environments",
        entry: "environments.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "k8s_deploy",
        entry: "k8s_deploy.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.yaml",
        format: Format::Yaml,
    },
    Case {
        dir: "k8s_deploy",
        entry: "k8s_deploy.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.toml",
        format: Format::Toml,
    },
    Case {
        dir: "ci_matrix",
        entry: "ci_matrix.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "feature_flags",
        entry: "feature_flags.aura",
        read: false,
        strict: false,
        env: &[("APP_ENV", "dev")],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "telegram_bot",
        entry: "bot.aura",
        read: false,
        strict: false,
        env: &[("BOT_ENV", "dev")],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "i18n",
        entry: "i18n.aura",
        read: true,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "validators",
        entry: "deploy.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "service_catalog",
        entry: "service_catalog.aura",
        read: true,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "nginx",
        entry: "nginx.aura",
        read: false,
        strict: false,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "device",
        entry: "device.aura",
        read: true,
        strict: true,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "pipeline",
        entry: "pipeline.aura",
        read: false,
        strict: true,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "trading",
        entry: "portfolio.aura",
        read: false,
        strict: true,
        env: &[],
        expected: "expected.json",
        format: Format::Json,
    },
    Case {
        dir: "showcase",
        entry: "showcase.aura",
        read: true,
        strict: false,
        env: &[("APP_ENV", "production")],
        expected: "expected.json",
        format: Format::Json,
    },
];

/// Each example, loaded through `Script` with the grants its CLI test passes,
/// gives the output pinned in its `expected.*` - the file `conformance.rs`
/// checks the binary against. One set of answers, two ways of reading.
#[test]
fn every_example_reads_the_same_from_rust_as_from_the_cli() {
    let mut failures = Vec::new();
    for c in CASES {
        let dir = examples().join(c.dir);
        let mut b = Script::builder().file(dir.join(c.entry));
        if c.read {
            b = b.allow_read(&dir);
        }
        if c.strict {
            b = b.strict();
        }
        if !c.env.is_empty() {
            b = b.env_values(c.env.iter().copied());
        }
        let s = match b.load() {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}/{}: {e}", c.dir, c.entry));
                continue;
            }
        };
        let out: serde_json::Value = s.output().expect("output");
        let want = std::fs::read_to_string(dir.join(c.expected))
            .expect("expected")
            .replace("\r\n", "\n");
        let same = match c.format {
            Format::Json => serde_json::from_str::<serde_json::Value>(&want).expect("json") == out,
            Format::Yaml => {
                aura_lang::serialize::json_to_yaml_string(&out).trim_end() == want.trim_end()
            }
            Format::Toml => aura_lang::serialize::json_to_toml_string(&out)
                .map(|t| t.trim_end() == want.trim_end())
                .unwrap_or(false),
        };
        if !same {
            failures.push(format!(
                "{}/{}: differs from {}",
                c.dir, c.expected, c.expected
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The table above covers every example the CLI conformance test checks
/// against an `expected.*`, so a new example cannot be left out of it.
#[test]
fn every_example_with_an_expected_output_is_in_the_table() {
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(examples()).expect("examples") {
        let path = entry.expect("entry").path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .to_string();
        let has_expected = std::fs::read_dir(&path)
            .expect("dir")
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("expected."));
        if has_expected && !CASES.iter().any(|c| c.dir == name) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "examples not read from Rust: {missing:?}"
    );
}
