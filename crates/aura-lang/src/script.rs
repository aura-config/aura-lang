//! The embedding API (D33): a loaded script a Rust host can call.
//!
//! A `Script` is loaded once — lexed, parsed, analysed and evaluated — and then
//! called as often as the host likes, from as many threads as it likes. Every
//! guarantee the CLI gives holds here too, because loading goes through the same
//! loader, analysis and capability model:
//!
//! - **hermetic by default**: with no grants, loading proves statically that the
//!   script reads no file, variable or clock (`E0505` otherwise);
//! - **a budget per call**, in steps and bytes, never in time (`E0325`/`E0326`);
//! - **constant memory per call**: each call runs in its own interpreter, whose
//!   environments are released when it returns;
//! - **a checked boundary**: a value Aura cannot hold exactly is refused at the
//!   door (`E0005`), and a result of the wrong shape names the path (`E0006`).
//!
//! What a call costs: an `Arc` clone of the shared method registry, a fresh
//! interpreter around it, and the conversion of arguments and result. Parsing,
//! analysis and module evaluation are never repeated.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::Severity;
use crate::eval::budget::Budget;
use crate::eval::methods::MethodRegistry;
use crate::eval::value::Value;
use crate::eval::{DenyFs, EnvCap, FileAccess, FileError, Interpreter, MemFs, Options, RealFs};
use crate::facade::{to_report, Report};
use crate::host::{HostCall, HostFn, HostKind, HostOutcome, HostTable};
use crate::source::SourceCache;
use crate::span::Span;
use crate::vfs::loader::Loader;
use crate::vfs::lockfile::Lockfile;
use crate::vfs::{FileResolver, ImportSpec, LocalFsResolver, MemoryResolver, ModuleId};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything that can go wrong loading or calling a script. It carries the
/// same reports the CLI prints: a code, a position when there is one, and the
/// help line.
#[derive(Debug, Clone)]
pub struct Error {
    reports: Vec<Report>,
}

impl Error {
    /// The code of the reason the operation stopped — `"E0505"`, `"E0325"`, …
    pub fn code(&self) -> &'static str {
        self.reports.first().map_or("E0001", |r| r.code)
    }

    /// Every report, the reason first.
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }

    fn host(code: &'static str, message: impl Into<String>) -> Self {
        Error {
            reports: vec![Report {
                code,
                severity: Severity::Error,
                message: message.into(),
                file: String::new(),
                line: 0,
                column: 0,
                help: None,
            }],
        }
    }
}

impl From<Vec<Report>> for Error {
    fn from(reports: Vec<Report>) -> Self {
        Error { reports }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, r) in self.reports.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{r}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Configures a script before it is loaded. Grants have the names and meaning
/// of the CLI's flags.
#[derive(Default)]
pub struct ScriptBuilder {
    sources: Vec<(String, String)>,
    entry: Option<String>,
    file: Option<PathBuf>,
    env: Option<Vec<String>>,
    read: Vec<PathBuf>,
    strict: bool,
    budget: Budget,
    host: Vec<HostFn>,
    host_for_imports: Vec<String>,
    host_call_steps: Option<u64>,
    watch: Option<(Duration, OnReload)>,
    settle: Option<Duration>,
}

impl ScriptBuilder {
    /// A source held in memory. Several may be given; imports resolve among them.
    pub fn source(mut self, name: impl Into<String>, text: impl Into<String>) -> Self {
        self.sources.push((name.into(), text.into()));
        self
    }

    /// Which in-memory source to start from. Defaults to the first one given.
    pub fn entry(mut self, name: impl Into<String>) -> Self {
        self.entry = Some(name.into());
        self
    }

    /// Load from disk instead; imports resolve beside it, as in `aura eval`.
    pub fn file(mut self, path: impl AsRef<Path>) -> Self {
        self.file = Some(path.as_ref().to_path_buf());
        self
    }

    /// `--allow-env=A,B`: the named variables only.
    pub fn allow_env<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.env
            .get_or_insert_with(Vec::new)
            .extend(names.into_iter().map(Into::into));
        self
    }

    /// `--allow-read=<dir>`.
    pub fn allow_read(mut self, dir: impl AsRef<Path>) -> Self {
        self.read.push(dir.as_ref().to_path_buf());
        self
    }

    /// `--strict`: warnings fail the load.
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// The budget each call gets. Loading is held to the same budget.
    pub fn budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// D35: a function the script may call as `import "host:geo" as geo` …
    /// `geo.region(ip)`, named `"geo.region"` here. *Pure* is the host's
    /// promise that the function is deterministic - the language cannot check
    /// it - and it is what lets the script stay hermetic. Return `Option` for a
    /// value that may be absent: `None` reaches the script as `null`.
    pub fn pure<Args, F: PureFn<Args>>(mut self, name: impl Into<String>, f: F) -> Self {
        self.host.push(HostFn {
            name: name.into(),
            kind: HostKind::Pure,
            arity: F::ARITY,
            for_imports: false,
            call: f.erase(),
        });
        self
    }

    /// D35: a function with an effect - it reads or changes something outside
    /// the script. Registering it is the grant, like `allow_env`, so the load
    /// is no longer hermetic. `Err` stops the call with `E0008`.
    pub fn effect<Args, F: EffectFn<Args>>(mut self, name: impl Into<String>, f: F) -> Self {
        self.host.push(HostFn {
            name: name.into(),
            kind: HostKind::Effect,
            arity: F::ARITY,
            for_imports: false,
            call: f.erase(),
        });
        self
    }

    /// D35 §3: let imported modules call this host function too. By default
    /// only the root module can - a package the script imports does not reach
    /// the host's services unless the host says so, one function at a time.
    pub fn grant_to_imports(mut self, name: impl Into<String>) -> Self {
        self.host_for_imports.push(name.into());
        self
    }

    /// D35 §5: what one call to a host function costs, in steps. The default
    /// is 100. Time spent inside the host is not measured; this is what bounds
    /// a loop of host calls.
    pub fn host_call_steps(mut self, steps: u64) -> Self {
        self.host_call_steps = Some(steps);
        self
    }

