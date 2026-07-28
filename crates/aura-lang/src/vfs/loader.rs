//! Module tree loader (SPEC §5.3): DFS with coloring (Loading/Loaded),
//! a cache of evaluated modules (each module is lexed/parsed/evaluated exactly
//! once), aura.lock integration, and capability isolation for imports (D1).

use std::collections::HashMap;

use super::lockfile::{integrity_of, LockEntry, Lockfile};
use super::{parse_version, version_satisfies, FileResolver, ImportSpec, ModuleId};
use crate::error::Diagnostic;
use crate::eval::value::Value;
use crate::eval::Interpreter;
use crate::lexer::Lexer;
use crate::parser::ast::ImportSource;
use crate::parser::Parser;
use crate::source::SourceCache;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Loaded,
}

pub struct Loader<'a, 'r> {
    pub cache: &'a SourceCache,
    pub resolver: &'r dyn FileResolver,
    pub lock: Lockfile,
    /// --frozen (CI): no matching lock entry is an E0403 error.
    pub frozen: bool,
    /// --hermetic: every effectful call is an analysis error (E0505), in every
    /// module including the root.
    pub hermetic: bool,
    /// Static analysis diagnostics for ALL loaded modules (SPEC §6.1):
    /// warnings accumulate here; analysis errors abort loading the module.
    pub diags: Vec<Diagnostic>,
    state: HashMap<ModuleId, LoadState>,
    values: HashMap<ModuleId, Value<'a>>,
    chain: Vec<ModuleId>,
}

fn rt(code: &'static str, msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(code, msg, span, "while loading modules")
}

impl<'a, 'r> Loader<'a, 'r> {
    pub fn new(cache: &'a SourceCache, resolver: &'r dyn FileResolver) -> Self {
        Loader {
            cache,
            resolver,
            lock: Lockfile::default(),
            frozen: false,
            hermetic: false,
            diags: Vec::new(),
            state: HashMap::new(),
            values: HashMap::new(),
            chain: Vec::new(),
        }
    }

