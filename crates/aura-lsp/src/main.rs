//! Aura Language Server.
//!
//! A small synchronous LSP server (rust-analyzer's `lsp-server` stack, no async
//! runtime) that publishes live diagnostics for `.aura` buffers. Everything
//! smart lives in `aura-core`; this binary is only the protocol layer.

mod diagnostics;
mod hover;
mod stdlib;

use std::collections::HashMap;

use lsp_server::{Connection, Message, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, HoverRequest, Request as _};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    Documentation, Hover, HoverContents, HoverParams, HoverProviderCapability, MarkupContent,
    MarkupKind, PublishDiagnosticsParams, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind,
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
                        // Context-free for now: offer the whole stdlib surface plus
                        // keywords; the editor filters by the typed prefix.
                        let items = completion_items(stdlib);
                        let result = serde_json::to_value(CompletionResponse::Array(items))?;
                        respond(connection, req.id, result)?;
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

/// Completion items for the whole stdlib surface: methods (deduped by name),
/// builtin functions, and keywords. Each carries a signature and doc.
fn completion_items(stdlib: &Stdlib) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in stdlib.methods() {
        // A name shared across receivers (to_str, get, …) is offered once.
        if seen.insert(e.name.clone()) {
            items.push(stdlib_item(e, CompletionItemKind::METHOD));
        }
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