    /// D36: watch the script's files, and reload when their content changes.
    /// Every `interval` a thread owned by the script looks at the files; a
    /// change is loaded once the files have been left alone for the settle
    /// period ([`ScriptBuilder::settle`]), and
    /// passes every check the first load did. `on_reload` is told of every
    /// automatic reload - the new version, or why it was refused while the old
    /// one keeps serving - so a failure cannot pass unnoticed. It runs on the
    /// watcher's thread. Only a script loaded from a file can be watched.
    pub fn watch(
        mut self,
        interval: Duration,
        on_reload: impl Fn(Result<ScriptVersion, Error>) + Send + 'static,
    ) -> Self {
        self.watch = Some((interval, Box::new(on_reload)));
        self
    }

    /// D36 §6: how long the files must stay unchanged before a watched script
    /// reloads them - no write, no change of size or modification time. A
    /// file being written is never at rest that long, so it is never loaded
    /// half done. The default is two intervals and at least a second; raise it
    /// for a slow network share. It may not be shorter than the interval.
    pub fn settle(mut self, settle: Duration) -> Self {
        self.settle = Some(settle);
        self
    }

    pub fn load(self) -> Result<Script, Error> {
        Script::build(self)
    }
}

// ---------------------------------------------------------------------------
// Script
// ---------------------------------------------------------------------------

/// What a call needs and the load produced: the module's exports and the
/// interpreter that evaluated them. That interpreter is never used again; it is
/// kept because it owns the environments the exported functions close over.
struct State<'a> {
    exports: IndexMap<String, Value<'a>>,
    registry: Arc<MethodRegistry<'a>>,
    fs: Arc<dyn FileAccess>,
    env_cap: EnvCap,
    host_call_steps: u64,
    strict: bool,
    _closures: Interpreter<'a>,
}

self_cell::self_cell!(
    struct Loaded {
        owner: SourceCache,
        #[not_covariant]
        dependent: State,
    }
);

/// Everything a load uses, taken from the builder once and kept, so that a
/// reload runs with exactly the grants the first load had (D36 §3).
struct Config {
    sources: Vec<(String, String)>,
    entry: Option<String>,
    file: Option<PathBuf>,
    read: Vec<PathBuf>,
    env_cap: EnvCap,
    granted: bool,
    strict: bool,
    budget: Budget,
    host: Arc<HostTable>,
}

/// Which version of a script is serving (D36 §8): the generation counts
/// successful loads from 1, and the hash covers every file the load read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptVersion {
    pub generation: u64,
    pub hash: String,
}

/// One loaded version: what calls run against, and the files it came from.
struct Version {
    loaded: Loaded,
    id: ScriptVersion,
    /// Every local file the load read, with what it read (D36 §5).
    files: Vec<(PathBuf, String)>,
}

/// What a version promises the host: each export's name, and the number of
/// arguments if it is a function (D36 §2).
type Contract = Vec<(String, Option<usize>)>;

/// The state every handle on a script shares, the watcher included.
struct Shared {
    config: Config,
    /// The lock as the first load left it. Every reload is frozen to it, so a
    /// reload cannot pull in a package the first load did not pin (D36 §4).
    lock: Lockfile,
    /// Fixed at the first load (D36 §2).
    contract: Contract,
    current: RwLock<Arc<Version>>,
    /// One install at a time, so generations follow each other.
    installing: Mutex<()>,
}

/// A load that was tried: its outcome, and the files it read on the way - kept
/// even when it failed, so the watcher can tell a half-written file from a
/// broken one (D36 §6).
struct Attempt {
    result: Result<(Loaded, Lockfile), Error>,
    files: Vec<(PathBuf, String)>,
    /// The generation it was loaded against; installed only onto that one.
    base: u64,
}

/// A loaded script. `Send + Sync`: one instance serves every thread, and a
/// call takes `&self`.
pub struct Script {
    shared: Arc<Shared>,
    watcher: Option<Watcher>,
}

impl Script {
    /// Load a file with no grants. Hermetic: the load fails if the script
    /// could read anything at all.
    pub fn load(path: impl AsRef<Path>) -> Result<Script, Error> {
        Self::builder().file(path).load()
    }

    pub fn builder() -> ScriptBuilder {
        ScriptBuilder::default()
    }

    fn build(mut b: ScriptBuilder) -> Result<Script, Error> {
        let host = Arc::new(host_table(&mut b)?);
        // An effect is a grant like any other (D35 §2); a pure function is not.
        let granted = b.env.is_some() || !b.read.is_empty() || host.has_effects();
        let env_cap = match &b.env {
            None => EnvCap::Deny,
            Some(names) => EnvCap::Allow(names.clone()),
        };
        let watch = b.watch.take();
        if let (Some((interval, _)), Some(settle)) = (&watch, b.settle) {
            if settle < *interval {
                return Err(Error::host(
                    "E0001",
                    format!(
                        "settle ({settle:?}) is shorter than the watch interval ({interval:?}): \
                         the files would be read before anyone looked at them twice"
                    ),
                ));
            }
        }
        if watch.is_some() && b.file.is_none() {
            return Err(Error::host(
                "E0001",
                "only a script loaded from a file can be watched; an in-memory source has no file",
            ));
        }
        let config = Config {
            sources: std::mem::take(&mut b.sources),
            entry: b.entry.take(),
            file: b.file.take(),
            read: std::mem::take(&mut b.read),
            env_cap,
            granted,
            strict: b.strict,
            budget: b.budget,
            host,
        };

        let lock = match &config.file {
            Some(path) => read_lock(path)?,
            None => Lockfile::default(),
        };
        let attempt = load_version(&config, lock, false, 0);
        let (loaded, lock) = attempt.result?;
        let contract = contract_of(&loaded);
        let version = Version {
            loaded,
            id: ScriptVersion {
                generation: 1,
                hash: hash_files(&attempt.files),
            },
            files: attempt.files,
        };
        let shared = Arc::new(Shared {
            config,
            lock,
            contract,
            current: RwLock::new(Arc::new(version)),
            installing: Mutex::new(()),
        });
        let settle = b.settle;
        let watcher = watch.map(|(interval, on_reload)| {
            let settle = settle.unwrap_or_else(|| default_settle(interval));
            Watcher::start(&shared, interval, settle, on_reload)
        });
        Ok(Script { shared, watcher })
    }

