//! A high-level facade for embedding Aura in Rust applications.
//!
//! ```no_run
//! let opts = aura_lang::facade::EvalOptions {
//!     allow_read: vec!["config/".into()],
//!     ..Default::default()
//! };
//! let out = aura_lang::facade::eval_file("config/app.aura".as_ref(), &opts).unwrap();
//! let cfg: serde_json::Value = out.json; // or serde_json::from_value::<MyConfig>(out.json)
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::analysis::has_blocking;
use crate::error::{Diagnostic, Severity};
use crate::eval::{DenyFs, EnvCap, FileAccess, Interpreter, MemFs, Options, RealFs};
use crate::source::SourceCache;
use crate::span::Span;
use crate::vfs::loader::Loader;
use crate::vfs::lockfile::Lockfile;
use crate::vfs::{FileResolver, ImportSpec, LocalFsResolver, MemoryResolver};

#[derive(Debug, Clone, Default)]
pub struct EvalOptions {
    pub strict: bool,
    /// Directories allowed for read_file() (D1); empty = denied.
    pub allow_read: Vec<PathBuf>,
    /// env() capabilities; denied by default.
    pub allow_env: EnvCap,
    pub allow_imports_io: bool,
    /// Registry cache directory; None = ~/.aura/registry.
    pub registry_dir: Option<PathBuf>,
    /// Resolve strictly via aura.lock (E0403 on a mismatch).
    pub frozen: bool,
    /// Deny all I/O statically: `env()` and `read_file()` become E0505 in every
    /// module, so `check` alone proves the manifest touches nothing. Setting this
    /// alongside `allow_read` or `allow_env` is a contradiction — the grants are
    /// ignored, and the CLI refuses the combination outright.
    pub hermetic: bool,
    /// Values `env()` sees before the process environment is consulted. A host
    /// without a process environment — wasm in a browser — supplies them here.
    pub env_overrides: HashMap<String, String>,
}

/// A plain-form diagnostic: the host renders it itself, with no dependency on ariadne.
#[derive(Debug, Clone)]
pub struct Report {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub file: String,
    /// 1-based; 0 means the position is unknown (e.g. a serialization error).
    pub line: u32,
    pub column: u32,
    pub help: Option<String>,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{sev}[{}]: {}", self.code, self.message)?;
        if self.line > 0 {
            write!(f, " at {}:{}:{}", self.file, self.line, self.column)?;
        }
        if let Some(h) = &self.help {
            write!(f, "\n  help: {h}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Evaluated {
    pub json: serde_json::Value,
    /// Analysis warnings from all modules (under strict their presence would already be an error).
    pub warnings: Vec<Report>,
    /// The updated aura.lock, if new entries appeared (the host decides whether to write it).
    pub updated_lockfile: Option<String>,
}

/// Evaluates a manifest with all its imports and returns a JSON representation.
///
/// Errors are a `Vec<Report>` (the first is the reason for stopping, the rest are related).
pub fn eval_file(path: &Path, opts: &EvalOptions) -> Result<Evaluated, Vec<Report>> {
    let cache = SourceCache::new();
    let entry = std::fs::canonicalize(path)
        .map_err(|e| vec![io_report(format!("cannot open {}: {e}", path.display()))])?;
    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let registry_dir = opts.registry_dir.clone().unwrap_or_else(|| {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".aura")
            .join("registry")
    });
    let resolver = LocalFsResolver {
        root: root.clone(),
        registry_dir,
    };

    let lock = match std::fs::read_to_string(root.join("aura.lock")) {
        Ok(text) => Lockfile::parse(&text)
            .map_err(|e| vec![io_report(format!("invalid aura.lock: {e}"))])?,
        Err(_) => Lockfile::default(),
    };
    let fs: Box<dyn FileAccess> = if opts.hermetic || opts.allow_read.is_empty() {
        Box::new(DenyFs)
    } else {
        Box::new(RealFs {
            allowed: opts.allow_read.clone(),
        })
    };
    let file_name = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    run(&cache, &resolver, lock, fs, &file_name, opts)
}

/// Evaluate a manifest that exists only in memory: `files` maps a name to its
/// text, `entry` is the one to start from. Imports resolve inside that map, so a
/// multi-file example works without touching a disk — this is what a browser
/// playground or a test harness needs.
///
/// `read_file()` reads the same map, and is still capability-gated exactly as on
/// disk: with an empty `allow_read` it is denied. The paths in `allow_read` are
/// not consulted otherwise, since there is no filesystem to confine.
pub fn eval_source(
    files: HashMap<String, String>,
    entry: &str,
    opts: &EvalOptions,
) -> Result<Evaluated, Vec<Report>> {
    let cache = SourceCache::new();
    let resolver = MemoryResolver {
        files: files.clone(),
    };
    let fs: Box<dyn FileAccess> = if opts.hermetic || opts.allow_read.is_empty() {
        Box::new(DenyFs)
    } else {
        Box::new(MemFs(files))
    };
    run(&cache, &resolver, Lockfile::default(), fs, entry, opts)
}

/// The part `eval_file` and `eval_source` share: everything after deciding where
/// modules and file reads come from.
fn run(
    cache: &SourceCache,
    resolver: &dyn FileResolver,
    lock: Lockfile,
    fs: Box<dyn FileAccess>,
    file_name: &str,
    opts: &EvalOptions,
) -> Result<Evaluated, Vec<Report>> {
    let mut loader = Loader::new(cache, resolver);
    loader.frozen = opts.frozen;
    loader.hermetic = opts.hermetic;
    loader.lock = lock;

    let mut interp = Interpreter::new(Options {
        strict: opts.strict,
        dry_run: false,
    });
    interp.fs = fs;
    // Belt as well as braces: analysis already rejects the calls, but a host that
    // sets `hermetic` should not be able to leave a capability open by accident.
    interp.env_cap = if opts.hermetic {
        EnvCap::Deny
    } else {
        opts.allow_env.clone()
    };
    interp.allow_imports_io = opts.allow_imports_io;
    interp.env_overrides = opts.env_overrides.clone();

    let result = interp_eval(&mut loader, &mut interp, file_name);

    let mut reports: Vec<Report> = loader.diags.iter().map(|d| to_report(d, cache)).collect();
    let value = match result {
        Ok(v) => v,
        Err(d) => {
            reports.insert(0, to_report(&d, cache));
            return Err(reports);
        }
    };
    if has_blocking(&loader.diags, opts.strict) {
        return Err(reports);
    }

    let json = crate::serialize::to_json(&value).map_err(|d| {
        reports.insert(0, to_report(&d, cache));
        reports.clone()
    })?;
    let updated_lockfile =
        (loader.lock.dirty && !opts.frozen).then(|| loader.lock.to_toml_string());
    Ok(Evaluated {
        json,
        warnings: reports,
        updated_lockfile,
    })
}

fn interp_eval<'a>(
    loader: &mut Loader<'a, '_>,
    interp: &mut Interpreter<'a>,
    file_name: &str,
) -> Result<crate::eval::value::Value<'a>, Diagnostic> {
    loader.eval_entry(interp, &ImportSpec::File(file_name))
}

