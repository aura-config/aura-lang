//! Evaluation engine (SPEC §4). Deterministic tree-walking interpreter
//! with a capability model for effects (D1).

pub mod env;
pub mod methods;
pub mod value;

use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::Diagnostic;
use crate::lexer::token::StrPart;
use crate::lexer::Lexer;
use crate::parser::ast::*;
use crate::parser::Parser;
use crate::span::Span;
use env::{Env, Environment};
use methods::MethodRegistry;
use value::{FuncBody, FunctionDef, SchemaDef, Value};

const MAX_CALL_DEPTH: u32 = 256;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub strict: bool,
    pub dry_run: bool,
}

/// Capability for reading environment variables (D1): denied by default.
#[derive(Debug, Clone, Default)]
pub enum EnvCap {
    #[default]
    Deny,
    AllowAll,
    Allow(Vec<String>),
}

#[derive(Debug)]
pub enum FileError {
    Denied,
    Io(String),
}

/// File access behind a trait: the VFS builds on this, dry-run wraps it in RecordingFs.
pub trait FileAccess {
    fn read(&self, path: &str) -> Result<String, FileError>;
}

/// D1 default: no I/O without explicit grants.
pub struct DenyFs;
impl FileAccess for DenyFs {
    fn read(&self, _path: &str) -> Result<String, FileError> {
        Err(FileError::Denied)
    }
}

pub struct MemFs(pub HashMap<String, String>);
impl FileAccess for MemFs {
    fn read(&self, path: &str) -> Result<String, FileError> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| FileError::Io(format!("not found: {path}")))
    }
}

/// Dry-run wrapper (SPEC §6.3): reads are performed but recorded into a report.
pub struct RecordingFs {
    pub inner: Box<dyn FileAccess>,
    pub log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}
impl FileAccess for RecordingFs {
    fn read(&self, path: &str) -> Result<String, FileError> {
        let result = self.inner.read(path);
        if result.is_ok() {
            self.log.borrow_mut().push(path.to_string());
        }
        result
    }
}

/// `--allow-read=<paths>`: canonicalization + prefix check (E0311).
pub struct RealFs {
    pub allowed: Vec<PathBuf>,
}
impl FileAccess for RealFs {
    fn read(&self, path: &str) -> Result<String, FileError> {
        let canon = std::fs::canonicalize(path).map_err(|e| FileError::Io(e.to_string()))?;
        let permitted = self.allowed.iter().any(|root| {
            std::fs::canonicalize(root)
                .map(|r| canon.starts_with(r))
                .unwrap_or(false)
        });
        if !permitted {
            return Err(FileError::Denied);
        }
        std::fs::read_to_string(&canon).map_err(|e| FileError::Io(e.to_string()))
    }
}

pub struct Interpreter<'a> {
    pub registry: MethodRegistry<'a>,
    pub fs: Box<dyn FileAccess>,
    pub env_cap: EnvCap,
    /// Environment overrides for tests/dry-run snapshots; take priority over the real env.
    pub env_overrides: HashMap<String, String>,
    pub options: Options,
    /// --allow-imports-io: grant imported modules the root's I/O capabilities (D1).
    pub allow_imports_io: bool,
    /// Set by the VFS loader: false while evaluating an imported module.
    pub current_root: bool,
    /// Evaluated modules by alias; populated by the VFS loader (or by tests directly).
    modules: HashMap<String, Value<'a>>,
    call_depth: u32,
}

fn rt(code: &'static str, msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(code, msg, span, "evaluated here")
}

impl<'a> Interpreter<'a> {
    pub fn new(options: Options) -> Self {
        Interpreter {
            registry: MethodRegistry::builtin(),
            fs: Box::new(DenyFs),
            env_cap: EnvCap::Deny,
            env_overrides: HashMap::new(),
            options,
            allow_imports_io: false,
            current_root: true,
            modules: HashMap::new(),
            call_depth: 0,
        }
    }

    pub fn provide_module(&mut self, alias: impl Into<String>, value: Value<'a>) {
        self.modules.insert(alias.into(), value);
    }