    /// The version serving now.
    pub fn version(&self) -> ScriptVersion {
        self.shared.current().id.clone()
    }

    /// Read the script's files again and, if they changed, load them as a new
    /// version (D36). The new version passes every check the first load did,
    /// with the same grants, against the same lock, and keeps every export the
    /// host may have bound; if anything fails, the old version keeps serving
    /// and the error says why. Content decides: files rewritten with the same
    /// bytes are not a new version.
    pub fn reload(&self) -> Result<ScriptVersion, Error> {
        if self.shared.config.file.is_none() {
            return Err(Error::host(
                "E0001",
                "a script built from in-memory sources has no files to reload; build a new Script",
            ));
        }
        let attempt = self.shared.attempt();
        Ok(self
            .shared
            .install(attempt)?
            .unwrap_or_else(|| self.version()))
    }

    /// A value the script exports, as any `Deserialize` type.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<T, Error> {
        refuse_reentry()?;
        let current = self.shared.current();
        let json = current.loaded.with_dependent(|cache, state| {
            let v = state.exports.get(name).ok_or_else(|| no_export(name))?;
            crate::serialize::to_json(v).map_err(|d| Error::from(vec![to_report(&d, cache)]))
        })?;
        from_json(json)
    }

    /// Call a function by name. For a function called often, bind it once with
    /// [`Script::function`] instead: the lookup and the checks then happen once.
    pub fn call<A: Args, R: DeserializeOwned>(&self, name: &str, args: A) -> Result<R, Error> {
        self.function::<A, R>(name)?.call(args)
    }

    /// Bind a function, checking now - not on the first request - that the name
    /// exists (`E0002`), names a function (`E0003`), and takes as many
    /// arguments as `A` has (`E0004`). A reload keeps all three true (D36 §2).
    pub fn function<A: Args, R: DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Function<'_, A, R>, Error> {
        self.shared.current().loaded.with_dependent(|_, state| {
            let value = state.exports.get(name).ok_or_else(|| no_export(name))?;
            let Value::Function(def) = value else {
                return Err(Error::host(
                    "E0003",
                    format!("'{name}' is a {}, not a function", value.type_name()),
                ));
            };
            if def.params.len() != A::ARITY {
                return Err(Error::host(
                    "E0004",
                    format!(
                        "'{name}' takes {} argument{}, but the host passes {}",
                        def.params.len(),
                        if def.params.len() == 1 { "" } else { "s" },
                        A::ARITY
                    ),
                ));
            }
            Ok(())
        })?;
        Ok(Function {
            script: self,
            name: name.to_string(),
            _types: PhantomData,
        })
    }

    /// One call, start to finish, in its own interpreter, on the version that
    /// was current when it started (D36 §9).
    fn invoke<A: Args>(&self, name: &str, args: A) -> Result<serde_json::Value, Error> {
        refuse_reentry()?;
        let budget = self.shared.config.budget;
        let current = self.shared.current();
        current.loaded.with_dependent(|cache, state| {
            // Found by name, not by position: a reload may reorder the exports
            // (D36 §2), and the contract guarantees the name is still there.
            let f = state.exports.get(name).ok_or_else(|| no_export(name))?;
            let mut it = Interpreter::with_registry(
                state.registry.clone(),
                Options {
                    strict: state.strict,
                    dry_run: false,
                },
            );
            it.fs = Box::new(SharedFs(state.fs.clone()));
            it.env_cap = state.env_cap.clone();
            it.host_call_steps = state.host_call_steps;
            let _meter = budget.begin(&mut it);

            let args = args.to_values().map_err(|e| Error::host("E0005", e.0))?;
            let span = Span::new(0, 0, 0);
            let result = it
                .call_value(f, &args, span)
                .map_err(|d| Error::from(vec![to_report(&d, cache)]))?;
            // Converted before `it` is dropped, which is what releases every
            // environment the call created.
            crate::serialize::to_json(&result).map_err(|d| {
                let mut e = Error::from(vec![to_report(&d, cache)]);
                e.reports[0].code = "E0006";
                e
            })
        })
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        if let Some(w) = self.watcher.take() {
            w.stop();
        }
    }
}

