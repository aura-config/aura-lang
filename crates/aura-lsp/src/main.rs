//! Aura Language Server.
//!
//! A small synchronous LSP server (rust-analyzer's `lsp-server` stack, no async
//! runtime) that publishes live diagnostics for `.aura` buffers. Everything
//! smart lives in `aura-core`; this binary is only the protocol layer.

mod diagnostics;

use lsp_server::{Connection, Message};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    PublishDiagnosticsParams, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
};

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        // Full document sync: each change carries the whole buffer, which the
        // zero-copy pipeline re-analyzes in microseconds.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    };
    connection.initialize(serde_json::to_value(capabilities)?)?;
    main_loop(&connection)?;
    // Drop the connection before joining: it owns the writer channel's sender, and
    // io_threads.join() only returns once that sender is gone (otherwise it hangs).
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                // No request-based features yet (completion/hover come later).
            }
            Message::Notification(not) => match not.method.as_str() {
                DidOpenTextDocument::METHOD => {
                    let p: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
                    publish(
                        connection,
                        p.text_document.uri,
                        &p.text_document.text,
                        Some(p.text_document.version),
                    )?;
                }
                DidChangeTextDocument::METHOD => {
                    let p: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
                    // FULL sync: the last change event holds the entire document.
                    if let Some(change) = p.content_changes.into_iter().last() {
                        publish(
                            connection,
                            p.text_document.uri,
                            &change.text,
                            Some(p.text_document.version),
                        )?;
                    }
                }
                DidCloseTextDocument::METHOD => {
                    let p: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
                    // Clear diagnostics for a closed file.
                    publish(connection, p.text_document.uri, "", None)?;
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
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
