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

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(msg: &str);
}

/// Runs when the module is instantiated. A panic in wasm aborts with a bare
/// `unreachable`, which tells the page nothing; this makes the reason visible in
/// the console instead of leaving a blank output pane behind.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        console_error(&format!("aura panicked: {info}"));
    }));
}

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

/// Canonical formatting, the same `aura fmt` applies.
///
/// Returns the formatted source, or the input unchanged if it does not lex —
/// a half-typed buffer should not be destroyed by pressing the button.
#[wasm_bindgen]
pub fn format(src: &str) -> String {
    aura_lang::fmt::format_source(src).unwrap_or_else(|_| src.to_string())
}

/// Syntax highlighting spans for `src`, as JSON `[{"s":start,"e":end,"k":kind}]`
/// over byte offsets.
///
/// The spans come from Aura's own lexer, so the playground's colours cannot drift
/// from the language the way a hand-maintained editor grammar does. Comments are
/// recovered rather than lexed: the lexer skips exactly whitespace and comments,
/// so any non-blank gap between two tokens is a comment, and that holds even for
/// a `#` inside a string, which is part of a token and never a gap.
///
/// Input that does not lex yields an empty list — the editor then shows plain
/// text, which is the right thing while a string is still unclosed.
#[wasm_bindgen]
pub fn highlight(src: &str) -> String {
    use aura_lang::lexer::token::TokenKind as T;

    let Ok(tokens) = aura_lang::lexer::Lexer::new(src, 0).tokenize() else {
        return "[]".to_string();
    };

    let mut spans: Vec<serde_json::Value> = Vec::new();
    let mut push = |s: u32, e: u32, k: &str| {
        if e > s {
            spans.push(serde_json::json!({ "s": s, "e": e, "k": k }));
        }
    };

    let mut cursor = 0u32;
    for t in &tokens {
        // The gap before this token: whitespace, or a comment.
        comments_in(src, cursor, t.span.start, &mut push);
        cursor = cursor.max(t.span.end);

        let kind = match t.kind {
            T::Import
            | T::As
            | T::Type
            | T::Enum
            | T::Def
            | T::End
            | T::Domain
            | T::New
            | T::Assert
            | T::Shadow
            | T::Pub
            | T::Cond
            | T::Else => "kw",
            T::True | T::False | T::Null => "lit",
            T::Str(_) | T::InterpStr(_) | T::ImportPath { .. } => "str",
            T::Int(_) | T::Float(_) => "num",
            T::Ident(_) => "id",
            T::Newline | T::Eof => continue,
            _ => "op",
        };
        push(t.span.start, t.span.end, kind);
    }
    // Trailing comment after the last token.
    comments_in(src, cursor, src.len() as u32, &mut push);

    spans.sort_by_key(|v| v["s"].as_u64().unwrap_or(0));
    serde_json::Value::Array(spans).to_string()
}

/// Mark every `#…end-of-line` run inside `src[from..to]`, a region the lexer
/// skipped.
fn comments_in(src: &str, from: u32, to: u32, push: &mut impl FnMut(u32, u32, &str)) {
    let (from, to) = (from as usize, (to as usize).min(src.len()));
    if from >= to {
        return;
    }
    let gap = &src[from..to];
    let mut i = 0;
    while let Some(hash) = gap[i..].find('#') {
        let start = i + hash;
        let end = gap[start..].find('\n').map_or(gap.len(), |n| start + n);
        push((from + start) as u32, (from + end) as u32, "cmt");
        i = end;
        if i >= gap.len() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(src: &str) -> Vec<(usize, usize, String)> {
        let v: serde_json::Value = serde_json::from_str(&highlight(src)).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x["s"].as_u64().unwrap() as usize,
                    x["e"].as_u64().unwrap() as usize,
                    x["k"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    fn texts(src: &str, kind: &str) -> Vec<String> {
        spans(src)
            .into_iter()
            .filter(|(_, _, k)| k == kind)
            .map(|(s, e, _)| src[s..e].to_string())
            .collect()
    }

    #[test]
    fn keywords_strings_numbers_and_identifiers() {
        let src = "def make(n)\n  port: 8080\n  name: \"api\"\nend\n";
        assert_eq!(texts(src, "kw"), vec!["def", "end"]);
        assert_eq!(texts(src, "num"), vec!["8080"]);
        assert_eq!(texts(src, "str"), vec!["\"api\""]);
        assert!(texts(src, "id").contains(&"make".to_string()));
    }

    /// The part that is easy to get wrong: a `#` inside a string is not a
    /// comment, because it belongs to a token rather than to a gap between them.
    #[test]
    fn hashes_inside_strings_are_not_comments() {
        let src = "a: \"colour #fff\"\nb: \"x#{y}z\"\n# a real comment\nc: 1 # trailing\n";
        let comments = texts(src, "cmt");
        assert_eq!(
            comments,
            vec!["# a real comment", "# trailing"],
            "got {comments:?}"
        );
    }

    #[test]
    fn a_comment_before_any_token_and_after_the_last_one() {
        let src = "# leading\nx: 1\n# trailing at eof";
        assert_eq!(texts(src, "cmt"), vec!["# leading", "# trailing at eof"]);
    }

    #[test]
    fn spans_are_ordered_non_overlapping_and_on_char_boundaries() {
        let src = "# ключевое слово\ndef f()\n  s: \"строка\"\nend\n";
        let sp = spans(src);
        let mut last_end = 0;
        for (s, e, k) in &sp {
            assert!(*s >= last_end, "overlap at {s}..{e} ({k}) in {sp:?}");
            assert!(
                src.is_char_boundary(*s) && src.is_char_boundary(*e),
                "{s}..{e}"
            );
            last_end = *e;
        }
    }

    #[test]
    fn input_that_does_not_lex_yields_no_spans() {
        // A half-typed string: plain text is the right fallback.
        assert_eq!(highlight("x: \"unterminated"), "[]");
    }
}
