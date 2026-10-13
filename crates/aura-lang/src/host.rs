//! Host functions (D35): what a host provides, and what a script asks for.
//!
//! Two halves, kept apart on purpose. [`HostTable`] is the host's side - the
//! functions it registered, their kind and arity. [`requirements`] is the
//! script's side - every use of a `host:` module, found by walking the AST
//! before anything runs. The loader checks one against the other at load, `aura
//! check` lists the second without the first, and the language server explains
//! the import line from it. One walk, three readers, so they cannot disagree
//! about what a script needs.

use std::sync::Arc;

use indexmap::IndexMap;

use crate::eval::value::{FuncBody, FunctionDef, Value};
use crate::lexer::token::StrPart;
use crate::lexer::Lexer;
use crate::parser::ast::{Expr, ImportSource, LambdaBody, LitValue, Module, ObjectBody, Stmt};
use crate::parser::Parser;
use crate::span::Span;

/// Whether the host vouches the function is deterministic (D35 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// Deterministic, by the host's promise. Allowed under `--hermetic`.
    Pure,
    /// A capability, like `env()`: refused under `--hermetic`.
    Effect,
}

/// What a host function produced, before it becomes an Aura value.
pub enum HostOutcome {
    /// Built by the exact serializer the boundary uses for arguments (D33), so
    /// a host result is held to the same rules: no NaN turned into `null`, no
    /// integer wrapped. Plain data, so it has no lifetime of its own.
    Value(Value<'static>),
    /// The host returned `Err` - `E0008`.
    Failed(String),
    /// An argument did not fit the function's types - `E0009`.
    BadArgument(String),
}

/// The erased function: JSON arguments in, an outcome out.
pub type HostCall = dyn Fn(&[serde_json::Value]) -> HostOutcome + Send + Sync;

/// One function a host provides.
pub struct HostFn {
    /// `geo.region`
    pub name: String,
    pub kind: HostKind,
    pub arity: usize,
    /// D35 §3: reachable from imported modules too, by explicit grant.
    pub for_imports: bool,
    pub call: Arc<HostCall>,
}

impl std::fmt::Debug for HostFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostFn")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("arity", &self.arity)
            .field("for_imports", &self.for_imports)
            .finish()
    }
}

/// Every function a host provides, by module and name.
#[derive(Debug, Default, Clone)]
pub struct HostTable {
    fns: IndexMap<String, Arc<HostFn>>,
    /// D35 §5: what one call costs in steps.
    pub call_steps: u64,
}

/// One step of evaluation is one expression; a host call is a system call, and
/// costs more. The host may change it (`host_call_steps`).
pub const DEFAULT_HOST_CALL_STEPS: u64 = 100;

impl HostTable {
    pub fn new() -> Self {
        HostTable {
            fns: IndexMap::new(),
            call_steps: DEFAULT_HOST_CALL_STEPS,
        }
    }

    pub fn insert(&mut self, f: HostFn) {
        self.fns.insert(f.name.clone(), Arc::new(f));
    }

    pub fn get(&self, module: &str, func: &str) -> Option<&Arc<HostFn>> {
        self.fns.get(&format!("{module}.{func}"))
    }

    /// Whether the host provides anything under this module name.
    pub fn has_module(&self, module: &str) -> bool {
        self.module_fns(module).next().is_some()
    }

    pub fn module_fns<'s>(&'s self, module: &'s str) -> impl Iterator<Item = &'s Arc<HostFn>> + 's {
        self.fns
            .values()
            .filter(move |f| f.name.split_once('.').is_some_and(|(m, _)| m == module))
    }

    pub fn fns(&self) -> impl Iterator<Item = &Arc<HostFn>> {
        self.fns.values()
    }

    /// Whether any registered function is an effect - a grant, so the load is
    /// no longer hermetic (D35 §2).
    pub fn has_effects(&self) -> bool {
        self.fns.values().any(|f| f.kind == HostKind::Effect)
    }
}

