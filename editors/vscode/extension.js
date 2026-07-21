// Aura VSCode client: launches the `aura-lsp` server over stdio and wires it to
// `.aura` documents. All language intelligence lives in the server (aura-lang);
// this file only starts and stops it.

const { workspace } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const cfg = workspace.getConfiguration("aura");
  if (cfg.get("server.enable") === false) {
    return;
  }
  const command = cfg.get("server.path") || "aura-lsp";
  const serverOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "aura" }],
  };
  client = new LanguageClient(
    "aura",
    "Aura Language Server",
    serverOptions,
    clientOptions
  );
  context.subscriptions.push(client);
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
