//! Aura Language Server.
//!
//! A small synchronous LSP server (rust-analyzer's `lsp-server` stack, no async
//! runtime) that publishes live diagnostics for `.aura` buffers. Everything
//! smart lives in `aura-lang`; this binary is only the protocol layer.

mod diagnostics;
mod goto;
mod hover;
mod infer;
mod rename;
mod signature;
mod stdlib;

use std::collections::HashMap;

use lsp_server::{Connection, Message, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, Formatting, GotoDefinition, HoverRequest,
    PrepareRenameRequest, References, Rename, Request as _, SignatureHelpRequest,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, Documentation,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, Location, MarkupContent, MarkupKind, OneOf, ParameterInformation,
    ParameterLabel, Position, PrepareRenameResponse, PublishDiagnosticsParams, Range,
    ReferenceParams, RenameOptions, RenameParams, ServerCapabilities, SignatureHelp,
    SignatureHelpOptions, SignatureHelpParams, SignatureInformation, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, WorkDoneProgressOptions,
    WorkspaceEdit,
};

use stdlib::Stdlib;

/// Open documents by URI string, kept so hover can read the buffer at a position.
type Docs = HashMap<String, String>;

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        // Full document sync: each change carries the whole buffer, which the
        // zero-copy pipeline re-analyzes in microseconds.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        // Enables format-on-save (when the editor's formatOnSave is on) and the
        // explicit "Format Document" command, reusing `aura fmt`.
        document_formatting_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        // `prepare_provider` lets the editor refuse before prompting for a name,
        // so an impossible rename is reported up front instead of after typing.
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let init = connection.initialize(serde_json::to_value(capabilities)?)?;
    let registry_dir = registry_dir_from(&init);
    // The completion database is built once by evaluating the embedded manifest.
    let stdlib = Stdlib::load();
    main_loop(&connection, &stdlib, &registry_dir)?;
    // Drop the connection before joining: it owns the writer channel's sender, and
    // io_threads.join() only returns once that sender is gone (otherwise it hangs).
    drop(connection);
    io_threads.join()?;
    Ok(())
}

