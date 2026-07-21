//! Phase 4 acceptance criteria (SPEC §8): cycles with a full chain, loading exactly once,
//! import I/O isolation (D1), lock file (E0402/E0403), manifest through the VFS.

use std::cell::RefCell;
use std::collections::HashMap;

use aura_lang::eval::value::Value;
use aura_lang::eval::{EnvCap, Interpreter, MemFs, Options};
use aura_lang::source::SourceCache;
use aura_lang::vfs::loader::Loader;
use aura_lang::vfs::lockfile::{integrity_of, LockEntry, Lockfile};
use aura_lang::vfs::{FileResolver, ImportSpec, MemoryResolver, ModuleId};

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

fn mem(files: &[(&str, &str)]) -> MemoryResolver {
    MemoryResolver {
        files: files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn get<'a>(v: &Value<'a>, key: &str) -> Value<'a> {
    let Value::Object(m) = v else {
        panic!("not an object")
    };
    m.get(key)
        .unwrap_or_else(|| panic!("no key '{key}'"))
        .clone()
}

#[test]
fn cyclic_import_reports_full_chain() {
    let resolver = mem(&[
        ("a.aura", "import \"b.aura\" as b\nx = 1"),
        ("b.aura", "import \"a.aura\" as a\ny = 2"),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    let err = loader
        .eval_entry(&mut it, &ImportSpec::File("a.aura"))
        .unwrap_err();
    assert_eq!(err.code, "E0401");
    assert!(
        err.message.contains("a.aura -> b.aura -> a.aura"),
        "chain missing: {}",
        err.message
    );
}

/// A resolver wrapper counting loads of each module.
struct Counting<'x> {
    inner: &'x MemoryResolver,
    loads: RefCell<HashMap<String, u32>>,
}
impl FileResolver for Counting<'_> {
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String> {
        self.inner.resolve(spec, importer)
    }
    fn load(&self, id: &ModuleId) -> Result<String, String> {
        *self.loads.borrow_mut().entry(id.to_string()).or_insert(0) += 1;
        self.inner.load(id)
    }
}

#[test]
fn shared_module_is_loaded_once() {
    let inner = mem(&[
        (
            "root.aura",
            "import \"b.aura\" as b\nimport \"c.aura\" as c\nx: b.v + c.v",
        ),
        ("b.aura", "import \"shared.aura\" as s\nv: s.base + 1"),
        ("c.aura", "import \"shared.aura\" as s\nv: s.base + 2"),
        ("shared.aura", "base: 10"),
    ]);
    let resolver = Counting {
        inner: &inner,
        loads: RefCell::new(HashMap::new()),
    };
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    let v = loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap();
    assert_eq!(get(&v, "x"), Value::Int(23));
    assert_eq!(
        resolver.loads.borrow()["shared.aura"],
        1,
        "shared module must be loaded exactly once"
    );
}

