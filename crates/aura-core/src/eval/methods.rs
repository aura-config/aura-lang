//! Method registry (SPEC §4.4). Dispatch by (receiver TypeTag, method name).
//! Registration via fn pointers: a new method = a function + `register`, the parser stays unchanged.

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
        r.register(TypeTag::Str, "parse_duration", m_parse_duration);
        r.register(TypeTag::Str, "parse_datetime", m_parse_datetime);
        r.register(TypeTag::Int, "format_duration", m_format_duration);
        r.register(TypeTag::Int, "format_datetime", m_format_datetime);
        r.register(TypeTag::Str, "parse_json", m_parse_json);
        r.register(TypeTag::Str, "parse_yaml", m_parse_yaml);
        r.register(TypeTag::List, "len", m_len);
        r.register(TypeTag::List, "compact", m_list_compact);
        r.register(TypeTag::List, "uniq", m_list_uniq);
        r.register(TypeTag::List, "map", m_list_map);
        r.register(TypeTag::List, "filter", m_list_filter);
        r.register(TypeTag::List, "get", m_get);
        r.register(TypeTag::List, "contains", m_contains);
        r.register(TypeTag::List, "join", m_list_join);
        r.register(TypeTag::Str, "contains", m_contains);
        r.register(TypeTag::Object, "keys", m_obj_keys);
        r.register(TypeTag::Object, "values", m_obj_values);
        r.register(TypeTag::Object, "contains", m_contains);
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

/// `.compact()` — removes Null while preserving order (SPEC §4.4).
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

/// `.uniq()` — deduplication keeping the first occurrence.
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

/// `.map (elem, index) -> ... end` — the callback receives the element and its index.
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

/// `.merge(other)` — the right-hand operand overrides the left-hand keys.
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

/// `.parse_toml()` — TOML integers → Int (D6).
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

/// `.get(index_or_key, default)` — safe access: a miss returns default (or Null).
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
    // Via serde_json::Value: gives a unified type mapping and preserve_order
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

/// `.keys()` — object keys in declaration order.
fn m_obj_keys<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Object(m) = recv else {
        unreachable!()
    };
    Ok(Value::list(m.keys().map(Value::str).collect()))
}

fn m_obj_values<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Object(m) = recv else {
        unreachable!()
    };
    Ok(Value::list(m.values().cloned().collect()))
}

/// `.contains(x)`: List — element; Object — key; Str — substring.
fn m_contains<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Some(needle) = args.first() else {
        return Err(rt("E0306", "contains() expects an argument", sp));
    };
    let found = match (recv, needle) {
        (Value::List(xs), n) => xs.contains(n),
        (Value::Object(m), Value::Str(k)) => m.contains_key(k.as_ref()),
        (Value::Str(s), Value::Str(sub)) => s.contains(sub.as_ref()),
        (Value::Object(_), n) => {
            return Err(rt(
                "E0306",
                format!(
                    "Object.contains expects a String key, got {}",
                    n.type_name()
                ),
                sp,
            ))
        }
        (Value::Str(_), n) => {
            return Err(rt(
                "E0306",
                format!("String.contains expects a String, got {}", n.type_name()),
                sp,
            ))
        }
        _ => unreachable!(),
    };
    Ok(Value::Bool(found))
}

/// `.join(sep)` — scalar elements joined by a separator (empty string if no argument).
fn m_list_join<'a>(
    it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::List(xs) = recv else {
        unreachable!()
    };
    let sep = match args.first() {
        Some(Value::Str(s)) => s.to_string(),
        None => String::new(),
        Some(other) => {
            return Err(rt(
                "E0306",
                format!(
                    "join() expects a String separator, got {}",
                    other.type_name()
                ),
                sp,
            ))
        }
    };
    let parts: Vec<String> = xs
        .iter()
        .map(|v| it.display(v, sp))
        .collect::<Result<_, _>>()?;
    Ok(Value::str(parts.join(&sep)))
}