/// Where cached registry packages live, so go-to-definition can open one.
/// `initializationOptions.registryDir` (the editor's `aura.registryDir` setting)
/// wins; otherwise the CLI's default, `~/.aura/registry`.
fn registry_dir_from(init: &serde_json::Value) -> std::path::PathBuf {
    if let Some(dir) = init
        .get("initializationOptions")
        .and_then(|o| o.get("registryDir"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return std::path::PathBuf::from(dir);
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(".aura")
        .join("registry")
}

/// `file://` URI for a filesystem path, so a resolved registry module can be
/// opened. The inverse of `uri_to_fs_path`.
fn fs_path_to_uri(path: &std::path::Path) -> Option<lsp_types::Uri> {
    let mut s = path.to_str()?.replace('\\', "/");
    if !s.starts_with('/') {
        s.insert(0, '/'); // "C:/…" -> "/C:/…"
    }
    format!("file://{s}").parse().ok()
}

fn main_loop(
    connection: &Connection,
    stdlib: &Stdlib,
    registry_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let mut docs = Docs::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                match req.method.as_str() {
                    Completion::METHOD => {
                        let p: CompletionParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position;
                        let uri = pos.text_document.uri.to_string();
                        let items = docs
                            .get(&uri)
                            .map(|text| {
                                let offset = diagnostics::LineIndex::new(text).offset(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                );
                                // An imported module is read from disk unless it is
                                // already open, in which case the buffer is newer.
                                let load = |rel: &str| {
                                    let target = sibling_uri(&uri, rel)?;
                                    let key = target.to_string();
                                    if let Some(open) = docs.get(&key) {
                                        return Some(open.clone());
                                    }
                                    std::fs::read_to_string(uri_to_fs_path(&key)?).ok()
                                };
                                completion_items(stdlib, text, offset, load)
                            })
                            .unwrap_or_default();
                        let result = serde_json::to_value(CompletionResponse::Array(items))?;
                        respond(connection, req.id, result)?;
                    }
                    Formatting::METHOD => {
                        let p: DocumentFormattingParams = serde_json::from_value(req.params)?;
                        let uri = p.text_document.uri.to_string();
                        // Reuse `aura fmt`; a whole-document replace. A file that
                        // does not even lex is left untouched (None -> no edits).
                        let edits = docs.get(&uri).and_then(|text| format_edits(text));
                        respond(connection, req.id, serde_json::to_value(edits)?)?;
                    }
                    GotoDefinition::METHOD => {
                        let p: GotoDefinitionParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position_params;
                        let uri = pos.text_document.uri;
                        let (line, ch) = (pos.position.line, pos.position.character);
                        let result = docs.get(&uri.to_string()).and_then(|text| {
                            // Cross-file: `alias.Member` jumps to the member's
                            // `def`/`type` declaration inside the module file.
                            if let Some((rel, member)) = goto::imported_member(text, line, ch) {
                                if let Some(target) = sibling_uri(&uri.to_string(), &rel) {
                                    let range = uri_to_fs_path(&target.to_string())
                                        .and_then(|p| std::fs::read_to_string(p).ok())
                                        .and_then(|m| goto::declaration_range_in(&m, &member));
                                    if let Some(range) = range {
                                        return Some(GotoDefinitionResponse::Scalar(Location {
                                            uri: target,
                                            range,
                                        }));
                                    }
                                }
                            }
                            // Cross-file: an import (path, alias, or a use of the
                            // alias) opens the imported module file.
                            if let Some(rel) = goto::import_target_path(text, line, ch) {
                                if let Some(target) = sibling_uri(&uri.to_string(), &rel) {
                                    return Some(GotoDefinitionResponse::Scalar(Location {
                                        uri: target,
                                        range: Range::default(),
                                    }));
                                }
                            }
                            // A registry import (`org/pkg@v1.2`) opens the cached
                            // module; on `pkg.Member` it jumps to the declaration.
                            if let Some(file) =
                                goto::registry_target_path(text, line, ch, registry_dir)
                            {
                                if let Some(target) = fs_path_to_uri(&file) {
                                    let range = std::fs::read_to_string(&file)
                                        .ok()
                                        .and_then(|m| {
                                            let (_, member) =
                                                goto::imported_member(text, line, ch)?;
                                            goto::declaration_range_in(&m, &member)
                                        })
                                        .unwrap_or_default();
                                    return Some(GotoDefinitionResponse::Scalar(Location {
                                        uri: target,
                                        range,
                                    }));
                                }
                            }
                            // Same-file definition.
                            goto::definition_range(text, line, ch).map(|range| {
                                GotoDefinitionResponse::Scalar(Location {
                                    uri: uri.clone(),
                                    range,
                                })
                            })
                        });
                        respond(connection, req.id, serde_json::to_value(result)?)?;
                    }
                    PrepareRenameRequest::METHOD => {
                        let p: TextDocumentPositionParams = serde_json::from_value(req.params)?;
                        let text = docs.get(&p.text_document.uri.to_string());
                        match text
                            .map(|t| rename::prepare(t, p.position.line, p.position.character))
                        {
                            Some(Ok(range)) => {
                                let r = PrepareRenameResponse::Range(range);
                                respond(connection, req.id, serde_json::to_value(r)?)?;
                            }
                            // A refusal is an error response: that is what makes the
                            // editor say why instead of opening a rename box.
                            Some(Err(refusal)) => {
                                respond_err(connection, req.id, &refusal.message())?
                            }
                            None => respond(connection, req.id, serde_json::Value::Null)?,
                        }
                    }
                    Rename::METHOD => {
                        let p: RenameParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position;
                        let uri = pos.text_document.uri;
                        let text = docs.get(&uri.to_string());
                        let result = text.map(|t| {
                            rename::edits(t, pos.position.line, pos.position.character, &p.new_name)
                        });
                        match result {
                            Some(Ok(ranges)) => {
                                let edits: Vec<TextEdit> = ranges
                                    .into_iter()
                                    .map(|range| TextEdit {
                                        range,
                                        new_text: p.new_name.clone(),
                                    })
                                    .collect();
                                // The key type is dictated by lsp_types' WorkspaceEdit;
                                // `Uri` only looks interior-mutable to clippy.
                                #[allow(clippy::mutable_key_type)]
                                let mut changes = HashMap::new();
                                changes.insert(uri, edits);
                                let edit = WorkspaceEdit {
                                    changes: Some(changes),
                                    ..Default::default()
                                };
                                respond(connection, req.id, serde_json::to_value(edit)?)?;
                            }
                            Some(Err(refusal)) => {
                                respond_err(connection, req.id, &refusal.message())?
                            }
                            None => respond(connection, req.id, serde_json::Value::Null)?,
                        }
                    }
                    References::METHOD => {
                        let p: ReferenceParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position;
                        let uri = pos.text_document.uri;
                        let locations: Vec<Location> = docs
                            .get(&uri.to_string())
                            .map(|text| {
                                goto::reference_ranges(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                )
                                .into_iter()
                                .map(|range| Location {
                                    uri: uri.clone(),
                                    range,
                                })
                                .collect()
                            })
                            .unwrap_or_default();
                        respond(connection, req.id, serde_json::to_value(locations)?)?;
                    }
                    SignatureHelpRequest::METHOD => {
                        let p: SignatureHelpParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position_params;
                        let help = docs
                            .get(&pos.text_document.uri.to_string())
                            .and_then(|text| {
                                let offset = diagnostics::LineIndex::new(text).offset(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                );
                                signature::signature_at(stdlib, text, offset).map(|s| {
                                    SignatureHelp {
                                        signatures: vec![SignatureInformation {
                                            label: s.label,
                                            documentation: s.doc.map(Documentation::String),
                                            parameters: Some(
                                                s.params
                                                    .into_iter()
                                                    .map(|p| ParameterInformation {
                                                        label: ParameterLabel::Simple(p),
                                                        documentation: None,
                                                    })
                                                    .collect(),
                                            ),
                                            active_parameter: Some(s.active),
                                        }],
                                        active_signature: Some(0),
                                        active_parameter: Some(s.active),
                                    }
                                })
                            });
                        respond(connection, req.id, serde_json::to_value(help)?)?;
                    }
                    DocumentSymbolRequest::METHOD => {
                        let p: DocumentSymbolParams = serde_json::from_value(req.params)?;
                        let symbols = docs.get(&p.text_document.uri.to_string()).map(|text| {
                            DocumentSymbolResponse::Nested(goto::document_symbols(text))
                        });
                        respond(connection, req.id, serde_json::to_value(symbols)?)?;
                    }
                    HoverRequest::METHOD => {
                        let p: HoverParams = serde_json::from_value(req.params)?;
                        let pos = p.text_document_position_params;
                        let uri = pos.text_document.uri.to_string();
                        let result = docs
                            .get(&uri)
                            .and_then(|text| {
                                hover::hover_markdown(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                    stdlib,
                                )
                            })
                            .map(|md| Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: md,
                                }),
                                range: None,
                            });
                        respond(connection, req.id, serde_json::to_value(result)?)?;
                    }
                    _ => {}
                }
            }
            Message::Notification(not) => match not.method.as_str() {
                DidOpenTextDocument::METHOD => {
                    let p: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
                    let uri = p.text_document.uri;
                    docs.insert(uri.to_string(), p.text_document.text.clone());
                    publish(
                        connection,
                        uri,
                        &p.text_document.text,
                        Some(p.text_document.version),
                    )?;
                }
                DidChangeTextDocument::METHOD => {
                    let p: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
                    // FULL sync: the last change event holds the entire document.
                    if let Some(change) = p.content_changes.into_iter().last() {
                        let uri = p.text_document.uri;
                        docs.insert(uri.to_string(), change.text.clone());
                        publish(connection, uri, &change.text, Some(p.text_document.version))?;
                    }
                }
                DidCloseTextDocument::METHOD => {
                    let p: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
                    let uri = p.text_document.uri;
                    docs.remove(&uri.to_string());
                    // Clear diagnostics for a closed file.
                    publish(connection, uri, "", None)?;
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// The URI of a file sibling to `doc_uri`, by replacing its last path segment
/// with a relative import path. String-only, so it works on every platform
/// without file-URI/path conversion.
fn sibling_uri(doc_uri: &str, rel: &str) -> Option<lsp_types::Uri> {
    let (base, _) = doc_uri.rsplit_once('/')?;
    format!("{base}/{rel}").parse().ok()
}

/// Filesystem path for a `file://` URI (to read an imported module). Percent-
/// decodes and, on Windows, drops the leading slash before a drive letter.
fn uri_to_fs_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut decoded = String::with_capacity(rest.len());
    let mut bytes = rest.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next()?;
            let lo = bytes.next()?;
            let byte = u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16).ok()?;
            decoded.push(byte as char);
        } else {
            decoded.push(b as char);
        }
    }
    // "/C:/…" -> "C:/…" on Windows.
    if cfg!(windows) {
        if let Some(stripped) = decoded.strip_prefix('/') {
            if stripped.as_bytes().get(1) == Some(&b':') {
                decoded = stripped.to_string();
            }
        }
    }
    Some(std::path::PathBuf::from(decoded))
}

