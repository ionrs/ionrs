# Ion Tooling

## CLI (`ion-cli/src/main.rs`)
- `ion script.ion` — run a script
- `ion --vm script.ion` — run with bytecode VM
- `ion` (no args) — REPL
- REPL commands: `:quit`/`:q`, `:vm` (toggle VM mode)
- Multi-line input: accumulates until braces balance
- Error display: uses `format_with_source()` for colored output with carets

## LSP (`ion-lsp/src/main.rs`)
- Protocol: lsp-server + lsp-types crates
- Capabilities:
  - **Diagnostics**: parse errors (lexer + parser) on open/change
  - **Hover**: builtins, stdlib and workspace module docs, keywords,
    user-defined fns/vars
  - **Completion**: keywords, builtins, stdlib and workspace modules, methods
    (after `.`), user definitions
  - **Go-to-definition**: user-defined functions and variables (same file)
  - **Document symbols**: functions and top-level variables
- Trigger: `.` for method completions and `:` for `module::` completions
- Method completions cover all string/list/dict/option/result/bytes/task/channel methods
- Syntax and completions include `async`, `spawn`, `.await`, `select`, native
  async channel methods, `sleep`, and `timeout`.

### Workspace Ion Doc Manifests

`ion-lsp` can load extra host/runtime documentation without editor plugin
changes. Built-in stdlib docs load first; workspace manifests may add modules or
override an existing module/member with the same key.

Discovery paths:
- `<workspace>/.ion/ion-docs/*.json`
- `<workspace>/ion-docs/*.json`
- `ION_LSP_DOCS`, with files or directories separated by the platform path
  separator (`:` on Unix, `;` on Windows). Directories load their `*.json` files.

The workspace base comes from LSP `workspaceFolders` or `rootUri`; if neither is
available, `ion-lsp` uses its current working directory. Invalid or missing
manifest files are ignored after a stderr warning so diagnostics and LSP
requests keep working.

Manifest format:

```json
{
  "ionDocVersion": 1,
  "profile": "ivex.sensor-runtime",
  "modules": [
    {
      "name": "sensor",
      "summary": "Sensor runtime control plane.",
      "members": [
        {
          "name": "call",
          "kind": "function",
          "signature": "sensor::call(method, params?, options?) -> Result<value, dict>",
          "doc": "Calls a sensor-api method through the selected KEX session."
        }
      ],
      "modules": [
        {
          "name": "session",
          "summary": "Per-process sensor script value store.",
          "members": [
            {
              "name": "set",
              "kind": "function",
              "signature": "sensor::session::set(key, value, options?) -> Result<dict, dict>",
              "doc": "Stores a value in the runtime session store."
            }
          ]
        }
      ]
    }
  ]
}
```

Members support `"kind": "function"` and `"kind": "constant"`. Nested modules
are addressed with Ion paths such as `sensor::session`; hovers work for module
overviews and members, and completions work after both `sensor::` and
`sensor::session::`.

## VSCode Extension (`editors/vscode/`)
- `syntaxes/ion.tmLanguage.json` — TextMate grammar
- Covers: keywords, builtins, types (Some/None/Ok/Err), operators (|>, ?),
  string interpolation (f"...{expr}..."), triple-quoted strings, byte strings,
  escape sequences (\n, \t, \xNN, \u{XXXX}), dict literals (#{), lambdas,
  function defs/calls, comments, numbers

## Error Reporting (`ion-core/src/error.rs`)
- `IonError` with kind, message, line, col, additional (multi-error)
- `format_with_source()`: colored output with line numbers, carets, help hints
- Error hints for: undefined variables, immutable assignment, type coercion,
  missing methods, missing semicolons, unmatched braces, division by zero,
  stack overflow, index out of bounds
- `ion_str!()` used for hint pattern matching (`&*ion_str!(...)` for coercion)