impl Shared {
    fn current(&self) -> Arc<Version> {
        // A poisoned lock means a thread panicked while swapping; the Arc it
        // holds is always a whole version, so it is safe to keep serving it.
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Load the files as they are on disk now, frozen to the first lock.
    fn attempt(&self) -> Attempt {
        let base = self.current().id.generation;
        load_version(&self.config, self.lock.clone(), true, base)
    }

    /// Make an attempt the current version. `Ok(None)` when its content is the
    /// content already serving, or another reload got there first.
    fn install(&self, attempt: Attempt) -> Result<Option<ScriptVersion>, Error> {
        let (loaded, _) = attempt.result?;
        let _one = self.installing.lock().unwrap_or_else(|e| e.into_inner());
        let current = self.current();
        let hash = hash_files(&attempt.files);
        if hash == current.id.hash || current.id.generation != attempt.base {
            return Ok(None);
        }
        check_contract(&self.contract, &contract_of(&loaded))?;
        let id = ScriptVersion {
            generation: current.id.generation + 1,
            hash,
        };
        let version = Arc::new(Version {
            loaded,
            id: id.clone(),
            files: attempt.files,
        });
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = version;
        Ok(Some(id))
    }
}

/// `aura.lock` beside the entry file, if there is one.
fn read_lock(path: &Path) -> Result<Lockfile, Error> {
    let entry = std::fs::canonicalize(path)
        .map_err(|e| Error::host("E0001", format!("cannot open {}: {e}", path.display())))?;
    let root = entry.parent().unwrap_or(Path::new("."));
    match std::fs::read_to_string(root.join("aura.lock")) {
        Ok(text) => Lockfile::parse(&text)
            .map_err(|e| Error::host("E0001", format!("invalid aura.lock: {e}"))),
        Err(_) => Ok(Lockfile::default()),
    }
}

/// One load, start to finish: resolve, analyse, check the host functions,
/// evaluate under the budget. The same function serves the first load and
/// every reload, which is what makes a reload no weaker than a load (D36 §3).
fn load_version(config: &Config, lock: Lockfile, frozen: bool, base: u64) -> Attempt {
    let mut files = Vec::new();
    let mut lock_after = None;
    let result = Loaded::try_new(SourceCache::new(), |cache| -> Result<State<'_>, Error> {
        // Where modules and file reads come from: memory or disk.
        let (resolver, fs, entry): (Box<dyn FileResolver>, Arc<dyn FileAccess>, String) =
            match &config.file {
                Some(path) => {
                    let entry = std::fs::canonicalize(path).map_err(|e| {
                        Error::host("E0001", format!("cannot open {}: {e}", path.display()))
                    })?;
                    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let registry_dir = std::env::var_os("USERPROFILE")
                        .or_else(|| std::env::var_os("HOME"))
                        .map(PathBuf::from)
                        .unwrap_or_default()
                        .join(".aura")
                        .join("registry");
                    let fs: Arc<dyn FileAccess> = if config.read.is_empty() {
                        Arc::new(DenyFs)
                    } else {
                        Arc::new(RealFs {
                            allowed: config.read.clone(),
                        })
                    };
                    let name = entry
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    (Box::new(LocalFsResolver { root, registry_dir }), fs, name)
                }
                None => {
                    let entry = config
                        .entry
                        .clone()
                        .or_else(|| config.sources.first().map(|(n, _)| n.clone()))
                        .ok_or_else(|| Error::host("E0001", "no source was given"))?;
                    let files: HashMap<String, String> = config.sources.iter().cloned().collect();
                    let fs: Arc<dyn FileAccess> = if config.read.is_empty() {
                        Arc::new(DenyFs)
                    } else {
                        Arc::new(MemFs(files.clone()))
                    };
                    (Box::new(MemoryResolver { files }), fs, entry)
                }
            };
        let recording = Recording {
            inner: resolver.as_ref(),
            seen: RefCell::new(Vec::new()),
        };

        let mut loader = Loader::new(cache, &recording);
        // D33 §1: nothing granted, so prove nothing is used.
        loader.hermetic = !config.granted;
        loader.lock = lock;
        loader.frozen = frozen;
        loader.host = config.host.clone();

        let registry = Arc::new(MethodRegistry::builtin());
        let mut interp = Interpreter::with_registry(
            registry.clone(),
            Options {
                strict: config.strict,
                dry_run: false,
            },
        );
        interp.fs = Box::new(SharedFs(fs.clone()));
        interp.env_cap = config.env_cap.clone();
        interp.host_call_steps = config.host.call_steps;
        // Loading is held to the call budget: a script that cannot finish
        // evaluating its own module must not hang the host's startup, nor a
        // reload (D36 §10).
        let _meter = config.budget.begin(&mut interp);

        let value = loader.eval_entry(&mut interp, &ImportSpec::File(&entry));
        files = recording.seen.take();
        let mut reports: Vec<Report> = loader.diags.iter().map(|d| to_report(d, cache)).collect();
        let value = match value {
            Ok(v) => v,
            Err(d) => {
                reports.insert(0, to_report(&d, cache));
                return Err(reports.into());
            }
        };
        if crate::analysis::has_blocking(&loader.diags, config.strict) {
            return Err(reports.into());
        }
        let Value::Object(exports) = value else {
            return Err(Error::host("E0001", "a module evaluates to an object"));
        };
        lock_after = Some(std::mem::take(&mut loader.lock));

        Ok(State {
            exports: (*exports).clone(),
            registry,
            fs,
            env_cap: config.env_cap.clone(),
            host_call_steps: config.host.call_steps,
            strict: config.strict,
            _closures: interp,
        })
    });
    Attempt {
        result: result.map(|loaded| (loaded, lock_after.unwrap_or_default())),
        files,
        base,
    }
}

/// Passes resolution through, and remembers every local file it was asked to
/// load and what it read: the file set of a version (D36 §5).
struct Recording<'r> {
    inner: &'r dyn FileResolver,
    seen: RefCell<Vec<(PathBuf, String)>>,
}

impl FileResolver for Recording<'_> {
    fn resolve(
        &self,
        spec: &ImportSpec<'_>,
        importer: Option<&ModuleId>,
    ) -> Result<ModuleId, String> {
        self.inner.resolve(spec, importer)
    }

    fn load(&self, id: &ModuleId) -> Result<String, String> {
        let text = self.inner.load(id)?;
        if let ModuleId::Local(path) = id {
            self.seen.borrow_mut().push((path.clone(), text.clone()));
        }
        Ok(text)
    }

    fn dir_of(&self, id: &ModuleId) -> Option<PathBuf> {
        self.inner.dir_of(id)
    }
}

