//! Aura as a bridge between configuration formats.
//!
//! A manifest that reads someone else's YAML or TOML and emits JSON is one of
//! the jobs the language is actually used for, and until this file existed
//! nothing tested it: the only place the three formats met was one showcase,
//! carrying data too simple to break anything.
//!
//! Probing it with a deliberately nasty document found three defects at once,
//! and each has a test here named after what went wrong:
//!
//! 1. YAML's merge key was not applied. `<<: *defaults` survived into the
//!    result as a literal key named `<<`, with the referenced mapping nested
//!    underneath it, and the keys it was supposed to contribute never arrived.
//!    The output was a plausible-looking, wrong configuration — the worst shape
//!    a defect can take, because nothing downstream reports it.
//!
//! 2. TOML came back in alphabetical order. `serde_json` was already built with
//!    `preserve_order` and `yaml-rust2` preserves order by construction, so
//!    reading a TOML file was the one path that silently rearranged a document
//!    its author had put in a deliberate order.
//!
//! 3. A TOML datetime becomes a string, and still does — see the test at the
//!    bottom, which pins the loss rather than pretending it is not there.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aura_lang::facade::{eval_file, EvalOptions};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-bridge-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// `read_file` resolves against the process's working directory, not the
/// manifest's, so each test writes `read_file("in.yaml")` for legibility and
/// this rewrites it to the scratch copy. Forward slashes, because the path
/// lands inside an Aura string literal where a backslash is an escape.
fn absolute_inputs(manifest: &str, dir: &Path) -> String {
    let base = dir.display().to_string().replace('\\', "/");
    manifest.replace("read_file(\"", &format!("read_file(\"{base}/"))
}

/// Write inputs, rewrite the manifest's paths, and place it in `dir`.
fn lay_out(dir: &Path, manifest: &str, inputs: &[(&str, &str)]) -> PathBuf {
    for (name, body) in inputs {
        fs::write(dir.join(name), body.as_bytes()).expect("write input");
    }
    let path = dir.join("bridge.aura");
    fs::write(&path, absolute_inputs(manifest, dir).as_bytes()).expect("write manifest");
    path
}

/// Write `inputs` next to a manifest, evaluate it with read access to that
/// directory only, and return the rendered JSON.
fn bridge(tag: &str, manifest: &str, inputs: &[(&str, &str)]) -> serde_json::Value {
    let dir = scratch(tag);
    let path = lay_out(&dir, manifest, inputs);
    let opts = EvalOptions {
        allow_read: vec![dir.clone()],
        ..EvalOptions::default()
    };
    eval_file(&path, &opts)
        .unwrap_or_else(|e| panic!("{tag} failed to evaluate: {e:?}"))
        .json
}

/// The same, for a manifest expected to fail. Returns the diagnostic codes.
fn bridge_err(tag: &str, manifest: &str, inputs: &[(&str, &str)]) -> Vec<String> {
    let dir = scratch(tag);
    let path = lay_out(&dir, manifest, inputs);
    let opts = EvalOptions {
        allow_read: vec![dir.clone()],
        ..EvalOptions::default()
    };
    match eval_file(&path, &opts) {
        Ok(_) => panic!("{tag} was expected to fail and did not"),
        Err(reports) => reports.iter().map(|r| r.code.to_string()).collect(),
    }
}

fn s(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .unwrap_or_else(|| panic!("no key {key} in {v}"))
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string"))
        .to_string()
}

// ---------------------------------------------------------------------------
// 1. Merge keys
// ---------------------------------------------------------------------------

/// The defect itself: a merge key must contribute its mapping's entries, and
/// must not survive into the result under its own name.
#[test]
fn a_yaml_merge_key_is_applied() {
    let out = bridge(
        "merge",
        "y = read_file(\"in.yaml\").parse_yaml()\nservice: y.service\n",
        &[(
            "in.yaml",
            "defaults: &def\n  retries: 3\n  timeout: 30\nservice:\n  <<: *def\n  name: \"api\"\n",
        )],
    );

    let svc = out.get("service").expect("service");
    assert!(
        svc.get("<<").is_none(),
        "the merge key leaked into the result as a literal key: {svc}"
    );
    assert_eq!(svc.get("retries").and_then(|v| v.as_i64()), Some(3));
    assert_eq!(svc.get("timeout").and_then(|v| v.as_i64()), Some(30));
    assert_eq!(svc.get("name").and_then(|v| v.as_str()), Some("api"));
}

