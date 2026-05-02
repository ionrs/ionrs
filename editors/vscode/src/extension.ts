import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

function splitCommandLine(value: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: "'" | "\"" | undefined;

  for (const ch of value) {
    if ((ch === "'" || ch === "\"") && !quote) {
      quote = ch;
      continue;
    }
    if (ch === quote) {
      quote = undefined;
      continue;
    }
    if (/\s/.test(ch) && !quote) {
      if (current.length > 0) {
        parts.push(current);
        current = "";
      }
      continue;
    }
    current += ch;
  }

  if (current.length > 0) {
    parts.push(current);
  }
  return parts;
}

function serverCommand(config: vscode.WorkspaceConfiguration): ServerOptions {
  const configuredPath = config.get<string>("path", "ion-lsp").trim();
  const configuredArgs = config.get<string[]>("args", []);
  const commandLine = configuredPath || "ion-lsp";
  const commandParts = commandPartsFromSetting(commandLine);
  const command = commandParts[0] || "ion-lsp";
  const args = [...commandParts.slice(1), ...configuredArgs];

  return { command, args };
}

function commandPartsFromSetting(value: string): string[] {
  const parts = splitCommandLine(value);
  if (parts.length <= 1) {
    return parts;
  }

  const first = parts[0].toLowerCase();
  if (
    value.startsWith("\"") ||
    value.startsWith("'") ||
    first === "wsl" ||
    first === "wsl.exe"
  ) {
    return parts;
  }

  return [value];
}

export function activate(context: vscode.ExtensionContext) {
  const config = vscode.workspace.getConfiguration("ion.lsp");
  const enabled = config.get<boolean>("enabled", true);

  if (!enabled) {
    return;
  }

  const serverOptions = serverCommand(config);

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "ion" },
      { scheme: "vscode-remote", language: "ion" },
    ],
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