    /// A module evaluates to an Object of its exported properties and domain blocks.
    pub fn eval_module(&mut self, module: &Module<'a>) -> Result<Value<'a>, Diagnostic> {
        let env = Environment::root();
        for imp in &module.imports {
            let v = self.modules.get(imp.alias).cloned().ok_or_else(|| {
                rt(
                    "E0410",
                    format!(
                        "unresolved import '{}': module loading arrives in Phase 4 (VFS)",
                        imp.alias
                    ),
                    imp.span,
                )
            })?;
            env.insert(imp.alias, v);
        }
        let mut exports: IndexMap<String, Value<'a>> = IndexMap::new();
        for stmt in &module.stmts {
            self.exec_stmt(&env, stmt, &mut exports)?;
        }
        Ok(Value::object(exports))
    }

    fn exec_stmt(
        &mut self,
        env: &Env<'a>,
        stmt: &Stmt<'a>,
        exports: &mut IndexMap<String, Value<'a>>,
    ) -> Result<(), Diagnostic> {
        match stmt {
            // D10: `=` is a private computation and is never exported; only
            // `key:` properties and domain/component blocks are.
            Stmt::Assign {
                name,
                shadow,
                value,
                span,
            } => {
                let v = self.eval_expr(env, value)?;
                self.define(env, name, v, *shadow, *span)?;
            }
            Stmt::Property { key, value, .. } => {
                let v = self.eval_expr(env, value)?;
                exports.insert(key.to_string(), v);
            }
            Stmt::TypeDecl(schema) => {
                let v = Value::Schema(Arc::new(SchemaDef {
                    name: schema.name,
                    fields: schema.fields.clone(),
                }));
                self.define(env, schema.name, v.clone(), false, schema.span)?;
                // D12: pub type is visible to importers via the module object
                if schema.public {
                    exports.insert(schema.name.to_string(), v);
                }
            }
            Stmt::FuncDecl {
                name,
                params,
                body,
                public,
                span,
            } => {
                let v = Value::Function(Arc::new(FunctionDef {
                    params: params.clone(),
                    body: FuncBody::Object(body.clone()),
                    closure: env.clone(),
                    defined_in_root: self.current_root,
                }));
                self.define(env, name, v.clone(), false, *span)?;
                // D12: pub def is visible to importers via the module object
                if *public {
                    exports.insert(name.to_string(), v);
                }
            }
            Stmt::Block(block) => {
                let label = self.eval_expr(env, &block.label)?;
                let obj = self.eval_block(env, block, &label)?;
                let key = match &label {
                    Value::Str(s) => s.to_string(),
                    other => {
                        return Err(rt(
                            "E0306",
                            format!("block label must be String, got {}", other.type_name()),
                            block.span,
                        ))
                    }
                };
                exports.insert(key, obj);
            }
            Stmt::Assert {
                cond,
                message,
                span,
            } => match self.eval_expr(env, cond)? {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    let msg = match message {
                        Some(m) => {
                            let v = self.eval_expr(env, m)?;
                            self.display(&v, *span)?
                        }
                        None => "assertion failed".to_string(),
                    };
                    return Err(rt("E0530", msg, *span));
                }
                other => {
                    return Err(rt(
                        "E0306",
                        format!("assert condition must be Bool, got {}", other.type_name()),
                        *span,
                    ))
                }
            },
            Stmt::Expr(e) => {
                self.eval_expr(env, e)?;
            }
        }
        Ok(())
    }

    /// Binding rules D7/E0301 (SPEC §4.2).
    fn define(
        &self,
        env: &Env<'a>,
        name: &str,
        v: Value<'a>,
        shadow: bool,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if env.defined_here(name) {
            return Err(rt(
                "E0301",
                format!("variable '{name}' is immutable and already defined in this scope"),
                span,
            ));
        }
        if !shadow && env.defined_in_ancestors(name) {
            let mut d = rt("E0302", format!("'{name}' shadows an outer variable"), span);
            d.help = Some(format!(
                "write `shadow {name} = ...` to make the shadowing explicit"
            ));
            return Err(d);
        }
        env.insert(name, v);
        Ok(())
    }

