//! Name resolution: which binding does each identifier occurrence refer to?
//!
//! The walk deliberately mirrors [`crate::analysis`], which in turn mirrors the
//! runtime `Environment` — a test asserts the two agree on every binding, so the
//! answers here cannot drift from what evaluation actually does. This is what
//! makes scope-precise editor features safe: `x` in one scope and `x` in another
//! are different bindings, and a rename must never conflate them.
//!
//! Consumers get exact byte spans for the declaration and for every use,
//! including uses inside `#{...}` interpolation.

use std::collections::HashMap;

use crate::lexer::token::StrPart;
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::ast::*;
use crate::parser::Parser;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Var,
    Param,
    Import,
    Func,
    /// `type` or `enum` — a declared name usable as a field type or in `new`.
    Type,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    /// The span of the declared *name* token, not of the whole statement.
    pub decl: Span,
    pub kind: BindingKind,
    /// Byte range of the scope this binding lives in; a rename's safety checks
    /// need it to reason about capture.
    pub scope: Span,
    /// `pub def` / `pub type` / `pub enum` (D12) — part of the module's API, so
    /// importers this file cannot see may depend on the name.
    pub public: bool,
}

/// One identifier occurrence that is not a declaration.
#[derive(Debug, Clone)]
pub struct Use {
    pub span: Span,
    /// `None` for builtins (`env`, `range`, …) and undefined names.
    pub binding: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Resolution {
    pub bindings: Vec<Binding>,
    pub uses: Vec<Use>,
}

impl Resolution {
    /// The binding the identifier covering `offset` refers to — whether the
    /// cursor sits on the declaration itself or on any use of it.
    pub fn binding_at(&self, offset: u32) -> Option<usize> {
        let covers = |s: &Span| s.start <= offset && offset <= s.end;
        if let Some(i) = self.bindings.iter().position(|b| covers(&b.decl)) {
            return Some(i);
        }
        self.uses
            .iter()
            .find(|u| covers(&u.span))
            .and_then(|u| u.binding)
    }

    /// The declaration plus every use of `binding`, in source order.
    pub fn occurrences(&self, binding: usize) -> Vec<Span> {
        let mut out: Vec<Span> = self
            .uses
            .iter()
            .filter(|u| u.binding == Some(binding))
            .map(|u| u.span)
            .collect();
        out.push(self.bindings[binding].decl);
        out.sort_by_key(|s| (s.start, s.end));
        out.dedup_by_key(|s| (s.start, s.end));
        out
    }

    /// Bindings of `name` whose scope overlaps `scope` — i.e. those an inner or
    /// outer scope would capture if something were renamed to `name`.
    pub fn conflicting(&self, name: &str, scope: Span) -> Vec<&Binding> {
        self.bindings
            .iter()
            .filter(|b| b.name == name && b.scope.start <= scope.end && scope.start <= b.scope.end)
            .collect()
    }
}

/// Resolve every name in `module`. `src` must be the exact text the module was
/// parsed from: declaration spans are narrowed to the name token by re-lexing it.
pub fn resolve(src: &str, module: &Module<'_>) -> Resolution {
    let toks = Lexer::new(src, module.span.source)
        .tokenize()
        .unwrap_or_default();
    let mut r = Resolver {
        src,
        toks,
        out: Resolution::default(),
        scopes: Vec::new(),
        bias: 0,
    };
    r.push(module.span);
    for imp in &module.imports {
        // The alias is the identifier after `as`.
        let decl = r.alias_span(imp).unwrap_or(imp.span);
        r.declare(imp.alias, decl, BindingKind::Import);
    }
    for stmt in &module.stmts {
        r.walk_stmt(stmt);
    }
    r.pop();
    r.out
}

/// A lexical scope: its byte range and the bindings declared directly in it.
struct Scope<'a> {
    range: Span,
    /// Indices into `Resolution::bindings`, grouped by name in declaration order.
    /// Keyed by name rather than a flat list so a lookup does not scan every
    /// binding in the scope — the module scope of a large manifest holds thousands,
    /// and each use would then cost a full pass.
    decls: HashMap<&'a str, Vec<usize>>,
}

