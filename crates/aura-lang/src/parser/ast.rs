//! Aura AST (SPEC §3.1). Zero-copy: names and strings are slices of the source.

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
    /// github/actions/rust-cache@v1.2 (version is mandatory, D8)
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
    /// `key: expr` | `key:` + an object block - inside domain/component
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
    /// D18: `[pub] enum Name` — a closed set of allowed string values
    EnumDecl(EnumDeclaration<'a>),
    /// `[pub] def ...` - public is exported to the module's importers (D12)
    FuncDecl {
        name: &'a str,
        params: Vec<&'a str>,
        /// D17: a code body — statements, like a module or a block.
        body: Vec<Stmt<'a>>,
        public: bool,
        span: Span,
    },
    Block(BlockDeclaration<'a>),
    Expr(Expr<'a>),
}

/// D18: `enum Tier "frontend" "backend" end` — a validation constraint, not a
/// wrapper type: values stay ordinary strings and serialize as such.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDeclaration<'a> {
    pub name: &'a str,
    pub members: Vec<&'a str>,
    /// `pub enum` — visible to the module's importers (D12)
    pub public: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDeclaration<'a> {
    pub name: &'a str,
    pub fields: Vec<SchemaField<'a>>,
    /// `pub type` - the schema is visible to the module's importers (D12)
    pub public: bool,
    pub span: Span,
}

/// A schema field. `default = Some(expr)` makes the field optional: if omitted at
/// `new`, the default expression is evaluated in the instantiation scope. A field
/// with no default is required (E0511 if missing). No nullable (`?`) fields yet.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaField<'a> {
    pub name: &'a str,
    pub ty: TypeName<'a>,
    pub default: Option<Expr<'a>>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDeclaration<'a> {
    pub kind: BlockKind,
    /// `"production-eu"` | `name` - an arbitrary expression
    pub label: Expr<'a>,
    pub body: Vec<Stmt<'a>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LitValue<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
    /// Raw parts; `#{...}` sub-expressions are parsed at evaluation time (Phase 3)
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
    /// `cond \n (bool -> value)+ else -> value \n end` (D14). Left of each `->`
    /// must be Bool; `else` is mandatory. First true arm wins.
    Cond {
        arms: Vec<(Expr<'a>, Expr<'a>)>,
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
    /// List indexing `xs[0]` (bracket=true) or a dynamic object key
    /// `obj."#{name}"` (bracket=false, D11). Brackets are forbidden on objects (E0318).
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
    /// Only through `new` (D4)
    /// `schema_alias` - an imported schema `new mod.Name` (D12)
    SchemaInstance {
        schema: &'a str,
        schema_alias: Option<&'a str>,
        body: ObjectBody<'a>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectBody<'a> {
    pub props: Vec<(&'a str, Expr<'a>, Span)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody<'a> {
    Expr(Box<Expr<'a>>),
    /// D17: a code body — statements, like a `def` body.
    Block(Vec<Stmt<'a>>),
}