fn to_report(d: &Diagnostic, cache: &SourceCache) -> Report {
    let (file, line, column) = locate(d.primary.0, cache);
    Report {
        code: d.code,
        severity: d.severity,
        message: d.message.clone(),
        file,
        line,
        column,
        help: d.help.clone(),
    }
}

fn locate(span: Span, cache: &SourceCache) -> (String, u32, u32) {
    let name = cache.name(span.source).unwrap_or_default();
    let Some(text) = cache.text(span.source) else {
        return (name, 0, 0);
    };
    let upto = &text[..(span.start as usize).min(text.len())];
    let line = upto.matches('\n').count() as u32 + 1;
    let column = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) as u32 + 1;
    (name, line, column)
}

fn io_report(message: String) -> Report {
    Report {
        code: "E0001",
        severity: Severity::Error,
        message,
        file: String::new(),
        line: 0,
        column: 0,
        help: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The playground scenario: several buffers, an import between them, no disk.
    #[test]
    fn eval_source_resolves_imports_between_in_memory_files() {
        let fs = files(&[
            (
                "main.aura",
                "import \"lib.aura\" as lib\nport: lib.default_port\nname: lib.label(\"api\")\n",
            ),
            (
                "lib.aura",
                "pub def label(n)\n  service: n\nend\n\ndefault_port: 8080\n",
            ),
        ]);
        let out = eval_source(fs, "main.aura", &EvalOptions::default())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(out.json["port"], 8080);
        assert_eq!(out.json["name"]["service"], "api");
    }

    /// Capabilities behave exactly as on disk: denied unless granted, and when
    /// granted the "filesystem" is the same set of buffers.
    #[test]
    fn read_file_is_gated_and_reads_the_same_buffers() {
        let fs = files(&[
            ("main.aura", "data: read_file(\"data.json\").parse_json()\n"),
            ("data.json", "{\"k\": 1}"),
        ]);

        let denied = eval_source(fs.clone(), "main.aura", &EvalOptions::default());
        let reports = denied.expect_err("read_file must be denied without a grant");
        assert!(
            reports.iter().any(|r| r.code == "E0310"),
            "expected E0310, got {reports:?}"
        );

        let allowed = eval_source(
            fs,
            "main.aura",
            &EvalOptions {
                allow_read: vec![".".into()],
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(allowed.json["data"]["k"], 1);
    }

    /// Diagnostics still carry the buffer name and a position, which the
    /// playground needs to put a marker in the right editor tab.
    #[test]
    fn errors_point_at_the_buffer_they_came_from() {
        let fs = files(&[
            ("main.aura", "import \"lib.aura\" as lib\nx: lib.boom\n"),
            ("lib.aura", "pub def boom()\n  y: undefined_name\nend\n"),
        ]);
        let reports = eval_source(fs, "main.aura", &EvalOptions::default())
            .expect_err("undefined name must fail");
        let first = &reports[0];
        assert_eq!(first.code, "E0504", "{reports:?}");
        assert_eq!(first.file, "lib.aura", "must name the buffer");
        assert!(first.line > 0, "must carry a line");
    }

    /// The capability boundary holds in memory too: an imported buffer gets no
    /// file access even when the root was granted it (D1).
    #[test]
    fn imports_get_no_file_access_in_memory_either() {
        let fs = files(&[
            ("main.aura", "import \"dep.aura\" as dep\nx: dep.data\n"),
            ("dep.aura", "data: read_file(\"secret.txt\")\n"),
            ("secret.txt", "s3cr3t"),
        ]);
        let reports = eval_source(
            fs,
            "main.aura",
            &EvalOptions {
                allow_read: vec![".".into()],
                ..Default::default()
            },
        )
        .expect_err("an import must not read files");
        assert!(
            reports.iter().any(|r| r.code == "E0310"),
            "expected E0310, got {reports:?}"
        );
    }

    /// Hermetic mode refuses the call itself, not the access — the difference
    /// between E0505 and E0310 is the whole point, since only the former is
    /// decidable without running the branch.
    #[test]
    fn hermetic_turns_effectful_calls_into_analysis_errors() {
        for source in ["x: env(\"HOME\", \"/\")\n", "x: read_file(\"data.json\")\n"] {
            let fs = files(&[("main.aura", source), ("data.json", "{}")]);
            let reports = eval_source(
                fs,
                "main.aura",
                &EvalOptions {
                    hermetic: true,
                    ..Default::default()
                },
            )
            .expect_err("hermetic mode must refuse effectful calls");
            assert!(
                reports.iter().any(|r| r.code == "E0505"),
                "expected E0505 for {source:?}, got {reports:?}"
            );
        }
    }

    /// A host that sets `hermetic` alongside grants gets the hermetic answer. The
    /// CLI rejects the combination outright, but an embedder assembling options
    /// from config could produce it, and the safe reading is the restrictive one.
    #[test]
    fn hermetic_wins_over_grants_that_contradict_it() {
        let fs = files(&[
            ("main.aura", "data: read_file(\"data.json\")\n"),
            ("data.json", "{}"),
        ]);
        let reports = eval_source(
            fs,
            "main.aura",
            &EvalOptions {
                hermetic: true,
                allow_read: vec![".".into()],
                allow_env: EnvCap::AllowAll,
                ..Default::default()
            },
        )
        .expect_err("a grant must not re-open a hermetic evaluation");
        assert!(
            reports.iter().any(|r| r.code == "E0505"),
            "expected E0505, got {reports:?}"
        );
    }

    /// It reaches imports too: a dependency that reads the environment cannot be
    /// used in a hermetic build, and that is reported before evaluation of the
    /// root gets anywhere near the value.
    #[test]
    fn hermetic_reaches_imported_modules() {
        let fs = files(&[
            ("main.aura", "import \"dep.aura\" as dep\nx: dep.who\n"),
            ("dep.aura", "who: env(\"USER\", \"nobody\")\n"),
        ]);
        let reports = eval_source(
            fs,
            "main.aura",
            &EvalOptions {
                hermetic: true,
                ..Default::default()
            },
        )
        .expect_err("an import must not perform I/O in hermetic mode");
        assert!(
            reports.iter().any(|r| r.code == "E0505"),
            "expected E0505, got {reports:?}"
        );
    }

    /// And a manifest that needs nothing is unaffected: the mode must not become a
    /// reason to avoid using it.
    #[test]
    fn hermetic_leaves_a_pure_manifest_alone() {
        let fs = files(&[("main.aura", "base = 8000\napi:\n  port: base + 80\nend\n")]);
        let out = eval_source(
            fs,
            "main.aura",
            &EvalOptions {
                hermetic: true,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(out.json["api"]["port"], 8080);
    }
}