fn respond(
    connection: &Connection,
    id: lsp_server::RequestId,
    result: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    connection.sender.send(Message::Response(Response {
        id,
        result: Some(result),
        error: None,
    }))?;
    Ok(())
}

/// Refuse a request with a message the editor shows to the user.
fn respond_err(
    connection: &Connection,
    id: lsp_server::RequestId,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    connection.sender.send(Message::Response(Response {
        id,
        result: None,
        error: Some(lsp_server::ResponseError {
            code: lsp_server::ErrorCode::RequestFailed as i32,
            message: message.to_string(),
            data: None,
        }),
    }))?;
    Ok(())
}

/// Format a document with `aura fmt` as a single whole-document edit, or `None`
/// if it does not lex (never rewrite a file we cannot tokenize).
fn format_edits(text: &str) -> Option<Vec<TextEdit>> {
    let formatted = aura_lang::fmt::format_source(text).ok()?;
    if formatted == text {
        return Some(Vec::new()); // already canonical: no edit
    }
    let end = diagnostics::LineIndex::new(text).position(text, text.len());
    Some(vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }])
}

/// Context-aware completion. After a `.` only stdlib methods are offered; at any
/// other position: builtins, keywords, and the file's declared names.
/// `load_module` resolves an import's relative path to that file's text, so a
/// field of an *imported* schema can offer its enum members too.
fn completion_items(
    stdlib: &Stdlib,
    text: &str,
    offset: usize,
    load_module: impl Fn(&str) -> Option<String>,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    // D18: inside `new Schema`, a field typed by an enum offers exactly its
    // members — the one place where the set of valid values is known exactly.
    if let Some(members) = infer::expected_enum_members(text, offset, load_module) {
        return members
            .into_iter()
            .map(|m| CompletionItem {
                label: format!("\"{m}\""),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some("enum member".to_string()),
                insert_text: Some(format!("\"{m}\"")),
                ..Default::default()
            })
            .collect();
    }
    if goto::is_method_context(text, offset) {
        // Type-aware when the receiver type can be inferred; otherwise all methods.
        let recv = infer::receiver_type(stdlib, text, offset);
        let mut seen = std::collections::HashSet::new();
        for e in stdlib.methods() {
            if recv.as_deref().is_some_and(|ty| ty != e.receiver) {
                continue;
            }
            // A name shared across receivers (to_str, get, …) is offered once.
            if seen.insert(e.name.clone()) {
                items.push(stdlib_item(e, CompletionItemKind::METHOD));
            }
        }
        return items;
    }
    for e in stdlib.builtins() {
        items.push(stdlib_item(e, CompletionItemKind::FUNCTION));
    }
    for kw in stdlib::keywords() {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }
    for name in goto::local_names(text) {
        items.push(CompletionItem {
            label: name,
            kind: Some(CompletionItemKind::VARIABLE),
            ..Default::default()
        });
    }
    items
}

