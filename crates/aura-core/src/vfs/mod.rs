//! Virtual File System (SPEC §5): резолв и загрузка модулей за трейтом.

pub mod loader;
pub mod lockfile;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModuleId {
    Local(PathBuf),
    /// Точная версия после резолва (D8).
    Registry { path: String, version: String },
    /// Задел под Deno-style импорты; в v1.2 не резолвится.
    Url(String),
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleId::Local(p) => write!(f, "{}", p.display()),
            ModuleId::Registry { path, version } => write!(f, "{path}@v{version}"),
            ModuleId::Url(u) => write!(f, "{u}"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ImportSpec<'s> {
    File(&'s str),
    /// version — как в исходнике, с префиксом `v` (диапазон: `v1`, `v1.2` или точная `v1.2.3`).
    Registry { path: &'s str, version: &'s str },
}

pub trait FileResolver {
    /// Канонизирует спецификатор относительно импортирующего модуля.
    fn resolve(&self, spec: &ImportSpec<'_>, importer: Option<&ModuleId>) -> Result<ModuleId, String>;
    fn load(&self, id: &ModuleId) -> Result<String, String>;
}

/// Версия как последовательность числовых компонент; диапазон — совпадение префикса
/// (`v1.2` удовлетворяют все `1.2.*`, SPEC §5.2).
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    s.split('.').map(|c| c.parse().ok()).collect()
}

pub fn version_satisfies(request: &[u64], exact: &[u64]) -> bool {
    exact.len() >= request.len() && request.iter().zip(exact).all(|(a, b)| a == b)
}

/// Локальный диск: файловые импорты относительно импортирующего файла,
/// registry — локальный кэш-каталог `<registry_dir>/<path>/<version>.aura` (сети в v1.2 нет).
pub struct LocalFsResolver {
    pub root: PathBuf,
    pub registry_dir: PathBuf,
}

impl FileResolver for LocalFsResolver {
    fn resolve(&self, spec: &ImportSpec<'_>, importer: Option<&ModuleId>) -> Result<ModuleId, String> {
        match spec {
            ImportSpec::File(rel) => {
                let base = match importer {
                    Some(ModuleId::Local(p)) => p.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    _ => self.root.clone(),
                };
                let joined = base.join(rel);
                let canon = std::fs::canonicalize(&joined)
                    .map_err(|e| format!("cannot resolve '{rel}': {e}"))?;
                Ok(ModuleId::Local(canon))
            }
            ImportSpec::Registry { path, version } => {
                let request = parse_version(version).ok_or_else(|| format!("malformed version '{version}'"))?;
                let dir = self.registry_dir.join(path);
                let mut best: Option<Vec<u64>> = None;
                let entries = std::fs::read_dir(&dir)
                    .map_err(|_| format!("module '{path}' not found in registry cache {}", dir.display()))?;
                for entry in entries.flatten() {
                    let file = entry.file_name();
                    let Some(stem) = Path::new(&file).file_stem().and_then(|s| s.to_str()) else { continue };
                    let Some(candidate) = parse_version(stem) else { continue };
                    if version_satisfies(&request, &candidate) && best.as_ref().map_or(true, |b| candidate > *b) {
                        best = Some(candidate);
                    }
                }
                let best = best.ok_or_else(|| format!("no cached version of '{path}' satisfies {version}"))?;
                let exact = best.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
                Ok(ModuleId::Registry { path: path.to_string(), version: exact })
            }
        }
    }

    fn load(&self, id: &ModuleId) -> Result<String, String> {
        match id {
            ModuleId::Local(p) => std::fs::read_to_string(p).map_err(|e| e.to_string()),
            ModuleId::Registry { path, version } => {
                let file = self.registry_dir.join(path).join(format!("{version}.aura"));
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))
            }
            ModuleId::Url(u) => Err(format!("url imports are not supported in v1.2: {u}")),
        }
    }
}

/// In-memory резолвер для тестов и dry-run снапшотов. Ключи: путь файла как есть,
/// registry — `"<path>@v<version>"` (точное совпадение, без диапазонов).
pub struct MemoryResolver {
    pub files: HashMap<String, String>,
}

impl FileResolver for MemoryResolver {
    fn resolve(&self, spec: &ImportSpec<'_>, _importer: Option<&ModuleId>) -> Result<ModuleId, String> {
        match spec {
            ImportSpec::File(p) => Ok(ModuleId::Local(PathBuf::from(p))),
            ImportSpec::Registry { path, version } => {
                let version = version.strip_prefix('v').unwrap_or(version);
                Ok(ModuleId::Registry { path: path.to_string(), version: version.to_string() })
            }
        }
    }

    fn load(&self, id: &ModuleId) -> Result<String, String> {
        let key = match id {
            ModuleId::Local(p) => p.to_string_lossy().replace('\\', "/"),
            ModuleId::Registry { path, version } => format!("{path}@v{version}"),
            ModuleId::Url(u) => u.clone(),
        };
        self.files.get(&key).cloned().ok_or_else(|| format!("not found: {key}"))
    }
}
