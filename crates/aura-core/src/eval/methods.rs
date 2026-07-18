//! Реестр методов (SPEC §4.4). Диспетчеризация по (TypeTag получателя, имя метода).
//! Регистрация через fn-указатели: новый метод = функция + `register`, парсер не меняется.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

use super::value::{TypeTag, Value};
use super::Interpreter;
use crate::error::Diagnostic;
use crate::span::Span;

pub type MethodFn<'a> =
    fn(&mut Interpreter<'a>, &Value<'a>, &[Value<'a>], Span) -> Result<Value<'a>, Diagnostic>;

pub struct MethodRegistry<'a> {
    table: HashMap<(TypeTag, &'static str), MethodFn<'a>>,
}

impl<'a> MethodRegistry<'a> {
    pub fn new() -> Self {
        MethodRegistry {
            table: HashMap::new(),
        }
    }

    pub fn register(&mut self, tag: TypeTag, name: &'static str, f: MethodFn<'a>) {
        self.table.insert((tag, name), f);
    }

    pub fn get(&self, tag: TypeTag, name: &str) -> Option<MethodFn<'a>> {
        self.table.get(&(tag, name)).copied()
    }

    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(TypeTag::Str, "upper", m_str_upper);
        r.register(TypeTag::Str, "lower", m_str_lower);
        r.register(TypeTag::Str, "len", m_len);
        r.register(TypeTag::Str, "parse_toml", m_parse_toml);
        r.register(TypeTag::Str, "parse_json", m_parse_json);
        r.register(TypeTag::Str, "parse_yaml", m_parse_yaml);
        r.register(TypeTag::List, "len", m_len);
        r.register(TypeTag::List, "compact", m_list_compact);
        r.register(TypeTag::List, "uniq", m_list_uniq);
        r.register(TypeTag::List, "map", m_list_map);
        r.register(TypeTag::List, "filter", m_list_filter);
        r.register(TypeTag::List, "get", m_get);
        r.register(TypeTag::List, "first", m_list_first);
        r.register(TypeTag::List, "last", m_list_last);
        r.register(TypeTag::Object, "len", m_len);
        r.register(TypeTag::Object, "merge", m_obj_merge);
        r.register(TypeTag::Object, "get", m_get);
        for tag in [TypeTag::Object, TypeTag::List] {
            r.register(tag, "to_json", m_to_json);
            r.register(tag, "to_yaml", m_to_yaml);
            r.register(tag, "to_toml", m_to_toml);
        }
        r
    }
}

impl Default for MethodRegistry<'_> {
    fn default() -> Self {
        Self::builtin()
    }
}

fn rt(code: &'static str, msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(code, msg, span, "in this method call")
}

fn m_str_upper<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    Ok(Value::str(s.to_uppercase()))
}

fn m_str_lower<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    Ok(Value::str(s.to_lowercase()))
}

fn m_len<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let n = match recv {
        Value::Str(s) => s.chars().count(),
        Value::List(xs) => xs.len(),
        Value::Object(m) => m.len(),
        _ => unreachable!(),
    };
    Ok(Value::Int(n as i64))
}

/// `.compact()` — удаляет Null с сохранением порядка (SPEC §4.4).
fn m_list_compact<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    Ok(Value::list(
        xs.iter()
            .filter(|v| !matches!(v, Value::Null))
            .cloned()
            .collect(),
    ))
}

/// `.uniq()` — дедупликация с сохранением первого вхождения.
fn m_list_uniq<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    let mut out: Vec<Value<'a>> = Vec::with_capacity(xs.len());
    for v in xs.iter() {
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    Ok(Value::list(out))
}

fn expect_lambda<'a, 'b>(
    args: &'b [Value<'a>],
    name: &str,
    sp: Span,
) -> Result<&'b Value<'a>, Diagnostic> {
    match args.last() {
        Some(f @ Value::Function(_)) => Ok(f),
        _ => Err(rt(
            "E0315",
            format!("`{name}` requires a lambda argument"),
            sp,
        )),
    }
}

/// `.map (elem, index) -> ... end` — колбэк получает элемент и индекс.
fn m_list_map<'a>(
    it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    let f = expect_lambda(args, "map", sp)?.clone();
    let mut out = Vec::with_capacity(xs.len());
    for (i, v) in xs.iter().enumerate() {
        out.push(it.call_value(&f, &[v.clone(), Value::Int(i as i64)], sp)?);
    }
    Ok(Value::list(out))
}