    /// domain/component body → Object; a component additionally gets "name" = label.
    fn eval_block(
        &mut self,
        outer: &Env<'a>,
        block: &BlockDeclaration<'a>,
        label: &Value<'a>,
    ) -> Result<Value<'a>, Diagnostic> {
        let env = Environment::child(outer);
        let mut exports: IndexMap<String, Value<'a>> = IndexMap::new();
        if block.kind == BlockKind::Component {
            exports.insert("name".to_string(), label.clone());
        }
        for stmt in &block.body {
            self.exec_stmt(&env, stmt, &mut exports)?;
        }
        Ok(Value::object(exports))
    }

    pub fn eval_expr(&mut self, env: &Env<'a>, e: &Expr<'a>) -> Result<Value<'a>, Diagnostic> {
        match e {
            Expr::Literal(lit, span) => self.eval_literal(env, lit, *span),
            Expr::Variable(name, span) => env.get(name).ok_or_else(|| {
                rt(
                    "E0504",
                    format!("use of undefined variable '{name}'"),
                    *span,
                )
            }),
            Expr::Unary { op, rhs, span } => {
                let v = self.eval_expr(env, rhs)?;
                match (op, v) {
                    (UnaryOp::Neg, Value::Int(n)) => n
                        .checked_neg()
                        .map(Value::Int)
                        .ok_or_else(|| rt("E0304", "integer overflow in negation", *span)),
                    (UnaryOp::Neg, Value::Float(n)) => Ok(Value::Float(-n)),
                    (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (_, v) => Err(rt(
                        "E0306",
                        format!("invalid operand type {}", v.type_name()),
                        *span,
                    )),
                }
            }
            Expr::Binary { op, lhs, rhs, span } => self.eval_binary(env, *op, lhs, rhs, *span),
            Expr::Ternary {
                cond,
                then,
                otherwise,
                span,
            } => match self.eval_expr(env, cond)? {
                Value::Bool(true) => self.eval_expr(env, then),
                Value::Bool(false) => self.eval_expr(env, otherwise),
                other => Err(rt(
                    "E0306",
                    format!("ternary condition must be Bool, got {}", other.type_name()),
                    *span,
                )),
            },
            // D14: first true arm wins; conditions must be Bool; `else` is the fallback.
            Expr::Cond {
                arms,
                otherwise,
                span,
            } => {
                for (condition, value) in arms {
                    match self.eval_expr(env, condition)? {
                        Value::Bool(true) => return self.eval_expr(env, value),
                        Value::Bool(false) => {}
                        other => {
                            return Err(rt(
                                "E0306",
                                format!("cond condition must be Bool, got {}", other.type_name()),
                                *span,
                            ))
                        }
                    }
                }
                self.eval_expr(env, otherwise)
            }
            Expr::Call { callee, args, span } => {
                let argv: Vec<Value<'a>> = args
                    .iter()
                    .map(|a| self.eval_expr(env, a))
                    .collect::<Result<_, _>>()?;
                if let Expr::Variable(name, _) = callee.as_ref() {
                    if let Some(r) = self.try_builtin_call(name, &argv, *span)? {
                        return Ok(r);
                    }
                }
                let f = self.eval_expr(env, callee)?;
                self.call_value(&f, &argv, *span)
            }
            Expr::MethodCall {
                recv,
                method,
                args,
                lambda,
                span,
            } => {
                let recv_v = self.eval_expr(env, recv)?;
                let mut argv: Vec<Value<'a>> = args
                    .iter()
                    .map(|a| self.eval_expr(env, a))
                    .collect::<Result<_, _>>()?;
                if let Some(l) = lambda {
                    argv.push(self.eval_expr(env, l)?);
                }
                if let Some(f) = self.registry.get(recv_v.tag(), method) {
                    return f(self, &recv_v, &argv, *span);
                }
                // D12: calling an exported module function — obj.fn(args);
                // builtin methods take precedence over same-named fields
                if let Value::Object(m) = &recv_v {
                    if let Some(f @ Value::Function(_)) = m.get(*method) {
                        let f = f.clone();
                        return self.call_value(&f, &argv, *span);
                    }
                }
                Err(rt(
                    "E0309",
                    format!("unknown method '{}' on {}", method, recv_v.type_name()),
                    *span,
                ))
            }
            Expr::FieldAccess { recv, field, span } => {
                let v = self.eval_expr(env, recv)?;
                match v {
                    Value::Object(m) => m
                        .get(*field)
                        .cloned()
                        .ok_or_else(|| rt("E0308", format!("unknown field '{field}'"), *span)),
                    other => Err(rt(
                        "E0306",
                        format!("cannot access field '{}' on {}", field, other.type_name()),
                        *span,
                    )),
                }
            }
            // D11: `xs[int]` for lists; `obj."#{key}"` (bracket=false) for objects
            Expr::Index {
                recv,
                key,
                bracket,
                span,
            } => {
                let r = self.eval_expr(env, recv)?;
                let k = self.eval_expr(env, key)?;
                match (&r, &k) {
                    (Value::List(xs), Value::Int(i)) => {
                        let i = *i;
                        if i < 0 || i as usize >= xs.len() {
                            return Err(rt(
                                "E0317",
                                format!("index {i} out of bounds (list has {} elements)", xs.len()),
                                *span,
                            ));
                        }
                        Ok(xs[i as usize].clone())
                    }
                    (Value::Object(m), Value::Str(s)) => {
                        if *bracket {
                            let mut d =
                                rt("E0318", "bracket access on objects is not supported", *span);
                            d.help = Some(format!(
                                "use dot access instead: `.\"{s}\"` or `.get(\"{s}\", default)`"
                            ));
                            return Err(d);
                        }
                        m.get(s.as_ref())
                            .cloned()
                            .ok_or_else(|| rt("E0308", format!("unknown field '{s}'"), *span))
                    }
                    _ => Err(rt(
                        "E0306",
                        format!("cannot index {} with {}", r.type_name(), k.type_name()),
                        *span,
                    )),
                }
            }
            Expr::ObjectLiteral(body) => self.eval_object_body(env, body),
            Expr::ListLiteral(items, _) => {
                let vs: Vec<Value<'a>> = items
                    .iter()
                    .map(|i| self.eval_expr(env, i))
                    .collect::<Result<_, _>>()?;
                Ok(Value::list(vs))
            }
            Expr::Lambda { params, body, .. } => Ok(Value::Function(Arc::new(FunctionDef {
                params: params.clone(),
                body: FuncBody::Lambda(body.clone()),
                closure: env.clone(),
                defined_in_root: self.current_root,
            }))),
            Expr::SchemaInstance {
                schema,
                schema_alias,
                body,
                span,
            } => {
                // D12: `new alias.Schema` — a schema from an imported module's object
                let resolved = match schema_alias {
                    Some(alias) => match env.get(alias) {
                        Some(Value::Object(m)) => m.get(*schema).cloned(),
                        _ => None,
                    },
                    None => env.get(schema),
                };
                let Some(Value::Schema(def)) = resolved else {
                    return Err(rt("E0504", format!("unknown schema '{schema}'"), *span));
                };
                let provided = self.eval_object_body(env, body)?;
                let Value::Object(pmap) = &provided else {
                    unreachable!()
                };
                // Apply defaults for optional fields omitted here (in schema order,
                // after the provided fields). Evaluated in the instantiation scope.
                let mut map: IndexMap<String, Value<'a>> = (**pmap).clone();
                for f in &def.fields {
                    if !map.contains_key(f.name) {
                        if let Some(default_expr) = &f.default {
                            let v = self.eval_expr(env, default_expr)?;
                            map.insert(f.name.to_string(), v);
                        }
                    }
                }
                let obj = Value::object(map);
                self.validate_schema(&def, &obj, *span)?;
                Ok(obj)
            }
            Expr::Block(block) => {
                let label = self.eval_expr(env, &block.label)?;
                self.eval_block(env, block, &label)
            }
        }
    }

    fn eval_object_body(
        &mut self,
        env: &Env<'a>,
        body: &ObjectBody<'a>,
    ) -> Result<Value<'a>, Diagnostic> {
        let mut map: IndexMap<String, Value<'a>> = IndexMap::with_capacity(body.props.len());
        for (key, expr, _) in &body.props {
            map.insert(key.to_string(), self.eval_expr(env, expr)?);
        }
        Ok(Value::object(map))
    }

    fn eval_literal(
        &mut self,
        env: &Env<'a>,
        lit: &LitValue<'a>,
        span: Span,
    ) -> Result<Value<'a>, Diagnostic> {
        Ok(match lit {
            LitValue::Int(n) => Value::Int(*n),
            LitValue::Float(n) => Value::Float(*n),
            LitValue::Bool(b) => Value::Bool(*b),
            LitValue::Null => Value::Null,
            LitValue::Str(s) => Value::Str(unescape(s)),
            LitValue::InterpStr(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        StrPart::Lit(s) => out.push_str(&unescape(s)),
                        StrPart::Interp(src) => {
                            let v = self.eval_interp(env, src, span)?;
                            out.push_str(&self.display(&v, span)?);
                        }
                    }
                }
                Value::str(out)
            }
        })
    }

    /// `#{expr}` — lazy parse of the raw slice (SPEC §2.1), evaluated in the current scope.
    fn eval_interp(
        &mut self,
        env: &Env<'a>,
        src: &'a str,
        span: Span,
    ) -> Result<Value<'a>, Diagnostic> {
        let toks = Lexer::new(src, span.source).tokenize().map_err(|d| {
            rt(
                "E0316",
                format!("invalid interpolation: {}", d.message),
                span,
            )
        })?;
        let expr = Parser::new(toks).parse_expression().map_err(|d| {
            rt(
                "E0316",
                format!("invalid interpolation: {}", d.message),
                span,
            )
        })?;
        self.eval_expr(env, &expr)
    }

    /// Only scalars are allowed in interpolation and join (E0307).
    pub(crate) fn display(&self, v: &Value<'a>, span: Span) -> Result<String, Diagnostic> {
        Ok(match v {
            Value::Str(s) => s.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            other => {
                return Err(rt(
                    "E0307",
                    format!("cannot interpolate {}", other.type_name()),
                    span,
                ))
            }
        })
    }

    fn eval_binary(
        &mut self,
        env: &Env<'a>,
        op: BinOp,
        lhs: &Expr<'a>,
        rhs: &Expr<'a>,
        span: Span,
    ) -> Result<Value<'a>, Diagnostic> {
        // Short-circuit logic
        if matches!(op, BinOp::And | BinOp::Or) {
            let Value::Bool(l) = self.eval_expr(env, lhs)? else {
                return Err(rt("E0306", "logical operand must be Bool", span));
            };
            return match (op, l) {
                (BinOp::And, false) => Ok(Value::Bool(false)),
                (BinOp::Or, true) => Ok(Value::Bool(true)),
                _ => match self.eval_expr(env, rhs)? {
                    Value::Bool(r) => Ok(Value::Bool(r)),
                    _ => Err(rt("E0306", "logical operand must be Bool", span)),
                },
            };
        }
        let l = self.eval_expr(env, lhs)?;
        let r = self.eval_expr(env, rhs)?;
        use BinOp::*;
        use Value::*;
        match op {
            Eq => return Ok(Bool(l == r)),
            Ne => return Ok(Bool(l != r)),
            _ => {}
        }
        // Arithmetic and comparisons (SPEC §4.1, D6): Int⊕Int→Int (checked), Float is contagious.
        match (op, &l, &r) {
            (Add, Int(a), Int(b)) => a
                .checked_add(*b)
                .map(Int)
                .ok_or_else(|| rt("E0304", "integer overflow", span)),
            (Sub, Int(a), Int(b)) => a
                .checked_sub(*b)
                .map(Int)
                .ok_or_else(|| rt("E0304", "integer overflow", span)),
            (Mul, Int(a), Int(b)) => a
                .checked_mul(*b)
                .map(Int)
                .ok_or_else(|| rt("E0304", "integer overflow", span)),
            (Div, Int(a), Int(b)) => {
                if *b == 0 {
                    Err(rt("E0305", "division by zero", span))
                } else {
                    Ok(Int(a / b))
                }
            }
            (Rem, Int(a), Int(b)) => {
                if *b == 0 {
                    Err(rt("E0305", "division by zero", span))
                } else {
                    Ok(Int(a % b))
                }
            }
            (Add | Sub | Mul | Div | Rem, _, _)
                if l.tag() == value::TypeTag::Float || r.tag() == value::TypeTag::Float =>
            {
                let (a, b) = (as_float(&l, span)?, as_float(&r, span)?);
                Ok(Float(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    Rem => a % b,
                    _ => unreachable!(),
                }))
            }
            (Lt | Gt | Le | Ge, Str(a), Str(b)) => {
                Ok(Bool(cmp_ord(op, a.as_ref().cmp(b.as_ref()))))
            }
            (Lt | Gt | Le | Ge, _, _) => {
                let (a, b) = (as_float(&l, span)?, as_float(&r, span)?);
                let ord = a
                    .partial_cmp(&b)
                    .ok_or_else(|| rt("E0306", "NaN comparison", span))?;
                Ok(Bool(cmp_ord(op, ord)))
            }
            _ => Err(rt(
                "E0306",
                format!(
                    "invalid operand types: {} {} {}",
                    l.type_name(),
                    op_name(op),
                    r.type_name()
                ),
                span,
            )),
        }
    }

    /// Calls a Value::Function (def / lambda). Extra arguments are ignored
    /// (map passes elem+index, a lambda may declare only elem); too few — E0312.
    pub fn call_value(
        &mut self,
        f: &Value<'a>,
        args: &[Value<'a>],
        span: Span,
    ) -> Result<Value<'a>, Diagnostic> {
        let Value::Function(def) = f else {
            return Err(rt(
                "E0306",
                format!("{} is not callable", f.type_name()),
                span,
            ));
        };
        if args.len() < def.params.len() {
            return Err(rt(
                "E0312",
                format!(
                    "function expects {} arguments, got {}",
                    def.params.len(),
                    args.len()
                ),
                span,
            ));
        }
        self.call_depth += 1;
        if self.call_depth > MAX_CALL_DEPTH {
            self.call_depth -= 1;
            return Err(rt("E0399", "maximum call depth (256) exceeded", span));
        }
        let env = Environment::child(&def.closure);
        for (p, a) in def.params.iter().zip(args) {
            env.insert(p, a.clone());
        }
        // D1×D12: the body runs with the capabilities of the function's origin module
        let saved_root = self.current_root;
        self.current_root = def.defined_in_root;
        let result = match &def.body {
            FuncBody::Object(body) => self.eval_object_body(&env, body),
            FuncBody::Lambda(LambdaBody::Expr(e)) => self.eval_expr(&env, e),
            FuncBody::Lambda(LambdaBody::Object(body)) => self.eval_object_body(&env, body),
        };
        self.current_root = saved_root;
        self.call_depth -= 1;
        result
    }

    /// Effectful builtins under capability control (D1, SPEC §4.3).
    fn try_builtin_call(
        &mut self,
        name: &str,
        args: &[Value<'a>],
        span: Span,
    ) -> Result<Option<Value<'a>>, Diagnostic> {
        // D1: imported modules get no I/O without --allow-imports-io (SPEC §4.3).
        if matches!(name, "env" | "read_file") && !self.current_root && !self.allow_imports_io {
            let mut d = rt(
                "E0310",
                format!("imported module has no capability to call {name}()"),
                span,
            );
            d.help = Some(
                "pass --allow-imports-io to grant imports the root I/O capabilities".to_string(),
            );
            return Err(d);
        }
        match name {
            // D13: current time does not exist in Aura — configs are deterministic
            "now" | "timestamp" => {
                let mut d = rt(
                    "E0533",
                    format!("{name}() does not exist: Aura configs are reproducible by design"),
                    span,
                );
                d.help = Some(
                    "pass a build timestamp from the host instead: env(\"BUILD_TIME\", ...) with --allow-env=BUILD_TIME"
                        .to_string(),
                );
                Err(d)
            }
            "env" => {
                let Some(Value::Str(var)) = args.first() else {
                    return Err(rt("E0306", "env() expects a String name", span));
                };
                let allowed = match &self.env_cap {
                    EnvCap::Deny => false,
                    EnvCap::AllowAll => true,
                    EnvCap::Allow(list) => list.iter().any(|n| n == var.as_ref()),
                };
                if !allowed {
                    let mut d = rt(
                        "E0310",
                        format!("no capability to read env var '{var}'"),
                        span,
                    );
                    d.help = Some(format!("pass --allow-env={var} to grant access"));
                    return Err(d);
                }
                let value = self
                    .env_overrides
                    .get(var.as_ref())
                    .cloned()
                    .or_else(|| std::env::var(var.as_ref()).ok());
                Ok(Some(match value {
                    Some(s) => Value::str(s),
                    None => args.get(1).cloned().unwrap_or(Value::Null),
                }))
            }
            "read_file" => {
                let Some(Value::Str(path)) = args.first() else {
                    return Err(rt("E0306", "read_file() expects a String path", span));
                };
                match self.fs.read(path) {
                    Ok(s) => Ok(Some(Value::str(s))),
                    Err(FileError::Denied) => {
                        let mut d = rt("E0310", format!("no capability to read '{path}'"), span);
                        d.help = Some("pass --allow-read=<dir> to grant access".to_string());
                        Err(d)
                    }
                    Err(FileError::Io(e)) => {
                        Err(rt("E0313", format!("cannot read '{path}': {e}"), span))
                    }
                }
            }
            "fail" => {
                let msg = match args.first() {
                    Some(v) => self.display(v, span)?,
                    None => "fail() called".to_string(),
                };
                Err(rt("E0531", msg, span))
            }
            // Pure generator: range(n) = [0, 1, ..., n-1]. Deterministic, no capability.
            "range" => {
                let Some(Value::Int(n)) = args.first() else {
                    return Err(rt("E0306", "range() expects an Int argument", span));
                };
                if *n < 0 {
                    return Err(rt(
                        "E0306",
                        format!("range() argument must be non-negative, got {n}"),
                        span,
                    ));
                }
                // Guard against accidental OOM; configs never need a huge range.
                const RANGE_LIMIT: i64 = 1_000_000;
                if *n > RANGE_LIMIT {
                    return Err(rt(
                        "E0306",
                        format!("range() argument {n} exceeds the limit of {RANGE_LIMIT}"),
                        span,
                    ));
                }
                Ok(Some(Value::list((0..*n).map(Value::Int).collect())))
            }
            _ => Ok(None),
        }
    }

    /// Schema validation (SPEC §6.2): E0511 missing, E0512 type, E0513 extra (strict only).
    fn validate_schema(
        &self,
        def: &SchemaDef<'a>,
        obj: &Value<'a>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Value::Object(map) = obj else {
            unreachable!()
        };
        for f in &def.fields {
            let Some(v) = map.get(f.name) else {
                // Defaults are applied before validation, so a still-missing field
                // has no default and is genuinely required.
                return Err(rt(
                    "E0511",
                    format!("missing field '{}' required by schema {}", f.name, def.name),
                    span,
                ));
            };
            let ok = match f.ty {
                TypeName::String => matches!(v, Value::Str(_)),
                TypeName::Int => matches!(v, Value::Int(_)),
                TypeName::Float => matches!(v, Value::Float(_)),
                TypeName::Bool => matches!(v, Value::Bool(_)),
                TypeName::List => matches!(v, Value::List(_)),
                TypeName::Object | TypeName::Custom(_) => matches!(v, Value::Object(_)),
            };
            if !ok {
                return Err(rt(
                    "E0512",
                    format!(
                        "field '{}' of schema {} expects {:?}, got {}",
                        f.name,
                        def.name,
                        f.ty,
                        v.type_name()
                    ),
                    span,
                ));
            }
        }
        if self.options.strict {
            for key in map.keys() {
                if !def.fields.iter().any(|f| f.name == key) {
                    return Err(rt(
                        "E0513",
                        format!(
                            "unknown field '{}' not declared in schema {}",
                            key, def.name
                        ),
                        span,
                    ));
                }
            }
        }
        Ok(())
    }
}