struct Resolver<'a> {
    src: &'a str,
    toks: Vec<Token<'a>>,
    out: Resolution,
    scopes: Vec<Scope<'a>>,
    /// Offset added to recorded use spans. Non-zero only while walking a `#{...}`
    /// sub-expression, which is lexed standalone and so reports spans from 0.
    bias: u32,
}

impl<'a> Resolver<'a> {
    fn push(&mut self, range: Span) {
        self.scopes.push(Scope {
            range,
            decls: HashMap::new(),
        });
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &'a str, decl: Span, kind: BindingKind) {
        self.declare_vis(name, decl, kind, false)
    }

    fn declare_vis(&mut self, name: &'a str, decl: Span, kind: BindingKind, public: bool) {
        let scope = self.scopes.last().map(|s| s.range).unwrap_or(decl);
        self.out.bindings.push(Binding {
            name: name.to_string(),
            decl,
            kind,
            scope,
            public,
        });
        let idx = self.out.bindings.len() - 1;
        if let Some(s) = self.scopes.last_mut() {
            s.decls.entry(name).or_default().push(idx);
        }
    }

    /// Innermost binding of `name` visible at `at`. Within one scope a use
    /// prefers a declaration at or before it, so a shadowing `=` later in the
    /// same scope does not capture earlier uses.
    fn lookup(&self, name: &str, at: u32) -> Option<usize> {
        for scope in self.scopes.iter().rev() {
            let Some(candidates) = scope.decls.get(name) else {
                continue;
            };
            let mut fallback = None;
            for &i in candidates.iter().rev() {
                if self.out.bindings[i].decl.start <= at {
                    return Some(i);
                }
                fallback = Some(i);
            }
            if let Some(i) = fallback {
                return Some(i);
            }
        }
        None
    }

    fn use_of(&mut self, name: &str, span: Span) {
        let span = Span {
            source: span.source,
            start: span.start + self.bias,
            end: span.end + self.bias,
        };
        let binding = self.lookup(name, span.start);
        self.out.uses.push(Use { span, binding });
    }

    // ---- narrowing statement spans down to the identifier token ----

