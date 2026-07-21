//! Conformance runner: exercises examples/*/ through the real `aura` binary
//! and diffs stdout against the pinned expected.*.
//!
//! This is a language-agnostic corpus: any future Aura implementation must
//! produce the same outputs on the same inputs. Failure cases are checked by error code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap()
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn aura(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aura"));
    cmd.current_dir(dir).args(args);
    // Determinism: the environment is set only explicitly
    cmd.env_remove("APP_ENV");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to spawn aura binary");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        stderr: String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
        code: out.status.code().unwrap_or(-1),
    }
}

fn expected(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file))
        .unwrap_or_else(|e| panic!("cannot read {file} in {}: {e}", dir.display()))
        .replace("\r\n", "\n")
}

/// Success case: stdout is byte-identical (with normalized line endings) to expected.
fn check_ok(
    subdir: &str,
    manifest: &str,
    args: &[&str],
    env: &[(&str, &str)],
    expected_file: &str,
) {
    let dir = examples_dir().join(subdir);
    let mut full_args = vec!["eval", manifest];
    full_args.extend_from_slice(args);
    let run = aura(&dir, &full_args, env);
    assert_eq!(
        run.code, 0,
        "{subdir}: expected success, stderr:\n{}",
        run.stderr
    );
    let want = expected(&dir, expected_file);
    assert_eq!(
        run.stdout.trim_end(),
        want.trim_end(),
        "{subdir}: output mismatch"
    );
}

#[test]
fn environments() {
    check_ok(
        "environments",
        "environments.aura",
        &[],
        &[],
        "expected.json",
    );
}

#[test]
fn k8s_deploy_yaml() {
    check_ok(
        "k8s_deploy",
        "k8s_deploy.aura",
        &["--format", "yaml"],
        &[],
        "expected.yaml",
    );
}

#[test]
fn ci_matrix() {
    check_ok("ci_matrix", "ci_matrix.aura", &[], &[], "expected.json");
}

#[test]
fn feature_flags_dev() {
    check_ok(
        "feature_flags",
        "feature_flags.aura",
        &["--allow-env=APP_ENV"],
        &[],
        "expected.json",
    );
}

#[test]
fn feature_flags_prod_assert_fires() {
    let dir = examples_dir().join("feature_flags");
    let run = aura(
        &dir,
        &["eval", "feature_flags.aura", "--allow-env=APP_ENV"],
        &[("APP_ENV", "production")],
    );
    assert_eq!(run.code, 1, "assert must fail in production");
    assert!(
        run.stderr.contains("E0530"),
        "expected E0530, got:\n{}",
        run.stderr
    );
}

#[test]
fn telegram_bot_dev_mode() {
    check_ok(
        "telegram_bot",
        "bot.aura",
        &["--allow-env=BOT_ENV"],
        &[],
        "expected.json",
    );
}

#[test]
fn telegram_bot_prod_switches_mode() {
    let dir = examples_dir().join("telegram_bot");
    let run = aura(
        &dir,
        &["eval", "bot.aura", "--allow-env=BOT_ENV"],
        &[("BOT_ENV", "production")],
    );
    assert_eq!(run.code, 0, "stderr:\n{}", run.stderr);
    assert!(run.stdout.contains("\"mode\": \"webhook\""));
    assert!(run.stdout.contains("\"messages_per_minute\": 20"));
}

#[test]
fn i18n_fallback_merge() {
    check_ok(
        "i18n",
        "i18n.aura",
        &["--allow-read=."],
        &[],
        "expected.json",
    );
}

#[test]
fn validators_package_d12() {
    check_ok("validators", "deploy.aura", &[], &[], "expected.json");
    // the package validator stops an impossible port
    let dir = examples_dir().join("validators");
    let run = aura(&dir, &["eval", "bad.aura"], &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains("E0531") && run.stderr.contains("99999"),
        "package validator must fire: {}",
        run.stderr
    );
}

#[test]
fn service_catalog() {
    check_ok(
        "service_catalog",
        "service_catalog.aura",
        &["--allow-read=."],
        &[],
        "expected.json",
    );
}

#[test]
fn security_demo_denies_import_io() {
    let dir = examples_dir().join("security_demo");
    let run = aura(&dir, &["eval", "main.aura", "--allow-read=."], &[]);
    assert_eq!(run.code, 1, "import I/O must be denied");
    assert!(
        run.stderr.contains("E0310"),
        "expected E0310, got:\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("evil_dependency.aura"),
        "error must point into the module"
    );
    // --allow-imports-io lifts exactly the "import without capability" denial;
    // the next barrier then fires, and it is platform-dependent: on Linux
    // /etc/passwd exists but lies outside --allow-read=. (E0310 about the path),
    // on Windows the file does not exist (E0313 I/O). The one invariant checked
    // here: the "imported module has no capability" error is gone.
    let run2 = aura(
        &dir,
        &["eval", "main.aura", "--allow-read=.", "--allow-imports-io"],
        &[],
    );
    assert!(
        !run2.stderr.contains("imported module has no capability"),
        "with --allow-imports-io the import-capability error must go away, got:\n{}",
        run2.stderr
    );
}

