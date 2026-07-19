//! A high-level facade for embedding Aura in Rust applications.
//!
//! ```no_run
//! let opts = aura_core::facade::EvalOptions {
//!     allow_read: vec!["config/".into()],
//!     ..Default::default()
//! };
//! let out = aura_core::facade::eval_file("config/app.aura".as_ref(), &opts).unwrap();
//! let cfg: serde_json::Value = out.json; // or serde_json::from_value::<MyConfig>(out.json)
//! ```

use std::path::{Path, PathBuf};

use crate::analysis::has_blocking;
use crate::error::{Diagnostic, Severity};
use crate::eval::{DenyFs, EnvCap, Interpreter, Options, RealFs};
use crate::source::SourceCache;
use crate::span::Span;
use crate::vfs::loader::Loader;
use crate::vfs::lockfile::Lockfile;
use crate::vfs::{ImportSpec, LocalFsResolver};

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

    let mut loader = Loader::new(&cache, &resolver);
    loader.frozen = opts.frozen;
    if let Ok(text) = std::fs::read_to_string(root.join("aura.lock")) {
        loader.lock = Lockfile::parse(&text)
            .map_err(|e| vec![io_report(format!("invalid aura.lock: {e}"))])?;
    }

    let mut interp = Interpreter::new(Options {
        strict: opts.strict,
        dry_run: false,
    });
    if !opts.allow_read.is_empty() {
        interp.fs = Box::new(RealFs {
            allowed: opts.allow_read.clone(),
        });
    } else {
        interp.fs = Box::new(DenyFs);
    }
    interp.env_cap = opts.allow_env.clone();
    interp.allow_imports_io = opts.allow_imports_io;

    let file_name = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let result = interp_eval(&mut loader, &mut interp, &file_name);

    let mut reports: Vec<Report> = loader.diags.iter().map(|d| to_report(d, &cache)).collect();
    let value = match result {
        Ok(v) => v,
        Err(d) => {
            reports.insert(0, to_report(&d, &cache));
            return Err(reports);
        }
    };
    if has_blocking(&loader.diags, opts.strict) {
        return Err(reports);
    }

    let json = crate::serialize::to_json(&value).map_err(|d| {
        reports.insert(0, to_report(&d, &cache));
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