fn m_list_filter<'a>(
    it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    let f = expect_lambda(args, "filter", sp)?.clone();
    let mut out = Vec::new();
    for (i, v) in xs.iter().enumerate() {
        match it.call_value(&f, &[v.clone(), Value::Int(i as i64)], sp)? {
            Value::Bool(true) => out.push(v.clone()),
            Value::Bool(false) => {}
            other => {
                return Err(rt(
                    "E0306",
                    format!("filter lambda must return Bool, got {}", other.type_name()),
                    sp,
                ))
            }
        }
    }
    Ok(Value::list(out))
}

/// `.merge(other)` — правый операнд перекрывает ключи левого.
fn m_obj_merge<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Object(base) = recv else {
        unreachable!()
    };
    let Some(Value::Object(other)) = args.first() else {
        return Err(rt("E0306", "merge expects an Object argument", sp));
    };
    let mut out: IndexMap<String, Value<'a>> = (**base).clone();
    for (k, v) in other.iter() {
        out.insert(k.clone(), v.clone());
    }
    Ok(Value::object(out))
}

/// `.parse_toml()` — целые TOML → Int (D6).
fn m_parse_toml<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    let parsed: toml::Value =
        toml::from_str(s).map_err(|e| rt("E0314", format!("invalid TOML: {e}"), sp))?;
    Ok(toml_to_value(parsed))
}

/// `.get(index_or_key, default)` — безопасный доступ: промах отдаёт default (или Null).
fn m_get<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let default = || args.get(1).cloned().unwrap_or(Value::Null);
    match (recv, args.first()) {
        (Value::List(xs), Some(Value::Int(i))) => Ok(usize::try_from(*i)
            .ok()
            .and_then(|i| xs.get(i))
            .cloned()
            .unwrap_or_else(default)),
        (Value::Object(m), Some(Value::Str(k))) => {
            Ok(m.get(k.as_ref()).cloned().unwrap_or_else(default))
        }
        (Value::List(_), _) => Err(rt("E0306", "List.get expects an Int index", sp)),
        _ => Err(rt("E0306", "Object.get expects a String key", sp)),
    }
}

fn m_list_first<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    xs.first()
        .cloned()
        .ok_or_else(|| rt("E0317", "first() on an empty list", sp))
}

fn m_list_last<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    xs.last()
        .cloned()
        .ok_or_else(|| rt("E0317", "last() on an empty list", sp))
}

fn m_parse_json<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    let parsed: serde_json::Value =
        serde_json::from_str(s).map_err(|e| rt("E0314", format!("invalid JSON: {e}"), sp))?;
    Ok(json_to_value(parsed))
}

fn m_parse_yaml<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    // Через serde_json::Value: даёт единый маппинг типов и preserve_order
    let parsed: serde_json::Value =
        serde_yaml::from_str(s).map_err(|e| rt("E0314", format!("invalid YAML: {e}"), sp))?;
    Ok(json_to_value(parsed))
}

fn m_to_json<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let json = crate::serialize::to_json(recv).map_err(|d| rt(d.code, d.message, sp))?;
    Ok(Value::str(
        serde_json::to_string(&json).expect("valid json"),
    ))
}

fn m_to_yaml<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    crate::serialize::to_yaml_string(recv)
        .map(Value::str)
        .map_err(|d| rt(d.code, d.message, sp))
}

fn m_to_toml<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    crate::serialize::to_toml_string(recv)
        .map(Value::str)
        .map_err(|d| rt(d.code, d.message, sp))
}

fn json_to_value<'a>(j: serde_json::Value) -> Value<'a> {
    match j {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => Value::str(s),
        serde_json::Value::Array(xs) => Value::list(xs.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(m) => Value::Object(Arc::new(
            m.into_iter().map(|(k, v)| (k, json_to_value(v))).collect(),
        )),
    }
}

fn toml_to_value<'a>(t: toml::Value) -> Value<'a> {
    match t {
        toml::Value::String(s) => Value::str(s),
        toml::Value::Integer(n) => Value::Int(n),
        toml::Value::Float(n) => Value::Float(n),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::str(d.to_string()),
        toml::Value::Array(xs) => Value::list(xs.into_iter().map(toml_to_value).collect()),
        toml::Value::Table(t) => Value::Object(Arc::new(
            t.into_iter().map(|(k, v)| (k, toml_to_value(v))).collect(),
        )),
    }
}
