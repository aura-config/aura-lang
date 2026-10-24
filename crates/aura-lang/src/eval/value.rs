//! Runtime values (SPEC §4.1). Immutable containers in Arc: cloning is O(1).
//! Value is parameterized by the source's lifetime: names and function AST bodies are zero-copy.

use indexmap::IndexMap;
use std::fmt;
use std::sync::Arc;

use super::env::WeakEnv;
use crate::parser::ast::{LambdaBody, SchemaField, Stmt};

#[derive(Debug, Clone)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    List(Arc<Items<'a>>),
    /// IndexMap: key order = declaration order (deterministic JSON).
    Object(Arc<Fields<'a>>),
    Schema(Arc<SchemaDef<'a>>),
    /// D18: a declared enum — a closed set of allowed string values.
    Enum(Arc<EnumDef<'a>>),
    Function(Arc<FunctionDef<'a>>),
}

/// D43: how deeply values may nest. Every walk over a value - freeing it,
/// writing it out, comparing it, checking it against a schema - recurses once
/// per level, so a value deeper than the stack is a crash wherever it is next
/// touched. A loop can build one (`reduce` wrapping its accumulator) without any
/// expression nesting at all, which D42 cannot see. So the depth is bounded
/// where a container is made, once, and every walk inherits the bound. The
/// same number as D42's limits: one rule for how deep anything goes.
pub const MAX_VALUE_DEPTH: u32 = 256;

/// A list's elements and how deep it nests (D43). Reads as the `Vec`.
#[derive(Debug, Clone, Default)]
pub struct Items<'a> {
    depth: u32,
    items: Vec<Value<'a>>,
}

/// An object's fields and how deep it nests (D43). Reads as the `IndexMap`.
#[derive(Debug, Clone, Default)]
pub struct Fields<'a> {
    depth: u32,
    map: IndexMap<String, Value<'a>>,
}

impl<'a> std::ops::Deref for Items<'a> {
    type Target = Vec<Value<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl<'a> std::ops::Deref for Fields<'a> {
    type Target = IndexMap<String, Value<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl PartialEq for Items<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl PartialEq for Fields<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
    }
}

impl<'a> Items<'a> {
    pub fn into_vec(self) -> Vec<Value<'a>> {
        self.items
    }
}

impl<'a> Fields<'a> {
    pub fn into_map(self) -> IndexMap<String, Value<'a>> {
        self.map
    }

    /// A copy of the fields, to build a new object from.
    pub fn to_map(&self) -> IndexMap<String, Value<'a>> {
        self.map.clone()
    }
}

/// The depth of a container whose deepest child is `deepest` levels deep. Past
/// `MAX_VALUE_DEPTH` the evaluation is told to stop (D43).
#[inline]
fn bounded(deepest: u32) -> u32 {
    let depth = deepest + 1;
    if depth > MAX_VALUE_DEPTH {
        super::budget::value_too_deep();
    }
    depth.min(MAX_VALUE_DEPTH + 1)
}

/// D18. Members keep declaration order so diagnostics list them predictably.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef<'a> {
    pub name: &'a str,
    pub members: Vec<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDef<'a> {
    pub name: &'a str,
    pub fields: Vec<SchemaField<'a>>,
    /// D18: for each field typed by an enum, the enum resolved **at declaration
    /// time** — so an imported schema validates against the enum that was in
    /// scope in its own module, not one that happens to exist at the call site.
    pub field_enums: std::collections::HashMap<&'a str, Arc<EnumDef<'a>>>,
    /// D28: rules the schema states about its own values, carried with it so an
    /// imported schema validates by the same rules in every manifest that uses
    /// it — which is the point of moving them off the construction site.
    pub invariants: Vec<crate::parser::ast::SchemaInvariant<'a>>,
    /// D41: for each field typed by a schema - itself, or as the element of a
    /// list at any depth - that schema, resolved at declaration time like
    /// `field_enums`, so an imported schema checks its fields by the schemas of
    /// its own module. `None` is the schema itself (`children: [Node]`), which
    /// does not exist yet while it is being declared.
    pub field_schemas: std::collections::HashMap<&'a str, Option<Arc<SchemaDef<'a>>>>,
}

