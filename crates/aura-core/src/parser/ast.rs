//! AST Aura v1.2 (SPEC §3.1). Zero-copy: имена и строки — срезы исходника.

use crate::lexer::token::StrPart;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Module<'a> {
    pub imports: Vec<Import<'a>>,
    pub stmts: Vec<Stmt<'a>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import<'a> {
    pub source: ImportSource<'a>,
    pub alias: &'a str,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportSource<'a> {
    /// github/actions/rust-cache@v1.2 (версия обязательна, D8)
    Registry {
        path: &'a str,
        version: &'a str,
    },
    File(&'a str),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt<'a> {
    /// `[shadow] name = expr` (D7)
    Assign {
        name: &'a str,
        shadow: bool,
        value: Expr<'a>,
        span: Span,
    },
    /// `key: expr` | `key:` + объектный блок — внутри domain/component
    Property {
        key: &'a str,
        value: Expr<'a>,
        span: Span,
    },
    /// `assert cond[, "msg"]` (D5)
    Assert {
        cond: Expr<'a>,
        message: Option<Expr<'a>>,
        span: Span,
    },
    TypeDecl(SchemaDeclaration<'a>),
    /// `[pub] def ...` — public экспортируется импортёрам модуля (D12)
    FuncDecl {
        name: &'a str,
        params: Vec<&'a str>,
        body: ObjectBody<'a>,
        public: bool,
        span: Span,
    },
    Block(BlockDeclaration<'a>),
    Expr(Expr<'a>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDeclaration<'a> {
    pub name: &'a str,
    pub fields: Vec<(&'a str, TypeName<'a>)>,
    /// `pub type` — схема видима импортёрам модуля (D12)
    pub public: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypeName<'a> {
    String,
    Int,
    Float,
    Bool,
    List,
    Object,
    Custom(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Domain,
    Component,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDeclaration<'a> {
    pub kind: BlockKind,
    /// `"production-eu"` | `name` — произвольное выражение
    pub label: Expr<'a>,
    pub body: Vec<Stmt<'a>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LitValue<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
    /// Сырые части; под-выражения `#{...}` парсятся при вычислении (Фаза 3)
    InterpStr(Vec<StrPart<'a>>),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr<'a> {
    Literal(LitValue<'a>, Span),
    Variable(&'a str, Span),
    Unary {
        op: UnaryOp,
        rhs: Box<Expr<'a>>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr<'a>>,
        rhs: Box<Expr<'a>>,
        span: Span,
    },
    Ternary {
        cond: Box<Expr<'a>>,
        then: Box<Expr<'a>>,
        otherwise: Box<Expr<'a>>,
        span: Span,
    },
    Call {
        callee: Box<Expr<'a>>,
        args: Vec<Expr<'a>>,
        span: Span,
    },
    MethodCall {
        recv: Box<Expr<'a>>,
        method: &'a str,
        args: Vec<Expr<'a>>,
        /// Trailing-lambda: `xs.map (a, b) -> ... end`
        lambda: Option<Box<Expr<'a>>>,
        span: Span,
    },
    FieldAccess {
        recv: Box<Expr<'a>>,
        field: &'a str,
        span: Span,
    },
    /// Индексация списка `xs[0]` (bracket=true) либо динамический ключ объекта
    /// `obj."#{name}"` (bracket=false, D11). На объектах скобки запрещены (E0318).
    Index {
        recv: Box<Expr<'a>>,
        key: Box<Expr<'a>>,
        bracket: bool,
        span: Span,
    },
    ObjectLiteral(ObjectBody<'a>),
    ListLiteral(Vec<Expr<'a>>, Span),
    Lambda {
        params: Vec<&'a str>,
        body: LambdaBody<'a>,
        span: Span,
    },
    /// Только через `new` (D4)
    /// `schema_alias` — импортированная схема `new mod.Name` (D12)
    SchemaInstance {
        schema: &'a str,
        schema_alias: Option<&'a str>,
        body: ObjectBody<'a>,
        span: Span,
    },
    /// `component name ... end` в позиции выражения (внутри map)
    Block(Box<BlockDeclaration<'a>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectBody<'a> {
    pub props: Vec<(&'a str, Expr<'a>, Span)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody<'a> {
    Expr(Box<Expr<'a>>),
    Object(ObjectBody<'a>),
}
