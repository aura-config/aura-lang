//! The playground's built-in examples are the most-read Aura code in the project:
//! one of them loads the moment anyone opens the page, before they have installed
//! anything. Nothing was checking them — `docs_snippets` only scans Markdown — so a
//! non-canonical or broken example would have been found by a visitor.
//!
//! Every buffer is checked three ways: it parses, it is exactly what `aura fmt`
//! produces, and the example still does what its title claims. The last one
//! matters most for the two that are *supposed* to fail: an example demonstrating
//! a refusal is worthless the day it quietly stops refusing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aura_lang::eval::EnvCap;
use aura_lang::facade::{eval_source, EvalOptions};
use aura_lang::fmt::format_source;
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// One entry of the playground's `EXAMPLES` object.
struct Example {
    title: String,
    entry: String,
    allow_read: bool,
    env: String,
    files: HashMap<String, String>,
}

/// Extract the examples from `playground/app.js`.
///
/// Reading them out of the source the page actually ships is the point: a copy in
/// the test would be a second definition that drifts. The shape is fixed and
/// narrow — a quoted title, then `entry` / `allowRead` / `env`, then a `files`
/// object whose values are template literals — so a scanner is enough and pulling
/// in a JavaScript parser is not warranted.
fn parse_examples(js: &str) -> Vec<Example> {
    let mut out = Vec::new();
    let mut rest = js;

    // Each example begins at a two-space-indented quoted title followed by `: {`.
    while let Some(start) = rest.find("\n  \"") {
        let after = &rest[start + 4..];
        let Some(quote) = after.find('"') else { break };
        let title = after[..quote].to_string();
        if !after[quote..].starts_with("\": {") {
            rest = &rest[start + 4..];
            continue;
        }
        let body = &after[quote..];

        let field = |name: &str| -> Option<String> {
            let at = body.find(&format!("{name}: "))?;
            let value = &body[at + name.len() + 2..];
            let end = value.find(['\n', ','])?;
            Some(value[..end].trim().trim_matches('"').to_string())
        };

        let entry = field("entry").unwrap_or_default();
        let allow_read = field("allowRead").as_deref() == Some("true");
        let env = field("env").unwrap_or_default();

        // The `files` object: `"name.aura": ` followed by a template literal.
        let mut files = HashMap::new();
        if let Some(files_at) = body.find("files: {") {
            let mut scan = &body[files_at..];
            while let Some(tick) = scan.find("`") {
                // The key is the quoted name just before the backtick.
                let head = &scan[..tick];
                let Some(close) = head.rfind("\": ") else {
                    break;
                };
                let Some(open) = head[..close].rfind('"') else {
                    break;
                };
                let name = head[open + 1..close].to_string();

                // Read to the closing backtick, honouring \` escapes.
                let content_start = tick + 1;
                let bytes = scan.as_bytes();
                let mut i = content_start;
                let mut text = String::new();
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' if i + 1 < bytes.len() => {
                            // Only the escapes the examples actually use.
                            text.push(match bytes[i + 1] {
                                b'`' => '`',
                                b'\\' => '\\',
                                other => other as char,
                            });
                            i += 2;
                        }
                        b'`' => break,
                        _ => {
                            let ch_len = utf8_len(bytes[i]);
                            text.push_str(&scan[i..i + ch_len]);
                            i += ch_len;
                        }
                    }
                }
                files.insert(name, text);
                scan = &scan[i.min(scan.len())..];
                if scan.starts_with('`') {
                    scan = &scan[1..];
                }
                // Stop at the end of this example's `files` object.
                if let Some(brace) = scan.find("\n    },") {
                    if scan[..brace].find('`').is_none() {
                        break;
                    }
                }
            }
        }

        out.push(Example {
            title,
            entry,
            allow_read,
            env,
            files,
        });
        rest = &rest[start + 4..];
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn parse_env(spec: &str) -> HashMap<String, String> {
    spec.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn examples() -> Vec<Example> {
    let js =
        std::fs::read_to_string(repo_root().join("playground/app.js")).expect("playground/app.js");
    let found = parse_examples(&js);
    assert!(
        found.len() >= 5,
        "expected the playground's examples to be found; got {}. If the shape of \
         EXAMPLES changed, this extractor needs updating — silently finding none \
         would make the whole file pass for the wrong reason.",
        found.len()
    );
    found
}

/// Every `.aura` buffer parses. A playground that opens on a syntax error is the
/// worst possible first impression.
#[test]
fn every_buffer_parses() {
    for ex in examples() {
        for (name, text) in &ex.files {
            if !name.ends_with(".aura") {
                continue;
            }
            let toks = Lexer::new(text, 0)
                .tokenize()
                .unwrap_or_else(|d| panic!("{}/{name} does not lex: {}", ex.title, d.message));
            Parser::new(toks).parse_module().unwrap_or_else(|ds| {
                panic!("{}/{name} does not parse: {}", ex.title, ds[0].message)
            });
        }
    }
}

/// And is exactly what `aura fmt` produces — the same standard the repository's
/// own `.aura` files and Markdown snippets are held to. People copy this code.
#[test]
fn every_buffer_is_canonically_formatted() {
    let mut offenders = Vec::new();
    for ex in examples() {
        for (name, text) in &ex.files {
            if !name.ends_with(".aura") {
                continue;
            }
            if let Ok(formatted) = format_source(text) {
                if formatted != *text {
                    offenders.push(format!("{}/{name}", ex.title));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "playground examples are not `aura fmt` output: {offenders:?}"
    );
}

/// The examples still demonstrate what their titles claim. Checked through
/// `eval_source`, which is the same entry point the WebAssembly module uses, with
/// the same `allowRead` and `env` the page ships for each one.
#[test]
fn every_example_still_demonstrates_its_title() {
    for ex in examples() {
        let opts = EvalOptions {
            allow_read: if ex.allow_read {
                vec![".".into()]
            } else {
                Vec::new()
            },
            allow_env: {
                let vars = parse_env(&ex.env);
                if vars.is_empty() {
                    EnvCap::Deny
                } else {
                    EnvCap::Allow(vars.keys().cloned().collect())
                }
            },
            env_overrides: parse_env(&ex.env),
            ..Default::default()
        };
        let result = eval_source(ex.files.clone(), &ex.entry, &opts);

        match ex.title.as_str() {
            "Schemas and enums" => {
                let out = result.unwrap_or_else(|e| panic!("{}: {e:?}", ex.title));
                // env() reached the evaluation and the production branch was taken.
                assert_eq!(
                    out.json["checkout"]["api"]["replicas"], 6,
                    "the example is meant to show APP_ENV=production selecting 6"
                );
                // An omitted optional field still appears, filled from its default.
                assert_eq!(out.json["checkout"]["api"]["port"], 8080);
            }
            "Imports across files" => {
                let out = result.unwrap_or_else(|e| panic!("{}: {e:?}", ex.title));
                // A property crosses the module boundary…
                assert_eq!(out.json["port"], 8080);
                assert_eq!(out.json["api"]["service"], "checkout");
                // …and a `=` binding does not. That contrast is the example.
                assert!(
                    out.json.get("internal").is_none(),
                    "a private `=` binding must not be exported: {}",
                    out.json
                );
            }
            "Capabilities: an import cannot read" => {
                // The whole point: read access IS granted, and the import is still
                // refused. If this ever succeeds, the example proves the opposite
                // of what it says.
                assert!(ex.allow_read, "this example must ship with read granted");
                let reports = result.err().unwrap_or_else(|| {
                    panic!(
                        "{}: the import must be refused, but evaluation succeeded",
                        ex.title
                    )
                });
                assert!(
                    reports.iter().any(|r| r.code == "E0310"),
                    "expected E0310, got {reports:?}"
                );
            }
            "Generating a non-JSON file" => {
                let out = result.unwrap_or_else(|e| panic!("{}: {e:?}", ex.title));
                let conf = out.json["nginx_conf"].as_str().expect("a string");
                // Interpolation ran, and nginx's own braces survived verbatim.
                assert!(conf.contains("listen 443;"), "{conf}");
                assert!(conf.contains("server_name gateway.example.com;"), "{conf}");
                assert!(conf.contains("server {") && conf.contains('}'), "{conf}");
            }
            "An assertion stops the build" => {
                let reports = result.err().unwrap_or_else(|| {
                    panic!(
                        "{}: the assertion must fail, but evaluation succeeded",
                        ex.title
                    )
                });
                assert!(
                    reports.iter().any(|r| r.code == "E0530"),
                    "expected E0530, got {reports:?}"
                );
            }
            other => panic!(
                "unrecognised playground example {other:?}. Add an expectation for it: \
                 an example nobody checks is an example that can quietly break."
            ),
        }
    }
}

/// The landing page carries an Aura snippet too, and it is the first code a
/// visitor sees. Held to the same standard.
#[test]
fn the_landing_page_snippet_is_valid_and_canonical() {
    let html =
        std::fs::read_to_string(repo_root().join("site-index.html")).expect("site-index.html");
    let open = html.find("<pre>").expect("a <pre> block") + 5;
    let close = html[open..].find("</pre").expect("a closing </pre>") + open;
    let snippet = html[open..close]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_string()
        + "\n";

    let toks = Lexer::new(&snippet, 0)
        .tokenize()
        .unwrap_or_else(|d| panic!("landing snippet does not lex: {}", d.message));
    Parser::new(toks)
        .parse_module()
        .unwrap_or_else(|ds| panic!("landing snippet does not parse: {}", ds[0].message));
    assert_eq!(
        format_source(&snippet).expect("formats"),
        snippet,
        "the landing page snippet is not `aura fmt` output"
    );

    // And it evaluates: a snippet on the front page should not be aspirational.
    let files = HashMap::from([("main.aura".to_string(), snippet)]);
    let out = eval_source(files, "main.aura", &EvalOptions::default())
        .unwrap_or_else(|e| panic!("landing snippet does not evaluate: {e:?}"));
    assert_eq!(out.json["api"]["tier"], "backend");
}
