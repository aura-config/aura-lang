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
    /// D28: `assert` statements in the schema body, checked on every `new`.
    pub invariants: Vec<SchemaInvariant<'a>>,
    /// `pub type` - the schema is visible to the module's importers (D12)
    pub public: bool,
    pub span: Span,
}

/// D28: a rule the schema states about its own values.
///
/// Written the same way as any other `assert`, and checked after the fields are
/// filled and type-checked — so it can assume the types are right and only has
/// to say what the values must be to each other.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInvariant<'a> {
    pub cond: Expr<'a>,
    pub message: Option<Expr<'a>>,
    pub span: Span,
}

/// A schema field. `default = Some(expr)` makes the field optional: if omitted at
/// `new`, the default expression is evaluated in the instantiation scope. A field
/// with no default is required (E0511 if missing).
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaField<'a> {
    pub name: &'a str,
    pub ty: TypeName<'a>,
    pub default: Option<Expr<'a>>,
    /// D27: `name: Int?` widens the field to admit `null`.
    ///
    /// Nullability belongs to the **field**, not to `TypeName`, and that is a
    /// decision rather than an implementation detail: it makes `[Int?]`
    /// unrepresentable, so there is never a question of whether the list or its
    /// elements may be empty. `[Int]?` — a field that is either a list or null —
    /// is still expressible, and means only one thing.
    pub nullable: bool,
    /// The type name's own position, so an unknown type points at the field
    /// rather than at the whole `type` block.
    pub ty_span: Span,
}

/// Not `Copy`: `List` carries an optional element type, and that box is what
/// lets `[[Int]]` nest without a special case for depth.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeName<'a> {
    String,
    Int,
    Float,
    Bool,
    /// `List` is the bare, unconstrained form; `List(Some(t))` is `[t]` (D26).
    /// Keeping one variant rather than adding a second means every site that
    /// asks "is this a list" keeps working, and only sites that care about the
    /// element have to look inside.
    List(Option<Box<TypeName<'a>>>),
    Object,
    Custom(&'a str),
}

/// The built-in type names a schema field may use, in the order a reader meets
/// them. This is the list the parser matches on, and the one `aura docs --agent`
/// prints — the agent reference once said `Str`, which is the *Rust* variant
/// name and produces a bewildering `E0504: use of undefined variable 'Str'`.
/// Deriving the documentation from here means that cannot recur.
pub const BUILTIN_TYPE_NAMES: &[&str] = &["String", "Int", "Float", "Bool", "List", "Object"];

impl<'a> TypeName<'a> {
    /// The user-declared name this type ultimately refers to, looking through
    /// `[T]` (D26). Tooling asks this to decide whether a `type` or `enum` is
    /// used: without the element case, `services: [Service]` left `Service`
    /// reported as dead code while it was doing the validating.
    pub fn custom_name(&self) -> Option<&'a str> {
        match self {
            TypeName::Custom(n) => Some(n),
            TypeName::List(Some(el)) => el.custom_name(),
            _ => None,
        }
    }
}

impl TypeName<'_> {
    /// Parses a built-in name; anything else is a user-declared `type` or `enum`.
    pub fn builtin(name: &str) -> Option<TypeName<'static>> {
        Some(match name {
            "String" => TypeName::String,
            "Int" => TypeName::Int,
            "Float" => TypeName::Float,
            "Bool" => TypeName::Bool,
            "List" => TypeName::List(None),
            "Object" => TypeName::Object,
            _ => return None,
        })
    }
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
