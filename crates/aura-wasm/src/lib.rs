//! WebAssembly bindings for Aura.
//!
//! One entry point: hand it the playground's buffers, get back JSON or a list of
//! diagnostics. Everything is evaluated in memory (`facade::eval_source`), so the
//! browser needs no filesystem — and the capability model is the same one the CLI
//! enforces, including the part where an imported buffer gets no file access.

use std::collections::HashMap;

use aura_lang::eval::EnvCap;
use aura_lang::facade::{eval_source, EvalOptions};
use wasm_bindgen::prelude::*;

/// Evaluate `files` starting at `entry`.
///
/// `files` and `env` are plain JSON objects of string to string. `format` is one
/// of `json`, `yaml`, `toml`. Returns a JSON object: either
/// `{"ok": true, "output": "...", "warnings": [...]}` or
/// `{"ok": false, "diagnostics": [...]}`, so the page never has to parse an error
/// string — every diagnostic carries its code, file, line and column, which is
/// what puts a marker in the right editor tab.
#[wasm_bindgen]
pub fn evaluate(
    files: JsValue,
    entry: &str,
    format: &str,
    allow_read: bool,
    env: JsValue,
) -> String {
    let files: HashMap<String, String> = match serde_wasm_from(files) {
        Ok(f) => f,
        Err(e) => return fail(&format!("cannot read files: {e}")),
    };
    let env_vars: HashMap<String, String> = serde_wasm_from(env).unwrap_or_default();

    let opts = EvalOptions {
        // In the browser the "filesystem" is the buffer set itself; the grant is
        // still explicit, so a demo can show the denial as well as the success.
        allow_read: if allow_read {
            vec![".".into()]
        } else {
            Vec::new()
        },
        allow_env: if env_vars.is_empty() {
            EnvCap::Deny
        } else {
            EnvCap::Allow(env_vars.keys().cloned().collect())
        },
        // A browser has no process environment, so `env()` reads these instead.
        env_overrides: env_vars,
        ..Default::default()
    };

    match eval_source(files, entry, &opts) {
        Ok(out) => {
            let text = match format {
                "yaml" => Ok(aura_lang::serialize::json_to_yaml_string(&out.json)),
                "toml" => aura_lang::serialize::json_to_toml_string(&out.json),
                _ => Ok(serde_json::to_string_pretty(&out.json).unwrap_or_default()),
            };
            match text {
                Ok(text) => serde_json::json!({
                    "ok": true,
                    "output": text,
                    "warnings": out.warnings.iter().map(report_json).collect::<Vec<_>>(),
                })
                .to_string(),
                Err(d) => fail(&d.message),
            }
        }
        Err(reports) => serde_json::json!({
            "ok": false,
            "diagnostics": reports.iter().map(report_json).collect::<Vec<_>>(),
        })
        .to_string(),
    }
}

fn report_json(r: &aura_lang::facade::Report) -> serde_json::Value {
    serde_json::json!({
        "code": r.code,
        "severity": match r.severity { aura_lang::error::Severity::Error => "error", _ => "warning" },
        "message": r.message,
        "file": r.file,
        "line": r.line,
        "column": r.column,
        "help": r.help,
    })
}

fn fail(message: &str) -> String {
    serde_json::json!({
        "ok": false,
        "diagnostics": [{ "code": "E0000", "severity": "error", "message": message,
                          "file": "", "line": 0, "column": 0, "help": null }],
    })
    .to_string()
}

fn serde_wasm_from(v: JsValue) -> Result<HashMap<String, String>, String> {
    let s = js_sys::JSON::stringify(&v)
        .map_err(|_| "not JSON-serializable".to_string())?
        .as_string()
        .ok_or_else(|| "not a string".to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}
