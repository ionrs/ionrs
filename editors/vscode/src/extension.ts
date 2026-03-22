import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext) {
  const config = vscode.workspace.getConfiguration("ion.lsp");
  const enabled = config.get<boolean>("enabled", true);

  if (!enabled) {
    return;
  }

  const serverPath = config.get<string>("path", "ion-lsp");

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: [],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "ion" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.ion"),
    },
  };

  client = new LanguageClient(
    "ion-lsp",
    "Ion Language Server",
    serverOptions,
    clientOptions
  );

  client.start().catch((err) => {
    // LSP binary not found — syntax highlighting still works
    const msg = err?.message || String(err);
    if (msg.includes("ENOENT") || msg.includes("spawn")) {
      // Silently degrade: LSP not installed
      console.log(
        "ion-lsp not found, running without language server. Install with: cargo install --path ion-lsp"
      );
    } else {
      vscode.window.showWarningMessage(`Ion LSP failed to start: ${msg}`);
    }
  });

  context.subscriptions.push({
    dispose: () => {
      if (client) {
        client.stop();
      }
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  if (client) {
    return client.stop();
  }
  return undefined;
}
