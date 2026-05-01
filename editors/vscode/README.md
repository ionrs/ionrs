# Ion Language for VS Code

Syntax highlighting and language server support for [Ion](https://github.com/chutuananh2k/ion-lang), an embeddable scripting language.

## Features

### Syntax Highlighting
- Keywords, control flow, operators
- Strings, triple-quoted strings, f-strings with interpolation, byte strings
- Numbers (int and float, including `_` separators)
- Comments (`//`)
- Type annotations (`let x: int = 5`)
- Collections, comprehensions, pattern matching, modules, and async/concurrency syntax

### Language Server (optional)
- **Diagnostics** — parse errors shown inline as you type
- **Hover** — type info and documentation on hover
- **Completions** — keywords, builtins, methods, type annotations
- **Go to Definition** — jump to function/variable definitions
- **Document Symbols** — functions and variables in the outline/breadcrumb

## Setup

### Syntax highlighting only

Install the extension — syntax highlighting works immediately with no additional setup.

### With LSP (full IDE features)

Build and install the LSP server:

```bash
cd ion-lang
cargo install --path ion-lsp
```

The extension automatically connects to `ion-lsp` on the PATH. To use a custom path:

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

## Install from .vsix

```bash
code --install-extension ion-lang-0.3.2.vsix
```

## Development

```bash
cd editors/vscode
npm install
npm run compile
```

Press F5 in VS Code to launch an Extension Development Host for testing.

### Packaging

```bash
npx @vscode/vsce package
```