fn as_float(v: &Value<'_>, span: Span) -> Result<f64, Diagnostic> {
    match v {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(n) => Ok(*n),
        other => Err(rt(
            "E0306",
            format!("expected a number, got {}", other.type_name()),
            span,
        )),
    }
}

fn cmp_ord(op: BinOp, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        BinOp::Lt => ord == Less,
        BinOp::Gt => ord == Greater,
        BinOp::Le => ord != Greater,
        BinOp::Ge => ord != Less,
        _ => unreachable!(),
    }
}

fn op_name(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Rem => "%",
        Eq => "==",
        Ne => "!=",
        Lt => "<",
        Gt => ">",
        Le => "<=",
        Ge => ">=",
        And => "&&",
        Or => "||",
    }
}

/// Lazy unescaping (SPEC §2.3): zero cost when there is no `\`.
fn unescape(s: &str) -> Arc<str> {
    if !s.contains('\\') {
        return Arc::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('#') => out.push('#'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Arc::from(out.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str) -> Result<Value<'_>, Diagnostic> {
        eval_with(src, Options::default())
    }

    fn eval_with(src: &str, options: Options) -> Result<Value<'_>, Diagnostic> {
        let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
        let module = Parser::new(toks).parse_module().expect("parse ok");
        Interpreter::new(options).eval_module(&module)
    }

    fn get<'a>(v: &Value<'a>, key: &str) -> Value<'a> {
        let Value::Object(m) = v else {
            panic!("not an object: {v:?}")
        };
        m.get(key).unwrap_or_else(|| panic!("no key {key}")).clone()
    }

    #[test]
    fn arithmetic_int_float_d6() {
        // D10: only `key:` properties are exported
        let v = eval("a: 7 / 2\nb: 7.0 / 2\nc: 2 + 3 * 4").unwrap();
        assert_eq!(get(&v, "a"), Value::Int(3)); // integer division
        assert_eq!(get(&v, "b"), Value::Float(3.5));
        assert_eq!(get(&v, "c"), Value::Int(14));
    }

    #[test]
    fn int_overflow_is_e0304() {
        assert_eq!(
            eval("x = 9223372036854775807 + 1").unwrap_err().code,
            "E0304"
        );
        assert_eq!(eval("x = 1 / 0").unwrap_err().code, "E0305");
    }

    #[test]
    fn redefine_in_same_scope_is_e0301() {
        assert_eq!(eval("x = 1\nx = 2").unwrap_err().code, "E0301");
    }

    #[test]
    fn shadow_requires_keyword_d7() {
        let src_bad = "x = 1\ndomain \"d\"\n  x = 2\nend";
        assert_eq!(eval(src_bad).unwrap_err().code, "E0302");
        let src_ok = "x = 1\ndomain \"d\"\n  shadow x = 2\n  y: x\nend";
        let v = eval(src_ok).unwrap();
        assert_eq!(get(&get(&v, "d"), "y"), Value::Int(2));
    }

    #[test]
    fn capability_denied_by_default_d1() {
        assert_eq!(
            eval("x = read_file(\"/etc/passwd\")").unwrap_err().code,
            "E0310"
        );
        assert_eq!(eval("x = env(\"HOME\", \"\")").unwrap_err().code, "E0310");
    }

    #[test]
    fn schema_validation() {
        let base = "type M\n  name: String\n  port: Int\nend\n";
        let ok = format!("{base}m = new M\n  name: \"a\"\n  port: 1\nend");
        assert!(eval(&ok).is_ok());
        let missing = format!("{base}m = new M\n  name: \"a\"\nend");
        assert_eq!(eval(&missing).unwrap_err().code, "E0511");
        let wrong_ty = format!("{base}m = new M\n  name: \"a\"\n  port: \"x\"\nend");
        assert_eq!(eval(&wrong_ty).unwrap_err().code, "E0512");
        let extra = format!("{base}m = new M\n  name: \"a\"\n  port: 1\n  zzz: 1\nend");
        assert!(eval(&extra).is_ok()); // non-strict: the field is kept
        assert_eq!(
            eval_with(
                &extra,
                Options {
                    strict: true,
                    dry_run: false
                }
            )
            .unwrap_err()
            .code,
            "E0513"
        );
    }

    #[test]
    fn list_methods_and_lambda() {
        let v = eval("xs = [\"b\"\n\"a\"\nnull\n\"b\"]\nys = xs.compact().uniq()\nn: ys.len()\nup: ys.map (s, i) -> s.upper() end").unwrap();
        assert_eq!(get(&v, "n"), Value::Int(2));
        let Value::List(up) = get(&v, "up") else {
            panic!()
        };
        assert_eq!(up[0], Value::str("B"));
        assert_eq!(up[1], Value::str("A"));
    }

    #[test]
    fn interpolation_and_ternary() {
        let v = eval("name = \"auth\"\nport = 8000\ns: \"svc-#{name}:#{port + 1}\"\nt: port > 100 ? \"big\" : \"small\"").unwrap();
        assert_eq!(get(&v, "s"), Value::str("svc-auth:8001"));
        assert_eq!(get(&v, "t"), Value::str("big"));
    }

    #[test]
    fn indexing_and_get_d11() {
        let v = eval("xs = [10, 20, 30]\na: xs[1]\nb: xs.get(9, 99)\nc: xs.first()\nd: xs.last()")
            .unwrap();
        assert_eq!(get(&v, "a"), Value::Int(20));
        assert_eq!(get(&v, "b"), Value::Int(99));
        assert_eq!(get(&v, "c"), Value::Int(10));
        assert_eq!(get(&v, "d"), Value::Int(30));
        assert_eq!(eval("x: [1][5]").unwrap_err().code, "E0317");
    }

    #[test]
    fn string_keys_and_dynamic_access_d11() {
        let src = "def mk()\n  \"eu west\": 8080\nend\nregion = \"eu west\"\np: mk().\"eu west\"\nq: mk().\"#{region}\"\ng: mk().get(\"nope\", 0)";
        let v = eval(src).unwrap();
        assert_eq!(get(&v, "p"), Value::Int(8080));
        assert_eq!(get(&v, "q"), Value::Int(8080));
        assert_eq!(get(&v, "g"), Value::Int(0));
        // dynamic key via brackets on an object is forbidden
        let bad = "def mk()\n  a: 1\nend\nk = \"a\"\nx: mk()[k]";
        assert_eq!(eval(bad).unwrap_err().code, "E0318");
    }

    #[test]
    fn keys_values_contains_join() {
        let src = "def mk()\n  a: 1\n  b: 2\nend\nks: mk().keys().join(\",\")\nvs: mk().values()\nhk: mk().contains(\"a\")\nhe: [1, 2].contains(3)\nhs: \"hello\".contains(\"ell\")";
        let v = eval(src).unwrap();
        assert_eq!(get(&v, "ks"), Value::str("a,b"));
        assert_eq!(
            get(&v, "vs"),
            Value::list(vec![Value::Int(1), Value::Int(2)])
        );
        assert_eq!(get(&v, "hk"), Value::Bool(true));
        assert_eq!(get(&v, "he"), Value::Bool(false));
        assert_eq!(get(&v, "hs"), Value::Bool(true));
    }

    #[test]
    fn stdlib_string_and_numeric_methods() {
        let src = concat!(
            "parts: \"a,b,c\".split(\",\")\n",
            "trimmed: \"  hi \".trim()\n",
            "repl: \"a-b-c\".replace(\"-\", \"_\")\n",
            "sw: \"hello\".starts_with(\"he\")\n",
            "ew: \"hello\".ends_with(\"lo\")\n",
            "n: \" 42 \".to_int()\n",
            "f: \"3.5\".to_float()\n",
            "neg: (0 - 7).abs()\n",
            "s: 123.to_str()\n"
        );
        let v = eval(src).unwrap();
        assert_eq!(
            get(&v, "parts"),
            Value::list(vec![Value::str("a"), Value::str("b"), Value::str("c")])
        );
        assert_eq!(get(&v, "trimmed"), Value::str("hi"));
        assert_eq!(get(&v, "repl"), Value::str("a_b_c"));
        assert_eq!(get(&v, "sw"), Value::Bool(true));
        assert_eq!(get(&v, "ew"), Value::Bool(true));
        assert_eq!(get(&v, "n"), Value::Int(42));
        assert_eq!(get(&v, "f"), Value::Float(3.5));
        assert_eq!(get(&v, "neg"), Value::Int(7));
        assert_eq!(get(&v, "s"), Value::str("123"));
        assert_eq!(eval("x: \"nope\".to_int()").unwrap_err().code, "E0314");
    }

    #[test]
    fn schema_optional_fields_with_defaults() {
        // omitted optional fields take their defaults; a default can reference a var.
        let src = concat!(
            "base = 8000\n",
            "type Service\n",
            "  name: String\n",
            "  port: Int = 8080\n",
            "  tier: String = \"backend\"\n",
            "  offset: Int = base + 1\n",
            "end\n",
            "full: new Service\n",
            "  name: \"a\"\n",
            "  port: 9090\n",
            "  tier: \"frontend\"\n",
            "  offset: 5\n",
            "end\n",
            "defaulted: new Service\n",
            "  name: \"b\"\n",
            "end\n"
        );
        let v = eval(src).unwrap();
        let full = get(&v, "full");
        assert_eq!(get(&full, "port"), Value::Int(9090));
        assert_eq!(get(&full, "tier"), Value::str("frontend"));
        let d = get(&v, "defaulted");
        assert_eq!(get(&d, "name"), Value::str("b"));
        assert_eq!(get(&d, "port"), Value::Int(8080)); // default
        assert_eq!(get(&d, "tier"), Value::str("backend")); // default
        assert_eq!(get(&d, "offset"), Value::Int(8001)); // default references `base`
                                                         // a required field (no default) is still E0511 when missing
        let missing = "type S\n  name: String\n  port: Int\nend\nx: new S\n  name: \"z\"\nend";
        assert_eq!(eval(missing).unwrap_err().code, "E0511");
        // a default of the wrong type is caught as E0512
        let badty = "type S\n  n: Int = \"oops\"\nend\nx: new S\nend";
        assert_eq!(eval(badty).unwrap_err().code, "E0512");
    }

    #[test]
    fn cond_expression() {
        let src = concat!(
            "region = \"us\"\n",
            "tier: cond\n",
            "  region == \"eu\" -> \"frankfurt\"\n",
            "  region == \"us\" -> \"virginia\"\n",
            "  else -> \"singapore\"\n",
            "end\n",
            "fallback: cond\n",
            "  false -> 1\n",
            "  else -> 42\n",
            "end\n"
        );
        let v = eval(src).unwrap();
        assert_eq!(get(&v, "tier"), Value::str("virginia"));
        assert_eq!(get(&v, "fallback"), Value::Int(42));
        // non-Bool condition -> E0306 (eval-level)
        let bad = "x: cond\n  \"nope\" -> 1\n  else -> 2\nend";
        assert_eq!(eval(bad).unwrap_err().code, "E0306");
    }

    #[test]
    fn range_builtin() {
        let v = eval("a: range(3)\nb: range(0)\nc: range(2).map (i, _) -> i * 10 end").unwrap();
        assert_eq!(
            get(&v, "a"),
            Value::list(vec![Value::Int(0), Value::Int(1), Value::Int(2)])
        );
        assert_eq!(get(&v, "b"), Value::list(vec![]));
        assert_eq!(
            get(&v, "c"),
            Value::list(vec![Value::Int(0), Value::Int(10)])
        );
        assert_eq!(eval("x: range(-1)").unwrap_err().code, "E0306");
        assert_eq!(eval("x: range(\"3\")").unwrap_err().code, "E0306");
    }

    #[test]
    fn stdlib_list_methods() {
        let src = concat!(
            "sorted: [3, 1, 2].sort()\n",
            "rev: [1, 2, 3].reverse()\n",
            "total: [1, 2, 3].sum()\n",
            "totalf: [1, 2.5].sum()\n",
            "lo: [3, 1, 2].min()\n",
            "hi: [3, 1, 2].max()\n",
            "flat: [[1, 2], [3], 4].flatten()\n",
            "sl: [10, 20, 30, 40].slice(1, 3)\n"
        );
        let v = eval(src).unwrap();
        assert_eq!(
            get(&v, "sorted"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
        assert_eq!(
            get(&v, "rev"),
            Value::list(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
        );
        assert_eq!(get(&v, "total"), Value::Int(6));
        assert_eq!(get(&v, "totalf"), Value::Float(3.5));
        assert_eq!(get(&v, "lo"), Value::Int(1));
        assert_eq!(get(&v, "hi"), Value::Int(3));
        assert_eq!(
            get(&v, "flat"),
            Value::list(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4)
            ])
        );
        assert_eq!(
            get(&v, "sl"),
            Value::list(vec![Value::Int(20), Value::Int(30)])
        );
        // incomparable sort and empty min are errors
        assert_eq!(eval("x: [1, \"a\"].sort()").unwrap_err().code, "E0306");
        assert_eq!(eval("x: [].min()").unwrap_err().code, "E0317");
    }

    #[test]
    fn format_parsing_and_emitting() {
        let v = eval("d = \"{\\\"a\\\": [1, 2]}\".parse_json()\nx: d.a[1]").unwrap();
        assert_eq!(get(&v, "x"), Value::Int(2));

        let v = eval("d = \"a: 1\\nb:\\n  - x\\n\".parse_yaml()\nfirst_b: d.b[0]").unwrap();
        assert_eq!(get(&v, "first_b"), Value::str("x"));

        let v =
            eval("def mk()\n  a: 1\nend\nj: mk().to_json()\ny: mk().to_yaml()\nt: mk().to_toml()")
                .unwrap();
        assert_eq!(get(&v, "j"), Value::str("{\"a\":1}"));
        assert_eq!(get(&v, "y"), Value::str("a: 1\n"));
        assert_eq!(get(&v, "t"), Value::str("a = 1\n"));
    }

    #[test]
    fn durations_and_datetimes() {
        let v = eval(
            "a: \"1h30m\".parse_duration()\nb: \"2d\".parse_duration()\nc: 5400.format_duration()\nd: 0.format_duration()\ne: \"2000-01-01T00:00:00Z\".parse_datetime()\nf: \"2000-01-01T02:00:00+02:00\".parse_datetime()\ng: 946684800.format_datetime()\nh: \"2026-07-18\".parse_datetime().format_datetime()",
        )
        .unwrap();
        assert_eq!(get(&v, "a"), Value::Int(5400));
        assert_eq!(get(&v, "b"), Value::Int(172800));
        assert_eq!(get(&v, "c"), Value::str("1h30m"));
        assert_eq!(get(&v, "d"), Value::str("0s"));
        // 2000-01-01 UTC — a well-known epoch value
        assert_eq!(get(&v, "e"), Value::Int(946684800));
        // the +02:00 offset denotes the same instant
        assert_eq!(get(&v, "f"), Value::Int(946684800));
        assert_eq!(get(&v, "g"), Value::str("2000-01-01T00:00:00Z"));
        assert_eq!(get(&v, "h"), Value::str("2026-07-18T00:00:00Z"));

        assert_eq!(
            eval("x: \"90 minutes\".parse_duration()").unwrap_err().code,
            "E0319"
        );
        assert_eq!(
            eval("x: \"2026-13-01\".parse_datetime()").unwrap_err().code,
            "E0320"
        );
    }

    #[test]
    fn now_is_banned_d13() {
        let d = eval("x: now()").unwrap_err();
        assert_eq!(d.code, "E0533");
        assert!(d.help.is_some());
    }

    #[test]
    fn assert_failure_is_e0530() {
        let d = eval("assert 1 > 2, \"nope\"").unwrap_err();
        assert_eq!(d.code, "E0530");
        assert_eq!(d.message, "nope");
    }

    #[test]
    fn def_call_and_object_merge() {
        let v = eval("def labels(app)\n  app: app\n  managed_by: \"aura\"\nend\na = labels(\"x\")\nb = labels(\"y\")\norig: a\nm: a.merge(b)").unwrap();
        assert_eq!(get(&get(&v, "orig"), "app"), Value::str("x"));
        assert_eq!(get(&get(&v, "m"), "app"), Value::str("y")); // right side wins
        assert_eq!(get(&get(&v, "m"), "managed_by"), Value::str("aura"));
    }
}