    /// Tokens whose span lies inside `range`.
    ///
    /// Tokens are sorted by start offset, so the sub-range is found by binary
    /// search and then walked until it leaves `range`. Scanning the whole vector
    /// instead — one full pass per declaration — made resolution quadratic in file
    /// size, and an editor pays this on every keystroke: 119 ms on a 190 KB
    /// manifest, against 6.2 ms with this and the name-keyed `Scope::decls`. The
    /// indexing costs ~2 µs on a small file, which is the right trade.
    /// See `benches/resolve.rs`.
    fn toks_in(&self, range: Span) -> impl Iterator<Item = (usize, &Token<'a>)> {
        let first = self.toks.partition_point(|t| t.span.start < range.start);
        self.toks[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, t)| t.span.start <= range.end)
            .filter(move |(_, t)| t.span.end <= range.end)
            .map(move |(i, t)| (first + i, t))
    }

    /// The first `Ident(name)` token inside `range`.
    fn name_span(&self, range: Span, name: &str) -> Option<Span> {
        self.toks_in(range)
            .find(|(_, t)| matches!(t.kind, TokenKind::Ident(n) if n == name))
            .map(|(_, t)| t.span)
    }

    fn alias_span(&self, imp: &Import<'_>) -> Option<Span> {
        let mut seen_as = false;
        for (_, t) in self.toks_in(imp.span) {
            if seen_as {
                if matches!(t.kind, TokenKind::Ident(n) if n == imp.alias) {
                    return Some(t.span);
                }
            } else if matches!(t.kind, TokenKind::As) {
                seen_as = true;
            }
        }
        None
    }

    /// Parameter name spans: the `Ident` tokens of the first parenthesised group
    /// in `range`. Scanning the group rather than the whole statement keeps
    /// `def f(f)` from pointing the parameter at the function's own name.
    fn param_spans(&self, range: Span, params: &[&str]) -> Vec<Option<Span>> {
        let mut idents: Vec<(&str, Span)> = Vec::new();
        let mut depth = 0usize;
        for (_, t) in self.toks_in(range) {
            match t.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Ident(n) if depth == 1 => idents.push((n, t.span)),
                _ => {}
            }
        }
        // Match positionally; fall back to by-name if the shapes disagree.
        params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                idents
                    .get(i)
                    .filter(|(n, _)| n == p)
                    .or_else(|| idents.iter().find(|(n, _)| n == p))
                    .map(|(_, s)| *s)
            })
            .collect()
    }

    // ---- the walk (mirrors analysis::SemanticAnalyzer) ----

    fn walk_stmt(&mut self, stmt: &Stmt<'a>) {
        match stmt {
            Stmt::Assign {
                name, value, span, ..
            } => {
                self.walk_expr(value); // the value does not see its own name
                let decl = self.name_span(*span, name).unwrap_or(*span);
                self.declare(name, decl, BindingKind::Var);
            }
            Stmt::Property { value, .. } => self.walk_expr(value),
            Stmt::Assert { cond, message, .. } => {
                self.walk_expr(cond);
                if let Some(m) = message {
                    self.walk_expr(m);
                }
            }
            Stmt::EnumDecl(en) => {
                let decl = self.name_span(en.span, en.name).unwrap_or(en.span);
                self.declare_vis(en.name, decl, BindingKind::Type, en.public);
            }
            Stmt::TypeDecl(schema) => {
                for f in &schema.fields {
                    // A custom field type is a use of that schema/enum name.
                    if let TypeName::Custom(name) = f.ty {
                        if let Some(s) = self.field_type_span(schema, f.name, name) {
                            self.use_of(name, s);
                        }
                    }
                    if let Some(default) = &f.default {
                        self.walk_expr(default);
                    }
                }
                let decl = self
                    .name_span(schema.span, schema.name)
                    .unwrap_or(schema.span);
                self.declare_vis(schema.name, decl, BindingKind::Type, schema.public);
            }
            Stmt::FuncDecl {
                name,
                params,
                body,
                public,
                span,
            } => {
                let decl = self.name_span(*span, name).unwrap_or(*span);
                self.declare_vis(name, decl, BindingKind::Func, *public);
                self.push(*span);
                for (p, s) in params.iter().zip(self.param_spans(*span, params)) {
                    self.declare(p, s.unwrap_or(*span), BindingKind::Param);
                }
                for s in body {
                    self.walk_stmt(s);
                }
                self.pop();
            }
            Stmt::Block(block) => {
                self.walk_expr(&block.label);
                self.push(block.span);
                for s in &block.body {
                    self.walk_stmt(s);
                }
                self.pop();
            }
            Stmt::Expr(e) => self.walk_expr(e),
        }
    }

    /// The span of a field's custom type name, e.g. `Tier` in `tier: Tier`.
    /// Looked up after the field's own name so a schema whose field shares the
    /// type's name still points at the type.
    fn field_type_span(
        &self,
        schema: &SchemaDeclaration<'a>,
        field: &str,
        ty: &str,
    ) -> Option<Span> {
        let mut after_field = false;
        for (_, t) in self.toks_in(schema.span) {
            match t.kind {
                TokenKind::Ident(n) if n == field && !after_field => after_field = true,
                TokenKind::Ident(n) if after_field && n == ty => return Some(t.span),
                TokenKind::Newline if after_field => after_field = false,
                _ => {}
            }
        }
        None
    }

    fn walk_object_body(&mut self, body: &ObjectBody<'a>) {
        for (_, value, _) in &body.props {
            self.walk_expr(value);
        }
    }

    fn walk_expr(&mut self, e: &Expr<'a>) {
        match e {
            Expr::Literal(LitValue::InterpStr(parts), span) => {
                for part in parts {
                    if let StrPart::Interp(sub) = part {
                        self.walk_interp(sub, *span);
                    }
                }
            }
            Expr::Literal(..) => {}
            Expr::Variable(name, span) => self.use_of(name, *span),
            Expr::Unary { rhs, .. } => self.walk_expr(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
                ..
            } => {
                self.walk_expr(cond);
                self.walk_expr(then);
                self.walk_expr(otherwise);
            }
            Expr::Cond {
                arms, otherwise, ..
            } => {
                for (c, v) in arms {
                    self.walk_expr(c);
                    self.walk_expr(v);
                }
                self.walk_expr(otherwise);
            }
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::MethodCall {
                recv, args, lambda, ..
            } => {
                // `method` is a stdlib name, never a binding.
                self.walk_expr(recv);
                for a in args {
                    self.walk_expr(a);
                }
                if let Some(l) = lambda {
                    self.walk_expr(l);
                }
            }
            // `field` is a key in the receiver, not a binding.
            Expr::FieldAccess { recv, .. } => self.walk_expr(recv),
            Expr::Index { recv, key, .. } => {
                self.walk_expr(recv);
                self.walk_expr(key);
            }
            Expr::ObjectLiteral(body) => self.walk_object_body(body),
            Expr::ListLiteral(items, _) => {
                for i in items {
                    self.walk_expr(i);
                }
            }
            Expr::Lambda { params, body, span } => {
                self.push(*span);
                for (p, s) in params.iter().zip(self.param_spans(*span, params)) {
                    self.declare(p, s.unwrap_or(*span), BindingKind::Param);
                }
                match body {
                    LambdaBody::Expr(e) => self.walk_expr(e),
                    LambdaBody::Block(stmts) => {
                        for s in stmts {
                            self.walk_stmt(s);
                        }
                    }
                }
                self.pop();
            }
            Expr::SchemaInstance {
                schema,
                schema_alias,
                body,
                span,
            } => {
                // `new Name` uses the schema; `new mod.Name` uses the module alias.
                let target = schema_alias.unwrap_or(schema);
                if let Some(s) = self.name_span(*span, target) {
                    self.use_of(target, s);
                }
                self.walk_object_body(body);
            }
        }
    }

    /// Uses inside `#{...}`. The part is a subslice of `src`, so its absolute
    /// offset is recoverable and the spans stay usable for renaming.
    fn walk_interp(&mut self, sub: &'a str, outer: Span) {
        let Some(base) = subslice_offset(self.src, sub) else {
            return;
        };
        let Ok(expr) = Lexer::new(sub, outer.source)
            .tokenize()
            .and_then(|toks| Parser::new(toks).parse_expression())
        else {
            return;
        };
        // Walk it in the *current* scopes — interpolation sees the enclosing
        // bindings — with the bias that turns its spans absolute, so resolution
        // (which is position-sensitive) sees the real offsets too.
        let outer_bias = self.bias;
        self.bias = base;
        self.walk_expr(&expr);
        self.bias = outer_bias;
    }
}

