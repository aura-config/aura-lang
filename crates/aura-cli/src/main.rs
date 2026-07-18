//! CLI-слой (SPEC §7.2) и рендеринг диагностик через ariadne (§7.3).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};

use aura_core::analysis::{analyze, has_blocking};
use aura_core::error::{Diagnostic, Severity};
use aura_core::eval::{DenyFs, EnvCap, Interpreter, Options, RealFs};
use aura_core::lexer::Lexer;
use aura_core::parser::Parser;
use aura_core::serialize::{to_json, to_json_flat, to_toml_string, to_yaml_string};
use aura_core::source::SourceCache;
use aura_core::span::Span;
use aura_core::vfs::loader::Loader;
use aura_core::vfs::lockfile::Lockfile;
use aura_core::vfs::{ImportSpec, LocalFsResolver};

fn version_string() -> String {
    format!(
        "{} (Aura language v{})",
        env!("CARGO_PKG_VERSION"),
        aura_core::LANGUAGE_VERSION
    )
}

#[derive(ClapParser)]
#[command(name = "aura", version = version_string(), about = "Aura configuration language")]
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
    /// Вычислить манифест и экспортировать JSON
    Eval {
        file: PathBuf,
        /// Предупреждения анализа становятся ошибками; жёсткая валидация схем (E0513)
        #[arg(long)]
        strict: bool,
        /// Вычисление без записи на диск (JSON и aura.lock не записываются)
        #[arg(long)]
        dry_run: bool,
        /// CI-режим: резолв только по aura.lock (E0403), лок не переписывается
        #[arg(long)]
        frozen: bool,
        /// Разрешить read_file() внутри каталогов (можно повторять)
        #[arg(long = "allow-read", value_name = "DIR")]
        allow_read: Vec<PathBuf>,
        /// Разрешить env(): без значения — все переменные, либо список A,B
        #[arg(long = "allow-env", value_name = "VARS", num_args = 0..=1, default_missing_value = "")]
        allow_env: Option<String>,
        /// Выдать импортированным модулям I/O-права корня (D1)
        #[arg(long)]
        allow_imports_io: bool,
        #[arg(long, value_enum, default_value = "json")]
        format: OutFormat,
        /// Записать JSON в файл вместо stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Каталог локального registry-кэша (по умолчанию ~/.aura/registry)
        #[arg(long = "registry-dir", value_name = "DIR")]
        registry_dir: Option<PathBuf>,
    },
    /// Только lex + parse + статический анализ
    Check {
        file: PathBuf,
        #[arg(long)]
        strict: bool,
    },
    /// Канонизировать отступы и пустые строки (пишет файлы на место)
    Fmt {
        files: Vec<PathBuf>,
        /// Не менять файлы, только проверить (exit 1, если есть отличия)
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Check { file, strict } => run_check(&file, strict),
        Cmd::Fmt { files, check } => run_fmt(&files, check),
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

/// lex + parse + analysis; возвращает блокирует ли результат (exit 1).
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
        let formatted = match aura_core::fmt::format_source(&src) {
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
    // Анализ всех модулей (включая импортируемые) выполняет загрузчик VFS;
    // его диагностики рендерятся после загрузки (SPEC §6.1).
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

    let lock_path = root.join("aura.lock");
    let mut loader = Loader::new(&cache, &resolver);
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
    // --strict: предупреждения анализа (по всем модулям) блокируют вывод
    if has_blocking(&loader.diags, cfg.strict) {
        return ExitCode::from(1);
    }

    // aura.lock: дозапись (SPEC §5.2); в dry-run — только отчёт (§6.3)
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

/// Рендеринг Diagnostic через ariadne (SPEC §7.3). Спаны без исходника — плоский вывод.
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
        // ariadne индексирует по символам, спаны Aura — байтовые
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