/// `"1h30m".parse_duration()` → seconds (Int). Units: d, h, m, s.
/// A deterministic alternative to timeout "magic numbers".
fn m_parse_duration<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    let err = || {
        rt(
            "E0319",
            format!("invalid duration '{s}': expected e.g. \"1h30m\", units d/h/m/s"),
            sp,
        )
    };
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut total: i64 = 0;
    let mut components = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i == bytes.len() {
            return Err(err());
        }
        let n: i64 = s[start..i].parse().map_err(|_| err())?;
        let mult: i64 = match bytes[i] {
            b'd' => 86400,
            b'h' => 3600,
            b'm' => 60,
            b's' => 1,
            _ => return Err(err()),
        };
        i += 1;
        total = n
            .checked_mul(mult)
            .and_then(|x| total.checked_add(x))
            .ok_or_else(|| rt("E0304", "duration overflows i64 seconds", sp))?;
        components += 1;
    }
    if components == 0 {
        return Err(err());
    }
    Ok(Value::Int(total))
}

/// `5400.format_duration()` → "1h30m" (compact, no zero components).
fn m_format_duration<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Int(total) = recv else {
        unreachable!()
    };
    if *total < 0 {
        return Err(rt(
            "E0306",
            "format_duration expects a non-negative number of seconds",
            sp,
        ));
    }
    if *total == 0 {
        return Ok(Value::str("0s"));
    }
    let (mut rest, mut out) = (*total, String::new());
    for (unit, secs) in [("d", 86400), ("h", 3600), ("m", 60), ("s", 1)] {
        let n = rest / secs;
        if n > 0 {
            out.push_str(&format!("{n}{unit}"));
            rest %= secs;
        }
    }
    Ok(Value::str(out))
}

/// `"2026-07-18T12:00:00Z".parse_datetime()` → epoch seconds (Int, UTC).
/// Formats: `YYYY-MM-DD` (midnight UTC) and RFC3339 with `Z` or a `±HH:MM` offset.
fn m_parse_datetime<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Str(s) = recv else { unreachable!() };
    parse_rfc3339(s).map(Value::Int).ok_or_else(|| {
        rt(
            "E0320",
            format!("invalid datetime '{s}': expected RFC3339, e.g. \"2026-07-18T12:00:00Z\""),
            sp,
        )
    })
}

/// `epoch.format_datetime()` → an RFC3339 string in UTC.
fn m_format_datetime<'a>(
    _it: &mut Interpreter<'a>,
    recv: &Value<'a>,
    _args: &[Value<'a>],
    _sp: Span,
) -> Result<Value<'a>, Diagnostic> {
    let Value::Int(epoch) = recv else {
        unreachable!()
    };
    let days = epoch.div_euclid(86400);
    let secs = epoch.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    Ok(Value::str(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )))
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    let num = |t: &str| -> Option<i64> {
        if t.bytes().all(|b| b.is_ascii_digit()) && !t.is_empty() {
            t.parse().ok()
        } else {
            None
        }
    };
    let (date, rest) = if s.len() > 10 {
        s.split_at(10)
    } else {
        (s, "")
    };
    let mut dp = date.split('-');
    let (y, m, d) = (num(dp.next()?)?, num(dp.next()?)?, num(dp.next()?)?);
    if dp.next().is_some() || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    let mut epoch = days_from_civil(y, m, d) * 86400;
    if rest.is_empty() {
        return Some(epoch);
    }
    let rest = rest.strip_prefix('T')?;
    if rest.len() < 9 {
        return None;
    }
    let (time, zone) = rest.split_at(8);
    let mut tp = time.split(':');
    let (hh, mm, ss) = (num(tp.next()?)?, num(tp.next()?)?, num(tp.next()?)?);
    if tp.next().is_some() || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    epoch += hh * 3600 + mm * 60 + ss;
    match zone {
        "Z" => Some(epoch),
        _ => {
            let sign = match zone.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (oh, om) = zone[1..].split_once(':')?;
            let (oh, om) = (num(oh)?, num(om)?);
            if oh > 23 || om > 59 {
                return None;
            }
            Some(epoch - sign * (oh * 3600 + om * 60))
        }
    }
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
    }
}

/// Howard Hinnant's algorithms: proleptic Gregorian calendar ↔ days since the epoch.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
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