/// Byte offset of `sub` within `outer`, if `sub` really is a subslice of it.
fn subslice_offset(outer: &str, sub: &str) -> Option<u32> {
    let (o, s) = (outer.as_ptr() as usize, sub.as_ptr() as usize);
    (s >= o && s + sub.len() <= o + outer.len()).then(|| (s - o) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(src: &str) -> Resolution {
        let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
        let module = Parser::new(toks).parse_module().expect("parse ok");
        resolve(src, &module)
    }

    /// The text of each occurrence of the binding under the cursor at `offset`.
    fn occurrences_at(src: &str, offset: u32) -> Vec<(u32, &str)> {
        let r = resolved(src);
        let b = r.binding_at(offset).expect("a binding under the cursor");
        r.occurrences(b)
            .into_iter()
            .map(|s| (s.start, &src[s.start as usize..s.end as usize]))
            .collect()
    }

    #[test]
    fn same_name_in_two_scopes_is_two_bindings() {
        // The `x` inside the def is a parameter; the top-level `x` is unrelated.
        // Conflating them is exactly the corruption a rename must never cause.
        let src = "x = 1\ndef f(x)\n  y: x\nend\nz: x\n";
        let outer = occurrences_at(src, 0); // on the top-level `x`
        let inner = occurrences_at(src, 12); // on the parameter
        assert_eq!(outer.len(), 2, "decl + the use in `z: x`: {outer:?}");
        assert_eq!(inner.len(), 2, "param + the use in `y: x`: {inner:?}");
        // No offset appears in both sets.
        for (o, _) in &outer {
            assert!(!inner.iter().any(|(i, _)| i == o), "{outer:?} vs {inner:?}");
        }
    }

    #[test]
    fn shadow_is_a_distinct_binding() {
        let src = "p = 1\ndomain \"d\"\n  shadow p = 2\n  a: p\nend\nb: p\n";
        let outer = occurrences_at(src, 0);
        let inner = occurrences_at(src, src.find("shadow p").unwrap() as u32 + 7);
        // The outer binding is used only by `b: p` after the block.
        assert_eq!(outer.len(), 2, "{outer:?}");
        // The shadowing binding owns the use inside the block.
        assert_eq!(inner.len(), 2, "{inner:?}");
        let a_use = src.find("a: p").unwrap() as u32 + 3;
        assert!(inner.iter().any(|(o, _)| *o == a_use), "{inner:?}");
    }

    #[test]
    fn uses_inside_interpolation_are_found_at_absolute_offsets() {
        let src = "name = \"api\"\nid: \"#{name}-1\"\n";
        let occ = occurrences_at(src, 0);
        let inside = src.find("#{name}").unwrap() as u32 + 2;
        assert!(
            occ.iter().any(|(o, t)| *o == inside && *t == "name"),
            "interpolated use must be an occurrence: {occ:?}"
        );
    }

    #[test]
    fn lambda_params_and_body_bindings() {
        let src = "xs = [1]\nys: xs.map (v, i) ->\n  d = v * 2\n  out: d + i\nend\n";
        let v = src.find("(v, i)").unwrap() as u32 + 1;
        let occ = occurrences_at(src, v);
        assert_eq!(
            occ.len(),
            2,
            "param `v` and its use in `d = v * 2`: {occ:?}"
        );
        let d = src.find("d = ").unwrap() as u32;
        assert_eq!(occurrences_at(src, d).len(), 2, "`d` decl + use");
    }

    #[test]
    fn field_names_and_methods_are_not_bindings() {
        // `host` is a schema field and a property key, never a variable; `map` is
        // a stdlib method. None of them may be returned as a binding.
        let src = "type E\n  host: String\nend\ne: new E\n  host: \"h\"\nend\n";
        let r = resolved(src);
        assert!(
            r.bindings.iter().all(|b| b.name != "host"),
            "{:?}",
            r.bindings
        );
        // `new E` is a use of the schema.
        let e = src.find("type E").unwrap() as u32 + 5;
        assert_eq!(occurrences_at(src, e).len(), 2, "type decl + `new E`");
    }

    #[test]
    fn conflicting_reports_overlapping_scopes_only() {
        let src = "a = 1\ndef f()\n  b = 2\n  c: b\nend\n";
        let r = resolved(src);
        let a = r.binding_at(0).unwrap();
        // `b` lives in the def's scope, which is nested inside `a`'s module scope.
        assert_eq!(r.conflicting("b", r.bindings[a].scope).len(), 1);
        assert!(r.conflicting("nope", r.bindings[a].scope).is_empty());
    }

    /// The doc claim that resolution matches the interpreter's scopes, checked
    /// against the analyzer that mirrors the Environment: on input the analyzer
    /// accepts, every non-builtin use must resolve to a binding. A scope bug here
    /// (a missed `push`, a body walked in the wrong scope) breaks this.
    #[test]
    fn every_use_resolves_on_input_the_analyzer_accepts() {
        for src in [
            include_str!("../../../../examples/showcase/showcase.aura"),
            include_str!("../../../../examples/showcase/lib.aura"),
            include_str!("../../../../examples/production_deploy.aura"),
        ] {
            let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
            let module = Parser::new(toks).parse_module().expect("parse ok");
            let diags = crate::analysis::analyze(&module, true);
            let undefined: Vec<_> = diags.iter().filter(|d| d.code == "E0504").collect();
            assert!(undefined.is_empty(), "fixture must be clean: {undefined:?}");

            let r = resolve(src, &module);
            let unresolved: Vec<&str> = r
                .uses
                .iter()
                .filter(|u| u.binding.is_none())
                .map(|u| &src[u.span.start as usize..u.span.end as usize])
                .filter(|n| !BUILTIN_NAMES.contains(n))
                .collect();
            assert!(unresolved.is_empty(), "unresolved uses: {unresolved:?}");
        }
    }

    /// Mirrors `analysis::BUILTINS` — names that resolve to no binding by design.
    const BUILTIN_NAMES: &[&str] = &["env", "read_file", "fail", "range"];
}
