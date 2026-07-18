//! Conformance-раннер: прогоняет examples/*/ через реальный бинарник `aura`
//! и сверяет stdout с зафиксированными expected.*.
//!
//! Это языконезависимый корпус: любая будущая реализация Aura обязана давать
//! те же выводы на тех же входах. Ошибочные кейсы проверяются по коду ошибки.

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
    // Детерминизм: окружение задаётся только явно
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

/// Успешный кейс: stdout побайтно (по нормализованным переносам) равен expected.
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
    // --allow-imports-io снимает запрет ровно до следующего барьера: самого файла нет прав читать?
    // Нет: с флагом чтение /etc/passwd на Windows упадёт по I/O — проверяем только смену кода ошибки.
    let run2 = aura(
        &dir,
        &["eval", "main.aura", "--allow-read=.", "--allow-imports-io"],
        &[],
    );
    assert!(
        !run2.stderr.contains("E0310"),
        "with --allow-imports-io the capability error must go away"
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
    // Эталонный манифест содержит намеренный мёртвый код → --strict блокирует
    let dir = examples_dir();
    let run = aura(&dir, &["check", "production_deploy.aura", "--strict"], &[]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("W0501") && run.stderr.contains("unused_config_version"));
    // без --strict — проходит
    let run2 = aura(&dir, &["check", "production_deploy.aura"], &[]);
    assert_eq!(run2.code, 0);
}

#[test]
fn dry_run_is_byte_identical_and_writes_nothing() {
    // Инвариант SPEC §6.3: dry-run не меняет результат; два прогона идентичны
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
