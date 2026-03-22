# Ion Language for VS Code

Syntax highlighting and language server support for [Ion](https://github.com/chutuananh2k/ion-lang), an embeddable scripting language.

## Features

- **Syntax highlighting** for `.ion` files (keywords, strings, f-strings, numbers, operators, comments)
- **Error diagnostics** via the Ion LSP server (parse errors shown inline)
- **Document symbols** (functions and variables in the outline view)
- **Bracket matching** and auto-closing pairs

## Setup

### Syntax highlighting only

Install the extension — syntax highlighting works immediately with no additional setup.

### With LSP (diagnostics + symbols)

Build and install the LSP server:

```bash
cd ion-lang
cargo install --path ion-lsp
```

The extension will automatically connect to `ion-lsp` on the PATH. To use a custom path:

```json
{
  "ion.lsp.path": "/path/to/ion-lsp"
}
```

To disable the LSP (syntax highlighting still works):

```json
{
  "ion.lsp.enabled": false
}
```

## Development

```bash
cd editors/vscode
npm install
npm run compile
```

To test: press F5 in VS Code to launch an Extension Development Host.

## Packaging

```bash
npx vsce package
```

This creates an `ion-lang-0.1.0.vsix` file you can install with:

```bash
code --install-extension ion-lang-0.1.0.vsix
```