/// Generating a non-JSON format: a `text … end` block string builds an
/// nginx.conf as a String value.
#[test]
fn nginx_generates_conf() {
    check_ok("nginx", "nginx.aura", &[], &[], "expected.json");
}

/// Full-language showcase: every core construct in one manifest end-to-end.
#[test]
fn showcase_all_constructs() {
    check_ok(
        "showcase",
        "showcase.aura",
        &["--allow-read=.", "--allow-env=APP_ENV"],
        &[],
        "expected.json",
    );
}

#[test]
fn reference_manifest() {
    check_ok(
        "",
        "production_deploy.aura",
        &[
            "--allow-read=.",
            "--allow-env=APP_ENV",
            "--registry-dir=registry",
        ],
        &[("APP_ENV", "production")],
        "expected_production_deploy.json",
    );
}

#[test]
fn check_command_strict_blocks_dead_code() {
    // The reference manifest contains intentional dead code → --strict blocks it
    let dir = examples_dir();
    let run = aura(&dir, &["check", "production_deploy.aura", "--strict"], &[]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("W0501") && run.stderr.contains("unused_config_version"));
    // without --strict it passes
    let run2 = aura(&dir, &["check", "production_deploy.aura"], &[]);
    assert_eq!(run2.code, 0);
}

/// Strips ANSI escape sequences (ariadne colors) for stable comparison.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for esc in chars.by_ref() {
                if esc.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// SPEC §6.3 (RecordingResolver): dry-run reports every read performed.
#[test]
fn dry_run_reports_reads() {
    let dir = examples_dir().join("service_catalog");
    let run = aura(
        &dir,
        &[
            "eval",
            "service_catalog.aura",
            "--allow-read=.",
            "--dry-run",
        ],
        &[],
    );
    assert_eq!(run.code, 0, "stderr:\n{}", run.stderr);
    // the module and both data files ended up in the report
    assert!(
        run.stderr.contains("[dry-run] read:"),
        "no read report:\n{}",
        run.stderr
    );
    assert!(run.stderr.contains("Cargo.toml"), "{}", run.stderr);
    assert!(run.stderr.contains("team.json"), "{}", run.stderr);
    // the result is identical to a normal run
    let normal = aura(
        &dir,
        &["eval", "service_catalog.aura", "--allow-read=."],
        &[],
    );
    assert_eq!(run.stdout, normal.stdout);
}

/// SPEC §8 (Phase 6): ariadne's visual reports are pinned by a golden snapshot.
#[test]
fn diagnostics_render_golden() {
    let dir = examples_dir();
    let run = aura(&dir, &["check", "production_deploy.aura", "--strict"], &[]);
    assert_eq!(run.code, 1);
    let got = strip_ansi(&run.stderr).replace("\r\n", "\n");
    let want = expected(&dir, "expected_check_strict_stderr.txt");
    assert_eq!(got.trim_end(), want.trim_end(), "ariadne report changed");
}

/// Full package cycle: `aura add --from` → offline `eval` through the cache and lock.
#[test]
fn add_then_eval_offline() {
    let work = std::env::temp_dir().join(format!("aura-add-test-{}", std::process::id()));
    let registry = work.join("registry");
    std::fs::create_dir_all(&work).unwrap();

    // Install validators.aura as the package pkg/validators@v1.0.0
    let pkg_src = examples_dir().join("validators/validators.aura");
    let run = aura(
        &work,
        &[
            "add",
            "pkg/validators@v1.0.0",
            "--from",
            pkg_src.to_str().unwrap(),
            "--registry-dir",
            registry.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(run.code, 0, "add failed: {}", run.stderr);
    assert!(registry.join("pkg/validators/1.0.0.aura").exists());
    let lock = std::fs::read_to_string(work.join("aura.lock")).unwrap();
    assert!(
        lock.contains("pkg/validators") && lock.contains("sha256-"),
        "{lock}"
    );

    // A range import @v1 resolves to the installed 1.0.0 without the network
    std::fs::write(
        work.join("main.aura"),
        "import pkg/validators@v1 as v\napi: v.service(\"api\", 8080)\n",
    )
    .unwrap();
    let run = aura(
        &work,
        &[
            "eval",
            "main.aura",
            "--registry-dir",
            registry.to_str().unwrap(),
            "--frozen",
        ],
        &[],
    );
    assert_eq!(run.code, 0, "offline eval failed: {}", run.stderr);
    assert!(run.stdout.contains("\"port\": 8080"));

    std::fs::remove_dir_all(&work).ok();
}

#[test]
fn dry_run_is_byte_identical_and_writes_nothing() {
    // SPEC §6.3 invariant: dry-run does not change the result; two runs are identical
    let dir = examples_dir().join("environments");
    let args = ["eval", "environments.aura", "--dry-run"];
    let a = aura(&dir, &args, &[]);
    let b = aura(&dir, &args, &[]);
    assert_eq!(a.code, 0);
    assert_eq!(a.stdout, b.stdout, "dry-run must be deterministic");
    let normal = aura(&dir, &["eval", "environments.aura"], &[]);
    assert_eq!(
        a.stdout, normal.stdout,
        "dry-run must not change the result"
    );
}