fn stdlib_item(e: &stdlib::Entry, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: e.name.clone(),
        kind: Some(kind),
        detail: Some(e.signature()),
        documentation: Some(Documentation::String(e.doc.clone())),
        ..Default::default()
    }
}

fn publish(
    connection: &Connection,
    uri: lsp_types::Uri,
    text: &str,
    version: Option<i32>,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics: diagnostics::analyze_source(text),
        version,
    };
    let not = lsp_server::Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    };
    connection.sender.send(Message::Notification(not))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolved registry module is handed to the editor as a URI, so the path
    /// conversion must round-trip — including the Windows drive-letter form, where
    /// `file:///C:/…` and `C:\…` differ by more than separators.
    #[test]
    fn fs_path_and_uri_round_trip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(path.exists(), "fixture must exist");
        let uri = fs_path_to_uri(&path).expect("a uri");
        let back = uri_to_fs_path(&uri.to_string()).expect("a path");
        // Compare canonically: the round trip normalises separators.
        assert_eq!(
            std::fs::canonicalize(&back).unwrap(),
            std::fs::canonicalize(&path).unwrap(),
            "uri was {uri:?}"
        );
    }

    #[test]
    fn registry_dir_comes_from_initialization_options_then_falls_back() {
        let given = serde_json::json!({"initializationOptions": {"registryDir": "/tmp/reg"}});
        assert_eq!(
            registry_dir_from(&given),
            std::path::PathBuf::from("/tmp/reg")
        );
        // Absent or empty -> the CLI's default location, not an empty path.
        for v in [
            serde_json::json!({}),
            serde_json::json!({"initializationOptions": {"registryDir": ""}}),
        ] {
            let d = registry_dir_from(&v);
            assert!(d.ends_with("registry"), "{d:?}");
            assert!(d.parent().is_some_and(|p| p.ends_with(".aura")), "{d:?}");
        }
    }

    #[test]
    fn formatting_reindents_and_is_a_noop_when_canonical() {
        // messy indentation -> one whole-document edit with canonical text
        let messy = "domain \"d\"\n      x: 1\nend\n";
        let edit = format_edits(messy).expect("lexes").pop().expect("an edit");
        assert_eq!(edit.new_text, "domain \"d\"\n  x: 1\nend\n");
        assert_eq!(edit.range.start, Position::new(0, 0));
        // already-canonical input yields no edits
        assert!(format_edits(&edit.new_text).unwrap().is_empty());
    }
}
