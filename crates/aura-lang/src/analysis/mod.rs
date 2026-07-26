//! Static analysis (SPEC §6.1): a single AST pass with a scope stack
//! mirroring the Environment. Runs before the runtime.
//!
//! Errors (always): E0504 undefined variable, static E0301/E0302.
//! Warnings (promoted to errors by the caller under --strict):
//! W0501 unused variable, W0502 unused import, W0503 unused function/type,
//! W0303 useless shadow, W0512 effectful call in imported module.

use indexmap::IndexMap;

use crate::error::{Diagnostic, Severity};
use crate::lexer::token::StrPart;
use crate::lexer::Lexer;
use crate::parser::ast::*;
use crate::parser::Parser;
use crate::span::Span;

const BUILTINS: &[&str] = &["env", "read_file", "fail", "range"];
const EFFECTFUL: &[&str] = &["env", "read_file"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclKind {
    Var,
    /// Function/lambda parameters are never considered dead code (the signature may require them).
    Param,
    Import,
    Func,
    Type,
}

struct Decl {
    span: Span,
    used: bool,
    kind: DeclKind,
}

pub struct SemanticAnalyzer<'a> {
    scopes: Vec<IndexMap<&'a str, Decl>>,
    diags: Vec<Diagnostic>,
    is_root: bool,
}

/// `is_root = false` — analysis of an imported module (includes W0512).
pub fn analyze<'a>(module: &Module<'a>, is_root: bool) -> Vec<Diagnostic> {
    let mut a = SemanticAnalyzer {
        scopes: Vec::new(),
        diags: Vec::new(),
        is_root,
    };
    a.push_scope();
    for imp in &module.imports {
        a.declare(imp.alias, imp.span, DeclKind::Import, false);
    }
    for stmt in &module.stmts {
        a.walk_stmt(stmt);
    }
    a.pop_scope();
    a.diags
}

/// Whether there are blocking diagnostics: errors always, warnings too under strict (SPEC §6.1).
pub fn has_blocking(diags: &[Diagnostic], strict: bool) -> bool {
    diags
        .iter()
        .any(|d| strict || d.severity == Severity::Error)
}

impl<'a> SemanticAnalyzer<'a> {
    fn push_scope(&mut self) {
        self.scopes.push(IndexMap::new());
    }

    /// On leaving a scope — report dead code (SPEC §6.1, step 3).
    fn pop_scope(&mut self) {
        let scope = self.scopes.pop().expect("scope stack underflow");
        for (name, decl) in scope {
            if decl.used {
                continue;
            }
            let (code, what) = match decl.kind {
                DeclKind::Var => ("W0501", "variable"),
                DeclKind::Import => ("W0502", "import"),
                DeclKind::Func => ("W0503", "function"),
                DeclKind::Type => ("W0503", "type"),
                DeclKind::Param => continue,
            };
            self.diags.push(Diagnostic::warning(
                code,
                format!("unused {what} '{name}'"),
                decl.span,
                "never used",
            ));
        }
    }

    fn declare(&mut self, name: &'a str, span: Span, kind: DeclKind, shadow: bool) {
        let in_current = self.scopes.last().is_some_and(|s| s.contains_key(name));
        if in_current {
            self.diags.push(Diagnostic::error(
                "E0301",
                format!("'{name}' is already defined in this scope"),
                span,
                "duplicate definition",
            ));
            return;
        }
        let outer = self.lookup_span(name);
        match (shadow, outer) {
            (false, Some(orig)) if kind == DeclKind::Var => {
                let mut d = Diagnostic::error(
                    "E0302",
                    format!("'{name}' shadows an outer variable"),
                    span,
                    "add `shadow`",
                );
                d.secondary
                    .push((orig, "outer variable declared here".to_string()));
                d.help = Some(format!(
                    "write `shadow {name} = ...` to make the shadowing explicit"
                ));
                self.diags.push(d);
            }
            (true, None) => {
                self.diags.push(Diagnostic::warning(
                    "W0303",
                    format!("`shadow` on '{name}' shadows nothing"),
                    span,
                    "remove `shadow`",
                ));
            }
            _ => {}
        }
        self.scopes.last_mut().unwrap().insert(
            name,
            Decl {
                span,
                used: false,
                kind,
            },
        );
    }

