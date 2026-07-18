//! Критерий приёмки Фазы 3 (SPEC §8): эталонный манифест вычисляется целиком
//! (импорты подставляются вручную до появления VFS в Фазе 4).

use std::collections::HashMap;
use std::sync::Arc;

use aura_core::eval::value::Value;
use aura_core::eval::{EnvCap, Interpreter, MemFs, Options};
use aura_core::lexer::Lexer;
use aura_core::parser::Parser;
use indexmap::IndexMap;

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

fn get<'a>(v: &Value<'a>, key: &str) -> Value<'a> {
    let Value::Object(m) = v else { panic!("not an object") };
    m.get(key).unwrap_or_else(|| panic!("no key '{key}'")).clone()
}

fn eval_manifest() -> Value<'static> {
    let toks = Lexer::new(MANIFEST, 0).tokenize().expect("lex ok");
    let module = Parser::new(toks).parse_module().expect("parse ok");
    // Box::leak: в тесте модуль должен пережить возврат значения ('static)
    let module = Box::leak(Box::new(module));

    let mut it = Interpreter::new(Options { strict: true, dry_run: false });
    it.fs = Box::new(MemFs(HashMap::from([(
        "./Cargo.toml".to_string(),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\n".to_string(),
    )])));
    it.env_cap = EnvCap::Allow(vec!["APP_ENV".to_string()]);
    it.env_overrides.insert("APP_ENV".to_string(), "production".to_string());

    // Импорты: rust — пустой модуль, defaults — объект с global_labels
    it.provide_module("rust", Value::object(IndexMap::new()));
    let mut labels = IndexMap::new();
    labels.insert("team".to_string(), Value::Str(Arc::from("core")));
    let mut defaults = IndexMap::new();
    defaults.insert("global_labels".to_string(), Value::object(labels));
    it.provide_module("defaults", Value::object(defaults));

    it.eval_module(module).unwrap_or_else(|d| panic!("eval failed: {d:#?}"))
}

#[test]
fn manifest_evaluates_end_to_end() {
    let root = eval_manifest();

    // D10: `=` приватно — в экспорте только свойства и блоки
    let Value::Object(m) = &root else { panic!() };
    assert_eq!(m.keys().collect::<Vec<_>>(), vec!["production-eu"]);

    let domain = get(&root, "production-eu");
    // shadow-затенение: log_path читает затенённое значение
    assert_eq!(get(&domain, "log_path"), Value::str("/var/log/aura.log"));
    // is_prod == true → replicas 3
    assert_eq!(get(&domain, "replicas"), Value::Int(3));

    // meta = new ServiceMeta: transform_name("auth") → "AUTH", port = 8000 + 1
    let meta = get(&domain, "meta");
    assert_eq!(get(&meta, "name"), Value::str("AUTH"));
    assert_eq!(get(&meta, "port"), Value::Int(8001));

    // Вложенные объектные блоки
    let security = get(&domain, "security");
    assert_eq!(get(&security, "tls_enabled"), Value::Bool(true));
    assert_eq!(get(&get(&security, "certificates"), "cert_path"), Value::str("/etc/ssl/certs/server.crt"));

    // apps: map + component + интерполяция + merge импортированных labels
    let Value::List(apps) = get(&domain, "apps") else { panic!() };
    assert_eq!(apps.len(), 3);
    assert_eq!(get(&apps[0], "name"), Value::str("auth"));
    assert_eq!(get(&apps[0], "image"), Value::str("company/auth:1.2.3"));
    let labels = get(&apps[0], "labels");
    assert_eq!(get(&labels, "tier"), Value::str("backend"));
    assert_eq!(get(&labels, "managed_by"), Value::str("aura-engine"));
    assert_eq!(get(&labels, "team"), Value::str("core")); // из defaults.global_labels
    assert_eq!(get(&apps[2], "image"), Value::str("company/frontend:1.2.3"));
}