/// SHA-256 over every file's path and content, in path order.
fn hash_files(files: &[(PathBuf, String)]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (path, text) in sorted {
        h.update(path.to_string_lossy().as_bytes());
        h.update([0]);
        h.update(text.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn contract_of(loaded: &Loaded) -> Contract {
    loaded.with_dependent(|_, s| {
        s.exports
            .iter()
            .map(|(k, v)| {
                let arity = match v {
                    Value::Function(d) => Some(d.params.len()),
                    _ => None,
                };
                (k.clone(), arity)
            })
            .collect()
    })
}

/// D36 §2: everything the first version exported is still there, of the same
/// kind and arity. New exports are fine - nothing the host bound can use them.
fn check_contract(fixed: &Contract, new: &Contract) -> Result<(), Error> {
    let describe = |k: &Option<usize>| match k {
        Some(n) => format!(
            "a function of {n} argument{}",
            if *n == 1 { "" } else { "s" }
        ),
        None => "a value".to_string(),
    };
    for (name, kind) in fixed {
        let msg = match new.iter().find(|(n, _)| n == name) {
            None => format!("the new version no longer exports '{name}'"),
            Some((_, k)) if k != kind => format!(
                "'{name}' was {}, and the new version makes it {}",
                describe(kind),
                describe(k)
            ),
            Some(_) => continue,
        };
        let mut e = Error::host("E0011", msg);
        e.reports[0].help = Some(
            "the host may have bound it, so the reload was refused and the old version keeps \
             serving; restart the application to change what the script exports"
                .to_string(),
        );
        return Err(e);
    }
    Ok(())
}

/// The thread that watches a script's files (D36 §1). It holds the script only
/// weakly and stops when the `Script` is dropped.
struct Watcher {
    stop: mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

/// The handler `watch` requires (D36 §12).
type OnReload = Box<dyn Fn(Result<ScriptVersion, Error>) + Send>;

impl Watcher {
    fn start(
        shared: &Arc<Shared>,
        interval: Duration,
        settle: Duration,
        on_reload: OnReload,
    ) -> Watcher {
        let (stop, stopped) = mpsc::channel();
        let weak = Arc::downgrade(shared);
        let thread = std::thread::Builder::new()
            .name("aura-watch".into())
            .spawn(move || watch_loop(weak, interval, settle, stopped, on_reload))
            .expect("spawn the watcher thread");
        Watcher { stop, thread }
    }

    fn stop(self) {
        drop(self.stop);
        // The handler may own the last handle on the script, and so drop it on
        // the watcher's own thread: joining there would wait for itself.
        if self.thread.thread().id() != std::thread::current().id() {
            let _ = self.thread.join();
        }
    }
}

/// Waits one interval; false when the script is gone and the watcher must end.
fn tick(stopped: &mpsc::Receiver<()>, interval: Duration) -> bool {
    matches!(
        stopped.recv_timeout(interval),
        Err(mpsc::RecvTimeoutError::Timeout)
    )
}

/// What the file system says about the files without reading them: a cheap
/// reason to look, never a reason to reload (D36 §1).
type Stats = Vec<Option<(u64, Option<std::time::SystemTime>)>>;

fn stats(files: &[(PathBuf, String)]) -> Stats {
    files
        .iter()
        .map(|(p, _)| {
            std::fs::metadata(p)
                .ok()
                .map(|m| (m.len(), m.modified().ok()))
        })
        .collect()
}

/// Every file read again, with the file system's word that nothing wrote to
/// them while they were read: size and modification time the same before and
/// after. `None` if one can no longer be read, or something wrote meanwhile.
fn read_still(files: &[(PathBuf, String)]) -> Option<(Stats, Vec<(PathBuf, String)>)> {
    let before = stats(files);
    let read = reread(files)?;
    (stats(files) == before).then_some((before, read))
}

/// Every file read again; `None` if one can no longer be read.
fn reread(files: &[(PathBuf, String)]) -> Option<Vec<(PathBuf, String)>> {
    files
        .iter()
        .map(|(p, _)| std::fs::read_to_string(p).ok().map(|t| (p.clone(), t)))
        .collect()
}

/// D36 §6: when the files may be read for a reload. Pure - no clock, no file
/// system - so the rule is tested with made-up times, not with the luck of a
/// scheduler.
///
/// The rule is a quiet period: the file system has reported the same size and
/// modification time for every file for `settle`. Two reads that agree are not
/// enough - a writer that stalls between truncating a file and filling it
/// leaves one that reads the same, empty, for as long as it stalls; a writer
/// cut off at the end of a rule leaves one that is valid and loads. A quiet
/// period longer than any such stall is what keeps a half-written file out.
#[derive(Debug)]
struct Settle {
    settle: Duration,
    /// What the files looked like, and since when.
    seen: Option<(Stats, Instant)>,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Something changed, or has not been still for long enough.
    Wait,
    /// Still for `settle`: read the files.
    Look,
}

impl Settle {
    fn new(settle: Duration) -> Self {
        Settle { settle, seen: None }
    }

    fn observe(&mut self, now: Instant, stats: &Stats) -> Step {
        match &self.seen {
            Some((s, since)) if s == stats => {
                if now.saturating_duration_since(*since) >= self.settle {
                    Step::Look
                } else {
                    Step::Wait
                }
            }
            _ => {
                self.seen = Some((stats.clone(), now));
                Step::Wait
            }
        }
    }

    /// Start counting again, from the next observation.
    fn reset(&mut self) {
        self.seen = None;
    }
}

/// The settle period when the host does not set one: two intervals, and never
/// less than a second - longer than any pause a writer takes between two
/// writes on a busy machine, and short beside how often rules are edited.
fn default_settle(interval: Duration) -> Duration {
    (interval * 2).max(Duration::from_secs(1))
}

fn watch_loop(
    shared: std::sync::Weak<Shared>,
    interval: Duration,
    settle: Duration,
    stopped: mpsc::Receiver<()>,
    on_reload: OnReload,
) {
    let mut gate = Settle::new(settle);
    // The files and the state the serving version was read from: nothing to do
    // while they look like this.
    let mut quiet: Option<Stats> = None;
    // Content already reported as failing, by the state it was read in.
    let mut failed: Option<Stats> = None;
    // The files to watch: the serving version's, or - once a load found new
    // imports - the set that load read, so they settle too.
    let mut tracked: Option<Vec<(PathBuf, String)>> = None;
    loop {
        if !tick(&stopped, interval) {
            return;
        }
        let Some(script) = shared.upgrade() else {
            return;
        };
        let current = script.current();
        let files = tracked.clone().unwrap_or_else(|| current.files.clone());
        let now = stats(&files);
        if quiet.as_ref() == Some(&now) || failed.as_ref() == Some(&now) {
            gate.reset();
            continue;
        }
        if gate.observe(Instant::now(), &now) == Step::Wait {
            continue;
        }
        // Still for the whole settle period. Only content counts now: a touch
        // leaves the hash as it was, and nothing happens.
        let Some((read_stats, read)) = read_still(&files) else {
            gate.reset();
            continue;
        };
        if read_stats != now {
            gate.reset();
            continue;
        }
        if tracked.is_none() && hash_files(&read) == current.id.hash {
            quiet = Some(now);
            gate.reset();
            continue;
        }
        let attempt = script.attempt();
        drop(current);

        // The load read the files once more. If it found a different set -
        // a new import - those files have not settled yet: watch the new set
        // and try again once it has.
        let same_set = attempt.files.len() == files.len()
            && attempt.files.iter().zip(&files).all(|(a, b)| a.0 == b.0);
        if !same_set {
            tracked = Some(attempt.files.clone());
            gate.reset();
            continue;
        }
        // And it must have read what was still - nothing written meanwhile.
        if attempt.files != read || stats(&attempt.files) != now {
            gate.reset();
            continue;
        }
        tracked = None;
        match script.install(attempt) {
            Ok(Some(v)) => {
                failed = None;
                quiet = None;
                on_reload(Ok(v));
            }
            Ok(None) => quiet = Some(now),
            Err(e) => {
                failed = Some(now);
                on_reload(Err(e));
            }
        }
        gate.reset();
    }
}

/// D35 §6: a host function may not call back into a script. Re-entrancy across
/// the boundary is the class of defect behind the 2016 DAO exploit; a closure
/// can capture the script, so the type system cannot rule it out.
fn refuse_reentry() -> Result<(), Error> {
    if crate::host::inside_host() {
        return Err(Error::host(
            "E0010",
            "a script was called from inside one of its host functions",
        ));
    }
    Ok(())
}

/// The functions the builder was given, checked as a whole: each named
/// `module.function`, none twice, none shadowed by a built-in object method,
/// and every name granted to imports one that exists.
fn host_table(b: &mut ScriptBuilder) -> Result<HostTable, Error> {
    let registry = MethodRegistry::builtin();
    let mut table = HostTable::new();
    if let Some(steps) = b.host_call_steps {
        table.call_steps = steps;
    }
    let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    for mut f in std::mem::take(&mut b.host) {
        let Some((module, func)) = f.name.split_once('.').filter(|(m, n)| ident(m) && ident(n))
        else {
            return Err(Error::host(
                "E0001",
                format!(
                    "host function '{}' must be named module.function, like \"geo.region\"",
                    f.name
                ),
            ));
        };
        if table.get(module, func).is_some() {
            return Err(Error::host(
                "E0001",
                format!("host function '{}' is provided twice", f.name),
            ));
        }
        // Builtin methods win over an object's fields (D12), so `geo.keys()`
        // would never reach the host. Refused here rather than shadowed silently.
        if registry
            .get(crate::eval::value::TypeTag::Object, func)
            .is_some()
        {
            return Err(Error::host(
                "E0001",
                format!(
                    "host function '{}' is named like the built-in object method '{func}'",
                    f.name
                ),
            ));
        }
        f.for_imports = b.host_for_imports.contains(&f.name);
        table.insert(f);
    }
    for n in &b.host_for_imports {
        if !table.fns().any(|f| f.name == *n) {
            return Err(Error::host(
                "E0001",
                format!("grant_to_imports names '{n}', which is not provided"),
            ));
        }
    }
    Ok(table)
}

/// A Rust closure that can be given to [`ScriptBuilder::pure`]: up to six
/// arguments that `Deserialize`, and a result that `Serialize`s.
pub trait PureFn<Args>: Send + Sync + 'static {
    const ARITY: usize;
    #[doc(hidden)]
    fn erase(self) -> Arc<HostCall>;
}

/// A Rust closure that can be given to [`ScriptBuilder::effect`]: as
/// [`PureFn`], returning `Result<R, E>` where `E` explains the failure.
pub trait EffectFn<Args>: Send + Sync + 'static {
    const ARITY: usize;
    #[doc(hidden)]
    fn erase(self) -> Arc<HostCall>;
}

/// A host result into an Aura value, by the same exact rules as arguments.
fn host_result<R: Serialize>(r: R) -> HostOutcome {
    match r.serialize(ToValue(PhantomData)) {
        Ok(v) => HostOutcome::Value(v),
        Err(e) => HostOutcome::BadArgument(format!("its result: {e}")),
    }
}

macro_rules! host_fn {
    ($n:expr $(, $t:ident $v:ident $i:tt)*) => {
        impl<F, R, $($t),*> PureFn<($($t,)*)> for F
        where
            F: Fn($($t),*) -> R + Send + Sync + 'static,
            R: Serialize,
            $($t: DeserializeOwned,)*
        {
            const ARITY: usize = $n;
            fn erase(self) -> Arc<HostCall> {
                Arc::new(move |args: &[serde_json::Value]| {
                    $(
                        let $v: $t = match serde_json::from_value(args[$i].clone()) {
                            Ok(v) => v,
                            Err(e) => {
                                return HostOutcome::BadArgument(format!("argument {}: {e}", $i + 1))
                            }
                        };
                    )*
                    let _ = args;
                    host_result(self($($v),*))
                })
            }
        }

        impl<F, R, Er, $($t),*> EffectFn<($($t,)*)> for F
        where
            F: Fn($($t),*) -> Result<R, Er> + Send + Sync + 'static,
            R: Serialize,
            Er: fmt::Display,
            $($t: DeserializeOwned,)*
        {
            const ARITY: usize = $n;
            fn erase(self) -> Arc<HostCall> {
                Arc::new(move |args: &[serde_json::Value]| {
                    $(
                        let $v: $t = match serde_json::from_value(args[$i].clone()) {
                            Ok(v) => v,
                            Err(e) => {
                                return HostOutcome::BadArgument(format!("argument {}: {e}", $i + 1))
                            }
                        };
                    )*
                    let _ = args;
                    match self($($v),*) {
                        Ok(r) => host_result(r),
                        Err(e) => HostOutcome::Failed(e.to_string()),
                    }
                })
            }
        }
    };
}

host_fn!(0);
host_fn!(1, A a 0);
host_fn!(2, A a 0, B b 1);
host_fn!(3, A a 0, B b 1, C c 2);
host_fn!(4, A a 0, B b 1, C c 2, D d 3);
host_fn!(5, A a 0, B b 1, C c 2, D d 3, E e 4);
host_fn!(6, A a 0, B b 1, C c 2, D d 3, E e 4, G g 5);

fn no_export(name: &str) -> Error {
    Error::host(
        "E0002",
        format!("the script exports nothing named '{name}'"),
    )
}

fn from_json<T: DeserializeOwned>(json: serde_json::Value) -> Result<T, Error> {
    serde_path_to_error::deserialize(json).map_err(|e| {
        let path = e.path().to_string();
        Error::host(
            "E0006",
            format!(
                "the result does not match the host's type at {path}: {}",
                e.inner()
            ),
        )
    })
}

/// A bound function: the checks are done, so a call only converts, runs and
/// converts back.
pub struct Function<'s, A, R> {
    script: &'s Script,
    name: String,
    _types: PhantomData<fn(A) -> R>,
}

impl<A: Args, R: DeserializeOwned> Function<'_, A, R> {
    pub fn call(&self, args: A) -> Result<R, Error> {
        from_json(self.script.invoke(&self.name, args)?)
    }
}

