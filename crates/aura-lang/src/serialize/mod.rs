//! Value -> serde_json serialization (SPEC §7.1).
//! Int -> JSON integer with no precision loss (D6); a Schema/Function/Enum deep in the tree is E0601 with a key path.

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
                if path.is_empty()
                    && matches!(item, Value::Schema(_) | Value::Function(_) | Value::Enum(_))
                {
                    continue;
                }
                path.push(k.clone());
                out.insert(k.clone(), go(item, path)?);
                path.pop();
            }
            J::Object(out)
        }
        Value::Schema(_) | Value::Function(_) | Value::Enum(_) => {
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
///
/// Written here rather than delegated to a YAML crate on purpose: the emitted
/// bytes are part of Aura's output contract (D13 — the same manifest must always
/// produce the same file), and delegating means a dependency upgrade can silently
/// change every generated config. It also keeps the dependency tree to a parser
/// only. The subset emitted is exactly what `to_json` can produce: null, bool,
/// number, string, sequence, mapping.
pub fn to_yaml_string(v: &Value<'_>) -> Result<String, Diagnostic> {
    let json = to_json(v)?;
    let mut out = String::new();
    emit_yaml(&json, 0, &mut out);
    Ok(out)
}

/// Emit `v` at `indent` spaces. Mappings nest by two spaces; a sequence sits at
/// the same indentation as the key that introduced it, which is the conventional
/// (and previously emitted) layout.
fn emit_yaml(v: &serde_json::Value, indent: usize, out: &mut String) {
    use serde_json::Value as J;
    let pad = " ".repeat(indent);
    match v {
        J::Object(map) if map.is_empty() => out.push_str("{}\n"),
        J::Array(xs) if xs.is_empty() => out.push_str("[]\n"),
        J::Object(map) => {
            for (i, (k, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(&pad);
                }
                out.push_str(&yaml_scalar(k));
                out.push(':');
                emit_child(val, indent, out);
            }
        }
        J::Array(xs) => {
            for (i, item) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(&pad);
                }
                out.push_str("- ");
                match item {
                    // A nested collection continues on the dash's own line.
                    J::Object(m) if !m.is_empty() => emit_yaml(item, indent + 2, out),
                    J::Array(a) if !a.is_empty() => emit_yaml(item, indent + 2, out),
                    _ => {
                        out.push_str(&yaml_atom(item));
                        out.push('\n');
                    }
                }
            }
        }
        _ => {
            out.push_str(&yaml_atom(v));
            out.push('\n');
        }
    }
}

/// The value part of `key:` — inline for scalars, on following lines otherwise.
fn emit_child(v: &serde_json::Value, indent: usize, out: &mut String) {
    use serde_json::Value as J;
    match v {
        J::Object(m) if !m.is_empty() => {
            out.push('\n');
            out.push_str(&" ".repeat(indent + 2));
            emit_yaml(v, indent + 2, out);
        }
        // A sequence is not indented past its key.
        J::Array(a) if !a.is_empty() => {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            emit_yaml(v, indent, out);
        }
        _ => {
            out.push(' ');
            out.push_str(&yaml_atom(v));
            out.push('\n');
        }
    }
}

fn yaml_atom(v: &serde_json::Value) -> String {
    use serde_json::Value as J;
    match v {
        J::Null => "null".to_string(),
        J::Bool(b) => b.to_string(),
        J::Number(n) => n.to_string(),
        J::String(s) => yaml_scalar(s),
        J::Object(_) => "{}".to_string(),
        J::Array(_) => "[]".to_string(),
    }
}

/// A string as a YAML scalar: plain when it round-trips unambiguously, quoted
/// when it would otherwise be read back as a number, a bool, null, or as
/// structure. Getting this wrong is how a version string like `1.0` silently
/// becomes a float on the next parse.
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Anything needing escapes goes double-quoted.
    if s.contains(|c: char| c.is_control()) {
        return format!("{:?}", s);
    }
    let first = s.chars().next().unwrap();
    let looks_structural = "-?:,[]{}#&*!|>'\"%@`".contains(first)
        || s.contains(": ")
        || s.contains(" #")
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.contains('\t');
    if looks_structural || reads_back_as_non_string(s) {
        // Single quotes need only doubling of the quote itself.
        return format!("'{}'", s.replace('\'', "''"));
    }
    s.to_string()
}

