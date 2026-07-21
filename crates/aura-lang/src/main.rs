//! CLI layer (SPEC §7.2) and diagnostic rendering via ariadne (§7.3).

// Diagnostic by value: errors are the cold path (see aura-lang/src/lib.rs)
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};

use aura_lang::analysis::{analyze, has_blocking};
use aura_lang::error::{Diagnostic, Severity};
use aura_lang::eval::{DenyFs, EnvCap, Interpreter, Options, RealFs};
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;
use aura_lang::serialize::{to_json, to_json_flat, to_toml_string, to_yaml_string};
use aura_lang::source::SourceCache;
use aura_lang::span::Span;
use aura_lang::vfs::loader::Loader;
use aura_lang::vfs::lockfile::Lockfile;
use aura_lang::vfs::{ImportSpec, LocalFsResolver};

#[derive(ClapParser)]
#[command(name = "aura", version, about = "Aura configuration language")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutFormat {
    Json,
    JsonFlat,
    Yaml,
    Toml,
}

#[derive(Subcommand)]
enum Cmd {
    /// Evaluate a manifest and export JSON
    Eval {
        file: PathBuf,
        /// Analysis warnings become errors; strict schema validation (E0513)
        #[arg(long)]
        strict: bool,
        /// Evaluate without writing to disk (neither JSON nor aura.lock)
        #[arg(long)]
        dry_run: bool,
        /// CI mode: resolve strictly via aura.lock (E0403), the lock is never rewritten
        #[arg(long)]
        frozen: bool,
        /// Allow read_file() inside these directories (repeatable)
        #[arg(long = "allow-read", value_name = "DIR")]
        allow_read: Vec<PathBuf>,
        /// Allow env(): no value — all variables, or a comma-separated list A,B
        #[arg(long = "allow-env", value_name = "VARS", num_args = 0..=1, default_missing_value = "")]
        allow_env: Option<String>,
        /// Grant imported modules the root's I/O capabilities (D1)
        #[arg(long)]
        allow_imports_io: bool,
        #[arg(long, value_enum, default_value = "json")]
        format: OutFormat,
        /// Write JSON to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Local registry cache directory (default ~/.aura/registry)
        #[arg(long = "registry-dir", value_name = "DIR")]
        registry_dir: Option<PathBuf>,
    },
    /// Lex + parse + static analysis only
    Check {
        file: PathBuf,
        #[arg(long)]
        strict: bool,
    },
    /// Install a package into the local registry cache and pin it in aura.lock.
    /// The network is used ONLY here: eval always runs offline.
    Add {
        /// Specifier: github/<owner>/<repo>@vX.Y.Z
        package: String,
        /// Install from a local file instead of the network (tests, private packages)
        #[arg(long, value_name = "FILE")]
        from: Option<PathBuf>,
        /// Local registry cache directory (default ~/.aura/registry)
        #[arg(long = "registry-dir", value_name = "DIR")]
        registry_dir: Option<PathBuf>,
    },
    /// Canonical formatter: indentation, spacing, and column alignment (rewrites files in place)
    Fmt {
        files: Vec<PathBuf>,
        /// Do not modify files, only check (exit 1 on differences)
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Check { file, strict } => run_check(&file, strict),
        Cmd::Fmt { files, check } => run_fmt(&files, check),
        Cmd::Add {
            package,
            from,
            registry_dir,
        } => run_add(&package, from.as_deref(), registry_dir),
        Cmd::Eval {
            file,
            strict,
            dry_run,
            frozen,
            allow_read,
            allow_env,
            allow_imports_io,
            format,
            output,
            registry_dir,
        } => run_eval(EvalConfig {
            file,
            strict,
            dry_run,
            frozen,
            allow_read,
            allow_env,
            allow_imports_io,
            format,
            output,
            registry_dir,
        }),
    }
}

