//! Рантайм-значения (SPEC §4.1). Иммутабельные контейнеры в Arc: клонирование — O(1).
//! Value параметризован временем жизни исходника: имена и AST-тела функций — zero-copy.

use indexmap::IndexMap;
use std::fmt;
use std::sync::Arc;

use super::env::Env;
use crate::parser::ast::{LambdaBody, ObjectBody, TypeName};

#[derive(Debug, Clone)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    List(Arc<Vec<Value<'a>>>),
    /// IndexMap: порядок ключей = порядок объявления (детерминированный JSON).
    Object(Arc<IndexMap<String, Value<'a>>>),
    Schema(Arc<SchemaDef<'a>>),
    Function(Arc<FunctionDef<'a>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDef<'a> {
    pub name: &'a str,
    pub fields: Vec<(&'a str, TypeName<'a>)>,
}

pub struct FunctionDef<'a> {
    pub params: Vec<&'a str>,
    pub body: FuncBody<'a>,
    /// Лексическое замыкание (SPEC §4.2).
    pub closure: Env<'a>,
}

#[derive(Clone)]
pub enum FuncBody<'a> {
    Lambda(LambdaBody<'a>),
    /// Тело `def` — объектный литерал.
    Object(ObjectBody<'a>),
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
            TypeTag::Function => "Function",
        }
    }

    pub fn object(map: IndexMap<String, Value<'a>>) -> Self {
        Value::Object(Arc::new(map))
    }

    pub fn list(items: Vec<Value<'a>>) -> Self {
        Value::List(Arc::new(items))
    }

    pub fn str(s: impl AsRef<str>) -> Self {
        Value::Str(Arc::from(s.as_ref()))
    }
}

/// Структурное равенство; Int == Float — по математическому значению;
/// Function/Schema — по идентичности Arc (SPEC §4.1).
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