impl<A, R> fmt::Debug for Function<'_, A, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .finish()
    }
}

impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let current = self.shared.current();
        let exports: Vec<String> = current
            .loaded
            .with_dependent(|_, s| s.exports.keys().cloned().collect());
        f.debug_struct("Script")
            .field("exports", &exports)
            .field("version", &current.id)
            .field("budget", &self.shared.config.budget)
            .finish()
    }
}

/// Reads through a grant shared by every call of one script.
struct SharedFs(Arc<dyn FileAccess>);

impl FileAccess for SharedFs {
    fn read(&self, path: &str) -> Result<String, FileError> {
        self.0.read(path)
    }
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// The arguments of a call: a tuple of `Serialize` values. The arity is a
/// constant, so binding a function can check it before any call is made.
pub trait Args {
    const ARITY: usize;
    #[doc(hidden)]
    fn to_values<'a>(&self) -> Result<Vec<Value<'a>>, BoundaryError>;
}

macro_rules! args_tuple {
    ($n:expr $(, $t:ident $i:tt)*) => {
        impl<$($t: Serialize),*> Args for ($($t,)*) {
            const ARITY: usize = $n;
            #[allow(unused_variables)]
            fn to_values<'a>(&self) -> Result<Vec<Value<'a>>, BoundaryError> {
                Ok(vec![$(self.$i.serialize(ToValue(PhantomData))?),*])
            }
        }
    };
}

args_tuple!(0);
args_tuple!(1, A 0);
args_tuple!(2, A 0, B 1);
args_tuple!(3, A 0, B 1, C 2);
args_tuple!(4, A 0, B 1, C 2, D 3);
args_tuple!(5, A 0, B 1, C 2, D 3, E 4);
args_tuple!(6, A 0, B 1, C 2, D 3, E 4, F 5);

/// A host value Aura cannot hold exactly. Refused rather than coerced: an
/// integer wrapped, or a NaN turned into `null`, would be a different input
/// than the host meant to give.
#[derive(Debug)]
pub struct BoundaryError(String);

impl fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BoundaryError {}

impl serde::ser::Error for BoundaryError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        BoundaryError(msg.to_string())
    }
}