    /// Entry point: the root module receives the interpreter's I/O capabilities.
    pub fn eval_entry(
        &mut self,
        interp: &mut Interpreter<'a>,
        spec: &ImportSpec<'_>,
    ) -> Result<Value<'a>, Diagnostic> {
        let dummy = Span::new(0, 0, 0);
        let id = self
            .resolver
            .resolve(spec, None)
            .map_err(|e| rt("E0404", e, dummy))?;
        self.eval_module_rec(interp, id, dummy, true)
    }

    fn eval_module_rec(
        &mut self,
        interp: &mut Interpreter<'a>,
        id: ModuleId,
        err_span: Span,
        is_root: bool,
    ) -> Result<Value<'a>, Diagnostic> {
        match self.state.get(&id) {
            Some(LoadState::Loaded) => return Ok(self.values[&id].clone()),
            Some(LoadState::Loading) => {
                let mut chain: Vec<String> = self.chain.iter().map(ModuleId::to_string).collect();
                chain.push(id.to_string());
                return Err(rt(
                    "E0401",
                    format!("cyclic import: {}", chain.join(" -> ")),
                    err_span,
                ));
            }
            None => {}
        }
        self.state.insert(id.clone(), LoadState::Loading);
        self.chain.push(id.clone());

        let result = self.eval_module_inner(interp, &id, err_span, is_root);

        self.chain.pop();
        match &result {
            Ok(v) => {
                self.state.insert(id.clone(), LoadState::Loaded);
                self.values.insert(id, v.clone());
            }
            Err(_) => {
                self.state.remove(&id);
            }
        }
        result
    }

    fn eval_module_inner(
        &mut self,
        interp: &mut Interpreter<'a>,
        id: &ModuleId,
        err_span: Span,
        is_root: bool,
    ) -> Result<Value<'a>, Diagnostic> {
        let text = self
            .resolver
            .load(id)
            .map_err(|e| rt("E0404", format!("cannot load module '{id}': {e}"), err_span))?;
        self.verify_lock(id, &text, err_span)?;

        let (source_id, src) = self.cache.add(id.to_string(), text);
        let toks = Lexer::new(src, source_id).tokenize()?;
        let module = match Parser::new(toks).parse_module() {
            Ok(m) => m,
            Err(mut ds) => {
                let first = ds.remove(0);
                self.diags.extend(ds);
                return Err(first);
            }
        };

        // Static analysis of every loaded module (SPEC §6.1): errors
        // (E0504 etc.) block the module before evaluation; warnings (W0501,
        // W0512 in imports) accumulate in self.diags — the host decides the strict policy.
        let mut analysis = crate::analysis::analyze_with(&module, is_root, self.hermetic);
        if let Some(pos) = analysis
            .iter()
            .position(|d| d.severity == crate::error::Severity::Error)
        {
            let first = analysis.remove(pos);
            self.diags.extend(analysis);
            return Err(first);
        }
        self.diags.extend(analysis);

        // First recursively evaluate all imports, then publish the aliases:
        // child modules overwrite the global alias map during their own eval.
        let mut resolved: Vec<(&str, Value<'a>)> = Vec::with_capacity(module.imports.len());
        for imp in &module.imports {
            let spec = match &imp.source {
                ImportSource::File(p) => ImportSpec::File(p),
                ImportSource::Registry { path, version } => ImportSpec::Registry { path, version },
            };
            let child_id = self.resolve_with_lock(&spec, Some(id), imp.span)?;
            let value = self.eval_module_rec(interp, child_id, imp.span, false)?;
            resolved.push((imp.alias, value));
        }
        for (alias, value) in resolved {
            interp.provide_module(alias, value);
        }

        // D1: imported modules have no I/O capabilities (SPEC §4.3).
        let saved_root = interp.current_root;
        interp.current_root = is_root;
        let result = interp.eval_module(&module);
        interp.current_root = saved_root;
        result
    }

    /// Resolution prioritizes aura.lock (SPEC §5.2): a matching lock entry wins;
    /// --frozen forbids resolving around the lock (E0403).
    fn resolve_with_lock(
        &mut self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
        span: Span,
    ) -> Result<ModuleId, Diagnostic> {
        if let ImportSpec::Registry { path, version } = spec {
            let request = parse_version(version)
                .ok_or_else(|| rt("E0404", format!("malformed version '{version}'"), span))?;
            if let Some(entry) = self.lock.entries.get(*path) {
                if parse_version(&entry.version)
                    .is_some_and(|locked| version_satisfies(&request, &locked))
                {
                    return Ok(ModuleId::Registry {
                        path: path.to_string(),
                        version: entry.version.clone(),
                    });
                }
            }
            if self.frozen {
                return Err(rt(
                    "E0403",
                    format!("--frozen: aura.lock has no entry satisfying '{path}@{version}'"),
                    span,
                ));
            }
        }
        self.resolver
            .resolve(spec, importer)
            .map_err(|e| rt("E0404", e, span))
    }

    /// Integrity check for registry modules (E0402) and appending to the lock.
    fn verify_lock(&mut self, id: &ModuleId, text: &str, span: Span) -> Result<(), Diagnostic> {
        let ModuleId::Registry { path, version } = id else {
            return Ok(());
        };
        let hash = integrity_of(text);
        match self.lock.entries.get(path) {
            Some(entry) if entry.version == *version => {
                // A lock written before token-stream hashing holds a `sha256-`
                // entry. Verify it the way it was written — failing an old lock
                // over an algorithm change would be indistinguishable from a real
                // tampering report, which is the one thing this must never be.
                let expected = crate::vfs::lockfile::recompute_like(&entry.integrity, text);
                if entry.integrity != expected {
                    return Err(rt(
                        "E0402",
                        format!(
                            "integrity mismatch for '{path}@v{version}': lock has {}, got {expected}",
                            entry.integrity
                        ),
                        span,
                    ));
                }
                // Verified under the old algorithm: upgrade the entry so the next
                // reformat of the package does not look like tampering. Under
                // --frozen the lock is read-only, so it stays as it is.
                if expected != hash && !self.frozen {
                    self.lock.entries.insert(
                        path.clone(),
                        LockEntry {
                            version: version.clone(),
                            integrity: hash,
                        },
                    );
                    self.lock.dirty = true;
                }
            }
            _ => {
                if self.frozen {
                    return Err(rt(
                        "E0403",
                        format!("--frozen: '{path}@v{version}' is not in aura.lock"),
                        span,
                    ));
                }
                self.lock.entries.insert(
                    path.clone(),
                    LockEntry {
                        version: version.clone(),
                        integrity: hash,
                    },
                );
                self.lock.dirty = true;
            }
        }
        Ok(())
    }
}
