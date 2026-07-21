//! Virtual File System (SPEC §5): module resolution and loading behind a trait.

pub mod loader;
pub mod lockfile;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModuleId {
    Local(PathBuf),
    /// The exact version after resolution (D8).
    Registry {
        path: String,
        version: String,
    },
    /// A placeholder for direct URL imports; not resolved yet (packages go through `aura add`).
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
    /// version — as written in the source, with a `v` prefix (a range: `v1`, `v1.2`, or exact `v1.2.3`).
    Registry {
        path: &'s str,
        version: &'s str,
    },
}

pub trait FileResolver {
    /// Canonicalizes the specifier relative to the importing module.
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String>;
    fn load(&self, id: &ModuleId) -> Result<String, String>;
}

/// A version as a sequence of numeric components; a range is a prefix match
/// (`v1.2` is satisfied by any `1.2.*`, SPEC §5.2).
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    s.split('.').map(|c| c.parse().ok()).collect()
}

pub fn version_satisfies(request: &[u64], exact: &[u64]) -> bool {
    exact.len() >= request.len() && request.iter().zip(exact).all(|(a, b)| a == b)
}

/// The network registry convention (used ONLY by the `aura add` command;
/// eval never touches the network): `github/<owner>/<repo>@vX.Y.Z` →
/// the `package.aura` raw file at the repo's `vX.Y.Z` tag.
pub fn registry_url(path: &str, version: &str) -> Result<String, String> {
    let version = version.strip_prefix('v').unwrap_or(version);
    if parse_version(version).is_none_or(|v| v.len() != 3) {
        return Err(format!(
            "network install requires an exact version (vX.Y.Z), got 'v{version}'"
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    match segments.as_slice() {
        ["github", owner, repo] => Ok(format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/v{version}/package.aura"
        )),
        _ => Err(format!(
            "unsupported registry path '{path}': expected github/<owner>/<repo>"
        )),
    }
}

/// Local disk: file imports relative to the importing file, registry —
/// a local cache directory `<registry_dir>/<path>/<version>.aura`,
/// populated by the `aura add` command (eval never touches the network).
pub struct LocalFsResolver {
    pub root: PathBuf,
    pub registry_dir: PathBuf,
}

impl FileResolver for LocalFsResolver {
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String> {
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
                let request = parse_version(version)
                    .ok_or_else(|| format!("malformed version '{version}'"))?;
                let dir = self.registry_dir.join(path);
                let mut best: Option<Vec<u64>> = None;
                let entries = std::fs::read_dir(&dir).map_err(|_| {
                    format!(
                        "module '{path}' not found in registry cache {}",
                        dir.display()
                    )
                })?;
                for entry in entries.flatten() {
                    let file = entry.file_name();
                    let Some(stem) = Path::new(&file).file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let Some(candidate) = parse_version(stem) else {
                        continue;
                    };
                    if version_satisfies(&request, &candidate)
                        && best.as_ref().is_none_or(|b| candidate > *b)
                    {
                        best = Some(candidate);
                    }
                }
                let best = best
                    .ok_or_else(|| format!("no cached version of '{path}' satisfies {version}"))?;
                let exact = best
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(".");
                Ok(ModuleId::Registry {
                    path: path.to_string(),
                    version: exact,
                })
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
            ModuleId::Url(u) => Err(format!("url imports are not supported yet: {u}")),
        }
    }
}

/// Dry-run wrapper (SPEC §6.3): module loads are performed but recorded into a report.
pub struct RecordingResolver<'r> {
    pub inner: &'r dyn FileResolver,
    pub log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl FileResolver for RecordingResolver<'_> {
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String> {
        self.inner.resolve(spec, importer)
    }
    fn load(&self, id: &ModuleId) -> Result<String, String> {
        let result = self.inner.load(id);
        if result.is_ok() {
            self.log.borrow_mut().push(id.to_string());
        }
        result
    }
}

/// An in-memory resolver for tests and dry-run snapshots. Keys: the file path as-is,
/// registry — `"<path>@v<version>"` (exact match, no ranges).
pub struct MemoryResolver {
    pub files: HashMap<String, String>,
}

impl FileResolver for MemoryResolver {
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        _importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String> {
        match spec {
            ImportSpec::File(p) => Ok(ModuleId::Local(PathBuf::from(p))),
            ImportSpec::Registry { path, version } => {
                let version = version.strip_prefix('v').unwrap_or(version);
                Ok(ModuleId::Registry {
                    path: path.to_string(),
                    version: version.to_string(),
                })
            }
        }
    }

    fn load(&self, id: &ModuleId) -> Result<String, String> {
        let key = match id {
            ModuleId::Local(p) => p.to_string_lossy().replace('\\', "/"),
            ModuleId::Registry { path, version } => format!("{path}@v{version}"),
            ModuleId::Url(u) => u.clone(),
        };
        self.files
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("not found: {key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::registry_url;

    #[test]
    fn registry_url_convention() {
        assert_eq!(
            registry_url("github/acme/aura-k8s", "v1.2.3").unwrap(),
            "https://raw.githubusercontent.com/acme/aura-k8s/v1.2.3/package.aura"
        );
        // network install requires an exact version
        assert!(registry_url("github/acme/aura-k8s", "v1.2").is_err());
        // unknown host / bare name
        assert!(registry_url("gitlab/acme/pkg", "v1.0.0").is_err());
        assert!(registry_url("just-a-name", "v1.0.0").is_err());
    }
}