/// Serialises a host value straight into an Aura value — no JSON detour, so no
/// precision is lost on the way and no NaN quietly becomes `null`.
struct ToValue<'a>(PhantomData<Value<'a>>);

fn refuse<T>(what: impl fmt::Display) -> Result<T, BoundaryError> {
    Err(BoundaryError(format!("{what} has no exact Aura value")))
}

impl<'a> serde::Serializer for ToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    type SerializeSeq = SeqToValue<'a>;
    type SerializeTuple = SeqToValue<'a>;
    type SerializeTupleStruct = SeqToValue<'a>;
    type SerializeTupleVariant = VariantToValue<SeqToValue<'a>>;
    type SerializeMap = MapToValue<'a>;
    type SerializeStruct = MapToValue<'a>;
    type SerializeStructVariant = VariantToValue<MapToValue<'a>>;

    fn serialize_bool(self, v: bool) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_i16(self, v: i16) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_i32(self, v: i32) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_i64(self, v: i64) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v))
    }
    fn serialize_i128(self, v: i128) -> Result<Value<'a>, BoundaryError> {
        i64::try_from(v).map(Value::Int).or_else(|_| refuse(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_u16(self, v: u16) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_u32(self, v: u32) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Int(v.into()))
    }
    fn serialize_u64(self, v: u64) -> Result<Value<'a>, BoundaryError> {
        i64::try_from(v).map(Value::Int).or_else(|_| refuse(v))
    }
    fn serialize_u128(self, v: u128) -> Result<Value<'a>, BoundaryError> {
        i64::try_from(v).map(Value::Int).or_else(|_| refuse(v))
    }
    fn serialize_f32(self, v: f32) -> Result<Value<'a>, BoundaryError> {
        self.serialize_f64(v.into())
    }
    fn serialize_f64(self, v: f64) -> Result<Value<'a>, BoundaryError> {
        if v.is_finite() {
            Ok(Value::Float(v))
        } else {
            refuse(v)
        }
    }
    fn serialize_char(self, v: char) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::str(v.encode_utf8(&mut [0; 4])))
    }
    fn serialize_str(self, v: &str) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::str(v))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Value<'a>, BoundaryError> {
        refuse("a byte string")
    }
    fn serialize_none(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<Value<'a>, BoundaryError> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::str(variant))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Value<'a>, BoundaryError> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<Value<'a>, BoundaryError> {
        let inner = v.serialize(ToValue(PhantomData))?;
        Ok(Value::object(IndexMap::from([(
            variant.to_string(),
            inner,
        )])))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqToValue<'a>, BoundaryError> {
        Ok(SeqToValue(Vec::with_capacity(len.unwrap_or(0))))
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqToValue<'a>, BoundaryError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<SeqToValue<'a>, BoundaryError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, BoundaryError> {
        Ok(VariantToValue(variant, self.serialize_seq(Some(len))?))
    }
    fn serialize_map(self, len: Option<usize>) -> Result<MapToValue<'a>, BoundaryError> {
        Ok(MapToValue {
            map: IndexMap::with_capacity(len.unwrap_or(0)),
            key: None,
        })
    }
    fn serialize_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<MapToValue<'a>, BoundaryError> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, BoundaryError> {
        Ok(VariantToValue(variant, self.serialize_map(Some(len))?))
    }
}

struct SeqToValue<'a>(Vec<Value<'a>>);

impl<'a> SeqToValue<'a> {
    fn push<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        self.0.push(v.serialize(ToValue(PhantomData))?);
        Ok(())
    }
}

impl<'a> serde::ser::SerializeSeq for SeqToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        self.push(v)
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::list(self.0))
    }
}

impl<'a> serde::ser::SerializeTuple for SeqToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        self.push(v)
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::list(self.0))
    }
}

