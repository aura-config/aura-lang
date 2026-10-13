//! D37: a relative path in `read_file` is read from the folder of the file
//! that calls it, as an import is - not from wherever the process was started.
//!
//! Before, `aura eval i18n/i18n.aura --allow-read=.` failed from `examples/`
//! and worked from `examples/i18n/`: one manifest, two results, depending on
//! nothing in the manifest. A service embedding it through `Script` would have
//! looked for its files in the service's own working directory.
//!
//! What a manifest may read is unchanged: every path is still canonicalised
//! and checked against the granted roots, which stay relative to the current
//! directory like any command-line flag.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use aura_lang::facade::{eval_source, EvalOptions};
use aura_lang::Script;

fn scratch(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aura-d37-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d.canonicalize().expect("canonical")
}

fn put(dir: &Path, name: &str, text: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("dirs");
    std::fs::write(p, text).expect("write");
}

struct Run {
    out: serde_json::Value,
    err: String,
    code: i32,
}

fn aura(cwd: &Path, args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_aura"))
        .current_dir(cwd)
        .arg("eval")
        .args(args)
        .output()
        .expect("run aura");
    let raw = String::from_utf8_lossy(&o.stderr);
    let mut err = String::new();
    let mut esc = false;
    for c in raw.chars() {
        match (esc, c) {
            (false, '\u{1b}') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => err.push(c),
        }
    }
    Run {
        out: serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null),
        err,
        code: o.status.code().unwrap_or(-1),
    }
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples")
}

fn expected(dir: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(examples().join(dir).join("expected.json")).expect("expected");
    serde_json::from_str(&text).expect("json")
}

/// The case that showed the defect: the same manifest, run from its parent.
#[test]
fn a_manifest_reads_the_same_files_from_any_directory() {
    let from_parent = aura(&examples(), &["i18n/i18n.aura", "--allow-read=."]);
    assert_eq!(from_parent.code, 0, "{}", from_parent.err);
    assert_eq!(from_parent.out, expected("i18n"));

    let from_its_own = aura(&examples().join("i18n"), &["i18n.aura", "--allow-read=."]);
    assert_eq!(from_its_own.out, from_parent.out);
}

/// A host embedding the script reads the files beside it, whatever its own
/// working directory is.
#[test]
fn a_script_reads_beside_its_file_not_beside_the_host() {
    let dir = examples().join("i18n");
    let s = Script::builder()
        .file(dir.join("i18n.aura"))
        .allow_read(&dir)
        .load()
        .expect("loads from a test whose working directory is the crate");
    let want = expected("i18n");
    let (key, value) = want
        .as_object()
        .and_then(|o| o.iter().next())
        .expect("a property");
    assert_eq!(&s.get::<serde_json::Value>(key).expect("get"), value);
}

/// An imported module reads beside its own file - also through a function it
/// exports and the root calls.
#[test]
fn an_import_reads_beside_its_own_file() {
    let d = scratch("import");
    put(&d, "data.txt", "root");
    put(&d, "sub/data.txt", "sub");
    put(
        &d,
        "sub/lib.aura",
        "here: read_file(\"data.txt\")\npub def load()\n  v: read_file(\"./data.txt\")\nend\n",
    );
    put(
        &d,
        "main.aura",
        "import \"./sub/lib.aura\" as lib\nown: read_file(\"data.txt\")\nlib_value: lib.here\nlib_call: lib.load().v\n",
    );
    let r = aura(&d, &["main.aura", "--allow-read=.", "--allow-imports-io"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out["own"], "root");
    assert_eq!(r.out["lib_value"], "sub");
    assert_eq!(
        r.out["lib_call"], "sub",
        "a function reads beside the file that defined it"
    );
}

/// A package from the registry reads beside its own cached file, like any
/// other import - not beside the manifest that imported it.
#[test]
fn a_registry_package_reads_beside_its_cached_file() {
    let d = scratch("registry");
    put(
        &d,
        "reg/github/acme/pkg/1.0.0.aura",
        "here: read_file(\"data.txt\")\n",
    );
    put(&d, "reg/github/acme/pkg/data.txt", "package");
    put(&d, "app/data.txt", "app");
    put(
        &d,
        "app/main.aura",
        "import github/acme/pkg@v1.0.0 as pkg\nv: pkg.here\n",
    );
    let r = aura(
        &d.join("app"),
        &[
            "main.aura",
            "--registry-dir=../reg",
            "--allow-read=..",
            "--allow-imports-io",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out["v"], "package");
}

/// The grant does not grow to follow the manifest. Run from elsewhere with
/// `--allow-read=.`, a manifest's neighbours are outside the root: E0311, and
/// the help says why and what to grant - which, run, works.
#[test]
fn a_manifest_outside_the_grant_is_refused_and_told_what_to_grant() {
    let d = scratch("outside");
    put(&d, "app/config.aura", "v: read_file(\"value.txt\")\n");
    put(&d, "app/value.txt", "ok");
    std::fs::create_dir_all(d.join("elsewhere")).expect("dir");

    let r = aura(
        &d.join("elsewhere"),
        &["../app/config.aura", "--allow-read=."],
    );
    assert_ne!(r.code, 0);
    assert!(r.err.contains("E0311"), "{}", r.err);
    assert!(
        r.err.contains("folder of the manifest"),
        "the help must say where relative paths are read from: {}",
        r.err
    );
    assert!(r.err.contains("--allow-read=../app"), "{}", r.err);

    let fixed = aura(
        &d.join("elsewhere"),
        &["../app/config.aura", "--allow-read=../app"],
    );
    assert_eq!(
        fixed.code, 0,
        "the help's remedy did not work: {}",
        fixed.err
    );
    assert_eq!(fixed.out["v"], "ok");
}

/// An absolute path means what it says.
#[test]
fn an_absolute_path_is_unchanged() {
    let d = scratch("absolute");
    put(&d, "value.txt", "abs");
    let abs = d.join("value.txt").display().to_string().replace('\\', "/");
    put(&d, "m/config.aura", &format!("v: read_file(\"{abs}\")\n"));
    let r = aura(&d, &["m/config.aura", "--allow-read=."]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out["v"], "abs");
}

/// In memory - the playground, `eval_source`, `Script::source` - the same
/// rule holds over the buffer names: `./x` is `x`, and a buffer in `sub/`
/// reads `sub/x`.
#[test]
fn in_memory_sources_follow_the_same_rule() {
    let files: HashMap<String, String> = [
        (
            "main.aura",
            "import \"sub/lib.aura\" as lib\nown: read_file(\"./x.txt\")\nlib_value: lib.here\n",
        ),
        ("sub/lib.aura", "here: read_file(\"x.txt\")\n"),
        ("x.txt", "root"),
        ("sub/x.txt", "sub"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let opts = EvalOptions {
        allow_read: vec![".".into()],
        allow_imports_io: true,
        ..Default::default()
    };
    let out = eval_source(files, "main.aura", &opts)
        .expect("evaluates")
        .json;
    assert_eq!(out["own"], "root");
    assert_eq!(out["lib_value"], "sub");
}