struct EvalConfig {
    file: PathBuf,
    strict: bool,
    dry_run: bool,
    frozen: bool,
    allow_read: Vec<PathBuf>,
    allow_env: Option<String>,
    allow_imports_io: bool,
    format: OutFormat,
    output: Option<PathBuf>,
    registry_dir: Option<PathBuf>,
}

/// lex + parse + analysis; returns whether the result blocks (exit 1).
fn front_end(cache: &SourceCache, file: &Path, strict: bool) -> Result<bool, ExitCode> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return Err(ExitCode::from(2));
        }
    };
    let (source_id, src) = cache.add(file.display().to_string(), text);
    let toks = match Lexer::new(src, source_id).tokenize() {
        Ok(t) => t,
        Err(d) => {
            render(&d, cache);
            return Ok(true);
        }
    };
    let module = match Parser::new(toks).parse_module() {
        Ok(m) => m,
        Err(ds) => {
            for d in &ds {
                render(d, cache);
            }
            return Ok(true);
        }
    };
    let diags = analyze(&module, true);
    for d in &diags {
        render(d, cache);
    }
    Ok(has_blocking(&diags, strict))
}

fn run_check(file: &Path, strict: bool) -> ExitCode {
    let cache = SourceCache::new();
    match front_end(&cache, file, strict) {
        Err(code) => code,
        Ok(true) => ExitCode::from(1),
        Ok(false) => {
            println!("ok: {}", file.display());
            ExitCode::SUCCESS
        }
    }
}

fn default_registry_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".aura")
        .join("registry")
}

/// `aura add pkg@vX.Y.Z`: download (or take from --from), validate,
/// store in the cache and pin in ./aura.lock with sha256 integrity.
fn run_add(package: &str, from: Option<&Path>, registry_dir: Option<PathBuf>) -> ExitCode {
    let Some((path, version)) = package.split_once('@') else {
        eprintln!("error: expected <path>@vX.Y.Z, got '{package}'");
        return ExitCode::from(2);
    };
    let version_num = version.strip_prefix('v').unwrap_or(version);

    // 1. Source: a local file or the network (the only place Aura ever touches the network)
    let text = match from {
        Some(file) => match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", file.display());
                return ExitCode::from(2);
            }
        },
        None => {
            let url = match aura_lang::vfs::registry_url(path, version) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
                }
            };
            eprintln!("fetching {url}");
            match ureq::get(&url).call() {
                Ok(resp) => match resp.into_string() {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: cannot read response: {e}");
                        return ExitCode::from(2);
                    }
                },
                Err(e) => {
                    eprintln!("error: download failed: {e}");
                    return ExitCode::from(2);
                }
            }
        }
    };

    // 2. Validate the package before installing (lex + parse + analysis as an imported module)
    let cache = SourceCache::new();
    let (source_id, src) = cache.add(format!("{path}@{version}"), text.clone());
    let module = match Lexer::new(src, source_id).tokenize().and_then(|toks| {
        Parser::new(toks)
            .parse_module()
            .map_err(|mut ds| ds.remove(0))
    }) {
        Ok(m) => m,
        Err(d) => {
            eprintln!("error: package is not valid Aura:");
            render(&d, &cache);
            return ExitCode::from(1);
        }
    };
    for d in analyze(&module, false) {
        render(&d, &cache);
    }

    // 3. Install into the cache
    let registry = registry_dir.unwrap_or_else(default_registry_dir);
    let target_dir = registry.join(path);
    if let Err(e) = std::fs::create_dir_all(&target_dir) {
        eprintln!("error: cannot create {}: {e}", target_dir.display());
        return ExitCode::from(2);
    }
    let target = target_dir.join(format!("{version_num}.aura"));
    if let Err(e) = std::fs::write(&target, text.as_bytes()) {
        eprintln!("error: cannot write {}: {e}", target.display());
        return ExitCode::from(2);
    }

    // 4. Pin in ./aura.lock
    let lock_path = Path::new("aura.lock");
    let mut lock = match std::fs::read_to_string(lock_path) {
        Ok(t) => match Lockfile::parse(&t) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error: invalid aura.lock: {e}");
                return ExitCode::from(2);
            }
        },
        Err(_) => Lockfile::default(),
    };
    let integrity = aura_lang::vfs::lockfile::integrity_of(&text);
    lock.entries.insert(
        path.to_string(),
        aura_lang::vfs::lockfile::LockEntry {
            version: version_num.to_string(),
            integrity: integrity.clone(),
        },
    );
    if let Err(e) = std::fs::write(lock_path, lock.to_toml_string()) {
        eprintln!("error: cannot write aura.lock: {e}");
        return ExitCode::from(2);
    }

    println!("installed {path}@v{version_num} -> {}", target.display());
    println!("locked    {integrity}");
    ExitCode::SUCCESS
}

