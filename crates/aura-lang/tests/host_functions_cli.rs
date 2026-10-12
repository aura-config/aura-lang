//! D35 §7: a script that imports host functions, run where there is no host.
//!
//! The CLI provides no host functions, so `aura eval` cannot run such a script.
//! What matters is how it says so. A plain "module not found" would send the
//! reader looking for a missing file; the error has to say that the module is
//! provided by the application at run time, and `aura check` has to list what
//! the script will need from whichever host runs it.

use std::path::PathBuf;
use std::process::{Command, Output};

fn manifest(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-host-{}", std::process::id()));
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

fn plain(o: &Output) -> String {
    // Diagnostics are coloured; strip the escapes before matching text.
    let raw = String::from_utf8_lossy(&o.stderr).into_owned();
    let mut out = String::new();
    let mut esc = false;
    for c in raw.chars() {
        match (esc, c) {
            (false, '\u{1b}') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => out.push(c),
        }
    }
    out
}

const ROUTING: &str = concat!(
    "import \"host:geo\" as geo\n",
    "import \"host:flags\" as flags\n",
    "pub def route(ip, customer)\n",
    "  backend: geo.region(ip)\n",
    "  canary:  flags.enabled(\"canary\", customer)\n",
    "end\n",
);

/// `eval` fails with E0007, and the explanation names the application - not a
/// missing file.
#[test]
fn eval_explains_that_the_host_provides_it() {
    let path = manifest("routing.aura", ROUTING);
    let out = aura(&["eval", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let err = plain(&out);
    assert!(err.contains("E0007"), "{err}");
    assert!(
        err.contains("provided by the application"),
        "the error must say where host functions come from: {err}"
    );
}

/// `check` lists every host function the script calls, so whoever embeds it
/// knows what to provide.
#[test]
fn check_lists_what_the_script_requires_from_its_host() {
    let path = manifest("routing.aura", ROUTING);
    let out = aura(&["check", path.to_str().unwrap()]);
    let err = plain(&out);
    for f in ["geo.region", "flags.enabled"] {
        assert!(err.contains(f), "`check` did not list {f}: {err}");
    }
}
