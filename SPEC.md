# Aura v1.2 — Master Specification интерпретатора (Rust)

Статус: эталон. Любая реализация обязана соответствовать данному документу. Версия v1.2 — ревизия дизайна v1.1 по итогам архитектурного аудита (см. §0.2).

---

## 0. Общие положения

- Язык: Rust (edition 2021+), MSRV 1.75.
- Принципы: Zero-Copy лексер/парсер (`&'a str`), иммутабельность значений, детерминизм вычислений, отсутствие значимых отступов, `\n` — разделитель, `end` — явный закрывающий терминатор, **capability-модель эффектов** (I/O запрещён по умолчанию для импортированных модулей).
- Внешние зависимости ядра: `serde`, `serde_json`, `toml`, `indexmap`, `ariadne` (диагностика), `clap` (CLI). Ядро (`lexer`, `parser`, `eval`) не зависит от `clap`/`ariadne`.

### 0.1. Эталонный манифест (v1.2)

Файл `tests/fixtures/production_deploy.aura` обязан проходить полный пайплайн (`parse → analyze → eval → json`) в каждом CI-прогоне:

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

### 0.2. Изменения дизайна v1.1 → v1.2 (нормативные)

| # | Было (v1.1) | Стало (v1.2) | Мотивация |
| --- | --- | --- | --- |
| D1 | `read_file`/`env` доступны везде | Capability-модель: эффекты разрешены только корневому модулю; импортам — только по явным флагам | Детерминизм и безопасность цепочки поставок |
| D2 | `\n` подавляется после бинарных операторов, в т.ч. в списках | Внутри `[]` перенос ВСЕГДА завершает элемент; многострочное выражение — только в `(...)` | Устранение неоднозначности `[a \n -b]` |
| D3 | Инлайн-блок `metrics port: 9090 path: "/metrics"` | Удалён. Только объектная форма `metrics: ... end` | −1 ветка грамматики, −1 неочевидное правило |
| D4 | Инстанс схемы по заглавной букве (`ServiceMeta ... end`) | Явное ключевое слово `new`: `new ServiceMeta ... end` | Опечатка в регистре — синтаксическая ошибка, а не смена семантики |
| D5 | Валидация через `cond ? fail(...) : null` | Statement `assert cond, "msg"`; `fail()` остаётся как выражение для веток | Читаемость, нет мусорного `null` |
| D6 | Числа — только `f64` | `Int(i64)` и `Float(f64)` — раздельные типы | Точность для байтовых лимитов и 64-битных ID |
| D7 | Тихое затенение внешних переменных | Затенение внешнего scope требует маркера `shadow`; без него — `E0302` | «Почему в проде другой путь» больше не молчит |
| D8 | `import github/actions/rust-cache` | Обязательная версия `@vX[.Y[.Z]]` + лок-файл `aura.lock` | Воспроизводимость CI/CD |
| D9 | `Arc<RwLock<Environment>>` | Иммутабельный `Arc<Environment>` с фазой заморозки | Нет дедлоков, меньше кода; мутаций всё равно нет |

---

## 1. Архитектурный граф и схема модулей

### 1.1. Структура каталогов

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
│           │   └── pratt.rs    # таблица приоритетов
│           ├── eval/
│           │   ├── mod.rs      # Interpreter
│           │   ├── value.rs    # Value
│           │   ├── env.rs      # Environment (frozen)
│           │   ├── caps.rs     # Capabilities
│           │   └── methods/    # MethodRegistry + встроенные методы
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

### 1.2. Поток данных

```text
&'a str (исходник, живёт в SourceCache)
   │  Lexer<'a>::tokenize()               — O(n), zero-copy
   ▼
Vec<Token<'a>>
   │  Parser<'a>::parse_module()          — рекурсивный спуск + Pratt
   ▼
Module<'a> (AST)  ──►  SemanticAnalyzer (--strict: dead code, схемы)
   │  Interpreter::eval_module()          — Environment + Capabilities + VFS
   ▼
Value (owned, иммутабельное)
   │  serialize::to_json()
   ▼
serde_json::Value  ──►  stdout / файл
```

Время жизни: `SourceCache` владеет `String` каждого файла; токены, AST и диагностики заимствуют из него. `Value` — полностью owned (обрыв заимствования на границе eval).

