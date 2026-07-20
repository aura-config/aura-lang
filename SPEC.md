# Aura v1.3 — Interpreter Master Specification (Rust)

Status: normative. Any implementation must conform to this document. Version v1.2 was a revision of the v1.1 design following an architecture audit; v1.3 added the package ecosystem (D12) and deterministic time (D13) — see §0.2.

---

## 0. General provisions

- Language: Rust (edition 2021+), MSRV 1.75.
- Principles: zero-copy lexer/parser (`&'a str`), immutable values, deterministic evaluation, no significant indentation, `\n` as a separator, `end` as an explicit closing terminator, **a capability model for effects** (I/O is denied by default for imported modules).
- Core external dependencies: `serde`, `serde_json`, `toml`, `indexmap`, `ariadne` (diagnostics), `clap` (CLI). The core (`lexer`, `parser`, `eval`) does not depend on `clap`/`ariadne`.

### 0.1. Reference manifest (v1.2)

The file `tests/fixtures/production_deploy.aura` must pass the full pipeline (`parse → analyze → eval → json`) on every CI run:

```aura
import github/actions/rust-cache@v1.2 as rust
import "templates/k8s_defaults.aura" as defaults

global_file_path = "/etc/aura/global.config"
base_port        = 8000
is_prod          = env("APP_ENV", "production") == "production"

type ServiceMeta
  name: String
  port: Int
end

unused_config_version = "v1.2.0"

def build_labels(app_name, tier)
  name: app_name
  tier: tier
  managed_by: "aura-engine"
end

transform_name = (s) -> s.upper() end

domain "production-eu"
  shadow global_file_path = "/var/log/aura.log"

  security:
    tls_enabled: true
    min_version: "1.3"
    certificates:
      cert_path: "/etc/ssl/certs/server.crt"
      key_path:  "/etc/ssl/certs/server.key"
    end
  end

  metrics:
    port: 9090
    path: "/metrics"
  end

  cargo_data  = read_file("./Cargo.toml").parse_toml()
  app_version = cargo_data.package.version

  services = [
    "auth"
    "billing"
    "frontend"
    null
  ]

  active_services = services.compact().uniq()

  meta = new ServiceMeta
    name: transform_name("auth")
    port: base_port + 1
  end

  apps = active_services.map (name, index) ->
    component name
      image: "company/#{name}:#{app_version}"
      labels: build_labels(name, "backend").merge(defaults.global_labels)
    end
  end

  assert active_services.len() >= 1, "Domain must have at least 1 service"
end
```

### 0.2. Design changes v1.1 → v1.3 (normative)

| # | Was (v1.1) | Now (v1.2) | Rationale |
| --- | --- | --- | --- |
| D1 | `read_file`/`env` available everywhere | Capability model: effects are only allowed for the root module; imports get them only via explicit flags | Determinism and supply-chain safety |
| D2 | `\n` is suppressed after binary operators, including inside lists | Inside `[]` a newline ALWAYS ends an element; a multi-line expression is only allowed inside `(...)` | Removes the `[a \n -b]` ambiguity |
| D3 | An inline block `metrics port: 9090 path: "/metrics"` | Removed. Only the object form `metrics: ... end` | −1 grammar branch, −1 non-obvious rule |
| D4 | A schema instance by capitalization (`ServiceMeta ... end`) | An explicit `new` keyword: `new ServiceMeta ... end` | A case typo becomes a syntax error, not a silent semantic change |
| D5 | Validation via `cond ? fail(...) : null` | An `assert cond, "msg"` statement; `fail()` remains as an expression for branches | Readability, no throwaway `null` |
| D6 | Numbers are `f64` only | `Int(i64)` and `Float(f64)` are separate types | Precision for byte limits and 64-bit IDs |
| D7 | Silent shadowing of outer variables | Shadowing an outer scope requires the `shadow` marker; without it — `E0302` | "Why is prod on a different path" no longer stays silent |
| D8 | `import github/actions/rust-cache` | A mandatory version `@vX[.Y[.Z]]` + a `aura.lock` lock file | CI/CD reproducibility |
| D9 | `Arc<RwLock<Environment>>` | An immutable `Arc<Environment>` with a freeze phase | No deadlocks, less code; there was never any mutation anyway |
| D10 | Every `x = ...` is exported to JSON | `key:` is exported, `x =` is a private computation (locals vs. outputs, as in Nix/Terraform) | Resolves the "output vs. dead code" conflict: dead-code analysis of `=` bindings becomes sound |
| D11 | Field access only by identifier | Fields: `.ident` and `."string"` (including `."#{dynamic}"`); lists: `xs[int]` (E0317 out of bounds); optionally: `.get(k, default)`; `obj["key"]` is E0318 with a hint; string keys in literals (`"app.io/name": v`) | One operator per operation: dot for fields, brackets only for list indices |
| D12 | `def`/`type` are always private | **Adopted in v1.3**: `pub def` / `pub type` land in the module's object — an importer calls `pkg.fn(...)` (builtin methods take precedence) and instantiates `new pkg.Schema ... end`; top-level pub items are silently excluded from the root JSON (deeper in the tree — E0601); pub is never considered dead code; **D1×D12**: the function runs with its origin module's capabilities, not the caller's; `pub` not immediately before `def`/`type` is E0206 | The foundation of the package ecosystem; explicit in the spirit of `shadow`/`new`; capability isolation cannot be bypassed via exported functions |
| D13 | — | **Adopted in v1.3**: `now()`/`timestamp()` do not exist and never will — calling them yields E0533 with a hint to pass the time in from outside (`env("BUILD_TIME", ...)`). Deterministic time: `"1h30m".parse_duration()` → seconds (Int; units d/h/m/s, E0319), `Int.format_duration()`, `"RFC3339".parse_datetime()` → epoch UTC (offsets ±HH:MM, E0320), `Int.format_datetime()` → RFC3339 UTC. Calendar arithmetic beyond an epoch Int is package territory | An unreproducible config cannot be written by construction; durations and dates cover config needs without a timezone quagmire in the core |
| D14 | — | **Adopted in v1.3**: a `cond` multi-way expression — `cond (bool -> value)+ else -> value end`. The left of each `->` must be `Bool` (E0306 otherwise, like a ternary condition); the right is any expression; `else` is mandatory (a missing `else` is the parse error E0207). First true arm wins. No value destructuring. | Fills the 3+ branch gap where nested ternaries become unreadable; deliberately simpler than a pattern-matching `match` |
| D15 | Every schema field is required | **Adopted in v1.3**: `name: Type = default` makes a field optional — if omitted at `new`, the default expression is evaluated in the instantiation scope (it may reference module vars, e.g. `= base + 1`) and inserted after the provided fields; the default is type-checked like any value (E0512). A field with no default is still required (E0511). No nullable (`?`) fields — optionality never introduces a `null`. | Fills the biggest schema gap (real configs need optional-with-default) with zero new null: every field always has a value, shape stays stable |