    fn lookup_span(&self, name: &str) -> Option<Span> {
        self.scopes
            .iter()
            .rev()
            .skip(1)
            .find_map(|s| s.get(name).map(|d| d.span))
    }

    /// Step 2 of SPEC §6.1: mark the nearest declaration (respects shadowing).
    fn mark_used(&mut self, name: &str, span: Span) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(decl) = scope.get_mut(name) {
                decl.used = true;
                return;
            }
        }
        if !BUILTINS.contains(&name) {
            self.diags.push(Diagnostic::error(
                "E0504",
                format!("use of undefined variable '{name}'"),
                span,
                "not found in any scope",
            ));
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt<'a>) {
        match stmt {
            Stmt::Assign {
                name,
                shadow,
                value,
                span,
            } => {
                self.walk_expr(value); // the value does not see its own name
                self.declare(name, *span, DeclKind::Var, *shadow);
            }
            Stmt::Property { value, .. } => self.walk_expr(value),
            Stmt::Assert { cond, message, .. } => {
                self.walk_expr(cond);
                if let Some(m) = message {
                    self.walk_expr(m);
                }
            }
            Stmt::TypeDecl(schema) => {
                for f in &schema.fields {
                    if let TypeName::Custom(name) = f.ty {
                        self.mark_used(name, schema.span);
                    }
                    // a default expression may reference variables (e.g. `= base_port`)
                    if let Some(default) = &f.default {
                        self.walk_expr(default);
                    }
                }
                self.declare(schema.name, schema.span, DeclKind::Type, false);
                // D12: pub is the module's API, not dead code
                if schema.public {
                    self.mark_used(schema.name, schema.span);
                }
            }
            Stmt::FuncDecl {
                name,
                params,
                body,
                public,
                span,
            } => {
                self.declare(name, *span, DeclKind::Func, false);
                // D12: pub is the module's API, not dead code
                if *public {
                    self.mark_used(name, *span);
                }
                self.push_scope();
                for p in params {
                    self.declare(p, *span, DeclKind::Param, false);
                }
                self.walk_stmt_body(body);
                self.pop_scope();
            }
            Stmt::Block(block) => self.walk_block(block),
            Stmt::Expr(e) => self.walk_expr(e),
        }
    }

    fn walk_block(&mut self, block: &BlockDeclaration<'a>) {
        self.walk_expr(&block.label);
        self.push_scope();
        for stmt in &block.body {
            self.walk_stmt(stmt);
        }
        self.pop_scope();
    }

    /// A code body (D17): statements in the caller's freshly pushed scope.
    fn walk_stmt_body(&mut self, body: &[Stmt<'a>]) {
        for stmt in body {
            self.walk_stmt(stmt);
        }
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
                    if let StrPart::Interp(src) = part {
                        self.walk_interp(src, *span);
                    }
                }
            }
            Expr::Literal(..) => {}
            Expr::Variable(name, span) => self.mark_used(name, *span),
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
                for (condition, value) in arms {
                    self.walk_expr(condition);
                    self.walk_expr(value);
                }
                self.walk_expr(otherwise);
            }
            Expr::Call { callee, args, span } => {
                // W0512: an effectful call in an imported module (SPEC §6.1, D1)
                if let Expr::Variable(name, _) = callee.as_ref() {
                    if !self.is_root && EFFECTFUL.contains(name) {
                        self.diags.push(Diagnostic::warning(
                            "W0512",
                            format!("effectful call {name}() in imported module"),
                            *span,
                            "imports have no I/O capability by default",
                        ));
                    }
                }
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::MethodCall {
                recv, args, lambda, ..
            } => {
                self.walk_expr(recv);
                for a in args {
                    self.walk_expr(a);
                }
                if let Some(l) = lambda {
                    self.walk_expr(l);
                }
            }
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
                self.push_scope();
                for p in params {
                    self.declare(p, *span, DeclKind::Param, false);
                }
                match body {
                    LambdaBody::Expr(e) => self.walk_expr(e),
                    LambdaBody::Block(b) => self.walk_stmt_body(b),
                }
                self.pop_scope();
            }
            Expr::SchemaInstance {
                schema,
                schema_alias,
                body,
                span,
            } => {
                // `new alias.Schema` uses the import alias; the schema itself lives in another module
                match schema_alias {
                    Some(alias) => self.mark_used(alias, *span),
                    None => self.mark_used(schema, *span),
                }
                self.walk_object_body(body);
            }
        }
    }

    /// `#{expr}` is parsed and analyzed as an ordinary expression — variables
    /// used only inside an interpolation are not considered dead.
    fn walk_interp(&mut self, src: &'a str, span: Span) {
        let parsed = Lexer::new(src, span.source)
            .tokenize()
            .and_then(|toks| Parser::new(toks).parse_expression());
        match parsed {
            Ok(expr) => self.walk_expr(&expr),
            Err(d) => self.diags.push(Diagnostic::error(
                "E0316",
                format!("invalid interpolation: {}", d.message),
                span,
                "in #{...}",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diags(src: &str) -> Vec<Diagnostic> {
        diags_as(src, true)
    }

    fn diags_as(src: &str, is_root: bool) -> Vec<Diagnostic> {
        let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
        let module = Parser::new(toks).parse_module().expect("parse ok");
        analyze(&module, is_root)
    }

    fn codes(src: &str) -> Vec<&'static str> {
        diags(src).into_iter().map(|d| d.code).collect()
    }

    #[test]
    fn dead_code_detection() {
        assert_eq!(codes("x = 1"), vec!["W0501"]);
        assert_eq!(
            codes("import \"a.aura\" as a\nx = 1\ny = x"),
            vec!["W0502", "W0501"]
        ); // a, y
        assert_eq!(codes("type T\n  a: Int\nend"), vec!["W0503"]);
        assert_eq!(codes("def f(x)\n  a: x\nend"), vec!["W0503"]);
        // used — not dead
        assert!(codes("x = 1\ny = x + 1\nassert y > 0").is_empty());
    }

    #[test]
    fn undefined_variable_is_e0504() {
        assert!(codes("x = nope + 1").contains(&"E0504"));
        // builtins are never considered undefined
        assert!(!codes("assert fail == fail").contains(&"E0504"));
    }

    #[test]
    fn static_shadow_rules() {
        assert!(codes("x = 1\ndomain \"d\"\n  x = 2\nend").contains(&"E0302"));
        assert!(codes("x = 1\nx = 2").contains(&"E0301"));
        assert!(codes("shadow x = 1\nassert x == 1").contains(&"W0303"));
        // a correct shadow — clean
        let src = "x = 1\ndomain \"d\"\n  shadow x = 2\n  assert x == 2\nend\nassert x == 1";
        assert!(codes(src).is_empty());
    }

    #[test]
    fn interpolation_uses_are_counted() {
        // x is used only inside #{} — not dead
        assert!(codes("x = 1\ns = \"v#{x}\"\nassert s == \"v1\"").is_empty());
        // an undefined variable inside #{} is caught
        assert!(codes("s = \"v#{nope}\"\nassert s == \"\"").contains(&"E0504"));
    }

    #[test]
    fn effectful_call_in_import_is_w0512() {
        let src = "x = env(\"A\", \"b\")\nassert x == x";
        assert!(!diags_as(src, true).iter().any(|d| d.code == "W0512"));
        assert!(diags_as(src, false).iter().any(|d| d.code == "W0512"));
    }

    #[test]
    fn strict_upgrades_warnings_to_blocking() {
        let ds = diags("x = 1");
        assert!(!has_blocking(&ds, false));
        assert!(has_blocking(&ds, true));
    }
}