/// The module value a script's alias is bound to: an object of functions.
pub fn module_value<'a>(table: &HostTable, module: &str) -> Value<'a> {
    // Parameter names are only ever shown in a function's `Debug` form.
    const NAMES: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];
    Value::object(
        table
            .module_fns(module)
            .map(|f| {
                let func = f.name.split_once('.').map_or("", |(_, n)| n).to_string();
                let def = FunctionDef {
                    params: (0..f.arity).map(|i| NAMES[i % NAMES.len()]).collect(),
                    body: FuncBody::Host(f.clone()),
                    closure: std::sync::Weak::new(),
                    defined_in_root: true,
                    origin_dir: None,
                };
                (func, Value::Function(Arc::new(def)))
            })
            .collect(),
    )
}

/// A host result, rebuilt in the caller's lifetime. Built through the usual
/// constructors, so its memory is charged to the call (D35 §5).
pub(crate) fn rebind<'a>(v: &Value<'static>) -> Value<'a> {
    match v {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Int(i) => Value::Int(*i),
        Value::Float(f) => Value::Float(*f),
        Value::Str(s) => Value::str(s),
        Value::List(xs) => Value::list(xs.iter().map(rebind).collect()),
        Value::Object(m) => Value::object(m.iter().map(|(k, v)| (k.clone(), rebind(v))).collect()),
        // The boundary serializer produces data only.
        Value::Schema(_) | Value::Enum(_) | Value::Function(_) => Value::Null,
    }
}

thread_local! {
    static INSIDE_HOST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the thread as running a host function, for as long as it lives
/// (D35 §6). A script entered while it is set refuses with `E0010`.
pub(crate) struct InsideHost(bool);

impl InsideHost {
    pub(crate) fn enter() -> Self {
        InsideHost(INSIDE_HOST.with(|c| c.replace(true)))
    }
}

impl Drop for InsideHost {
    fn drop(&mut self) {
        INSIDE_HOST.with(|c| c.set(self.0));
    }
}

/// Whether this thread is inside a host function called from a script.
pub(crate) fn inside_host() -> bool {
    INSIDE_HOST.with(|c| c.get())
}

// ---------------------------------------------------------------------------
// What a script asks for
// ---------------------------------------------------------------------------

/// One use of a host module in a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement<'a> {
    /// The host module, as written after `host:`.
    pub module: &'a str,
    /// The alias the script bound it to.
    pub alias: &'a str,
    /// The function, when the use is a direct call `alias.func(...)`. `None`
    /// for any other use - the module passed around, stored, inspected - which
    /// is then taken to use every function the module has: the check has to
    /// stay sound when it cannot see which one is called (D35).
    pub func: Option<&'a str>,
    /// The number of arguments, for a direct call.
    pub arity: Option<usize>,
    pub span: Span,
}

/// Every `import "host:..."` in a module: `(alias, module, span)`.
pub fn host_imports<'a>(module: &Module<'a>) -> Vec<(&'a str, &'a str, Span)> {
    module
        .imports
        .iter()
        .filter_map(|imp| match imp.source {
            ImportSource::Host(m) => Some((imp.alias, m, imp.span)),
            _ => None,
        })
        .collect()
}

/// Every use of a host module in `module`, including inside `#{...}`.
pub fn requirements<'a>(module: &Module<'a>) -> Vec<Requirement<'a>> {
    let aliases: Vec<(&'a str, &'a str)> = host_imports(module)
        .into_iter()
        .map(|(a, m, _)| (a, m))
        .collect();
    let mut w = Walker {
        aliases: &aliases,
        out: Vec::new(),
    };
    if !aliases.is_empty() {
        for s in &module.stmts {
            w.stmt(s);
        }
    }
    w.out
}

struct Walker<'w, 'a> {
    aliases: &'w [(&'a str, &'a str)],
    out: Vec<Requirement<'a>>,
}

