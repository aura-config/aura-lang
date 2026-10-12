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

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::Severity;
use crate::eval::budget::Budget;
use crate::eval::methods::MethodRegistry;
use crate::eval::value::Value;
use crate::eval::{DenyFs, EnvCap, FileAccess, FileError, Interpreter, MemFs, Options, RealFs};
use crate::facade::{to_report, Report};
use crate::source::SourceCache;
use crate::span::Span;
use crate::vfs::loader::Loader;
use crate::vfs::lockfile::Lockfile;
use crate::vfs::{FileResolver, ImportSpec, LocalFsResolver, MemoryResolver};

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

/// A loaded script. `Send + Sync`: one instance serves every thread, and a
/// call takes `&self`.
pub struct Script {
    loaded: Loaded,
    budget: Budget,
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

    fn build(b: ScriptBuilder) -> Result<Script, Error> {
        let granted = b.env.is_some() || !b.read.is_empty();
        let env_cap = match &b.env {
            None => EnvCap::Deny,
            Some(names) => EnvCap::Allow(names.clone()),
        };
        let budget = b.budget;

        let loaded = Loaded::try_new(SourceCache::new(), |cache| -> Result<State<'_>, Error> {
            // Where modules and file reads come from: memory or disk.
            let (resolver, fs, entry, lock): (
                Box<dyn FileResolver>,
                Arc<dyn FileAccess>,
                String,
                Lockfile,
            ) = match &b.file {
                Some(path) => {
                    let entry = std::fs::canonicalize(path).map_err(|e| {
                        Error::host("E0001", format!("cannot open {}: {e}", path.display()))
                    })?;
                    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let lock = match std::fs::read_to_string(root.join("aura.lock")) {
                        Ok(text) => Lockfile::parse(&text)
                            .map_err(|e| Error::host("E0001", format!("invalid aura.lock: {e}")))?,
                        Err(_) => Lockfile::default(),
                    };
                    let registry_dir = std::env::var_os("USERPROFILE")
                        .or_else(|| std::env::var_os("HOME"))
                        .map(PathBuf::from)
                        .unwrap_or_default()
                        .join(".aura")
                        .join("registry");
                    let fs: Arc<dyn FileAccess> = if b.read.is_empty() {
                        Arc::new(DenyFs)
                    } else {
                        Arc::new(RealFs {
                            allowed: b.read.clone(),
                        })
                    };
                    let name = entry
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    (
                        Box::new(LocalFsResolver { root, registry_dir }),
                        fs,
                        name,
                        lock,
                    )
                }
                None => {
                    let entry = b
                        .entry
                        .clone()
                        .or_else(|| b.sources.first().map(|(n, _)| n.clone()))
                        .ok_or_else(|| Error::host("E0001", "no source was given"))?;
                    let files: HashMap<String, String> = b.sources.iter().cloned().collect();
                    let fs: Arc<dyn FileAccess> = if b.read.is_empty() {
                        Arc::new(DenyFs)
                    } else {
                        Arc::new(MemFs(files.clone()))
                    };
                    (
                        Box::new(MemoryResolver { files }),
                        fs,
                        entry,
                        Lockfile::default(),
                    )
                }
            };

            let mut loader = Loader::new(cache, resolver.as_ref());
            // D33 §1: nothing granted, so prove nothing is used.
            loader.hermetic = !granted;
            loader.lock = lock;

            let registry = Arc::new(MethodRegistry::builtin());
            let mut interp = Interpreter::with_registry(
                registry.clone(),
                Options {
                    strict: b.strict,
                    dry_run: false,
                },
            );
            interp.fs = Box::new(SharedFs(fs.clone()));
            interp.env_cap = env_cap.clone();
            // Loading is held to the call budget: a script that cannot finish
            // evaluating its own module must not hang the host's startup.
            let _meter = budget.begin(&mut interp);

            let value = loader.eval_entry(&mut interp, &ImportSpec::File(&entry));
            let mut reports: Vec<Report> =
                loader.diags.iter().map(|d| to_report(d, cache)).collect();
            let value = match value {
                Ok(v) => v,
                Err(d) => {
                    reports.insert(0, to_report(&d, cache));
                    return Err(reports.into());
                }
            };
            if crate::analysis::has_blocking(&loader.diags, b.strict) {
                return Err(reports.into());
            }
            let Value::Object(exports) = value else {
                return Err(Error::host("E0001", "a module evaluates to an object"));
            };

            Ok(State {
                exports: (*exports).clone(),
                registry,
                fs,
                env_cap,
                strict: b.strict,
                _closures: interp,
            })
        })?;

        Ok(Script { loaded, budget })
    }

    /// A value the script exports, as any `Deserialize` type.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<T, Error> {
        let json = self.loaded.with_dependent(|cache, state| {
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

    /// Bind a function, checking now — not on the first request — that the name
    /// exists (`E0002`), names a function (`E0003`), and takes as many
    /// arguments as `A` has (`E0004`).
    pub fn function<A: Args, R: DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Function<'_, A, R>, Error> {
        let index = self.loaded.with_dependent(|_, state| {
            let (index, _, value) = state
                .exports
                .get_full(name)
                .ok_or_else(|| no_export(name))?;
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
            Ok(index)
        })?;
        Ok(Function {
            script: self,
            index,
            _types: PhantomData,
        })
    }

    /// One call, start to finish, in its own interpreter.
    fn invoke<A: Args>(&self, index: usize, args: A) -> Result<serde_json::Value, Error> {
        let budget = self.budget;
        self.loaded.with_dependent(|cache, state| {
            let f = &state.exports[index];
            let mut it = Interpreter::with_registry(
                state.registry.clone(),
                Options {
                    strict: state.strict,
                    dry_run: false,
                },
            );
            it.fs = Box::new(SharedFs(state.fs.clone()));
            it.env_cap = state.env_cap.clone();
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
    index: usize,
    _types: PhantomData<fn(A) -> R>,
}

impl<A: Args, R: DeserializeOwned> Function<'_, A, R> {
    pub fn call(&self, args: A) -> Result<R, Error> {
        from_json(self.script.invoke(self.index, args)?)
    }
}

impl<A, R> fmt::Debug for Function<'_, A, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .script
            .loaded
            .with_dependent(|_, s| s.exports.get_index(self.index).map(|(k, _)| k.clone()));
        f.debug_struct("Function").field("name", &name).finish()
    }
}

impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exports: Vec<String> = self
            .loaded
            .with_dependent(|_, s| s.exports.keys().cloned().collect());
        f.debug_struct("Script")
            .field("exports", &exports)
            .field("budget", &self.budget)
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