/// Whether a YAML reader would turn this plain scalar into something other than
/// a string.
fn reads_back_as_non_string(s: &str) -> bool {
    matches!(
        s,
        "null" | "Null" | "NULL" | "~" | "true" | "True" | "TRUE" | "false" | "False" | "FALSE"
    ) || s.parse::<i64>().is_ok()
        || s.parse::<f64>().is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml_of(json: serde_json::Value) -> String {
        let mut out = String::new();
        emit_yaml(&json, 0, &mut out);
        out
    }

    /// The dangerous cases: a string that a YAML reader would take for something
    /// else must come back a string. Getting this wrong turns a version, a ZIP
    /// code or a feature flag into a number or a bool on the next read.
    #[test]
    fn strings_that_look_like_other_types_are_quoted() {
        for s in [
            "3", "-1", "0.5", "1e3", "true", "false", "True", "FALSE", "null", "Null", "~",
        ] {
            let out = yaml_of(serde_json::json!({ "k": s }));
            assert_eq!(out, format!("k: '{s}'\n"), "{s} must be quoted");
        }
        // …while text that cannot be confused stays plain.
        for s in ["2.4.1", "gateway", "apps/v1", "company/img:2.4.1", "a-b_c"] {
            let out = yaml_of(serde_json::json!({ "k": s }));
            assert_eq!(out, format!("k: {s}\n"), "{s} must stay plain");
        }
    }

    #[test]
    fn structural_characters_force_quoting() {
        for s in [
            "- item", "? q", ": v", "a: b", "x #c", "#c", "[a]", "{a}", "&anchor", "*alias",
            "!tag", "|block", ">fold", "'q", "\"q", "%d", "@a", " lead", "trail ", "",
        ] {
            let out = yaml_of(serde_json::json!({ "k": s }));
            assert!(
                out.starts_with("k: '") || out.starts_with("k: \""),
                "{s:?} must be quoted, got {out:?}"
            );
        }
    }

    #[test]
    fn quotes_matter_only_where_yaml_gives_them_meaning() {
        // An apostrophe inside a plain scalar is just a character — quoting is
        // only significant at the start, so this must stay plain.
        assert_eq!(yaml_of(serde_json::json!({ "k": "it's 3" })), "k: it's 3\n");
        // Leading `'` does open a quoted scalar, so it must be quoted, and the
        // quote doubled — the only escape single-quoted YAML has.
        assert_eq!(
            yaml_of(serde_json::json!({ "k": "'quoted'" })),
            "k: '''quoted'''\n"
        );
    }

    #[test]
    fn nesting_and_sequences_match_the_conventional_layout() {
        let out = yaml_of(serde_json::json!({
            "spec": { "containers": [ { "name": "gw", "port": 8080 } ], "replicas": 3 }
        }));
        assert_eq!(
            out, "spec:\n  containers:\n  - name: gw\n    port: 8080\n  replicas: 3\n",
            "{out}"
        );
    }

    #[test]
    fn empty_collections_and_scalars() {
        assert_eq!(yaml_of(serde_json::json!({ "a": {} })), "a: {}\n");
        assert_eq!(yaml_of(serde_json::json!({ "a": [] })), "a: []\n");
        assert_eq!(yaml_of(serde_json::json!({ "a": null })), "a: null\n");
        assert_eq!(yaml_of(serde_json::json!({ "a": 1.5 })), "a: 1.5\n");
    }

    /// The property that matters: whatever we emit must read back as the same
    /// value. This is what a hand-written emitter has to earn.
    #[test]
    fn emitted_yaml_reads_back_identically() {
        let cases = vec![
            serde_json::json!({"n": 3, "s": "3", "b": true, "sb": "true", "z": null, "sz": "null"}),
            serde_json::json!({"list": ["a", "1", 1, true, "true", ""]}),
            serde_json::json!({"deep": {"a": {"b": {"c": "v: w"}}}}),
            serde_json::json!({"odd": ["- x", "#c", " pad ", "it's"]}),
            serde_json::json!({"empty_map": {}, "empty_list": [], "f": 0.25}),
        ];
        for case in cases {
            let text = yaml_of(case.clone());
            let docs = yaml_rust2::YamlLoader::load_from_str(&text)
                .unwrap_or_else(|e| panic!("emitted invalid YAML for {case}: {e}\n{text}"));
            assert_eq!(docs.len(), 1, "{text}");
            let back = yaml_to_json(&docs[0]);
            assert_eq!(back, case, "round-trip changed the value:\n{text}");
        }
    }

    /// Test-only mirror of the evaluator's YAML bridge, so the round-trip above
    /// compares against the same type mapping the language uses.
    fn yaml_to_json(y: &yaml_rust2::Yaml) -> serde_json::Value {
        use serde_json::Value as J;
        use yaml_rust2::Yaml as Y;
        match y {
            Y::Null | Y::BadValue | Y::Alias(_) => J::Null,
            Y::Boolean(b) => J::Bool(*b),
            Y::Integer(n) => J::Number((*n).into()),
            Y::Real(r) => r
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| J::String(r.clone()), J::Number),
            Y::String(s) => J::String(s.clone()),
            Y::Array(xs) => J::Array(xs.iter().map(yaml_to_json).collect()),
            Y::Hash(h) => J::Object(
                h.iter()
                    .map(|(k, v)| {
                        let key = match k {
                            Y::String(s) => s.clone(),
                            other => format!("{other:?}"),
                        };
                        (key, yaml_to_json(v))
                    })
                    .collect(),
            ),
        }
    }
}
