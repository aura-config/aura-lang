//! Aura Language Server.
//!
//! A small synchronous LSP server (rust-analyzer's `lsp-server` stack, no async
//! runtime) that publishes live diagnostics for `.aura` buffers. Everything
//! smart lives in `aura-core`; this binary is only the protocol layer.

mod diagnostics;
mod goto;
mod hover;
mod stdlib;

use std::collections::HashMap;

use lsp_server::{Connection, Message, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, Formatting, GotoDefinition, HoverRequest, References,
    Request as _,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, Documentation,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, Location, MarkupContent, MarkupKind, OneOf, Position,
    PublishDiagnosticsParams, Range, ReferenceParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
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
        ..Default::default()
    };
    connection.initialize(serde_json::to_value(capabilities)?)?;
    // The completion database is built once by evaluating the embedded manifest.
    let stdlib = Stdlib::load();
    main_loop(&connection, &stdlib)?;
    // Drop the connection before joining: it owns the writer channel's sender, and
    // io_threads.join() only returns once that sender is gone (otherwise it hangs).
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn main_loop(
    connection: &Connection,
    stdlib: &Stdlib,
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
                        let items = docs
                            .get(&pos.text_document.uri.to_string())
                            .map(|text| {
                                let offset = diagnostics::LineIndex::new(text).offset(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                );
                                completion_items(stdlib, text, offset)
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
                        let result = docs
                            .get(&uri.to_string())
                            .and_then(|text| {
                                goto::definition_range(
                                    text,
                                    pos.position.line,
                                    pos.position.character,
                                )
                            })
                            .map(|range| {
                                GotoDefinitionResponse::Scalar(Location {
                                    uri: uri.clone(),
                                    range,
                                })
                            });
                        respond(connection, req.id, serde_json::to_value(result)?)?;
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

/// Format a document with `aura fmt` as a single whole-document edit, or `None`
/// if it does not lex (never rewrite a file we cannot tokenize).
fn format_edits(text: &str) -> Option<Vec<TextEdit>> {
    let formatted = aura_core::fmt::format_source(text).ok()?;
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
fn completion_items(stdlib: &Stdlib, text: &str, offset: usize) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    if goto::is_method_context(text, offset) {
        let mut seen = std::collections::HashSet::new();
        for e in stdlib.methods() {
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