Корневой фасад:

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

## 2. Фаза 1 — Zero-Copy лексер

### 2.1. Типы

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span { pub source: SourceId, pub start: u32, pub end: u32 } // байтовые смещения

#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> { pub kind: TokenKind<'a>, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    // Литералы (zero-copy)
    Ident(&'a str),
    Int(i64),                  // D6: целые и дробные — разные токены
    Float(f64),
    Str(&'a str),              // содержимое без кавычек, без интерполяции
    InterpStr(Vec<StrPart<'a>>),
    ImportPath { path: &'a str, version: &'a str }, // github/actions/rust-cache@v1.2 (D8)
    True, False, Null,
    // Ключевые слова
    Import, As, Type, Def, End, Domain, Component,
    New, Assert, Shadow,       // D4, D5, D7
    // Разделители
    Newline,
    LParen, RParen, LBracket, RBracket,
    Colon, Comma, Dot, Assign, // : , . =
    Arrow,                     // ->
    Question,
    // Операторы
    Plus, Minus, Star, Slash, Percent,
    EqEq, NotEq, Lt, Gt, LtEq, GtEq, And, Or, Not,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart<'a> { Lit(&'a str), Interp(&'a str) } // Interp парсится Фазой 2
```

Инвариант zero-copy: `TokenKind` не содержит `String`; все текстовые поля — срезы исходника. Интерполяция `#{expr}` хранится сырым срезом, вложенный парсер запускается на нём в Фазе 2 (смещение span сохраняется).

### 2.2. ДКА: числа (D6)

- `[0-9]+` → `Int(i64)`; переполнение i64 → `E0103`.
- `[0-9]+ "." [0-9]+` → `Float(f64)`. `12.` — ошибка `E0101`; `12.foo` — откат: `Int(12)`, `Dot`, `Ident` (lookahead 1 символ).
- Минус — не часть числа (унарный оператор парсера).

### 2.3. ДКА: строки

- Открывается `"`. Escape: `\" \\ \n \t \#`. Строка без escape — чистый срез; с escape — сырой срез, разэкранирование лениво в eval (`Cow<'a, str>`).
- `#{` переключает в интерполяцию (баланс `{}`), результат — `InterpStr`.
- Незакрытая строка до `\n`/EOF — `E0102`.

### 2.4. Ключевые слова, идентификаторы, пути импорта

- Идентификатор: `[a-zA-Z_][a-zA-Z0-9_]*`, затем сверка с таблицей ключевых слов (`import as type def end domain component new assert shadow true false null`).
- Путь импорта — контекстный режим после `Import` (если следующий символ не `"`): `[a-zA-Z0-9_\-]+ ("/" [a-zA-Z0-9_\-]+)* "@" "v" [0-9]+ ("." [0-9]+){0,2}`. Отсутствие `@vX` — ошибка `E0104: registry import requires a version` (D8). Файловые импорты (`"..."`) версии не требуют.
- Комментарий: `#` вне строки — до конца строки.

### 2.5. Инварианты нормализации `\n` (D2)

Лексер ведёт стек скобок `paren_stack: Vec<Delim>` (`Paren` | `Bracket`).

1. Последовательность `\n+` (включая `\r\n`, пустые строки, комментарии) → один `Newline`.
2. Вне `[]` `Newline` подавляется, если предыдущий значимый токен ∈ { `(`, `=`, `->`, `?`, `,`, бинарный оператор, `.` }. `:` в набор НЕ входит: перенос после `key:` значим — он открывает объектный блок (§3.2); продолжение тернарного `? :` с новой строки после `:` требует скобок.
3. Вне `[]` `Newline` подавляется, если следующий значимый токен ∈ { `)`, `.` }.
4. Внутри `(...)` переносы подавляются полностью.
5. **Внутри `[...]` перенос ВСЕГДА эмитится как разделитель элементов**, кроме позиций сразу после `[` и перед `]`. Правило 2 внутри `[]` НЕ действует: строка списка обязана быть законченным выражением; многострочное выражение-элемент оборачивается в `(...)`. Это делает `[a \n -b]` однозначно списком `[a, -b]`.
6. Запятая внутри `[]` допустима как альтернативный/дублирующий разделитель (`[1, 2, 3]`); `, \n` схлопывается в один разделитель.
7. `Newline` в начале файла и перед `Eof` подавляется.

### 2.6. API

```rust
pub struct Lexer<'a> { src: &'a str, pos: usize, source: SourceId, expect_import_path: bool, paren_stack: Vec<Delim> }
impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, source: SourceId) -> Self;
    pub fn tokenize(self) -> Result<Vec<Token<'a>>, Diagnostic>; // fail-fast
}
```

---

## 3. Фаза 2 — AST и Pratt-парсер

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
    Property { key: &'a str, value: Expr<'a>, span: Span },              // key: expr | key: <объектный блок> — внутри domain/component
    Assert { cond: Expr<'a>, message: Option<Expr<'a>>, span: Span },    // assert cond, "msg" (D5)
    TypeDecl(SchemaDeclaration<'a>),
    FuncDecl { name: &'a str, params: Vec<&'a str>, body: ObjectBody<'a>, span: Span },
    Block(BlockDeclaration<'a>),
    Expr(Expr<'a>),
}

pub struct SchemaDeclaration<'a> { pub name: &'a str, pub fields: Vec<(&'a str, TypeName<'a>)>, pub span: Span }
pub enum TypeName<'a> { String, Int, Float, Bool, List, Object, Custom(&'a str) }

pub struct BlockDeclaration<'a> {
    pub kind: BlockKind,          // Domain | Component
    pub label: Expr<'a>,          // "production-eu" | name — выражение
    pub body: Vec<Stmt<'a>>,      // инлайн-форма удалена (D3)
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
    SchemaInstance { schema: &'a str, body: ObjectBody<'a>, span: Span }, // ТОЛЬКО через `new` (D4)
    Block(Box<BlockDeclaration<'a>>),  // component внутри map
}

pub struct ObjectBody<'a> { pub props: Vec<(&'a str, Expr<'a>, Span)> }
pub enum LambdaBody<'a> { Expr(Box<Expr<'a>>), Object(ObjectBody<'a>) }
```

### 3.2. Правила разбора конструкций

- `key: value` внутри тела объекта — свойство; `name = expr` — присваивание. Различаются по токену после идентификатора (`:` vs `=`).
- Вложенный объект: `key:` + непосредственный `Newline` (значения на строке нет) открывает `ObjectLiteral` до `end`. Это единственная блочная форма объекта (D3); в v1.1 существовавшая инлайн-форма `metrics port: 9090 ...` — синтаксическая ошибка `E0201` с подсказкой «use `metrics:` … `end`».
- `new Ident \n props end` → `SchemaInstance` (D4). Голый `Ident` в позиции выражения — всегда `Variable`, независимо от регистра.
- `assert expr [, expr]` — statement; допустим на любом уровне (модуль, `domain`, `def`). Провал → `E0530` с вычисленным сообщением.
- `shadow name = expr` — присваивание с разрешением затенения внешнего scope (D7).
- Trailing-lambda: `xs.map (a, b) -> ... end` — если после имени метода идёт `(` params `)` `->`, лямбда пишется в `MethodCall.lambda`.

### 3.3. Pratt-парсинг

| Приоритет | Операторы | Ассоциативность |
| --- | --- | --- |
| 1 | `? :` | правая |
| 2 | `\|\|` | левая |
| 3 | `&&` | левая |
| 4 | `== !=` | левая |
| 5 | `< > <= >=` | левая |
| 6 | `+ -` | левая |
| 7 | `* / %` | левая |
| 8 | унарные `! -` | префикс |
| 9 | `.` `(` | постфикс, левая |

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

Постфикс `.`: `Ident` + `(` → `MethodCall`, иначе `FieldAccess`. Цепочки — левоассоциативной постфиксной петлёй.

### 3.4. Восстановление после ошибок

Fail-fast внутри выражения; на уровне `Stmt` — синхронизация до следующего `Newline`/`end`, ошибки копятся в `Vec<Diagnostic>`.

---

## 4. Фаза 3 — движок вычислений

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
    Object(Arc<IndexMap<String, Value>>),   // порядок вставки детерминирован
    Schema(Arc<SchemaDef>),
    Function(Arc<FunctionDef>),
    Module(Arc<IndexMap<String, Value>>),
}

pub struct SchemaDef { pub name: String, pub fields: Vec<(String, TypeName<'static>)> }
pub struct FunctionDef {
    pub params: Vec<String>,
    pub body: OwnedExpr,
    pub closure: Env,                       // лексическое замыкание
    pub origin: ModuleId,                   // для capability-проверок (D1)
}
```

Инварианты:

- Контейнеры иммутабельны, в `Arc`: клонирование — O(1); методы возвращают новые значения.
- `Object` — `IndexMap`; `HashMap` запрещён (недетерминированный вывод).
- Арифметика: `Int ⊕ Int → Int` (переполнение → `E0304`, не wrap), `Float` участвует — результат `Float`; `Int / Int` — целочисленное деление, `/ 0` → `E0305`. Сравнение `Int == Float` — по математическому значению.
- `Function`/`Schema` сравниваются по `Arc::ptr_eq`.

### 4.2. Environment: иммутабельные фреймы (D9)

```rust
pub type Env = Arc<Environment>;

pub struct Environment {
    vars: IndexMap<String, Value>,
    parent: Option<Env>,
}
```

Жизненный цикл фрейма — два состояния:

1. **Построение**: интерпретатор владеет `EnvBuilder { vars, parent }` монопольно и выполняет statements блока по порядку, вставляя биндинги. Уже вставленные значения видимы последующим выражениям того же блока.
2. **Заморозка**: `EnvBuilder::freeze(self) -> Env` (`Arc::new`) — вызывается перед созданием любого замыкания/дочернего фрейма, ссылающегося на текущий. Технически: билдер замораживает текущий префикс фрейма; продолжение блока идёт в новом дочернем билдере (chain of frozen frames). Никаких `RwLock` — гонок нет по построению, дедлоки невозможны.

Правила биндинга:

- Повторное `=` для имени, уже определённого в **текущем** фрейме → `E0301: переменная иммутабельна` (всегда ошибка).
- `=` для имени, определённого во **внешнем** фрейме, без `shadow` → `E0302: shadowing requires explicit 'shadow' keyword` (D7). С `shadow` — легальное затенение в текущем фрейме; secondary-метка диагностики указывает на исходное объявление.
- `shadow` для имени, которое ничего не затеняет → `W0303 useless shadow`.
- Циклы `Arc` невозможны: ссылки идут строго вверх; `Weak` не требуется.

Новые фреймы: тело `domain`/`component`, `def`, лямбда, каждый вызов `map`-колбэка.

### 4.3. Capability-модель эффектов (D1)

```rust
#[derive(Clone, Default)]
pub struct Capabilities {
    pub read_paths: Vec<PathBuf>,   // --allow-read=./ (пусто = запрещено)
    pub env_vars: Option<Vec<String>>, // --allow-env[=A,B]; None = запрещено
    pub grant_to_imports: bool,     // --allow-imports-io; false по умолчанию
}
```

Инварианты:

- Эффектные builtin'ы (`read_file`, `env`) при вызове проверяют `origin` текущей функции/модуля: корневой модуль получает capabilities из CLI-флагов; **импортированные модули по умолчанию не имеют никаких** — вызов → `E0310: module 'github/...' has no read capability; pass --allow-imports-io to grant`.
- `read_file(path)`: путь канонизируется и проверяется на префикс из `read_paths`; выход за пределы (в т.ч. через `..`) → `E0311`.
- `env(name, default)`: имя должно входить в allow-список (или список = «все» при `--allow-env` без аргумента).
- По умолчанию (без флагов) корневой модуль тоже не имеет I/O: `aura eval config.aura` детерминирован по построению; эталонный манифест запускается как `aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV`.
- Проверка выполняется в eval, но `SemanticAnalyzer` дополнительно эмитит `W0512 effectful call in imported module` статически.

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
    pub fn resolve(&self, recv: &Value, name: &str) -> Option<Arc<dyn Method>>; // точный тип, затем Any
    pub fn builtin() -> Self;
}
```

- Парсер о методах не знает; добавление метода = файл в `eval/methods/` + `register`.
- `map`/`filter` получают лямбду как `Value::Function`; `ctx.call_function(...)`.
- Семантика: `.compact()` — удаляет `Null`; `.uniq()` — дедупликация с сохранением первого вхождения; `.merge(other)` — правый перекрывает; `.parse_toml()` — `toml::from_str` → `Value` (целые TOML → `Int`); `.len()` → `Int`; `.upper()`/`.lower()` — ASCII+Unicode через `str` API.

### 4.5. EvalCtx

```rust
pub struct EvalCtx {
    pub registry: Arc<MethodRegistry>,
    pub resolver: Arc<dyn FileResolver>,
    pub modules: ModuleCache,
    pub caps: Capabilities,
    pub current_module: ModuleId,   // для проверок D1
    pub options: Options,           // strict, dry_run
    pub call_depth: u32,            // лимит 256 → E0399
}
```

Результат модуля — `Value::Object` из топ-уровневых биндингов + `domain`-блоков (ключ = label).

---

## 5. Фаза 4 — VFS и модульность

### 5.1. FileResolver

```rust
pub trait FileResolver: Send + Sync {
    fn resolve(&self, spec: &ImportSpec, importer: Option<&ModuleId>) -> Result<ModuleId, AuraError>;
    fn load(&self, id: &ModuleId) -> Result<String, AuraError>;
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ModuleId {
    Local(PathBuf),
    Registry { path: String, version: Version },  // точная версия после резолва
    Url(String),                                  // задел под Deno-style
}
```

Реализации: `LocalFsResolver`, `MemoryResolver` (тесты, dry-run), в будущем `HttpResolver`.

### 5.2. Версии и лок-файл (D8)

- `import github/actions/rust-cache@v1.2 as rust`: `@v1` и `@v1.2` — диапазоны (semver caret), `@v1.2.3` — точная версия.
- `aura.lock` (TOML, рядом с корневым манифестом) фиксирует резолв: `path = { version = "1.2.7", integrity = "sha256-..." }`. Алгоритм: если запись в локе удовлетворяет диапазон — берётся она и сверяется хэш содержимого (`E0402 integrity mismatch`); иначе резолвер выбирает максимальную подходящую версию из локального кэша `~/.aura/registry/` и дописывает лок. Флаг `--frozen` (CI): отсутствие/несовпадение лока → ошибка `E0403`, лок не переписывается.
- Сети в v1.2 нет: registry — локальный кэш-каталог; `HttpResolver` появится за тем же трейтом.

### 5.3. Граф модулей, циклы, кэш AST

```rust
pub struct ModuleCache {
    asts: HashMap<ModuleId, Arc<ParsedModule>>,
    values: HashMap<ModuleId, Value>,
    state: HashMap<ModuleId, LoadState>,     // Loading | Loaded
    import_stack: Vec<ModuleId>,
}
```

DFS с раскраской: `Loaded` → кэш `values`; `Loading` → цикл, эмитится `E0401: cyclic import: a.aura → b.aura → a.aura` с полной цепочкой из `import_stack`; иначе `Loading` → `load → tokenize → parse` (AST кэшируется лениво) → `eval` → `Loaded`. Каждый модуль лексируется/парсится/вычисляется ровно один раз.

---

## 6. Фаза 5 — статический анализ (`--strict`, `--dry-run`)

### 6.1. SemanticAnalyzer (по AST, до рантайма)

```rust
pub struct SemanticAnalyzer<'a> { scopes: Vec<ScopeInfo<'a>>, diags: Vec<Diagnostic> }
struct ScopeInfo<'a> { defined: IndexMap<&'a str, (Span, /*used:*/ bool)> }
```

Один проход со стеком областей, зеркалирующим Environment:

1. `push_scope` на входе в модуль/`def`/`domain`/лямбду; объявления (`Assign`, параметры, `import as`, `def`, `type`) регистрируются `(span, used=false)`.
2. `Variable`/корень `FieldAccess` помечает ближайшее объявление `used=true` (снизу вверх — учитывает shadowing).
3. `pop_scope`: неиспользованные → `W0501 unused variable` / `W0502 unused import` / `W0503 unused function/type` (в манифесте — `unused_config_version`).
4. `E0504 use of undefined variable` — всегда ошибка.
5. Статические проверки D7 (`shadow` обязателен/бесполезен) и D1 (`W0512` — эффектный вызов в импортируемом модуле).

В `--strict` все `W05xx` → ошибки, exit code ≠ 0.

### 6.2. Инварианты `--strict` (рантайм)

- `new Schema ... end`: отсутствующее поле → `E0511`; несоответствие типа (`Int` vs `Str`, `Int` vs `Float` — тоже несоответствие) → `E0512` — всегда ошибки. Лишнее поле → `E0513` в `--strict`, иначе предупреждение (поле сохраняется).

### 6.3. Инварианты `--dry-run`

- `resolver` оборачивается в `RecordingResolver`: чтения выполняются, но логируются в отчёт.
- Сетевые источники (будущие) подменяются снапшот-кэшем; промах → `E0521 network access forbidden in dry-run`.
- Запись на диск не выполняется: `[dry-run] would write N bytes to <path>` + JSON в stdout.
- Capability-проверки D1 действуют так же, как в обычном режиме; `assert`/`fail` работают — dry-run обязан находить отказ валидации.
- Инвариант: `--dry-run` не меняет результат вычисления; два прогона дают побайтно идентичный JSON.

---

## 7. Фаза 6 — сериализация и CLI

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

- `Int` → JSON integer без потери точности (D6). `Float` c `NaN`/`Inf` → `E0602`.
- `Schema`/`Function` в глубине дерева → `E0601` с путём ключа; топ-уровневые `def`/`type`/лямбды исключаются из экспорта.
- Режимы: `--format json` (pretty, порядок ключей = порядок объявления) и `--format json-flat` (`a.b.c = v`).

### 7.2. CLI

```text
aura eval <file.aura> [--strict] [--dry-run] [--frozen]
                      [--allow-read=<paths>] [--allow-env[=A,B]] [--allow-imports-io]
                      [--format json|json-flat] [-o out.json]
aura check <file.aura> [--strict]        # lex + parse + analysis
aura fmt --check <file.aura>             # задел
```

Exit codes: 0 — успех; 1 — диагностические ошибки; 2 — I/O/аргументы.

### 7.3. Диагностика (ariadne)

```rust
pub struct Diagnostic {
    pub code: &'static str,        // "E0302"
    pub severity: Severity,
    pub message: String,
    pub primary: (Span, String),
    pub secondary: Vec<(Span, String)>, // «переменная объявлена здесь»
    pub help: Option<String>,           // «add 'shadow' keyword», «use metrics: ... end»
}
```

- `SourceCache` реализует `ariadne::Cache`; `Span` → строка/колонка средствами `ariadne`.
- Ядро возвращает `Vec<Diagnostic>`; рендеринг — только в `aura-cli` (ядро пригодно для WASM/LSP).

---

## 8. План разработки и критерии приёмки фаз

| Фаза | Артефакт | Критерий приёмки |
| --- | --- | --- |
| 1 | `lexer` | Токенизация манифеста v1.2; snapshot-тест; property-тест «конкатенация span'ов = исходник»; тесты D2 (`[a \n -b]` → 2 элемента) и D8 (`E0104` без версии) |
| 2 | `parser` | AST-snapshot; Pratt-приоритеты; `E0201` на инлайн-блоке; `new`/`assert`/`shadow`; негативные тесты с позициями |
| 3 | `eval` | Манифест c `MemoryResolver`; тесты `E0301`/`E0302`/shadow; Int/Float арифметика и переполнение; capability-отказы `E0310`/`E0311`; все builtin-методы |
| 4 | `vfs` | Цикл a→b→a с полной цепочкой; лок-файл: резолв диапазона, `E0402`/`E0403 --frozen`; каждый файл парсится один раз |
| 5 | `analysis` | `unused_config_version` детектируется; `--strict` падает на `E0513`; `--dry-run` — идентичный JSON без записи |
| 6 | `cli`, `serialize` | Golden-тест манифест → `production_deploy.json`; `Int` без `.0`; snapshot-отчёты ariadne |

Тестовый инструментарий: `insta` (snapshots), `proptest` (лексер), интеграционные тесты в `tests/` поверх `Pipeline`.
