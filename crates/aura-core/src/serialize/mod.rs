//! Value -> serde_json serialization (SPEC §7.1).
//! Int -> JSON integer with no precision loss (D6); a Schema/Function deep in the tree is E0601 with a key path.

use crate::error::Diagnostic;
use crate::eval::value::Value;
use crate::span::Span;

fn err(code: &'static str, msg: String) -> Diagnostic {
    Diagnostic::error(code, msg, Span::new(0, 0, 0), "during serialization")
}

pub fn to_json(v: &Value<'_>) -> Result<serde_json::Value, Diagnostic> {
    let mut path: Vec<String> = Vec::new();
    go(v, &mut path)
}

fn go(v: &Value<'_>, path: &mut Vec<String>) -> Result<serde_json::Value, Diagnostic> {
    use serde_json::Value as J;
    Ok(match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(n) => J::Number((*n).into()),
        Value::Float(n) => J::Number(
            serde_json::Number::from_f64(*n)
                .ok_or_else(|| err("E0602", format!("non-finite float at '{}'", path.join("."))))?,
        ),
        Value::Str(s) => J::String(s.to_string()),
        Value::List(xs) => {
            let mut out = Vec::with_capacity(xs.len());
            for (i, item) in xs.iter().enumerate() {
                path.push(format!("[{i}]"));
                out.push(go(item, path)?);
                path.pop();
            }
            J::Array(out)
        }
        Value::Object(m) => {
            // Key order is preserved: IndexMap -> serde_json::Map (preserve_order)
            let mut out = serde_json::Map::with_capacity(m.len());
            for (k, item) in m.iter() {
                // D12: a top-level pub def/type is API for importers,
                // silently excluded from the root JSON; deeper it is still an E0601 error
                if path.is_empty() && matches!(item, Value::Schema(_) | Value::Function(_)) {
                    continue;
                }
                path.push(k.clone());
                out.insert(k.clone(), go(item, path)?);
                path.pop();
            }
            J::Object(out)
        }
        Value::Schema(_) | Value::Function(_) => {
            return Err(err(
                "E0601",
                format!(
                    "{} is not serializable (at '{}')",
                    v.type_name(),
                    path.join(".")
                ),
            ))
        }
    })
}

/// The YAML emitter (used by the `.to_yaml()` method and `--format yaml`).
pub fn to_yaml_string(v: &Value<'_>) -> Result<String, Diagnostic> {
    let json = to_json(v)?;
    serde_yaml::to_string(&json).map_err(|e| err("E0603", format!("cannot emit YAML: {e}")))
}

/// The TOML emitter: requires an object at the top level; TOML has no null.
pub fn to_toml_string(v: &Value<'_>) -> Result<String, Diagnostic> {
    let json = to_json(v)?;
    if !json.is_object() {
        return Err(err(
            "E0603",
            "TOML requires an object at the top level".to_string(),
        ));
    }
    toml::to_string_pretty(&json).map_err(|e| {
        err(
            "E0603",
            format!("cannot emit TOML (note: TOML has no null): {e}"),
        )
    })
}

/// `--format json-flat`: nested objects are flattened into `a.b.c`; lists and scalars are leaves.
pub fn to_json_flat(v: &Value<'_>) -> Result<serde_json::Value, Diagnostic> {
    let json = to_json(v)?;
    let serde_json::Value::Object(map) = json else {
        return Ok(json);
    };
    let mut out = serde_json::Map::new();
    flatten("", &serde_json::Value::Object(map), &mut out);
    Ok(serde_json::Value::Object(out))
}

fn flatten(
    prefix: &str,
    v: &serde_json::Value,
    out: &mut serde_json::Map<String, serde_json::Value>,
) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, inner) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, inner, out);
            }
        }
        leaf => {
            out.insert(prefix.to_string(), leaf.clone());
        }
    }
}