fn run_fmt(files: &[PathBuf], check: bool) -> ExitCode {
    if files.is_empty() {
        eprintln!("error: no input files");
        return ExitCode::from(2);
    }
    let mut dirty = false;
    for file in files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", file.display());
                return ExitCode::from(2);
            }
        };
        let cache = SourceCache::new();
        cache.add(file.display().to_string(), src.clone());
        let formatted = match aura_lang::fmt::format_source(&src) {
            Ok(f) => f,
            Err(d) => {
                render(&d, &cache);
                return ExitCode::from(1);
            }
        };
        if formatted == src {
            continue;
        }
        dirty = true;
        if check {
            println!("would reformat: {}", file.display());
        } else {
            if let Err(e) = std::fs::write(file, formatted.as_bytes()) {
                eprintln!("error: cannot write {}: {e}", file.display());
                return ExitCode::from(2);
            }
            println!("formatted: {}", file.display());
        }
    }
    if check && dirty {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run_eval(cfg: EvalConfig) -> ExitCode {
    // Analysis of all modules (including imports) is done by the VFS loader;
    // its diagnostics are rendered after loading (SPEC §6.1).
    let cache = SourceCache::new();
    let entry = match std::fs::canonicalize(&cfg.file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let registry_dir = cfg.registry_dir.clone().unwrap_or_else(|| {
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

    // Dry-run (SPEC §6.3): all reads are performed but recorded into a report
    let read_log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let recording;
    let resolver_ref: &dyn aura_lang::vfs::FileResolver = if cfg.dry_run {
        recording = aura_lang::vfs::RecordingResolver {
            inner: &resolver,
            log: read_log.clone(),
        };
        &recording
    } else {
        &resolver
    };

    let lock_path = root.join("aura.lock");
    let mut loader = Loader::new(&cache, resolver_ref);
    loader.frozen = cfg.frozen;
    if let Ok(text) = std::fs::read_to_string(&lock_path) {
        match Lockfile::parse(&text) {
            Ok(l) => loader.lock = l,
            Err(e) => {
                eprintln!("error: invalid {}: {e}", lock_path.display());
                return ExitCode::from(2);
            }
        }
    }

    let mut interp = Interpreter::new(Options {
        strict: cfg.strict,
        dry_run: cfg.dry_run,
    });
    interp.allow_imports_io = cfg.allow_imports_io;
    if !cfg.allow_read.is_empty() {
        interp.fs = Box::new(RealFs {
            allowed: cfg.allow_read.clone(),
        });
    } else {
        interp.fs = Box::new(DenyFs);
    }
    if cfg.dry_run {
        let inner = std::mem::replace(&mut interp.fs, Box::new(DenyFs));
        interp.fs = Box::new(aura_lang::eval::RecordingFs {
            inner,
            log: read_log.clone(),
        });
    }
    interp.env_cap = match &cfg.allow_env {
        None => EnvCap::Deny,
        Some(s) if s.is_empty() => EnvCap::AllowAll,
        Some(s) => EnvCap::Allow(s.split(',').map(str::to_string).collect()),
    };

    let file_name = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let result = loader.eval_entry(&mut interp, &ImportSpec::File(&file_name));
    for d in &loader.diags {
        render(d, &cache);
    }
    let value = match result {
        Ok(v) => v,
        Err(d) => {
            render(&d, &cache);
            return ExitCode::from(1);
        }
    };
    // --strict: analysis warnings (across all modules) block the output
    if has_blocking(&loader.diags, cfg.strict) {
        return ExitCode::from(1);
    }

    // Dry-run report of the reads performed (modules + read_file)
    if cfg.dry_run {
        let mut reads = read_log.borrow().clone();
        reads.dedup();
        for r in reads {
            eprintln!("[dry-run] read: {r}");
        }
    }

    // aura.lock: append new entries (SPEC §5.2); dry-run only reports (§6.3)
    if loader.lock.dirty && !cfg.frozen {
        let text = loader.lock.to_toml_string();
        if cfg.dry_run {
            eprintln!(
                "[dry-run] would write {} bytes to {}",
                text.len(),
                lock_path.display()
            );
        } else if let Err(e) = std::fs::write(&lock_path, &text) {
            eprintln!("error: cannot write {}: {e}", lock_path.display());
            return ExitCode::from(2);
        }
    }

    let rendered = match cfg.format {
        OutFormat::Json => {
            to_json(&value).map(|j| serde_json::to_string_pretty(&j).expect("valid json"))
        }
        OutFormat::JsonFlat => {
            to_json_flat(&value).map(|j| serde_json::to_string_pretty(&j).expect("valid json"))
        }
        OutFormat::Yaml => to_yaml_string(&value),
        OutFormat::Toml => to_toml_string(&value),
    };
    let pretty = match rendered {
        Ok(s) => s,
        Err(d) => {
            render(&d, &cache);
            return ExitCode::from(1);
        }
    };

    match &cfg.output {
        Some(path) if !cfg.dry_run => {
            if let Err(e) = std::fs::write(path, pretty.as_bytes()) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
        Some(path) => {
            eprintln!(
                "[dry-run] would write {} bytes to {}",
                pretty.len(),
                path.display()
            );
            println!("{pretty}");
        }
        None => println!("{pretty}"),
    }
    ExitCode::SUCCESS
}

/// Renders a Diagnostic via ariadne (SPEC §7.3). Spans without a source fall back to plain output.
fn render(d: &Diagnostic, cache: &SourceCache) {
    use ariadne::{sources, Color, Config, Label, Report, ReportKind};

    let has_source = cache.text(d.primary.0.source).is_some() && d.primary.0.end > 0;
    if !has_source {
        eprintln!("{} [{}]: {}", severity_str(d.severity), d.code, d.message);
        if let Some(h) = &d.help {
            eprintln!("  help: {h}");
        }
        return;
    }

    let mut files: Vec<(String, String)> = Vec::new();
    let mut resolve = |sp: Span| -> (String, std::ops::Range<usize>) {
        let name = cache
            .name(sp.source)
            .unwrap_or_else(|| "<input>".to_string());
        let text = cache.text(sp.source).unwrap_or("").to_string();
        // ariadne indexes by characters, Aura spans are byte offsets
        let start = char_off(&text, sp.start as usize);
        let end = char_off(&text, sp.end as usize).max(start + 1);
        if !files.iter().any(|(n, _)| *n == name) {
            files.push((name.clone(), text));
        }
        (name, start..end)
    };

    let kind = match d.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
    };
    let (pname, prange) = resolve(d.primary.0);
    let mut report = Report::build(kind, (pname.clone(), prange.clone()))
        .with_config(Config::default())
        .with_code(d.code)
        .with_message(&d.message)
        .with_label(
            Label::new((pname, prange))
                .with_message(&d.primary.1)
                .with_color(Color::Red),
        );
    for (sp, msg) in &d.secondary {
        let (n, r) = resolve(*sp);
        report = report.with_label(
            Label::new((n, r))
                .with_message(msg)
                .with_color(Color::Yellow),
        );
    }
    if let Some(h) = &d.help {
        report = report.with_help(h);
    }
    report.finish().eprint(sources(files)).ok();
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn char_off(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}
