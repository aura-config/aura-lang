//! Критерий приёмки Фазы 6 (SPEC §8): манифест → JSON без потери типов;
//! Int сериализуется без `.0`; функции в дереве — E0601 с путём.

use std::collections::HashMap;

use aura_core::eval::value::Value;
use aura_core::eval::{EnvCap, Interpreter, MemFs, Options};
use aura_core::serialize::{to_json, to_json_flat};
use aura_core::source::SourceCache;
use aura_core::vfs::loader::Loader;
use aura_core::vfs::{ImportSpec, MemoryResolver};

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

fn eval_manifest<'a>(cache: &'a SourceCache, resolver: &'a MemoryResolver) -> Value<'a> {
    let mut loader = Loader::new(cache, resolver);
    let mut it = Interpreter::new(Options::default());
    it.fs = Box::new(MemFs(HashMap::from([(
        "./Cargo.toml".to_string(),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\n".to_string(),
    )])));
    it.env_cap = EnvCap::Allow(vec!["APP_ENV".to_string()]);
    it.env_overrides.insert("APP_ENV".to_string(), "production".to_string());
    loader.eval_entry(&mut it, &ImportSpec::File("production_deploy.aura")).unwrap()
}

fn resolver() -> MemoryResolver {
    MemoryResolver {
        files: HashMap::from([
            ("production_deploy.aura".to_string(), MANIFEST.to_string()),
            ("templates/k8s_defaults.aura".to_string(), "global_labels:\n  team: \"core\"\nend".to_string()),
            ("github/actions/rust-cache@v1.2".to_string(), "cache_key = \"rust-v1\"".to_string()),
        ]),
    }
}

#[test]
fn manifest_to_json_golden() {
    let cache = SourceCache::new();
    let r = resolver();
    let value = eval_manifest(&cache, &r);
    let json = to_json(&value).unwrap();

    let domain = &json["production-eu"];
    // Int → JSON integer, не 9090.0 (D6)
    assert_eq!(domain["metrics"]["port"], serde_json::json!(9090));
    assert_eq!(domain["meta"], serde_json::json!({ "name": "AUTH", "port": 8001 }));
    assert_eq!(domain["apps"][0]["image"], serde_json::json!("company/auth:1.2.3"));
    assert_eq!(domain["apps"][0]["labels"]["team"], serde_json::json!("core"));
    assert_eq!(domain["security"]["certificates"]["key_path"], serde_json::json!("/etc/ssl/certs/server.key"));
    // Функции/схемы исключены из экспорта
    assert!(json.get("transform_name").is_none());
    assert!(json.get("ServiceMeta").is_none());
    // Порядок ключей = порядок объявления
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys[0], "global_file_path");
}

#[test]
fn flat_format() {
    let cache = SourceCache::new();
    let r = resolver();
    let value = eval_manifest(&cache, &r);
    let flat = to_json_flat(&value).unwrap();
    assert_eq!(flat["production-eu.metrics.port"], serde_json::json!(9090));
    assert_eq!(flat["production-eu.security.certificates.cert_path"], serde_json::json!("/etc/ssl/certs/server.crt"));
}

#[test]
fn function_inside_tree_is_e0601() {
    use aura_core::lexer::Lexer;
    use aura_core::parser::Parser;
    // Свойство с лямбдой попадает в экспорт объекта → несериализуемо
    let src = "domain \"d\"\n  hook: (x) -> x end\nend";
    let toks = Lexer::new(src, 0).tokenize().unwrap();
    let module = Parser::new(toks).parse_module().unwrap();
    let v = Interpreter::new(Options::default()).eval_module(&module).unwrap();
    let err = to_json(&v).unwrap_err();
    assert_eq!(err.code, "E0601");
    assert!(err.message.contains("d.hook"), "path missing: {}", err.message);
}