---

## 1. Architecture graph and module layout

### 1.1. Directory structure

```text
aura/
├── Cargo.toml                  # workspace
├── crates/
│   └── aura-core/
│       └── src/
│           ├── lib.rs
│           ├── span.rs         # Span, SourceId
│           ├── error.rs        # AuraError, ErrorKind, Diagnostic
│           ├── lexer/
│           │   ├── mod.rs      # Lexer<'a>
│           │   └── token.rs    # Token<'a>, TokenKind<'a>
│           ├── parser/
│           │   ├── mod.rs      # Parser<'a>
│           │   ├── ast.rs      # Expr, Stmt, Module
│           │   └── pratt.rs    # precedence table
│           ├── eval/
│           │   ├── mod.rs      # Interpreter
│           │   ├── value.rs    # Value
│           │   ├── env.rs      # Environment (frozen)
│           │   ├── caps.rs     # Capabilities
│           │   └── methods/    # MethodRegistry + builtin methods
│           │       ├── mod.rs
│           │       ├── string.rs
│           │       ├── list.rs
│           │       └── object.rs
│           ├── analysis/
│           │   ├── mod.rs      # SemanticAnalyzer
│           │   ├── dead_code.rs
│           │   └── schema_check.rs
│           ├── vfs/
│           │   ├── mod.rs      # FileResolver, ModuleGraph
│           │   ├── local.rs    # LocalFsResolver, MemoryResolver
│           │   └── lockfile.rs # aura.lock
│           └── serialize/
│               └── mod.rs      # Value -> serde_json::Value
└── src/
    └── main.rs                 # crate aura-cli: clap + ariadne
```

### 1.2. Data flow

```text
&'a str (source, lives in SourceCache)
   │  Lexer<'a>::tokenize()               — O(n), zero-copy
   ▼
Vec<Token<'a>>
   │  Parser<'a>::parse_module()          — recursive descent + Pratt
   ▼
Module<'a> (AST)  ──►  SemanticAnalyzer (--strict: dead code, schemas)
   │  Interpreter::eval_module()          — Environment + Capabilities + VFS
   ▼
Value (owned, immutable)
   │  serialize::to_json()
   ▼
serde_json::Value  ──►  stdout / file
```

Lifetimes: `SourceCache` owns the `String` of each file; tokens, the AST, and diagnostics borrow from it. `Value` is fully owned (the borrow ends at the eval boundary).

The root facade:

```rust
pub struct Pipeline {
    pub sources: SourceCache,
    pub resolver: Arc<dyn FileResolver>,
    pub options: Options, // strict, dry_run, caps: Capabilities
}
impl Pipeline {
    pub fn run(&mut self, entry: &Path) -> Result<serde_json::Value, Vec<Diagnostic>>;
}
```