impl<'a> Walker<'_, 'a> {
    fn module_of(&self, name: &str) -> Option<&'a str> {
        self.aliases
            .iter()
            .find(|(a, _)| *a == name)
            .map(|(_, m)| *m)
    }

    fn stmts(&mut self, body: &[Stmt<'a>]) {
        for s in body {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt<'a>) {
        match s {
            Stmt::Assign { value, .. } | Stmt::Property { value, .. } | Stmt::Expr(value) => {
                self.expr(value)
            }
            Stmt::Assert { cond, message, .. } => {
                self.expr(cond);
                if let Some(m) = message {
                    self.expr(m);
                }
            }
            Stmt::TypeDecl(t) => {
                for f in &t.fields {
                    if let Some(d) = &f.default {
                        self.expr(d);
                    }
                }
                for inv in &t.invariants {
                    self.expr(&inv.cond);
                    if let Some(m) = &inv.message {
                        self.expr(m);
                    }
                }
            }
            Stmt::EnumDecl(_) => {}
            Stmt::FuncDecl { body, .. } => self.stmts(body),
            Stmt::Block(b) => {
                self.expr(&b.label);
                self.stmts(&b.body);
            }
        }
    }

    fn body(&mut self, b: &ObjectBody<'a>) {
        for (_, e, _) in &b.props {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &Expr<'a>) {
        match e {
            Expr::Literal(LitValue::InterpStr(parts), span) => {
                for part in parts {
                    if let StrPart::Interp(src) = part {
                        // Interpolations are parsed lazily, so the walk has to
                        // parse them itself: a host call inside `"#{...}"` must
                        // not slip past the hermetic and import checks.
                        if let Ok(toks) = Lexer::new(src, span.source).tokenize() {
                            if let Ok(inner) = Parser::new(toks).parse_expression() {
                                self.expr_at(&inner, *span);
                            }
                        }
                    }
                }
            }
            Expr::Literal(..) => {}
            Expr::Variable(name, span) => {
                if let Some(module) = self.module_of(name) {
                    self.out.push(Requirement {
                        module,
                        alias: name,
                        func: None,
                        arity: None,
                        span: *span,
                    });
                }
            }
            Expr::MethodCall {
                recv,
                method,
                args,
                lambda,
                span,
            } => {
                let direct = match recv.as_ref() {
                    Expr::Variable(name, _) => self.module_of(name).map(|m| (name, m)),
                    _ => None,
                };
                match direct {
                    Some((alias, module)) => self.out.push(Requirement {
                        module,
                        alias,
                        func: Some(method),
                        arity: Some(args.len()),
                        span: *span,
                    }),
                    None => self.expr(recv),
                }
                for a in args {
                    self.expr(a);
                }
                if let Some(l) = lambda {
                    self.expr(l);
                }
            }
            Expr::Unary { rhs, .. } => self.expr(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
                ..
            } => {
                self.expr(cond);
                self.expr(then);
                self.expr(otherwise);
            }
            Expr::Cond {
                arms, otherwise, ..
            } => {
                for (c, v) in arms {
                    self.expr(c);
                    self.expr(v);
                }
                self.expr(otherwise);
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::FieldAccess { recv, .. } => self.expr(recv),
            Expr::Index { recv, key, .. } => {
                self.expr(recv);
                self.expr(key);
            }
            Expr::ObjectLiteral(b) => self.body(b),
            Expr::ListLiteral(xs, _) => {
                for x in xs {
                    self.expr(x);
                }
            }
            Expr::Lambda { body, .. } => match body {
                LambdaBody::Expr(e) => self.expr(e),
                LambdaBody::Block(b) => self.stmts(b),
            },
            Expr::SchemaInstance { body, .. } => self.body(body),
        }
    }

    /// An expression parsed out of an interpolation: its spans start at zero,
    /// so every requirement found in it is reported at the string's own span.
    fn expr_at(&mut self, e: &Expr<'a>, at: Span) {
        let before = self.out.len();
        self.expr(e);
        for r in &mut self.out[before..] {
            r.span = at;
        }
    }
}