#[test]
fn imported_modules_have_no_io_capability_d1() {
    let resolver = mem(&[
        ("root.aura", "import \"evil.aura\" as e\nx: e.home"),
        ("evil.aura", "home: env(\"HOME\", \"?\")"),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    it.env_cap = EnvCap::AllowAll; // the root has permissions, the import does not
    let err = loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap_err();
    assert_eq!(err.code, "E0310");

    // --allow-imports-io lifts the denial
    let cache2 = SourceCache::new();
    let mut loader2 = Loader::new(&cache2, &resolver);
    let mut it2 = Interpreter::new(Options::default());
    it2.env_cap = EnvCap::AllowAll;
    it2.allow_imports_io = true;
    it2.env_overrides
        .insert("HOME".to_string(), "/home/x".to_string());
    let v = loader2
        .eval_entry(&mut it2, &ImportSpec::File("root.aura"))
        .unwrap();
    assert_eq!(get(&v, "x"), Value::str("/home/x"));
}

#[test]
fn loader_analyzes_imported_modules() {
    // SPEC §6.1: analysis runs on EVERY loaded module, not just the root
    let resolver = mem(&[
        ("root.aura", "import \"lib.aura\" as lib\nx: lib.v"),
        ("lib.aura", "dead_var = 1\nv: env(\"A\", \"b\")"),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    it.env_cap = EnvCap::AllowAll;
    it.allow_imports_io = true;
    loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap();
    // dead code and an effectful call are found in the IMPORTED module
    assert!(loader
        .diags
        .iter()
        .any(|d| d.code == "W0501" && d.message.contains("dead_var")));
    assert!(loader.diags.iter().any(|d| d.code == "W0512"));
}

#[test]
fn d12_pub_functions_and_schemas_cross_module() {
    let resolver = mem(&[
        (
            "root.aura",
            "import \"pkg.aura\" as pkg\nl: pkg.labels(\"auth\")\nm: new pkg.Meta\n  name: \"a\"\n  port: 1\nend\nvia_field: pkg.version",
        ),
        (
            "pkg.aura",
            "pub def labels(app)\n  app: app\n  managed_by: \"pkg\"\nend\npub type Meta\n  name: String\n  port: Int\nend\ndef private_helper()\n  x: 1\nend\nversion: \"1.0\"\nassert private_helper().x == 1",
        ),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    let v = loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap();
    // pub def is called through the alias
    assert_eq!(get(&get(&v, "l"), "managed_by"), Value::str("pkg"));
    // pub type is instantiated via new pkg.Meta (with validation)
    assert_eq!(get(&get(&v, "m"), "port"), Value::Int(1));
    assert_eq!(get(&v, "via_field"), Value::str("1.0"));
    // a private def is not accessible to the importer
    let bad = mem(&[
        (
            "root.aura",
            "import \"pkg.aura\" as pkg\nx: pkg.private_helper()",
        ),
        (
            "pkg.aura",
            "def private_helper()\n  x: 1\nend\nassert private_helper().x == 1",
        ),
    ]);
    let cache2 = SourceCache::new();
    let mut loader2 = Loader::new(&cache2, &bad);
    let mut it2 = Interpreter::new(Options::default());
    let err = loader2
        .eval_entry(&mut it2, &ImportSpec::File("root.aura"))
        .unwrap_err();
    assert_eq!(err.code, "E0309");
}

#[test]
fn d12_exported_functions_keep_module_capabilities() {
    // An exported package function does NOT inherit the root's I/O capabilities (D1×D12)
    let resolver = mem(&[
        ("root.aura", "import \"pkg.aura\" as pkg\nx: pkg.leak()"),
        (
            "pkg.aura",
            "pub def leak()\n  home: env(\"HOME\", \"?\")\nend",
        ),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    it.env_cap = EnvCap::AllowAll; // the root has permissions
    let err = loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap_err();
    assert_eq!(
        err.code, "E0310",
        "package function must not inherit root capabilities"
    );
}

#[test]
fn lockfile_integrity_and_frozen() {
    let resolver = mem(&[
        ("root.aura", "import pkg/lib@v1.2 as lib\nx: lib.n"),
        ("pkg/lib@v1.2", "n: 42"),
    ]);

    // First run: the lock gets appended to
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options::default());
    let v = loader
        .eval_entry(&mut it, &ImportSpec::File("root.aura"))
        .unwrap();
    assert_eq!(get(&v, "x"), Value::Int(42));
    assert!(loader.lock.dirty);
    let entry = &loader.lock.entries["pkg/lib"];
    assert_eq!(entry.version, "1.2");
    assert_eq!(entry.integrity, integrity_of("n: 42"));

    // Integrity tampering → E0402
    let cache2 = SourceCache::new();
    let mut loader2 = Loader::new(&cache2, &resolver);
    loader2.lock.entries.insert(
        "pkg/lib".to_string(),
        LockEntry {
            version: "1.2".to_string(),
            integrity: "sha256-deadbeef".to_string(),
        },
    );
    let mut it2 = Interpreter::new(Options::default());
    let err = loader2
        .eval_entry(&mut it2, &ImportSpec::File("root.aura"))
        .unwrap_err();
    assert_eq!(err.code, "E0402");

    // --frozen without a lock → E0403
    let cache3 = SourceCache::new();
    let mut loader3 = Loader::new(&cache3, &resolver);
    loader3.frozen = true;
    let mut it3 = Interpreter::new(Options::default());
    let err = loader3
        .eval_entry(&mut it3, &ImportSpec::File("root.aura"))
        .unwrap_err();
    assert_eq!(err.code, "E0403");

    // Round-trip of lock serialization
    let text = loader.lock.to_toml_string();
    let parsed = Lockfile::parse(&text).unwrap();
    assert_eq!(parsed.entries, loader.lock.entries);
}

#[test]
fn manifest_end_to_end_through_vfs() {
    let resolver = mem(&[
        ("production_deploy.aura", MANIFEST),
        (
            "templates/k8s_defaults.aura",
            "global_labels:\n  team: \"core\"\nend",
        ),
        ("github/actions/rust-cache@v1.2", "cache_key = \"rust-v1\""),
    ]);
    let cache = SourceCache::new();
    let mut loader = Loader::new(&cache, &resolver);
    let mut it = Interpreter::new(Options {
        strict: true,
        dry_run: false,
    });
    it.fs = Box::new(MemFs(HashMap::from([(
        "./Cargo.toml".to_string(),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\n".to_string(),
    )])));
    it.env_cap = EnvCap::Allow(vec!["APP_ENV".to_string()]);
    it.env_overrides
        .insert("APP_ENV".to_string(), "production".to_string());

    let root = loader
        .eval_entry(&mut it, &ImportSpec::File("production_deploy.aura"))
        .unwrap();
    let domain = get(&root, "production-eu");
    assert_eq!(get(&domain, "log_path"), Value::str("/var/log/aura.log"));
    let Value::List(apps) = get(&domain, "apps") else {
        panic!()
    };
    assert_eq!(get(&apps[0], "image"), Value::str("company/auth:1.2.3"));
    assert_eq!(get(&get(&apps[0], "labels"), "team"), Value::str("core"));
    // the registry module is pinned in the lock
    assert_eq!(
        loader.lock.entries["github/actions/rust-cache"].version,
        "1.2"
    );
}
