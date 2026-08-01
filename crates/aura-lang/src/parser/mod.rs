//! Recursive descent + Pratt expression parser (SPEC §3).

pub mod ast;

use crate::error::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::span::Span;
use ast::*;

/// lbp of the ternary `? :`; right associativity — the else branch parses with min_bp = TERNARY_LBP - 1.
const TERNARY_LBP: u8 = 2;
const UNARY_RBP: u8 = 15;

fn infix_bp(kind: &TokenKind<'_>) -> Option<(u8, u8, BinOp)> {
    use TokenKind::*;
    Some(match kind {
        Or => (3, 4, BinOp::Or),
        And => (5, 6, BinOp::And),
        EqEq => (7, 8, BinOp::Eq),
        NotEq => (7, 8, BinOp::Ne),
        Lt => (9, 10, BinOp::Lt),
        Gt => (9, 10, BinOp::Gt),
        LtEq => (9, 10, BinOp::Le),
        GtEq => (9, 10, BinOp::Ge),
        Plus => (11, 12, BinOp::Add),
        Minus => (11, 12, BinOp::Sub),
        Star => (13, 14, BinOp::Mul),
        Slash => (13, 14, BinOp::Div),
        Percent => (13, 14, BinOp::Rem),
        _ => return None,
    })
}

/// Recursion-depth cap for the recursive-descent parser: crafted deeply-nested
/// input (`((((…`, `[[[[…`, nested blocks/objects) would otherwise overflow the
/// stack — a DoS on untrusted input (e.g. a package pulled via `aura add`).
const MAX_PARSE_DEPTH: u32 = 256;

pub struct Parser<'a> {
    toks: Vec<Token<'a>>,
    pos: usize,
    diags: Vec<Diagnostic>,
    depth: u32,
}

impl<'a> Parser<'a> {
    pub fn new(toks: Vec<Token<'a>>) -> Self {
        debug_assert!(matches!(toks.last().map(|t| &t.kind), Some(TokenKind::Eof)));
        Parser {
            toks,
            pos: 0,
            diags: Vec::new(),
            depth: 0,
        }
    }

    /// Enter one nesting level; E0208 past the cap. Paired with `leave()`.
    fn enter(&mut self) -> Result<(), Diagnostic> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            self.depth -= 1;
            return Err(self.err(
                "E0208",
                "nesting too deep",
                "simplify or split this expression/block",
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ---- Token navigation ----

    fn peek(&self) -> &TokenKind<'a> {
        &self.toks[self.pos].kind
    }

    fn peek_at(&self, off: usize) -> &TokenKind<'a> {
        self.toks
            .get(self.pos + off)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_end_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn bump(&mut self) {
        if !matches!(self.peek(), TokenKind::Eof) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, kind: &TokenKind<'_>) -> bool {
        if self.peek() == kind {
            self.bump();
            true
        } else {
            false
        }
    }

    fn join(&self, from: Span) -> Span {
        Span {
            source: from.source,
            start: from.start,
            end: self.prev_end_span().end,
        }
    }

    fn err(
        &self,
        code: &'static str,
        msg: impl Into<String>,
        label: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::error(code, msg, self.span(), label)
    }

    fn expect(&mut self, kind: &TokenKind<'_>, what: &str) -> Result<(), Diagnostic> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.err(
                "E0200",
                format!("expected {what}"),
                format!("found {:?}", self.peek()),
            ))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<(&'a str, Span), Diagnostic> {
        if let TokenKind::Ident(s) = self.peek() {
            let (s, sp) = (*s, self.span());
            self.bump();
            Ok((s, sp))
        } else {
            Err(self.err(
                "E0200",
                format!("expected {what}"),
                format!("found {:?}", self.peek()),
            ))
        }
    }

    /// Property key: identifier or string (D11: `"app.kubernetes.io/name": ...`).
    fn expect_key(&mut self) -> Result<(&'a str, Span), Diagnostic> {
        match self.peek() {
            TokenKind::Ident(s) | TokenKind::Str(s) => {
                let (s, sp) = (*s, self.span());
                self.bump();
                Ok((s, sp))
            }
            _ => Err(self.err(
                "E0200",
                "expected property key",
                format!("found {:?}", self.peek()),
            )),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.bump();
        }
    }

    /// Statement separator: Newline, or an allowed boundary (`end`/Eof).
    fn eat_separator(&mut self) -> Result<(), Diagnostic> {
        match self.peek() {
            TokenKind::Newline => {
                self.skip_newlines();
                Ok(())
            }
            TokenKind::End | TokenKind::Eof => Ok(()),
            _ => Err(self.err(
                "E0205",
                "expected end of statement",
                "statements are separated by newlines",
            )),
        }
    }

    /// Error recovery: skip to the next Newline / `end` / Eof (SPEC §3.4).
    fn recover(&mut self) {
        while !matches!(
            self.peek(),
            TokenKind::Newline | TokenKind::End | TokenKind::Eof
        ) {
            self.bump();
        }
        self.skip_newlines();
    }