/// A key written out in the mapping beats the one the merge would supply.
/// Getting this backwards would silently replace a deliberate override with a
/// default — the same class of wrong-but-plausible output as the original bug.
#[test]
fn an_explicit_key_beats_a_merged_one() {
    let out = bridge(
        "merge-override",
        "y = read_file(\"in.yaml\").parse_yaml()\nservice: y.service\n",
        &[(
            "in.yaml",
            "defaults: &def\n  timeout: 30\nservice:\n  <<: *def\n  timeout: 5\n",
        )],
    );
    assert_eq!(
        out.get("service")
            .and_then(|s| s.get("timeout"))
            .and_then(|v| v.as_i64()),
        Some(5),
        "the file says timeout: 5 after the merge; the merge must not win"
    );
}

/// `<<: [a, b]` — the earlier source wins, per the merge specification. The
/// order is not arbitrary: it is how a YAML author layers a specific profile
/// over a general one.
#[test]
fn in_a_merge_list_the_earlier_source_wins() {
    let out = bridge(
        "merge-list",
        "y = read_file(\"in.yaml\").parse_yaml()\nm: y.m\n",
        &[(
            "in.yaml",
            "a: &a\n  k: \"from-a\"\n  only_a: 1\nb: &b\n  k: \"from-b\"\n  only_b: 2\nm:\n  <<: [*a, *b]\n",
        )],
    );
    let m = out.get("m").expect("m");
    assert_eq!(m.get("k").and_then(|v| v.as_str()), Some("from-a"));
    // Both still contribute what the other does not have.
    assert_eq!(m.get("only_a").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(m.get("only_b").and_then(|v| v.as_i64()), Some(2));
}

/// Merging something that is not a mapping has no defined meaning. Guessing one
/// would change a configuration silently, so it is an error.
#[test]
fn merging_a_non_mapping_is_an_error() {
    let codes = bridge_err(
        "merge-scalar",
        "y = read_file(\"in.yaml\").parse_yaml()\nm: y.m\n",
        &[("in.yaml", "m:\n  <<: 7\n")],
    );
    assert!(
        codes.iter().any(|c| c == "E0314"),
        "expected E0314, got {codes:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Key order
// ---------------------------------------------------------------------------

/// Every reader must hand the document back in the order it was written. This
/// held for JSON and YAML and did not hold for TOML, which arrived sorted.
#[test]
fn every_format_preserves_the_order_it_was_written_in() {
    let out = bridge(
        "order",
        concat!(
            "t = read_file(\"in.toml\").parse_toml()\n",
            "y = read_file(\"in.yaml\").parse_yaml()\n",
            "j = read_file(\"in.json\").parse_json()\n",
            "from_toml: t.to_json()\n",
            "from_yaml: y.to_json()\n",
            "from_json: j.to_json()\n",
        ),
        &[
            ("in.toml", "zulu = 1\nalpha = 2\nmike = 3\n"),
            ("in.yaml", "zulu: 1\nalpha: 2\nmike: 3\n"),
            ("in.json", "{\"zulu\":1,\"alpha\":2,\"mike\":3}"),
        ],
    );

    let expected = "{\"zulu\":1,\"alpha\":2,\"mike\":3}";
    for key in ["from_toml", "from_yaml", "from_json"] {
        assert_eq!(
            s(&out, key),
            expected,
            "{key} rearranged the document. A reader that sorts keys makes the \
             output unreadable next to the input it came from."
        );
    }
}

// ---------------------------------------------------------------------------
// 3. The nasty document, end to end
// ---------------------------------------------------------------------------

/// One YAML document carrying everything that has historically broken a bridge,
/// converted to JSON and checked value by value.
///
/// Each entry is here because it is a known trap, not for volume: `no` and
/// `off` are the Norway problem, `"007"` is a quoted number that must not
/// become 8, `9007199254740993` is past the point where a double can hold an
/// integer, `1.20` must not become the string "1.2", block and folded scalars
/// have different trailing-newline rules, and keys containing spaces and
/// colons have to survive being written back out.
#[test]
fn a_hostile_yaml_document_converts_without_loss() {
    let yaml = concat!(
        "country: no\n",
        "switch: off\n",
        "really: true\n",
        "version: 1.20\n",
        "build: \"007\"\n",
        "big: 9007199254740993\n",
        "unicode: \"привет — ok\"\n",
        "block: |\n  line one\n  line two\n",
        "folded: >\n  folded\n  text\n",
        "nested:\n  - [1, 2]\n  - a: 1\n    b: [x, {y: z}]\n",
        "key with spaces: 1\n",
        "\"key:with:colons\": 2\n",
    );

    let out = bridge(
        "hostile",
        "y = read_file(\"in.yaml\").parse_yaml()\nj: y.to_json()\n",
        &[("in.yaml", yaml)],
    );

    let round: serde_json::Value = serde_json::from_str(&s(&out, "j")).expect("valid JSON emitted");
    let expect: BTreeMap<&str, serde_json::Value> = BTreeMap::from([
        // YAML 1.2: only true/false are booleans. A country code stays a string.
        ("country", serde_json::json!("no")),
        ("switch", serde_json::json!("off")),
        ("really", serde_json::json!(true)),
        ("version", serde_json::json!(1.2)),
        ("build", serde_json::json!("007")),
        ("big", serde_json::json!(9007199254740993i64)),
        ("unicode", serde_json::json!("привет — ok")),
        // A literal block keeps its newlines, including the final one.
        ("block", serde_json::json!("line one\nline two\n")),
        // A folded block joins its lines with spaces.
        ("folded", serde_json::json!("folded text\n")),
        (
            "nested",
            serde_json::json!([[1, 2], {"a": 1, "b": ["x", {"y": "z"}]}]),
        ),
        ("key with spaces", serde_json::json!(1)),
        ("key:with:colons", serde_json::json!(2)),
    ]);

    for (key, want) in expect {
        assert_eq!(
            round.get(key),
            Some(&want),
            "{key} did not survive the conversion"
        );
    }
}

/// YAML → TOML → back, for the subset TOML can hold. What TOML cannot hold is
/// the subject of the two tests below.
#[test]
fn yaml_reaches_toml_and_comes_back_unchanged() {
    let out = bridge(
        "roundtrip",
        concat!(
            "y = read_file(\"in.yaml\").parse_yaml()\n",
            "once: y.to_json()\n",
            "twice: y.to_toml().parse_toml().to_json()\n",
        ),
        &[(
            "in.yaml",
            "name: \"api\"\nport: 8080\nratio: 0.5\non: true\ntags: [\"a\", \"b\"]\nnested:\n  k: 1\n",
        )],
    );
    assert_eq!(
        s(&out, "once"),
        s(&out, "twice"),
        "a trip through TOML changed the document"
    );
}

// ---------------------------------------------------------------------------
// 4. What the bridge deliberately refuses or loses
// ---------------------------------------------------------------------------

/// TOML has no null, so emitting one has to fail rather than drop the key.
/// A dropped key is a configuration that is quietly missing a setting.
#[test]
fn a_null_cannot_be_written_to_toml() {
    let codes = bridge_err(
        "toml-null",
        "y = read_file(\"in.yaml\").parse_yaml()\nt: y.to_toml()\n",
        &[("in.yaml", "name: \"api\"\nmissing:\n")],
    );
    assert!(
        codes.iter().any(|c| c == "E0603"),
        "expected E0603, got {codes:?}"
    );
}

/// TOML's native datetime has no counterpart in Aura's value model, so it
/// arrives as a string and is written back out quoted. That is a real loss of
/// type, and this test exists to state it rather than to bless it: if Aura ever
/// grows a date, this test is where the decision surfaces.
#[test]
fn a_toml_datetime_degrades_to_a_string() {
    let out = bridge(
        "toml-datetime",
        "t = read_file(\"in.toml\").parse_toml()\nback: t.to_toml()\nj: t.to_json()\n",
        &[("in.toml", "when = 2026-09-16T10:00:00Z\n")],
    );

    assert_eq!(s(&out, "j"), "{\"when\":\"2026-09-16T10:00:00Z\"}");
    assert_eq!(
        s(&out, "back"),
        "when = \"2026-09-16T10:00:00Z\"\n",
        "the datetime comes back quoted: TOML in, string out"
    );
}

/// The same digits must mean the same number whichever format carried them.
///
/// They did not. JSON numbers were parsed by `serde_json`, whose float parser
/// disagrees with Rust's by one unit in the last place on some inputs, while
/// YAML numbers went through Rust's. `3.3333333333333333e+65` therefore read as
/// two different values depending on the file it came from — a manifest's
/// result depending on its input's format is the exact failure the language
/// exists to prevent. Found by `fuzz_bridge`, not by review.
#[test]
fn a_number_means_the_same_thing_in_every_format() {
    let digits = "3.3333333333333333e+65";
    let out = bridge(
        "ulp",
        concat!(
            "j = read_file(\"in.json\").parse_json()\n",
            "y = read_file(\"in.yaml\").parse_yaml()\n",
            "agree: j.x == y.x\n",
            "text: j.to_json()\n",
        ),
        &[
            ("in.json", &format!("{{\"x\": {digits}}}")),
            ("in.yaml", &format!("x: {digits}")),
        ],
    );

    assert_eq!(
        out.get("agree").and_then(|v| v.as_bool()),
        Some(true),
        "the same digits read as two different numbers"
    );
    // And the value survives being written back out and read again.
    assert_eq!(s(&out, "text"), format!("{{\"x\":{digits}}}"));
}
