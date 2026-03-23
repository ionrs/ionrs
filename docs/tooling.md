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
  - **Hover**: builtins (27), keywords (19), user-defined fns/vars
  - **Completion**: keywords, builtins, methods (after `.`), user definitions
  - **Go-to-definition**: user-defined functions and variables (same file)
  - **Document symbols**: functions and top-level variables
- Trigger: `.` for method completions
- Method completions cover all string/list/dict/option/result/bytes/task/channel methods

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