impl<'a> serde::ser::SerializeTupleStruct for SeqToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        self.push(v)
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::list(self.0))
    }
}

struct MapToValue<'a> {
    map: IndexMap<String, Value<'a>>,
    key: Option<String>,
}

impl<'a> MapToValue<'a> {
    fn finish(self) -> Value<'a> {
        Value::object(self.map)
    }
}

impl<'a> serde::ser::SerializeMap for MapToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), BoundaryError> {
        match key.serialize(ToValue(PhantomData))? {
            Value::Str(s) => {
                self.key = Some(s.to_string());
                Ok(())
            }
            other => refuse(format!("a map key of type {}", other.type_name())),
        }
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        let key = self.key.take().expect("serde calls serialize_key first");
        self.map.insert(key, v.serialize(ToValue(PhantomData))?);
        Ok(())
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(self.finish())
    }
}

impl<'a> serde::ser::SerializeStruct for MapToValue<'a> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), BoundaryError> {
        self.map
            .insert(key.to_string(), v.serialize(ToValue(PhantomData))?);
        Ok(())
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(self.finish())
    }
}

/// An enum variant carrying data: `{ variant: data }`, as serde's externally
/// tagged default.
struct VariantToValue<S>(&'static str, S);

impl<'a> serde::ser::SerializeTupleVariant for VariantToValue<SeqToValue<'a>> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), BoundaryError> {
        self.1.push(v)
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::object(IndexMap::from([(
            self.0.to_string(),
            Value::list(self.1 .0),
        )])))
    }
}

impl<'a> serde::ser::SerializeStructVariant for VariantToValue<MapToValue<'a>> {
    type Ok = Value<'a>;
    type Error = BoundaryError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), BoundaryError> {
        serde::ser::SerializeStruct::serialize_field(&mut self.1, key, v)
    }
    fn end(self) -> Result<Value<'a>, BoundaryError> {
        Ok(Value::object(IndexMap::from([(
            self.0.to_string(),
            self.1.finish(),
        )])))
    }
}

#[cfg(test)]
mod tests {
    // D36 §6, decided without a clock or a file system: each case is a story
    // of what the file system reported and when, and what the watcher may do.
    mod settle {
        use super::super::{Settle, Stats, Step};
        use std::time::{Duration, Instant, SystemTime};

        fn file(len: u64, mtime_ms: u64) -> Stats {
            vec![Some((
                len,
                Some(SystemTime::UNIX_EPOCH + Duration::from_millis(mtime_ms)),
            ))]
        }

        /// Feeds `(ms, stats)` observations and returns the steps.
        fn run(settle_ms: u64, story: &[(u64, Stats)]) -> Vec<Step> {
            let t0 = Instant::now();
            let mut g = Settle::new(Duration::from_millis(settle_ms));
            story
                .iter()
                .map(|(ms, s)| g.observe(t0 + Duration::from_millis(*ms), s))
                .collect()
        }

        #[test]
        fn files_left_alone_for_the_settle_period_are_read() {
            let s = file(100, 5);
            let steps = run(1000, &[(0, s.clone()), (500, s.clone()), (1000, s)]);
            assert_eq!(steps, [Step::Wait, Step::Wait, Step::Look]);
        }

        /// The CI case: a writer stalls between truncating and filling for
        /// longer than two short intervals. The file reads empty and still -
        /// but not for the settle period, so it is never read.
        #[test]
        fn a_writer_stalled_mid_write_is_not_read() {
            let empty = file(0, 40);
            let full = file(100, 90);
            let steps = run(
                1000,
                &[
                    (40, empty.clone()),
                    (60, empty.clone()),
                    (80, empty),
                    (100, full),
                ],
            );
            assert!(!steps.contains(&Step::Look), "{steps:?}");
        }

        /// A writer cut off at the end of a rule leaves a valid file. Still
        /// for 600 ms of a 1 s settle, then the rest arrives: never read.
        #[test]
        fn a_valid_prefix_that_does_not_last_is_not_read() {
            let prefix = file(60, 10);
            let whole = file(100, 610);
            let mut story: Vec<(u64, Stats)> =
                (0..=6).map(|i| (10 + i * 100, prefix.clone())).collect();
            story.push((610, whole.clone()));
            story.push((1600, whole.clone()));
            story.push((1610, whole));
            let steps = run(1000, &story);
            let looks: Vec<_> = steps
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == Step::Look)
                .collect();
            assert_eq!(looks.len(), 1, "{steps:?}");
            assert_eq!(
                looks[0].0,
                story.len() - 1,
                "read before the whole file settled"
            );
        }

        /// Any change - size or modification time - starts the count again.
        #[test]
        fn every_change_restarts_the_count() {
            let steps = run(
                1000,
                &[
                    (0, file(10, 0)),
                    (900, file(10, 900)),  // touched: mtime moved
                    (1800, file(11, 900)), // grew
                    (2700, file(11, 900)),
                    (2800, file(11, 900)),
                ],
            );
            assert_eq!(
                steps,
                [Step::Wait, Step::Wait, Step::Wait, Step::Wait, Step::Look]
            );
        }

        #[test]
        fn reset_starts_over() {
            let s = file(1, 1);
            let t0 = Instant::now();
            let mut g = Settle::new(Duration::from_millis(100));
            g.observe(t0, &s);
            g.reset();
            assert_eq!(g.observe(t0 + Duration::from_millis(200), &s), Step::Wait);
            assert_eq!(g.observe(t0 + Duration::from_millis(300), &s), Step::Look);
        }

        #[test]
        fn the_default_is_two_intervals_and_at_least_a_second() {
            use super::super::default_settle;
            assert_eq!(
                default_settle(Duration::from_millis(20)),
                Duration::from_secs(1)
            );
            assert_eq!(
                default_settle(Duration::from_secs(2)),
                Duration::from_secs(4)
            );
        }
    }

    use super::*;

    #[test]
    fn a_script_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Script>();
    }

    #[test]
    fn diagnostics_render_as_the_cli_does() {
        let e = Error::host("E0002", "the script exports nothing named 'g'");
        assert_eq!(e.code(), "E0002");
        assert!(e.to_string().contains("E0002"));
    }
}