pub struct FunctionDef<'a> {
    pub params: Vec<&'a str>,
    pub body: FuncBody<'a>,
    /// Lexical closure (SPEC §4.2). A `WeakEnv` to avoid the env<->function
    /// Arc cycle; the interpreter's env arena keeps the target env alive.
    pub closure: WeakEnv<'a>,
    /// D1xD12: the function runs with the capabilities of its origin module,
    /// not the caller's - an exported package function does not gain the root's rights.
    pub defined_in_root: bool,
    /// D37: the folder of the module that defined it. Its relative
    /// `read_file` paths are read from there, whoever calls it.
    pub origin_dir: Option<Arc<std::path::Path>>,
}

#[derive(Clone)]
pub enum FuncBody<'a> {
    Lambda(LambdaBody<'a>),
    /// A `def` body — statements (D17), like a block.
    Block(Vec<Stmt<'a>>),
    /// D35: a function the host provides. It has no body in the language and
    /// closes over nothing; `call_value` hands it to the host.
    Host(Arc<crate::host::HostFn>),
}

impl fmt::Debug for FunctionDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<fn({})>", self.params.join(", "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeTag {
    Null,
    Bool,
    Int,
    Float,
    Str,
    List,
    Object,
    Schema,
    Enum,
    Function,
}

impl<'a> Value<'a> {
    pub fn tag(&self) -> TypeTag {
        match self {
            Value::Null => TypeTag::Null,
            Value::Bool(_) => TypeTag::Bool,
            Value::Int(_) => TypeTag::Int,
            Value::Float(_) => TypeTag::Float,
            Value::Str(_) => TypeTag::Str,
            Value::List(_) => TypeTag::List,
            Value::Object(_) => TypeTag::Object,
            Value::Schema(_) => TypeTag::Schema,
            Value::Enum(_) => TypeTag::Enum,
            Value::Function(_) => TypeTag::Function,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self.tag() {
            TypeTag::Null => "Null",
            TypeTag::Bool => "Bool",
            TypeTag::Int => "Int",
            TypeTag::Float => "Float",
            TypeTag::Str => "String",
            TypeTag::List => "List",
            TypeTag::Object => "Object",
            TypeTag::Schema => "Schema",
            TypeTag::Enum => "Enum",
            TypeTag::Function => "Function",
        }
    }

    pub fn object(map: IndexMap<String, Value<'a>>) -> Self {
        // One pass over the fields for both the bytes they cost and how deep
        // they go (D43).
        let mut key_bytes = 0;
        let mut deepest = 0;
        for (k, v) in &map {
            key_bytes += k.len();
            deepest = deepest.max(v.depth());
        }
        super::budget::charge(map.len() * std::mem::size_of::<(String, Value<'a>)>() + key_bytes);
        let depth = bounded(deepest);
        Value::Object(Arc::new(Fields { depth, map }))
    }

    /// How many containers deep this value is: 0 for a scalar (D43).
    #[inline]
    fn depth(&self) -> u32 {
        match self {
            Value::List(xs) => xs.depth,
            Value::Object(m) => m.depth,
            _ => 0,
        }
    }

    pub fn list(items: Vec<Value<'a>>) -> Self {
        super::budget::charge_list(items.len());
        let depth = bounded(items.iter().map(Value::depth).max().unwrap_or(0));
        Value::List(Arc::new(Items { depth, items }))
    }

    pub fn str(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        super::budget::charge(s.len());
        Value::Str(Arc::from(s))
    }
}

/// Structural equality; Int == Float compares by mathematical value;
/// Function/Schema compare by Arc identity (SPEC §4.1).
impl PartialEq for Value<'_> {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Int(a), Float(b)) | (Float(b), Int(a)) => *a as f64 == *b,
            (Str(a), Str(b)) => a == b,
            (List(a), List(b)) => a == b,
            (Object(a), Object(b)) => a == b,
            (Schema(a), Schema(b)) => Arc::ptr_eq(a, b),
            (Function(a), Function(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