---

## 2. Phase 1 — the zero-copy lexer

### 2.1. Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span { pub source: SourceId, pub start: u32, pub end: u32 } // byte offsets

#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> { pub kind: TokenKind<'a>, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    // Literals (zero-copy)
    Ident(&'a str),
    Int(i64),                  // D6: integers and floats are different tokens
    Float(f64),
    Str(&'a str),              // content without quotes, without interpolation
    InterpStr(Vec<StrPart<'a>>),
    ImportPath { path: &'a str, version: &'a str }, // github/actions/rust-cache@v1.2 (D8)
    True, False, Null,
    // Keywords
    Import, As, Type, Def, End, Domain, Component,
    New, Assert, Shadow,       // D4, D5, D7
    // Delimiters
    Newline,
    LParen, RParen, LBracket, RBracket,
    Colon, Comma, Dot, Assign, // : , . =
    Arrow,                     // ->
    Question,
    // Operators
    Plus, Minus, Star, Slash, Percent,
    EqEq, NotEq, Lt, Gt, LtEq, GtEq, And, Or, Not,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart<'a> { Lit(&'a str), Interp(&'a str) } // Interp is parsed by Phase 2
```

Zero-copy invariant: `TokenKind` contains no `String`; every text field is a slice of the source. The `#{expr}` interpolation is stored as a raw slice; a nested parser runs over it in Phase 2 (the span offset is preserved).

### 2.2. Number DFA (D6)

- `[0-9]+` → `Int(i64)`; an i64 overflow is `E0103`.
- `[0-9]+ "." [0-9]+` → `Float(f64)`. `12.` is an `E0101` error; `12.foo` rolls back to: `Int(12)`, `Dot`, `Ident` (1-character lookahead).
- The minus sign is not part of a number (it's the parser's unary operator).

### 2.3. String DFA

- Opens with `"`. Escapes: `\" \\ \n \t \#`. A string without escapes is a plain slice; with escapes it's a raw slice, unescaped lazily during eval (`Cow<'a, str>`).
- `#{` switches into interpolation (balanced on `{}`), the result is `InterpStr`.
- An unclosed string before `\n`/EOF is `E0102`.

### 2.4. Keywords, identifiers, import paths

- An identifier: `[a-zA-Z_][a-zA-Z0-9_]*`, then checked against the keyword table (`import as type def end domain component new assert shadow true false null`).
- An import path is a contextual mode after `Import` (if the next character is not `"`): `[a-zA-Z0-9_\-]+ ("/" [a-zA-Z0-9_\-]+)* "@" "v" [0-9]+ ("." [0-9]+){0,2}`. A missing `@vX` is an `E0104: registry import requires a version` error (D8). File imports (`"..."`) do not require a version.
- A comment: `#` outside a string — runs to the end of the line.

### 2.5. Newline-normalization invariants (D2)

The lexer maintains a bracket stack `paren_stack: Vec<Delim>` (`Paren` | `Bracket`).

1. A run of `\n+` (including `\r\n`, blank lines, comments) collapses into a single `Newline`.
2. Outside `[]`, a `Newline` is suppressed if the previous significant token is one of { `(`, `=`, `->`, `?`, `,`, a binary operator, `.` }. `:` is NOT in this set: a newline after `key:` is significant — it opens an object block (§3.2); continuing a ternary's `? :` on a new line after `:` requires parentheses.
3. Outside `[]`, a `Newline` is suppressed if the next significant token is one of { `)`, `.` }.
4. Inside `(...)`, newlines are suppressed entirely.
5. **Inside `[...]` a newline is ALWAYS emitted as an element separator**, except right after `[` and right before `]`. Rule 2 does NOT apply inside `[]`: a list-item line must be a complete expression; a multi-line item expression is wrapped in `(...)`. This makes `[a \n -b]` unambiguously the list `[a, -b]`.
6. A comma inside `[]` is allowed as an alternative/duplicate separator (`[1, 2, 3]`); `, \n` collapses into a single separator.
7. A `Newline` at the start of the file and right before `Eof` is suppressed.

### 2.6. API

```rust
pub struct Lexer<'a> { src: &'a str, pos: usize, source: SourceId, expect_import_path: bool, paren_stack: Vec<Delim> }
impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, source: SourceId) -> Self;
    pub fn tokenize(self) -> Result<Vec<Token<'a>>, Diagnostic>; // fail-fast
}
```

---

## 3. Phase 2 — the AST and the Pratt parser

### 3.1. AST

```rust
pub struct Module<'a> { pub imports: Vec<Import<'a>>, pub stmts: Vec<Stmt<'a>>, pub span: Span }

pub struct Import<'a> { pub source: ImportSource<'a>, pub alias: &'a str, pub span: Span }
pub enum ImportSource<'a> {
    Registry { path: &'a str, version: &'a str },  // github/actions/rust-cache@v1.2
    File(&'a str),                                  // "templates/x.aura"
}

pub enum Stmt<'a> {
    Assign { name: &'a str, shadow: bool, value: Expr<'a>, span: Span }, // [shadow] x = expr (D7)
    Property { key: &'a str, value: Expr<'a>, span: Span },              // key: expr | key: <object block> - inside domain/component
    Assert { cond: Expr<'a>, message: Option<Expr<'a>>, span: Span },    // assert cond, "msg" (D5)
    TypeDecl(SchemaDeclaration<'a>),
    FuncDecl { name: &'a str, params: Vec<&'a str>, body: ObjectBody<'a>, span: Span },
    Block(BlockDeclaration<'a>),
    Expr(Expr<'a>),
}

pub struct SchemaDeclaration<'a> { pub name: &'a str, pub fields: Vec<SchemaField<'a>>, pub span: Span }
pub struct SchemaField<'a> { pub name: &'a str, pub ty: TypeName<'a>, pub default: Option<Expr<'a>> } // `= default` -> optional (D15)
pub enum TypeName<'a> { String, Int, Float, Bool, List, Object, Custom(&'a str) }

pub struct BlockDeclaration<'a> {
    pub kind: BlockKind,          // Domain | Component
    pub label: Expr<'a>,          // "production-eu" | name - an expression
    pub body: Vec<Stmt<'a>>,      // the inline form was removed (D3)
    pub span: Span,
}

pub enum Expr<'a> {
    Literal(LitValue<'a>),        // Int/Float/Str/InterpStr/Bool/Null
    Variable(&'a str),
    Unary  { op: UnaryOp, rhs: Box<Expr<'a>>, span: Span },
    Binary { op: BinOp, lhs: Box<Expr<'a>>, rhs: Box<Expr<'a>>, span: Span },
    Ternary{ cond: Box<Expr<'a>>, then: Box<Expr<'a>>, otherwise: Box<Expr<'a>>, span: Span },
    Call       { callee: Box<Expr<'a>>, args: Vec<Expr<'a>>, span: Span },
    MethodCall { recv: Box<Expr<'a>>, method: &'a str, args: Vec<Expr<'a>>,
                 lambda: Option<Box<Expr<'a>>>, span: Span },
    FieldAccess{ recv: Box<Expr<'a>>, field: &'a str, span: Span },
    ObjectLiteral(ObjectBody<'a>),
    ListLiteral(Vec<Expr<'a>>, Span),
    Lambda { params: Vec<&'a str>, body: LambdaBody<'a>, span: Span },
    SchemaInstance { schema: &'a str, body: ObjectBody<'a>, span: Span }, // ONLY via `new` (D4)
    Block(Box<BlockDeclaration<'a>>),  // component inside map
}

pub struct ObjectBody<'a> { pub props: Vec<(&'a str, Expr<'a>, Span)> }
pub enum LambdaBody<'a> { Expr(Box<Expr<'a>>), Object(ObjectBody<'a>) }
```

### 3.2. Construct parsing rules

- `key: value` inside an object body is a property; `name = expr` is an assignment. Distinguished by the token after the identifier (`:` vs `=`).
- A nested object: `key:` followed immediately by a `Newline` (no value on the line) opens an `ObjectLiteral` up to `end`. This is the only block form of an object (D3); the v1.1 inline form `metrics port: 9090 ...` is now a syntax error `E0201` with a hint to "use `metrics:` … `end`".
- `new Ident \n props end` → `SchemaInstance` (D4). A bare `Ident` in expression position is always a `Variable`, regardless of case.
- `assert expr [, expr]` is a statement; allowed at any level (module, `domain`, `def`). A failure raises `E0530` with the evaluated message.
- `shadow name = expr` — an assignment that explicitly permits shadowing an outer scope (D7).
- Trailing lambda: `xs.map (a, b) -> ... end` — if the method name is followed by `(` params `)` `->`, the lambda is stored in `MethodCall.lambda`.

### 3.3. Pratt parsing

| Precedence | Operators | Associativity |
| --- | --- | --- |
| 1 | `? :` | right |
| 2 | `\|\|` | left |
| 3 | `&&` | left |
| 4 | `== !=` | left |
| 5 | `< > <= >=` | left |
| 6 | `+ -` | left |
| 7 | `* / %` | left |
| 8 | unary `! -` | prefix |
| 9 | `.` `(` | postfix, left |

```rust
fn parse_expr(&mut self, min_bp: u8) -> Result<Expr<'a>, Diagnostic> {
    let mut lhs = self.parse_prefix()?;
    loop {
        match self.peek() {
            t if t.is_postfix() => lhs = self.parse_postfix(lhs)?,
            t if let Some((lbp, rbp)) = infix_bp(t) => {
                if lbp < min_bp { break }
                self.bump();
                let rhs = self.parse_expr(rbp)?;
                lhs = Expr::Binary { .. };
            }
            TokenKind::Question if TERNARY_BP >= min_bp => {
                self.bump();
                let then = self.parse_expr(0)?;
                self.expect(TokenKind::Colon)?;
                let other = self.parse_expr(TERNARY_BP)?;
                lhs = Expr::Ternary { .. };
            }
            _ => break,
        }
    }
    Ok(lhs)
}
```

Postfix `.`: `Ident` + `(` → `MethodCall`, otherwise `FieldAccess`. Chains are handled by a left-associative postfix loop.

### 3.4. Error recovery

Fail-fast within an expression; at the `Stmt` level - synchronize to the next `Newline`/`end`, errors accumulate in a `Vec<Diagnostic>`.

---

## 4. Phase 3 — the evaluation engine

### 4.1. Value

```rust
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),                               // D6
    Float(f64),
    Str(Arc<str>),
    List(Arc<Vec<Value>>),
    Object(Arc<IndexMap<String, Value>>),   // insertion order is deterministic
    Schema(Arc<SchemaDef>),
    Function(Arc<FunctionDef>),
    Module(Arc<IndexMap<String, Value>>),
}

pub struct SchemaDef { pub name: String, pub fields: Vec<(String, TypeName<'static>)> }
pub struct FunctionDef {
    pub params: Vec<String>,
    pub body: OwnedExpr,
    pub closure: Env,                       // lexical closure
    pub origin: ModuleId,                   // for capability checks (D1)
}
```

Invariants:

- Containers are immutable, held in `Arc`: cloning is O(1); methods return new values.
- `Object` is an `IndexMap`; `HashMap` is forbidden (non-deterministic output).
- Arithmetic: `Int ⊕ Int → Int` (overflow → `E0304`, no wrapping), if a `Float` is involved the result is `Float`; `Int / Int` is integer division, `/ 0` → `E0305`. Comparing `Int == Float` is by mathematical value.
- `Function`/`Schema` compare by `Arc::ptr_eq`.

### 4.2. Environment: immutable frames (D9)

```rust
pub type Env = Arc<Environment>;

pub struct Environment {
    vars: IndexMap<String, Value>,
    parent: Option<Env>,
}
```

A frame's lifecycle has two states:

1. **Construction**: the interpreter owns an `EnvBuilder { vars, parent }` exclusively and executes the block's statements in order, inserting bindings. Values already inserted are visible to subsequent expressions in the same block.
2. **Freezing**: `EnvBuilder::freeze(self) -> Env` (`Arc::new`) is called before creating any closure/child frame that references the current one. Mechanically: the builder freezes the current frame prefix; the rest of the block continues in a new child builder (a chain of frozen frames). No `RwLock` - there are no races by construction, deadlocks are impossible.

Binding rules:

- Reassigning a name via `=` that is already defined in the **current** frame → `E0301: variable is immutable` (always an error).
- `=` for a name defined in an **outer** frame, without `shadow` → `E0302: shadowing requires explicit 'shadow' keyword` (D7). With `shadow` — legal shadowing in the current frame; the diagnostic's secondary label points to the original declaration.
- `shadow` on a name that shadows nothing → `W0303 useless shadow`.
- `Arc` cycles are impossible: references only point upward; `Weak` is not needed.

New frames: the body of a `domain`/`component`, a `def`, a lambda, each `map` callback invocation.

Implementation note (v0.1): instead of an explicit builder/freeze pair, a frame uses internal mutability (`RefCell<IndexMap>`) while a block is being built; the E0301/E0302 invariants and the strictly-upward references are preserved. Observable difference from strict freezing: a closure sees bindings from its own block declared after it - it just cannot be called before they are declared, given top-down execution.

### 4.3. Capability model for effects (D1)

```rust
#[derive(Clone, Default)]
pub struct Capabilities {
    pub read_paths: Vec<PathBuf>,   // --allow-read=./ (empty = denied)
    pub env_vars: Option<Vec<String>>, // --allow-env[=A,B]; None = denied
    pub grant_to_imports: bool,     // --allow-imports-io; false by default
}
```

Invariants:

- Effectful builtins (`read_file`, `env`) check the `origin` of the current function/module when called: the root module gets its capabilities from CLI flags; **imported modules have none by default** - calling one yields `E0310: module 'github/...' has no read capability; pass --allow-imports-io to grant`.
- `read_file(path)`: the path is canonicalized and checked against a prefix from `read_paths`; escaping it (including via `..`) is `E0311`.
- `env(name, default)`: the name must be in the allow-list (or the list means "all" when `--allow-env` is passed with no argument).
- By default (no flags), the root module also has no I/O: `aura eval config.aura` is deterministic by construction; the reference manifest is run as `aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV`.
- The check runs during eval, but `SemanticAnalyzer` additionally emits `W0512 effectful call in imported module` statically.

### 4.4. MethodRegistry

```rust
pub trait Method: Send + Sync {
    fn name(&self) -> &'static str;
    fn receiver(&self) -> TypeTag;   // Str | List | Object | Int | Float | Any
    fn call(&self, recv: &Value, args: &[Value], ctx: &mut EvalCtx) -> Result<Value, AuraError>;
}

pub struct MethodRegistry { table: HashMap<(TypeTag, &'static str), Arc<dyn Method>> }
impl MethodRegistry {
    pub fn register(&mut self, m: Arc<dyn Method>);
    pub fn resolve(&self, recv: &Value, name: &str) -> Option<Arc<dyn Method>>; // exact type, then Any
    pub fn builtin() -> Self;
}
```

- The parser knows nothing about methods; adding a method = a file in `eval/methods/` + `register`.
- Implementation note (v0.1): the registry stores `MethodFn<'a>` fn pointers instead of `Arc<dyn Method>`; the extensibility property (registering without touching the parser/interpreter) is preserved, trait objects will appear if stateful methods are ever needed.
- `map`/`filter` receive the lambda as a `Value::Function`; `ctx.call_function(...)`.
- Semantics: `.compact()` removes `Null` while preserving order; `.uniq()` deduplicates keeping the first occurrence; `.merge(other)` - the right side overrides; `.len()` → `Int`; `.upper()`/`.lower()` via the `str` API; `.first()`/`.last()` - E0317 on an empty list; `.get(index_or_key, default)` - safe access (a miss returns default/Null); `.keys()`/`.values()` - Object → List in declaration order; `.contains(x)` - element/key/substring for List/Object/Str; `.join(sep)` - list scalars joined by a separator.
- Interpolation invariant: inside `#{...}` ordinary expression syntax applies - string quotes are NOT escaped by the outer string's rules (the slice is scanned by `{}` balance).
- Formats (the D11 package): `.parse_toml()` / `.parse_json()` / `.parse_yaml()` on `Str` → `Value` (integers → `Int`, a unified mapping via serde_json); `.to_json()` / `.to_yaml()` / `.to_toml()` on `Object`/`List` → `Str` (E0603: TOML requires an object at the top level and has no null). The emitters are shared with the CLI's `--format json|json-flat|yaml|toml`.
- Time (D13, deterministic): `.parse_duration()` on `Str` → `Int` seconds (units d/h/m/s, E0319 on error); `.format_duration()` on `Int` → `Str`; `.parse_datetime()` on `Str` (RFC3339) → `Int` epoch UTC (offsets `±HH:MM`, E0320 on error); `.format_datetime()` on `Int` → `Str` (RFC3339 UTC). `now()`/`timestamp()` do not exist (E0533).
- Stdlib extension (v1.3): `String` — `.trim()`, `.split(sep)` (non-empty `sep` → List), `.replace(from, to)`, `.starts_with(p)`/`.ends_with(s)` → Bool, `.to_int()`/`.to_float()` (E0314 on failure); `List` — `.sort()` (ascending, mutually-comparable scalars, else E0306), `.reverse()`, `.sum()` (Int, or Float if any Float; E0304 on Int overflow), `.min()`/`.max()` (E0317 on empty), `.flatten()` (one level: spreads nested lists, keeps scalars), `.slice(start, end)` (half-open, indices clamped); `Int`/`Float` — `.abs()`; `Int`/`Float`/`Bool`/`Str` — `.to_str()` (same rendering as string interpolation).
- Global pure functions (not methods): `range(n)` → `[0, 1, ..., n-1]` (deterministic list generator; `n` must be a non-negative `Int` ≤ 1,000,000, else E0306). No capability needed. Alongside the effectful `env`/`read_file`/`fail` (§4.3) and the banned `now`/`timestamp` (E0533, D13).

### 4.5. EvalCtx

```rust
pub struct EvalCtx {
    pub registry: Arc<MethodRegistry>,
    pub resolver: Arc<dyn FileResolver>,
    pub modules: ModuleCache,
    pub caps: Capabilities,
    pub current_module: ModuleId,   // for D1 checks
    pub options: Options,           // strict, dry_run
    pub call_depth: u32,            // limit 256 -> E0399
}
```

A module evaluates to a `Value::Object` built only from `key:` properties and `domain` blocks (key = label); `=` bindings, `def`, `type` are private (D10). A block's body exports its properties and nested blocks under the same rule; a `component` additionally gets the key `name` = label.

---

## 5. Phase 4 — the VFS and modularity

### 5.1. FileResolver

```rust
pub trait FileResolver: Send + Sync {
    fn resolve(&self, spec: &ImportSpec, importer: Option<&ModuleId>) -> Result<ModuleId, AuraError>;
    fn load(&self, id: &ModuleId) -> Result<String, AuraError>;
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ModuleId {
    Local(PathBuf),
    Registry { path: String, version: Version },  // the exact version after resolution
    Url(String),                                  // a placeholder for a Deno-style scheme
}
```

Implementations: `LocalFsResolver`, `MemoryResolver` (tests, dry-run), `HttpResolver` in the future.

### 5.2. Versions and the lock file (D8)

- `import github/actions/rust-cache@v1.2 as rust`: `@v1` and `@v1.2` are ranges (semver-caret style), `@v1.2.3` is an exact version.
- `aura.lock` (TOML, next to the root manifest) pins the resolution: `path = { version = "1.2.7", integrity = "sha256-..." }`. Algorithm: if a lock entry satisfies the range, it's used and its content hash is checked (`E0402 integrity mismatch`); otherwise the resolver picks the highest matching version from the local cache `~/.aura/registry/` and appends to the lock. The `--frozen` flag (CI): a missing/mismatched lock entry is an `E0403` error, and the lock is never rewritten.
- The network (v1.3): exists ONLY in the `aura add <path>@vX.Y.Z` command - downloading by the convention `github/<owner>/<repo>` → the raw file `package.aura` at tag `vX.Y.Z`, validating the package (lex+parse+analysis), installing it into the cache and writing its integrity to `aura.lock`. **`eval` never touches the network** - evaluation's determinism never depends on network state; `--frozen` in CI guarantees that exactly what's installed is used. `aura add --from <file>` installs from a local file (private packages, tests). A network install requires an exact version; ranges (`@v1`) are resolved from the local cache.

### 5.3. Module graph, cycles, AST cache

```rust
pub struct ModuleCache {
    asts: HashMap<ModuleId, Arc<ParsedModule>>,
    values: HashMap<ModuleId, Value>,
    state: HashMap<ModuleId, LoadState>,     // Loading | Loaded
    import_stack: Vec<ModuleId>,
}
```

DFS with coloring: `Loaded` → served from the `values` cache; `Loading` → a cycle, emitting `E0401: cyclic import: a.aura → b.aura → a.aura` with the full chain from `import_stack`; otherwise `Loading` → `load → tokenize → parse` (the AST is cached lazily) → `eval` → `Loaded`. Every module is lexed/parsed/evaluated exactly once.

---

## 6. Phase 5 — static analysis (`--strict`, `--dry-run`)

### 6.1. SemanticAnalyzer (over the AST, before the runtime)

```rust
pub struct SemanticAnalyzer<'a> { scopes: Vec<ScopeInfo<'a>>, diags: Vec<Diagnostic> }
struct ScopeInfo<'a> { defined: IndexMap<&'a str, (Span, /*used:*/ bool)> }
```

A single pass with a scope stack mirroring the Environment:

1. `push_scope` on entering a module/`def`/`domain`/lambda; declarations (`Assign`, parameters, `import as`, `def`, `type`) are registered as `(span, used=false)`.
2. `Variable`/the root of a `FieldAccess` marks the nearest declaration `used=true` (searching bottom-up - this respects shadowing).
3. `pop_scope`: unused ones become `W0501 unused variable` / `W0502 unused import` / `W0503 unused function/type` (in the manifest - `unused_config_version`).
4. `E0504 use of undefined variable` - always an error.
5. Static checks for D7 (`shadow` required/useless) and D1 (`W0512` - an effectful call in an imported module).

Under `--strict`, every `W05xx` becomes an error, exit code ≠ 0.

### 6.2. `--strict` invariants (runtime)

- `new Schema ... end`: a missing field is `E0511`; a type mismatch (`Int` vs `Str`, `Int` vs `Float` also counts as a mismatch) is `E0512` - both always errors. An extra field is `E0513` under `--strict`, otherwise a warning (the field is kept).

### 6.3. `--dry-run` invariants

- The `resolver` is wrapped in a `RecordingResolver`: reads are performed, but recorded into a report.
- Network sources (future) are replaced by a snapshot cache; a miss is `E0521 network access forbidden in dry-run`.
- No writing to disk happens: `[dry-run] would write N bytes to <path>` plus JSON on stdout.
- The D1 capability checks behave the same as in normal mode; `assert`/`fail` still work - dry-run must be able to catch a validation failure.
- Invariant: `--dry-run` never changes the evaluation result; two runs produce byte-identical JSON.

---

## 7. Phase 6 — serialization and the CLI

### 7.1. Value → serde_json

```rust
impl serde::Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(n) => s.serialize_i64(*n),
            Value::Float(n) => s.serialize_f64(*n),
            Value::Str(v) => s.serialize_str(v),
            Value::List(xs) => xs.serialize(s),
            Value::Object(m) | Value::Module(m) => m.serialize(s),
            Value::Schema(_) | Value::Function(_) =>
                Err(S::Error::custom("E0601: schema/function is not serializable")),
        }
    }
}
```

- `Int` → a JSON integer with no precision loss (D6). `Float` with `NaN`/`Inf` is `E0602`.
- A `Schema`/`Function` deep in the tree is `E0601` with a key path; top-level `def`/`type`/lambdas are excluded from the export.
- Modes: `--format json` (pretty, key order = declaration order) and `--format json-flat` (`a.b.c = v`).

### 7.2. CLI

```text
aura eval <file.aura> [--strict] [--dry-run] [--frozen]
                      [--allow-read=<paths>] [--allow-env[=A,B]] [--allow-imports-io]
                      [--format json|json-flat|yaml|toml] [-o out.json]
                      [--registry-dir=<dir>]
aura check <file.aura> [--strict]        # lex + parse + analysis
aura fmt <files...> [--check]            # indentation canonicalization
aura add <path>@vX.Y.Z [--from <file>] [--registry-dir=<dir>]  # package install (the D12 package, §5.2)
```

`aura fmt` is line-oriented: it normalizes indentation (2 spaces/level by
token-depth: `domain`/`component`/`def`/`type`/`new`/`->`/`[`/`(`/a trailing
`key:` open a level, `end`/`]`/`)` close one; continuation lines after
`,`/`=`/an operator get +1), trailing whitespace and blank lines (≤1 in a
row); comments and intra-line alignment are preserved. Invariant: the token
stream is unchanged before/after; it's idempotent.

`--format yaml|toml` uses the emitters from §4.4 (the D11 package);
`--registry-dir` sets the local package cache (default `~/.aura/registry`),
used both by `eval` (resolving `import`) and by `add` (the install target).
`aura add`'s semantics are in §5.2.

Exit codes: 0 - success; 1 - diagnostic errors; 2 - I/O/argument errors.

### 7.3. Diagnostics (ariadne)

```rust
pub struct Diagnostic {
    pub code: &'static str,        // "E0302"
    pub severity: Severity,
    pub message: String,
    pub primary: (Span, String),
    pub secondary: Vec<(Span, String)>, // "variable declared here"
    pub help: Option<String>,           // "add 'shadow' keyword", "use metrics: ... end"
}
```

- `SourceCache` implements `ariadne::Cache`; `Span` → line/column via `ariadne`.
- The core returns `Vec<Diagnostic>`; rendering happens only in `aura-cli` (the core is WASM/LSP-ready).

---

## 8. Development plan and phase acceptance criteria

| Phase | Artifact | Acceptance criterion |
| --- | --- | --- |
| 1 | `lexer` | Tokenizes the v1.2 manifest; a snapshot test; a property test "concatenating spans = the source"; tests for D2 (`[a \n -b]` → 2 elements) and D8 (`E0104` without a version) |
| 2 | `parser` | An AST snapshot; Pratt precedences; `E0201` on an inline block; `new`/`assert`/`shadow`; negative tests with positions |
| 3 | `eval` | The manifest with `MemoryResolver`; tests for `E0301`/`E0302`/shadow; Int/Float arithmetic and overflow; capability denials `E0310`/`E0311`; every builtin method |
| 4 | `vfs` | A cycle a→b→a with the full chain; the lock file: range resolution, `E0402`/`E0403 --frozen`; every file is parsed exactly once |
| 5 | `analysis` | `unused_config_version` is detected; `--strict` fails on `E0513`; `--dry-run` produces identical JSON without writing |
| 6 | `cli`, `serialize` | A golden test: manifest → `production_deploy.json`; `Int` without `.0`; golden-text ariadne reports (ANSI-stripped, no external snapshot tool) |
| 6.5 | closing the §6.3/§8 gaps | `RecordingResolver`/`RecordingFs`: `--dry-run` logs the files it read into a report; a golden comparison of `check --strict`'s stderr on the reference manifest |
| 7 (v1.3) | `D10`-`D13`, `aura fmt`, `aura add` | `=` is not exported (D10); `.field`/`."str"`/`xs[i]`/`.get` (D11); `pub def`/`pub type` via the module object, with capabilities running from the origin module (D12); `now()` is forbidden, `parse_duration`/`parse_datetime` work (D13); `aura fmt` never changes the token stream and is idempotent; `aura add --from` → an offline `eval --frozen` through the installed package |

Test tooling: `proptest` (the lexer - fuzzing and span invariants), golden text files for ariadne reports (no external snapshot framework), conformance tests in `aura-cli/tests/` that exercise the real binary against `examples/*/expected.*`.

---

*[Русская версия / Russian version: SPEC.ru.md](SPEC.ru.md)*
