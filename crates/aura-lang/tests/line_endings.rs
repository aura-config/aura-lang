//! A manifest must evaluate to the same bytes whatever line endings it has.
//!
//! This is the promise the language is bought for: the same configuration on a
//! developer's Windows laptop and on a Linux runner. It was not kept. A block
//! string (`text … end`) in a CRLF checkout did not merely produce different
//! output — it failed to lex at all:
//!
//! ```text
//! [E0105] Error: unexpected character
//!  2 │   listen 80;
//!    │            ┬── not a valid Aura token
//! ```
//!
//! The opener check looked for a newline after `text` and skipped spaces and
//! tabs on the way, but not a carriage return, so `text` was never recognised as
//! an opener and the block's contents were parsed as code.
//!
//! The repository could not see this. Its own `.gitattributes` pins `eol=lf`, so
//! every file CI ever reads is LF no matter which of the three platforms it runs
//! on. That protection is right for this repository — the conformance fixtures
//! depend on it — but it made the defect invisible to the entire test suite.
//! This test therefore writes both variants itself, at run time, which is the
//! only way to reach the case a user hits on a clone without that file.

use std::fs;
use std::path::{Path, PathBuf};

use aura_lang::facade::{eval_file, EvalOptions};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples directory")
}

/// Self-contained manifests: no imports and no capabilities, so each can be
/// copied to a scratch directory alone. `nginx` is the one that matters — it is
/// the example built around block strings, the construct that was broken.
const SELF_CONTAINED: &[&str] = &[
    "nginx/nginx.aura",
    "ci_matrix/ci_matrix.aura",
    "k8s_deploy/k8s_deploy.aura",
];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-eol-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// Evaluate `source` written out with the given line ending, and return the JSON
/// rendered exactly as the CLI would render it.
fn eval_with_endings(name: &str, source: &str, crlf: bool) -> String {
    let lf = source.replace("\r\n", "\n");
    let body = if crlf { lf.replace('\n', "\r\n") } else { lf };
    let dir = scratch(if crlf { "crlf" } else { "lf" });
    let path = dir.join(name);
    fs::write(&path, body.as_bytes()).expect("write manifest");

    let out = eval_file(&path, &EvalOptions::default())
        .unwrap_or_else(|e| panic!("{name} failed to evaluate ({}): {e:?}", ending(crlf)));
    serde_json::to_string_pretty(&out.json).expect("render json")
}

fn ending(crlf: bool) -> &'static str {
    if crlf {
        "CRLF"
    } else {
        "LF"
    }
}

#[test]
fn every_example_evaluates_identically_under_crlf() {
    for rel in SELF_CONTAINED {
        let name = rel.rsplit('/').next().expect("file name");
        let source =
            fs::read_to_string(examples().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));

        let lf = eval_with_endings(name, &source, false);
        let crlf = eval_with_endings(name, &source, true);

        assert_eq!(
            lf, crlf,
            "{rel} produced different output under CRLF. A manifest's meaning \
             cannot depend on how git checked it out."
        );
    }
}

/// The specific construct that was broken, pinned on its own so a regression
/// names the cause rather than pointing at a whole example.
#[test]
fn a_block_string_survives_crlf_intact() {
    let source = concat!(
        "name = \"web\"\n",
        "cfg: text\n",
        "  server {\n",
        "    server_name #{name}.example.com;\n",
        "    listen 80;\n",
        "  }\n",
        "end\n",
    );

    let lf = eval_with_endings("block.aura", source, false);
    let crlf = eval_with_endings("block.aura", source, true);
    assert_eq!(lf, crlf);

    // And the content itself is LF-joined regardless of the source's endings:
    // a carriage return reaching the output would travel on into the generated
    // nginx.conf or Dockerfile, which is the whole point of block strings.
    assert!(
        !lf.contains("\\r"),
        "a carriage return leaked into the string value: {lf}"
    );
    assert!(
        lf.contains("server_name web.example.com;"),
        "interpolation must still work: {lf}"
    );
}