    /// Entry point for a single expression (`#{...}` interpolations, REPL).
    pub fn parse_expression(mut self) -> Result<Expr<'a>, Diagnostic> {
        let e = self.parse_expr(0)?;
        self.skip_newlines();
        if !matches!(self.peek(), TokenKind::Eof) {
            return Err(self.err(
                "E0204",
                "unexpected trailing tokens after expression",
                "expected end of expression",
            ));
        }
        Ok(e)
    }

    // ---- Module ----

    /// Debug builds parse on a thread with a 64 MiB stack. `MAX_PARSE_DEPTH`
    /// already caps the recursion at 256 frames, which a release build clears in
    /// tens of kilobytes — but a debug frame is roughly ten times larger, and 256
    /// of them can approach the ~1 MiB default stack on Windows before the guard
    /// trips.
    ///
    /// Release builds — and wasm, which has no threads at all — recurse on the
    /// stack they are given. This is not a weakening: the guard is what makes the
    /// parser DoS-safe, and it is unconditional. The thread only ever bought head
    /// room for fat debug frames.
    ///
    /// It bought that head room expensively. Spawning the thread costs about
    /// 115 µs, against 7.8 µs for parsing the reference manifest — sixteen times
    /// the work it protects, paid on every `check`, every `eval`, and every
    /// reparse the language server does while someone types.
    #[cfg(all(not(target_family = "wasm"), debug_assertions))]
    pub fn parse_module(self) -> Result<Module<'a>, Vec<Diagnostic>> {
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn_scoped(s, move || self.parse_module_impl())
                .expect("spawn parser thread")
                .join()
                .expect("parser thread panicked")
        })
    }

    /// See the note on the threaded version above.
    #[cfg(any(target_family = "wasm", not(debug_assertions)))]
    pub fn parse_module(self) -> Result<Module<'a>, Vec<Diagnostic>> {
        self.parse_module_impl()
    }

    fn parse_module_impl(mut self) -> Result<Module<'a>, Vec<Diagnostic>> {
        let start = self.span();
        let mut imports = Vec::new();
        let mut stmts = Vec::new();
        self.skip_newlines();
        while matches!(self.peek(), TokenKind::Import) {
            match self.parse_import() {
                Ok(i) => {
                    imports.push(i);
                    if let Err(d) = self.eat_separator() {
                        self.diags.push(d);
                        self.recover();
                    }
                }
                Err(d) => {
                    self.diags.push(d);
                    self.recover();
                }
            }
        }
        while !matches!(self.peek(), TokenKind::Eof) {
            let before = self.pos;
            match self.parse_stmt().and_then(|s| {
                self.eat_separator()?;
                Ok(s)
            }) {
                Ok(s) => stmts.push(s),
                Err(d) => {
                    self.diags.push(d);
                    self.recover();
                    // Guarantee forward progress: if the error left us on a token that
                    // `recover()` treats as a boundary (e.g. a stray `end`), consume it so
                    // the loop can't spin forever pushing the same diagnostic.
                    if self.pos == before {
                        self.bump();
                    }
                }
            }
        }
        if self.diags.is_empty() {
            Ok(Module {
                imports,
                stmts,
                span: self.join(start),
            })
        } else {
            Err(self.diags)
        }
    }

    fn parse_import(&mut self) -> Result<Import<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // import
        let source = match self.peek() {
            TokenKind::ImportPath { path, version } => {
                let s = ImportSource::Registry { path, version };
                self.bump();
                s
            }
            TokenKind::Str(p) => {
                let s = ImportSource::File(p);
                self.bump();
                s
            }
            _ => {
                return Err(self.err(
                    "E0200",
                    "expected import path or \"file.aura\"",
                    "invalid import source",
                ))
            }
        };
        self.expect(&TokenKind::As, "`as` after import source")?;
        let (alias, _) = self.expect_ident("import alias")?;
        Ok(Import {
            source,
            alias,
            span: self.join(start),
        })
    }

    // ---- Statements ----

    fn parse_stmt(&mut self) -> Result<Stmt<'a>, Diagnostic> {
        self.enter()?;
        let r = self.parse_stmt_impl();
        self.leave();
        r
    }

    fn parse_stmt_impl(&mut self) -> Result<Stmt<'a>, Diagnostic> {
        match self.peek() {
            TokenKind::Type => self.parse_type_decl(false),
            TokenKind::Enum => self.parse_enum_decl(false),
            TokenKind::Def => self.parse_func_decl(false),
            // D12: `pub` is only allowed before def/type
            TokenKind::Pub => {
                self.bump();
                match self.peek() {
                    TokenKind::Def => self.parse_func_decl(true),
                    TokenKind::Type => self.parse_type_decl(true),
                    TokenKind::Enum => self.parse_enum_decl(true),
                    _ => Err(self.err(
                        "E0206",
                        "`pub` is only allowed before `def`, `type` or `enum`",
                        "properties are exported by default; `=` bindings are always private",
                    )),
                }
            }
            TokenKind::Domain => Ok(Stmt::Block(self.parse_block(BlockKind::Domain)?)),
            TokenKind::Assert => self.parse_assert(),
            TokenKind::Shadow => {
                let start = self.span();
                self.bump();
                let (name, _) = self.expect_ident("variable name after `shadow`")?;
                self.expect(&TokenKind::Assign, "`=` in shadow assignment")?;
                let value = self.parse_expr(0)?;
                Ok(Stmt::Assign {
                    name,
                    shadow: true,
                    value,
                    span: self.join(start),
                })
            }
            // D11: property with a string key — `"app.io/name": value`
            TokenKind::Str(_) if matches!(self.peek_at(1), TokenKind::Colon) => {
                let start = self.span();
                let (key, _) = self.expect_key()?;
                self.bump(); // :
                let value = self.parse_property_value()?;
                Ok(Stmt::Property {
                    key,
                    value,
                    span: self.join(start),
                })
            }
            TokenKind::Ident(_) => {
                let start = self.span();
                match (self.peek_at(1), self.peek_at(2)) {
                    (TokenKind::Assign, _) => {
                        let (name, _) = self.expect_ident("name")?;
                        self.bump(); // =
                        let value = self.parse_expr(0)?;
                        Ok(Stmt::Assign {
                            name,
                            shadow: false,
                            value,
                            span: self.join(start),
                        })
                    }
                    (TokenKind::Colon, _) => {
                        let (key, _) = self.expect_ident("key")?;
                        self.bump(); // :
                        let value = self.parse_property_value()?;
                        Ok(Stmt::Property {
                            key,
                            value,
                            span: self.join(start),
                        })
                    }
                    // v1.1 inline block removed (D3): `metrics port: 9090 ...`
                    (TokenKind::Ident(_), TokenKind::Colon) => {
                        let mut d = self.err(
                            "E0201",
                            "inline blocks were removed in Aura v1.2",
                            "this looks like an inline block",
                        );
                        if let TokenKind::Ident(name) = self.peek() {
                            d.help = Some(format!(
                                "write it as an object block:\n{name}:\n  key: value\nend"
                            ));
                        }
                        Err(d)
                    }
                    _ => Ok(Stmt::Expr(self.parse_expr(0)?)),
                }
            }
            _ => Ok(Stmt::Expr(self.parse_expr(0)?)),
        }
    }

    /// Property value: an expr on the same line, or Newline → nested object block (SPEC §3.2).
    fn parse_property_value(&mut self) -> Result<Expr<'a>, Diagnostic> {
        if matches!(self.peek(), TokenKind::Newline) {
            self.bump();
            Ok(Expr::ObjectLiteral(self.parse_object_body()?))
        } else {
            self.parse_expr(0)
        }
    }

    /// Properties up to a closing `end` (def body / nested object / new Schema).
    /// A code body: statements until `end` (D17). Shared by blocks, `def`
    /// bodies and lambda bodies, so all three are scopes that accept `=`,
    /// `shadow` and `assert` — unlike an object literal, which is data only.
    fn parse_stmt_body(&mut self, what: &'static str) -> Result<Vec<Stmt<'a>>, Diagnostic> {
        self.enter()?;
        let r = self.parse_stmt_body_impl(what);
        self.leave();
        r
    }

    fn parse_stmt_body_impl(&mut self, what: &'static str) -> Result<Vec<Stmt<'a>>, Diagnostic> {
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::End) {
                return Ok(body);
            }
            if matches!(self.peek(), TokenKind::Eof) {
                return Err(self.err(
                    "E0203",
                    "missing `end`",
                    match what {
                        "block" => "block is not closed",
                        "function" => "function body is not closed",
                        _ => "lambda body is not closed",
                    },
                ));
            }
            let stmt = self.parse_stmt()?;
            self.eat_separator()?;
            body.push(stmt);
        }
    }

    fn parse_object_body(&mut self) -> Result<ObjectBody<'a>, Diagnostic> {
        self.enter()?;
        let r = self.parse_object_body_impl();
        self.leave();
        r
    }

    fn parse_object_body_impl(&mut self) -> Result<ObjectBody<'a>, Diagnostic> {
        let mut props = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::End) {
                return Ok(ObjectBody { props });
            }
            if matches!(self.peek(), TokenKind::Eof) {
                return Err(self.err("E0203", "missing `end`", "object block is not closed"));
            }
            let (key, kspan) = self.expect_key()?;
            self.expect(&TokenKind::Colon, "`:` after property key")?;
            let value = self.parse_property_value()?;
            props.push((key, value, kspan));
            if !matches!(self.peek(), TokenKind::Newline | TokenKind::End) {
                return Err(self.err(
                    "E0205",
                    "expected end of property",
                    "properties are separated by newlines",
                ));
            }
        }
    }

    /// D18: `enum Name` + one string member per line, until `end`.
    fn parse_enum_decl(&mut self, public: bool) -> Result<Stmt<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // enum
        let (name, _) = self.expect_ident("enum name")?;
        self.expect(&TokenKind::Newline, "newline after enum name")?;
        let mut members: Vec<&'a str> = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::End) {
                break;
            }
            let TokenKind::Str(m) = self.peek() else {
                return Err(self.err(
                    "E0211",
                    "expected an enum member",
                    "members are quoted strings, one per line",
                ));
            };
            if members.contains(m) {
                return Err(self.err(
                    "E0212",
                    format!("duplicate enum member '{m}'"),
                    "each member must be listed once",
                ));
            }
            members.push(m);
            self.bump();
            if !matches!(self.peek(), TokenKind::Newline | TokenKind::End) {
                return Err(self.err(
                    "E0205",
                    "expected end of enum member",
                    "members are separated by newlines",
                ));
            }
        }
        if members.is_empty() {
            return Err(self.err(
                "E0213",
                "enum has no members",
                "an empty enum could never be satisfied",
            ));
        }
        Ok(Stmt::EnumDecl(EnumDeclaration {
            name,
            members,
            public,
            span: self.join(start),
        }))
    }

    fn parse_type_decl(&mut self, public: bool) -> Result<Stmt<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // type
        let (name, _) = self.expect_ident("schema name")?;
        self.expect(&TokenKind::Newline, "newline after schema name")?;
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::End) {
                break;
            }
            let (field, _) = self.expect_ident("field name")?;
            self.expect(&TokenKind::Colon, "`:` after field name")?;
            let (ty, _) = self.expect_ident("field type")?;
            let ty = TypeName::builtin(ty).unwrap_or(TypeName::Custom(ty));
            // `name: Type = default` makes the field optional (default in the instance scope).
            let default = if self.eat(&TokenKind::Assign) {
                Some(self.parse_expr(0)?)
            } else {
                None
            };
            fields.push(SchemaField {
                name: field,
                ty,
                default,
            });
        }
        Ok(Stmt::TypeDecl(SchemaDeclaration {
            name,
            fields,
            public,
            span: self.join(start),
        }))
    }

    fn parse_func_decl(&mut self, public: bool) -> Result<Stmt<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // def
        let (name, _) = self.expect_ident("function name")?;
        self.expect(&TokenKind::LParen, "`(` after function name")?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::Newline, "newline after function signature")?;
        let body = self.parse_stmt_body("function")?;
        Ok(Stmt::FuncDecl {
            name,
            params,
            body,
            public,
            span: self.join(start),
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<&'a str>, Diagnostic> {
        let mut params = Vec::new();
        if !self.eat(&TokenKind::RParen) {
            loop {
                let (p, _) = self.expect_ident("parameter name")?;
                params.push(p);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)` after parameters")?;
        }
        Ok(params)
    }

    fn parse_assert(&mut self) -> Result<Stmt<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // assert
        let cond = self.parse_expr(0)?;
        let message = if self.eat(&TokenKind::Comma) {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        Ok(Stmt::Assert {
            cond,
            message,
            span: self.join(start),
        })
    }

    fn parse_block(&mut self, kind: BlockKind) -> Result<BlockDeclaration<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // domain | component
        let label = self.parse_expr(0)?;
        self.expect(&TokenKind::Newline, "newline after block label")?;
        let body = self.parse_stmt_body("block")?;
        Ok(BlockDeclaration {
            kind,
            label,
            body,
            span: self.join(start),
        })
    }

    // ---- Expressions (Pratt, SPEC §3.3) ----

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr<'a>, Diagnostic> {
        self.enter()?;
        let r = self.parse_expr_impl(min_bp);
        self.leave();
        r
    }

    fn parse_expr_impl(&mut self, min_bp: u8) -> Result<Expr<'a>, Diagnostic> {
        let start = self.span();
        let mut lhs = self.parse_prefix()?;
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    lhs = self.parse_postfix_dot(lhs, start)?;
                }
                TokenKind::LParen => {
                    let args = self.parse_args()?;
                    lhs = Expr::Call {
                        callee: Box::new(lhs),
                        args,
                        span: self.join(start),
                    };
                }
                // List indexing `xs[0]` (D11)
                TokenKind::LBracket => {
                    self.bump();
                    // obj["key"] — hint towards dot form (E0318)
                    if matches!(self.peek(), TokenKind::Str(_))
                        && matches!(self.peek_at(1), TokenKind::RBracket)
                    {
                        let mut d = self.err(
                            "E0318",
                            "bracket access on objects is not supported",
                            "string key in brackets",
                        );
                        if let TokenKind::Str(k) = self.peek() {
                            d.help = Some(format!("use dot access instead: `.\"{k}\"`"));
                        }
                        return Err(d);
                    }
                    let key = self.parse_expr(0)?;
                    self.expect(&TokenKind::RBracket, "closing `]` in index")?;
                    lhs = Expr::Index {
                        recv: Box::new(lhs),
                        key: Box::new(key),
                        bracket: true,
                        span: self.join(start),
                    };
                }
                TokenKind::Question if TERNARY_LBP >= min_bp => {
                    self.bump();
                    let then = self.parse_expr(0)?;
                    self.expect(&TokenKind::Colon, "`:` in ternary expression")?;
                    let otherwise = self.parse_expr(TERNARY_LBP - 1)?; // right associativity
                    lhs = Expr::Ternary {
                        cond: Box::new(lhs),
                        then: Box::new(then),
                        otherwise: Box::new(otherwise),
                        span: self.join(start),
                    };
                }
                k => {
                    let Some((lbp, rbp, op)) = infix_bp(k) else {
                        break;
                    };
                    if lbp < min_bp {
                        break;
                    }
                    self.bump();
                    let rhs = self.parse_expr(rbp)?;
                    lhs = Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span: self.join(start),
                    };
                }
            }
        }
        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr<'a>, Diagnostic> {
        let sp = self.span();
        match self.peek() {
            TokenKind::Int(n) => {
                let n = *n;
                self.bump();
                Ok(Expr::Literal(LitValue::Int(n), sp))
            }
            TokenKind::Float(n) => {
                let n = *n;
                self.bump();
                Ok(Expr::Literal(LitValue::Float(n), sp))
            }
            TokenKind::Str(s) => {
                let s = *s;
                self.bump();
                Ok(Expr::Literal(LitValue::Str(s), sp))
            }
            TokenKind::InterpStr(parts) => {
                let parts = parts.clone();
                self.bump();
                Ok(Expr::Literal(LitValue::InterpStr(parts), sp))
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr::Literal(LitValue::Bool(true), sp))
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr::Literal(LitValue::Bool(false), sp))
            }
            TokenKind::Null => {
                self.bump();
                Ok(Expr::Literal(LitValue::Null, sp))
            }
            TokenKind::Ident(s) => {
                let s = *s;
                self.bump();
                Ok(Expr::Variable(s, sp))
            }
            TokenKind::Minus => {
                self.bump();
                let rhs = self.parse_expr(UNARY_RBP)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    rhs: Box::new(rhs),
                    span: self.join(sp),
                })
            }
            TokenKind::Not => {
                self.bump();
                let rhs = self.parse_expr(UNARY_RBP)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    rhs: Box::new(rhs),
                    span: self.join(sp),
                })
            }
            TokenKind::LParen => {
                if self.lambda_ahead() {
                    self.parse_lambda()
                } else {
                    self.bump();
                    let e = self.parse_expr(0)?;
                    self.expect(&TokenKind::RParen, "closing `)`")?;
                    Ok(e)
                }
            }
            TokenKind::LBracket => self.parse_list(),
            TokenKind::New => {
                self.bump();
                let (first, _) = self.expect_ident("schema name after `new`")?;
                // D12: `new alias.Schema` — an imported schema
                let (schema, schema_alias) = if self.eat(&TokenKind::Dot) {
                    let (name, _) = self.expect_ident("schema name after module alias")?;
                    (name, Some(first))
                } else {
                    (first, None)
                };
                self.expect(&TokenKind::Newline, "newline after `new SchemaName`")?;
                let body = self.parse_object_body()?;
                Ok(Expr::SchemaInstance {
                    schema,
                    schema_alias,
                    body,
                    span: self.join(sp),
                })
            }
            TokenKind::Cond => self.parse_cond(),
            _ => Err(self.err(
                "E0204",
                "expected expression",
                format!("found {:?}", self.peek()),
            )),
        }
    }

    /// Lookahead from the current position (LParen): `(` [Ident (`,` Ident)*] `)` `->` ?
    fn lambda_ahead(&self) -> bool {
        let mut i = self.pos + 1;
        loop {
            match self.toks.get(i).map(|t| &t.kind) {
                Some(TokenKind::RParen) => {
                    return matches!(
                        self.toks.get(i + 1).map(|t| &t.kind),
                        Some(TokenKind::Arrow)
                    )
                }
                Some(TokenKind::Ident(_)) => match self.toks.get(i + 1).map(|t| &t.kind) {
                    Some(TokenKind::Comma) => i += 2,
                    Some(TokenKind::RParen) => {
                        return matches!(
                            self.toks.get(i + 2).map(|t| &t.kind),
                            Some(TokenKind::Arrow)
                        )
                    }
                    _ => return false,
                },
                _ => return false,
            }
        }
    }

    /// `(params) -> body end`; body is an expression or an object body (SPEC §3.1 LambdaBody).
    fn parse_lambda(&mut self) -> Result<Expr<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // (
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::Arrow, "`->` in lambda")?;
        // A statement body (`key: v`, `x = e`, `assert ...`) or a single expression.
        let stmt_body = matches!(
            (self.peek(), self.peek_at(1)),
            (TokenKind::Ident(_) | TokenKind::Str(_), TokenKind::Colon)
                | (TokenKind::Ident(_), TokenKind::Assign)
                | (
                    TokenKind::Shadow
                        | TokenKind::Assert
                        | TokenKind::Def
                        | TokenKind::Type
                        | TokenKind::Pub
                        | TokenKind::Domain,
                    _
                )
        );
        let body = if stmt_body {
            LambdaBody::Block(self.parse_stmt_body("lambda")?)
        } else {
            let e = self.parse_expr(0)?;
            self.skip_newlines();
            self.expect(&TokenKind::End, "`end` closing lambda body")?;
            LambdaBody::Expr(Box::new(e))
        };
        Ok(Expr::Lambda {
            params,
            body,
            span: self.join(start),
        })
    }

    fn parse_list(&mut self) -> Result<Expr<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // [
        let mut items = Vec::new();
        loop {
            while matches!(self.peek(), TokenKind::Newline | TokenKind::Comma) {
                self.bump();
            }
            if self.eat(&TokenKind::RBracket) {
                return Ok(Expr::ListLiteral(items, self.join(start)));
            }
            if matches!(self.peek(), TokenKind::Eof) {
                return Err(self.err("E0203", "missing `]`", "list is not closed"));
            }
            items.push(self.parse_expr(0)?);
            if !matches!(
                self.peek(),
                TokenKind::Newline | TokenKind::Comma | TokenKind::RBracket
            ) {
                return Err(self.err(
                    "E0205",
                    "expected list separator",
                    "elements are separated by newlines or commas",
                ));
            }
        }
    }

    /// `cond \n (bool -> value)+ else -> value \n end` (D14).
    fn parse_cond(&mut self) -> Result<Expr<'a>, Diagnostic> {
        let start = self.span();
        self.bump(); // cond
        self.expect(&TokenKind::Newline, "newline after `cond`")?;
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), TokenKind::Else) {
                break;
            }
            if matches!(self.peek(), TokenKind::End | TokenKind::Eof) {
                return Err(self.err(
                    "E0207",
                    "`cond` requires an `else` arm",
                    "add `else -> ...` before `end`",
                ));
            }
            let condition = self.parse_expr(0)?;
            self.expect(&TokenKind::Arrow, "`->` after a cond condition")?;
            let value = self.parse_expr(0)?;
            arms.push((condition, value));
            self.eat_separator()?;
        }
        self.bump(); // else
        self.expect(&TokenKind::Arrow, "`->` after `else`")?;
        let otherwise = self.parse_expr(0)?;
        self.skip_newlines();
        self.expect(&TokenKind::End, "`end` to close `cond`")?;
        Ok(Expr::Cond {
            arms,
            otherwise: Box::new(otherwise),
            span: self.join(start),
        })
    }

    fn parse_args(&mut self) -> Result<Vec<Expr<'a>>, Diagnostic> {
        self.bump(); // (
        let mut args = Vec::new();
        if !self.eat(&TokenKind::RParen) {
            loop {
                args.push(self.parse_expr(0)?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)` after arguments")?;
        }
        Ok(args)
    }

    /// `.field` | `."string key"` | `."#{dynamic}"` | `.method(args)`
    /// | `.method (params) -> ... end` (trailing lambda)
    fn parse_postfix_dot(&mut self, recv: Expr<'a>, start: Span) -> Result<Expr<'a>, Diagnostic> {
        self.bump(); // .
                     // D11: string key after the dot — access to an arbitrary field name
        match self.peek() {
            TokenKind::Str(key) => {
                let key = *key;
                self.bump();
                return Ok(Expr::FieldAccess {
                    recv: Box::new(recv),
                    field: key,
                    span: self.join(start),
                });
            }
            TokenKind::InterpStr(parts) => {
                let parts = parts.clone();
                let ksp = self.span();
                self.bump();
                let key = Expr::Literal(LitValue::InterpStr(parts), ksp);
                return Ok(Expr::Index {
                    recv: Box::new(recv),
                    key: Box::new(key),
                    bracket: false,
                    span: self.join(start),
                });
            }
            _ => {}
        }
        let (name, _) = self.expect_ident("field or method name after `.`")?;
        if matches!(self.peek(), TokenKind::LParen) {
            let (args, lambda) = if self.lambda_ahead() {
                (Vec::new(), Some(Box::new(self.parse_lambda()?)))
            } else {
                let args = self.parse_args()?;
                let lambda = if matches!(self.peek(), TokenKind::LParen) && self.lambda_ahead() {
                    Some(Box::new(self.parse_lambda()?))
                } else {
                    None
                };
                (args, lambda)
            };
            Ok(Expr::MethodCall {
                recv: Box::new(recv),
                method: name,
                args,
                lambda,
                span: self.join(start),
            })
        } else {
            Ok(Expr::FieldAccess {
                recv: Box::new(recv),
                field: name,
                span: self.join(start),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn module(src: &str) -> Module<'_> {
        let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
        Parser::new(toks).parse_module().expect("parse ok")
    }

    fn expr(src: &str) -> Expr<'_> {
        let m = module(src);
        match m.stmts.into_iter().next().expect("one stmt") {
            Stmt::Expr(e) => e,
            other => panic!("expected expr stmt, got {other:?}"),
        }
    }

    fn first_err(src: &str) -> &'static str {
        let toks = Lexer::new(src, 0).tokenize().expect("lex ok");
        Parser::new(toks)
            .parse_module()
            .expect_err("parse must fail")[0]
            .code
    }

    #[test]
    fn deep_nesting_is_e0208_not_a_crash() {
        // Crafted deeply-nested input must be rejected (E0208), never overflow the
        // stack. Worth running under `cargo test --release` as well as in debug:
        // release has no parser thread, so that run is what proves 256 frames fit
        // on an ordinary stack.
        let parens = format!("x: {}1{}", "(".repeat(2000), ")".repeat(2000));
        assert_eq!(first_err(&parens), "E0208");
        let brackets = format!("x: {}", "[".repeat(2000));
        assert_eq!(first_err(&brackets), "E0208");
        let blocks = format!(
            "{}\n{}",
            (0..2000)
                .map(|i| format!("domain \"d{i}\""))
                .collect::<Vec<_>>()
                .join("\n"),
            "end\n".repeat(2000)
        );
        assert_eq!(first_err(&blocks), "E0208");
        // realistic nesting (10 levels of parens) stays well under the cap
        let ok = format!("x: {}1{}", "(".repeat(10), ")".repeat(10));
        module(&ok);
    }

    #[test]
    fn enum_parses_d18() {
        let m = module(
            "enum Tier
  \"frontend\"
  \"backend\"
end
",
        );
        let Stmt::EnumDecl(e) = &m.stmts[0] else {
            panic!("expected an enum declaration")
        };
        assert_eq!(e.name, "Tier");
        assert_eq!(e.members, vec!["frontend", "backend"]);
        assert!(!e.public);
    }

    #[test]
    fn enum_errors_are_specific() {
        assert_eq!(
            first_err(
                "enum T
  bare
end
"
            ),
            "E0211"
        ); // not a string
        assert_eq!(
            first_err(
                "enum T
  \"a\"
  \"a\"
end
"
            ),
            "E0212"
        ); // duplicate
        assert_eq!(
            first_err(
                "enum T
end
"
            ),
            "E0213"
        ); // empty
    }

    #[test]
    fn cond_parses_d14() {
        let m = module("t: cond\n  a == 1 -> \"x\"\n  else -> \"y\"\nend");
        let Stmt::Property {
            value: Expr::Cond { arms, .. },
            ..
        } = &m.stmts[0]
        else {
            panic!("expected cond property, got {:?}", m.stmts[0])
        };
        assert_eq!(arms.len(), 1);
        // a missing `else` arm is a parse error (D14)
        assert_eq!(first_err("x: cond\n  true -> 1\nend"), "E0207");
    }

    #[test]
    fn mul_binds_tighter_than_add() {
        let Expr::Binary {
            op: BinOp::Add,
            rhs,
            ..
        } = expr("1 + 2 * 3")
        else {
            panic!()
        };
        assert!(matches!(*rhs, Expr::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn comparison_vs_logic_precedence() {
        // a == b && c < d  →  (a == b) && (c < d)
        let Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
            ..
        } = expr("a == b && c < d")
        else {
            panic!()
        };
        assert!(matches!(*lhs, Expr::Binary { op: BinOp::Eq, .. }));
        assert!(matches!(*rhs, Expr::Binary { op: BinOp::Lt, .. }));
    }

    #[test]
    fn ternary_is_right_associative() {
        // a ? b : c ? d : e  →  a ? b : (c ? d : e)
        let Expr::Ternary { otherwise, .. } = expr("a ? b : c ? d : e") else {
            panic!()
        };
        assert!(matches!(*otherwise, Expr::Ternary { .. }));
    }

    #[test]
    fn method_chains_are_left_associative() {
        // xs.compact().uniq() → MethodCall{ recv: MethodCall{ recv: xs, compact }, uniq }
        let Expr::MethodCall {
            method: "uniq",
            recv,
            ..
        } = expr("xs.compact().uniq()")
        else {
            panic!()
        };
        assert!(matches!(
            *recv,
            Expr::MethodCall {
                method: "compact",
                ..
            }
        ));
    }

    #[test]
    fn field_access_chain() {
        let Expr::FieldAccess {
            field: "version",
            recv,
            ..
        } = expr("cargo_data.package.version")
        else {
            panic!()
        };
        assert!(matches!(
            *recv,
            Expr::FieldAccess {
                field: "package",
                ..
            }
        ));
    }

    #[test]
    fn trailing_lambda_on_map() {
        let src = "xs.map (name, index) -> name end";
        let Expr::MethodCall {
            method: "map",
            args,
            lambda: Some(l),
            ..
        } = expr(src)
        else {
            panic!()
        };
        assert!(args.is_empty());
        let Expr::Lambda { params, .. } = *l else {
            panic!()
        };
        assert_eq!(params, vec!["name", "index"]);
    }

    #[test]
    fn list_with_unary_minus_element_d2() {
        let Expr::ListLiteral(items, _) = expr("[a\n-b]") else {
            panic!()
        };
        assert_eq!(items.len(), 2);
        assert!(matches!(
            items[1],
            Expr::Unary {
                op: UnaryOp::Neg,
                ..
            }
        ));
    }

    #[test]
    fn inline_block_is_e0201() {
        assert_eq!(first_err("metrics port: 9090 path: \"/metrics\""), "E0201");
    }

    #[test]
    fn shadow_assignment_d7() {
        let m = module("shadow x = 1");
        assert!(matches!(
            m.stmts[0],
            Stmt::Assign {
                name: "x",
                shadow: true,
                ..
            }
        ));
    }

    #[test]
    fn new_schema_instance_d4() {
        let m = module("m = new ServiceMeta\n  name: \"auth\"\n  port: 8001\nend");
        let Stmt::Assign {
            value:
                Expr::SchemaInstance {
                    schema: "ServiceMeta",
                    body,
                    ..
                },
            ..
        } = &m.stmts[0]
        else {
            panic!()
        };
        assert_eq!(body.props.len(), 2);
    }

    #[test]
    fn assert_with_message_d5() {
        let m = module("assert xs.len() >= 1, \"too few\"");
        assert!(matches!(
            m.stmts[0],
            Stmt::Assert {
                message: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn nested_object_blocks() {
        let m = module("domain \"d\"\n  security:\n    tls: true\n    certs:\n      path: \"/x\"\n    end\n  end\nend");
        let Stmt::Block(b) = &m.stmts[0] else {
            panic!()
        };
        let Stmt::Property {
            key: "security",
            value: Expr::ObjectLiteral(body),
            ..
        } = &b.body[0]
        else {
            panic!()
        };
        assert!(matches!(
            body.props[1],
            ("certs", Expr::ObjectLiteral(_), _)
        ));
    }

    #[test]
    fn dot_string_field_access_d11() {
        // a."eu west".port → FieldAccess(FieldAccess(a, "eu west"), "port")
        let Expr::FieldAccess {
            field: "port",
            recv,
            ..
        } = expr("a.\"eu west\".port")
        else {
            panic!()
        };
        assert!(matches!(
            *recv,
            Expr::FieldAccess {
                field: "eu west",
                ..
            }
        ));
    }

    #[test]
    fn dynamic_key_desugars_to_index_d11() {
        let Expr::Index {
            bracket: false,
            key,
            ..
        } = expr("a.\"#{r}\"")
        else {
            panic!()
        };
        assert!(matches!(*key, Expr::Literal(LitValue::InterpStr(_), _)));
    }

    #[test]
    fn list_indexing_d11() {
        // xs[0][1] — chained indexing
        let Expr::Index {
            bracket: true,
            recv,
            ..
        } = expr("xs[0][1]")
        else {
            panic!()
        };
        assert!(matches!(*recv, Expr::Index { bracket: true, .. }));
    }

    #[test]
    fn bracket_string_key_is_e0318() {
        assert_eq!(first_err("x = a[\"key\"]"), "E0318");
    }

    #[test]
    fn string_property_keys_d11() {
        let m = module("domain \"d\"\n  \"app.kubernetes.io/name\": \"auth\"\nend");
        let Stmt::Block(b) = &m.stmts[0] else {
            panic!()
        };
        assert!(matches!(
            b.body[0],
            Stmt::Property {
                key: "app.kubernetes.io/name",
                ..
            }
        ));
    }

    #[test]
    fn pub_def_and_type_d12() {
        let m = module("pub def f(x)\n  a: x\nend\npub type T\n  a: Int\nend\ndef g()\n  b: 1\nend\nassert g().b == 1 && f == f && T == T");
        assert!(matches!(m.stmts[0], Stmt::FuncDecl { public: true, .. }));
        assert!(matches!(&m.stmts[1], Stmt::TypeDecl(s) if s.public));
        assert!(matches!(m.stmts[2], Stmt::FuncDecl { public: false, .. }));
        // pub not before def/type — an error
        assert_eq!(first_err("pub x = 1"), "E0206");
    }

    #[test]
    fn new_with_module_alias_d12() {
        let m = module("m: new pkg.Meta\n  a: 1\nend");
        let Stmt::Property {
            value:
                Expr::SchemaInstance {
                    schema: "Meta",
                    schema_alias: Some("pkg"),
                    ..
                },
            ..
        } = &m.stmts[0]
        else {
            panic!()
        };
    }

    #[test]
    fn missing_end_is_e0203() {
        assert_eq!(first_err("domain \"d\"\n  x = 1\n"), "E0203");
    }
}
