use std::collections::HashMap;

use ion_core::ast::{Param, StmtKind, UseImports};
use ion_core::error::IonError;
use ion_core::lexer::Lexer;
use ion_core::parser::Parser;
use ion_core::token::Token;

use lsp_server::{
    Connection, ErrorCode, Message, Notification as LspNotification, Request, RequestId,
    Response, ResponseError,
};
use lsp_types::notification::{
    self, DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as NotificationTrait,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, References, Rename,
    Request as RequestTrait,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeResult, Location, MarkupContent, MarkupKind,
    OneOf, Position, Range, ReferenceParams, RenameParams, ServerCapabilities, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
};

// ---- Definition tracking ----

#[derive(Debug, Clone)]
struct Definition {
    name: String,
    kind: DefKind,
    line: u32, // 0-based
    col: u32,
    detail: String,
}

#[derive(Debug, Clone)]
enum DefKind {
    Function,
    Variable,
}

fn collect_definitions(source: &str) -> Vec<Definition> {
    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let mut parser = Parser::new(tokens);
    // Use recovering parse so definitions are available even with errors
    let output = parser.parse_program_recovering();
    let mut defs = Vec::new();
    collect_defs_from_stmts(&output.program.stmts, source, &mut defs);
    defs
}

fn collect_defs_from_stmts(
    stmts: &[ion_core::ast::Stmt],
    source: &str,
    defs: &mut Vec<Definition>,
) {
    for stmt in stmts {
        let line = if stmt.span.line > 0 {
            stmt.span.line as u32 - 1
        } else {
            0
        };
        let col = if stmt.span.col > 0 {
            stmt.span.col as u32 - 1
        } else {
            0
        };
        match &stmt.kind {
            StmtKind::FnDecl { name, params, body } => {
                defs.push(Definition {
                    name: name.clone(),
                    kind: DefKind::Function,
                    line,
                    col,
                    detail: format_fn_signature(name, params),
                });
                // Track parameters as definitions so they hover/jump correctly.
                for param in params {
                    let p_detail = match &param.default {
                        Some(_) => format!("(parameter) {} (with default)", param.name),
                        None => format!("(parameter) {}", param.name),
                    };
                    defs.push(Definition {
                        name: param.name.clone(),
                        kind: DefKind::Variable,
                        line,
                        col,
                        detail: p_detail,
                    });
                }
                collect_defs_from_stmts(body, source, defs);
            }
            StmtKind::Let {
                mutable,
                pattern: ion_core::ast::Pattern::Ident(name),
                type_ann,
                ..
            } => {
                defs.push(Definition {
                    name: name.clone(),
                    kind: DefKind::Variable,
                    line,
                    col,
                    detail: format_let_detail(source, line, *mutable, name, type_ann.as_ref()),
                });
            }
            StmtKind::For { body, .. } => collect_defs_from_stmts(body, source, defs),
            StmtKind::While { body, .. } => collect_defs_from_stmts(body, source, defs),
            StmtKind::WhileLet { body, .. } => collect_defs_from_stmts(body, source, defs),
            StmtKind::Loop { body, .. } => collect_defs_from_stmts(body, source, defs),
            StmtKind::ExprStmt { expr, .. } => collect_defs_from_expr(expr, source, defs),
            StmtKind::Use { path, imports } => {
                let module_path = path.join("::");
                let names: Vec<String> = match imports {
                    UseImports::Single(name) => vec![name.clone()],
                    UseImports::Names(names) => names.clone(),
                    UseImports::Glob => vec![format!("{}::*", module_path)],
                };
                for name in names {
                    defs.push(Definition {
                        name: name.clone(),
                        kind: DefKind::Variable,
                        line,
                        col,
                        detail: format!("use {}::{}", module_path, name),
                    });
                }
            }
            _ => {}
        }
    }
}

fn collect_defs_from_expr(
    expr: &ion_core::ast::Expr,
    source: &str,
    defs: &mut Vec<Definition>,
) {
    use ion_core::ast::ExprKind;
    match &expr.kind {
        ExprKind::If {
            then_body,
            else_body,
            ..
        } => {
            collect_defs_from_stmts(then_body, source, defs);
            if let Some(eb) = else_body {
                collect_defs_from_stmts(eb, source, defs);
            }
        }
        ExprKind::IfLet {
            then_body,
            else_body,
            ..
        } => {
            collect_defs_from_stmts(then_body, source, defs);
            if let Some(eb) = else_body {
                collect_defs_from_stmts(eb, source, defs);
            }
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                collect_defs_from_expr(&arm.body, source, defs);
            }
        }
        ExprKind::Block(stmts) => collect_defs_from_stmts(stmts, source, defs),
        ExprKind::TryCatch { body, handler, .. } => {
            collect_defs_from_stmts(body, source, defs);
            collect_defs_from_stmts(handler, source, defs);
        }
        _ => {}
    }
}

fn format_fn_signature(name: &str, params: &[Param]) -> String {
    let params_str: Vec<String> = params
        .iter()
        .map(|p| {
            if let Some(default) = &p.default {
                format!("{} = {:?}", p.name, default)
            } else {
                p.name.clone()
            }
        })
        .collect();
    format!("fn {}({})", name, params_str.join(", "))
}

/// Build a richer hover string for a `let` binding by reading the source line
/// where it was declared and extracting the initializer expression.
fn format_let_detail(
    source: &str,
    line: u32,
    mutable: bool,
    name: &str,
    type_ann: Option<&ion_core::ast::TypeAnn>,
) -> String {
    let mut_str = if mutable { "mut " } else { "" };
    let type_str = match type_ann {
        Some(t) => format!(": {}", format_type_ann(t)),
        None => String::new(),
    };
    let init = source.lines().nth(line as usize).and_then(extract_let_initializer);
    match init {
        Some(rhs) => format!("let {}{}{} = {}", mut_str, name, type_str, rhs.trim()),
        None => format!("let {}{}{}", mut_str, name, type_str),
    }
}

/// Pull the initializer text out of a single-line `let` statement. Returns
/// the slice between `=` and a trailing `;` (if present), trimmed.
fn extract_let_initializer(line_text: &str) -> Option<String> {
    let bytes = line_text.as_bytes();
    // Find the first top-level `=` (skip `==`, `<=`, `>=`, `!=`).
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return None; // hit a line comment before any `=`
        }
        if b == b'=' {
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            let next = bytes.get(i + 1).copied().unwrap_or(0);
            if next != b'=' && !matches!(prev, b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/') {
                let rest = &line_text[i + 1..];
                let rest = rest.trim_end();
                let rest = rest.strip_suffix(';').unwrap_or(rest);
                let trimmed = rest.trim();
                if trimmed.is_empty() {
                    return None;
                }
                return Some(trimmed.to_string());
            }
        }
        i += 1;
    }
    None
}

fn format_type_ann(t: &ion_core::ast::TypeAnn) -> String {
    use ion_core::ast::TypeAnn;
    match t {
        TypeAnn::Simple(s) => s.clone(),
        TypeAnn::Option(inner) => format!("Option<{}>", format_type_ann(inner)),
        TypeAnn::Result(ok, err) => {
            format!("Result<{}, {}>", format_type_ann(ok), format_type_ann(err))
        }
        TypeAnn::List(inner) => format!("list<{}>", format_type_ann(inner)),
        TypeAnn::Dict(k, v) => format!("dict<{}, {}>", format_type_ann(k), format_type_ann(v)),
    }
}

// ---- Word at position ----

fn word_at_position(source: &str, line: u32, col: u32) -> Option<String> {
    let target_line = source.lines().nth(line as usize)?;
    let col = col as usize;
    if col > target_line.len() {
        return None;
    }
    let bytes = target_line.as_bytes();
    let mut start = col;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(target_line[start..end].to_string())
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Context immediately preceding the word at cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrefixCtx {
    /// No special prefix.
    Plain,
    /// The word is preceded by `.` (method call).
    Method,
    /// The word is preceded by `<name>::` (module member).
    Module(String),
}

/// Compute the byte range of the word at `(line, col)` in the source line.
/// Returns `(start_byte, end_byte)` within the line, or `None` if the cursor
/// isn't on an identifier.
fn word_range_in_line(line_text: &str, col: usize) -> Option<(usize, usize)> {
    if col > line_text.len() {
        return None;
    }
    let bytes = line_text.as_bytes();
    let mut start = col;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start == end {
        None
    } else {
        Some((start, end))
    }
}

/// Detect what immediately precedes the word at the given position.
fn prefix_context_at(source: &str, line: u32, col: u32) -> PrefixCtx {
    let Some(line_text) = source.lines().nth(line as usize) else {
        return PrefixCtx::Plain;
    };
    let col = col as usize;
    let Some((start, _)) = word_range_in_line(line_text, col) else {
        return PrefixCtx::Plain;
    };
    let bytes = line_text.as_bytes();
    if start == 0 {
        return PrefixCtx::Plain;
    }
    let prev = bytes[start - 1];
    if prev == b'.' {
        return PrefixCtx::Method;
    }
    // Look for `<ident>::` ending exactly at `start`.
    if start >= 2 && bytes[start - 1] == b':' && bytes[start - 2] == b':' {
        let mod_end = start - 2;
        let mut mod_start = mod_end;
        while mod_start > 0 && is_ident_char(bytes[mod_start - 1]) {
            mod_start -= 1;
        }
        if mod_start < mod_end {
            let name = &line_text[mod_start..mod_end];
            return PrefixCtx::Module(name.to_string());
        }
    }
    PrefixCtx::Plain
}

/// Convert a (line, byte_start, byte_end) tuple to an LSP Range, treating
/// positions as UTF-16 code units (matching how editors send positions).
fn line_range(line: u32, line_text: &str, start_byte: usize, end_byte: usize) -> Range {
    let to_utf16 = |byte: usize| -> u32 {
        line_text[..byte.min(line_text.len())]
            .encode_utf16()
            .count() as u32
    };
    Range {
        start: Position::new(line, to_utf16(start_byte)),
        end: Position::new(line, to_utf16(end_byte)),
    }
}

// ---- Builtins ----

struct BuiltinInfo {
    name: &'static str,
    signature: &'static str,
    description: &'static str,
}

const BUILTINS: &[BuiltinInfo] = &[
    BuiltinInfo {
        name: "len",
        signature: "len(x)",
        description: "Length of list, string, dict, or bytes",
    },
    BuiltinInfo {
        name: "range",
        signature: "range(n) / range(start, end)",
        description: "Create a range [0..n) or [start..end)",
    },
    BuiltinInfo {
        name: "enumerate",
        signature: "enumerate(list)",
        description: "List of (index, value) tuples",
    },
    BuiltinInfo {
        name: "type_of",
        signature: "type_of(x)",
        description: "Returns type name as string",
    },
    BuiltinInfo {
        name: "str",
        signature: "str(x)",
        description: "Convert to string",
    },
    BuiltinInfo {
        name: "int",
        signature: "int(x)",
        description: "Convert to int",
    },
    BuiltinInfo {
        name: "float",
        signature: "float(x)",
        description: "Convert to float",
    },
    BuiltinInfo {
        name: "bytes",
        signature: "bytes() / bytes(list) / bytes(string) / bytes(n)",
        description: "Create bytes",
    },
    BuiltinInfo {
        name: "bytes_from_hex",
        signature: "bytes_from_hex(string)",
        description: "Bytes from hex string",
    },
    BuiltinInfo {
        name: "assert",
        signature: "assert(cond) / assert(cond, msg)",
        description: "Error if condition is false",
    },
    BuiltinInfo {
        name: "assert_eq",
        signature: "assert_eq(a, b) / assert_eq(a, b, msg)",
        description: "Error if values are not equal",
    },
    BuiltinInfo {
        name: "channel",
        signature: "channel(buffer_size)",
        description: "Create a buffered channel (tx, rx)",
    },
    BuiltinInfo {
        name: "set",
        signature: "set() / set(list)",
        description: "Create a set from a list (deduplicates elements)",
    },
    BuiltinInfo {
        name: "cell",
        signature: "cell(value)",
        description: "Create a mutable reference cell for shared closure state",
    },
    BuiltinInfo {
        name: "sleep",
        signature: "sleep(ms)",
        description: "Sleep for given milliseconds (requires concurrency feature)",
    },
    BuiltinInfo {
        name: "timeout",
        signature: "timeout(ms, fn)",
        description: "Run function with time limit, returns Option (Some or None on timeout)",
    },
];

const KEYWORDS: &[&str] = &[
    "let", "mut", "fn", "if", "else", "while", "for", "loop", "break", "continue", "return",
    "match", "in", "true", "false", "None", "Some", "Ok", "Err", "async", "spawn", "select", "try",
    "catch", "use",
];

// ---- Methods (shared by hover and completion) ----

struct MethodInfo {
    name: &'static str,
    signature: &'static str,
    doc: &'static str,
}

const METHODS: &[MethodInfo] = &[
    // String methods
    MethodInfo { name: "len", signature: "len()", doc: "Length" },
    MethodInfo { name: "is_empty", signature: "is_empty()", doc: "True if empty" },
    MethodInfo { name: "contains", signature: "contains(sub)", doc: "Contains substring/element" },
    MethodInfo { name: "starts_with", signature: "starts_with(prefix)", doc: "Starts with prefix" },
    MethodInfo { name: "ends_with", signature: "ends_with(suffix)", doc: "Ends with suffix" },
    MethodInfo { name: "trim", signature: "trim()", doc: "Strip whitespace" },
    MethodInfo { name: "trim_start", signature: "trim_start()", doc: "Trim leading whitespace" },
    MethodInfo { name: "trim_end", signature: "trim_end()", doc: "Trim trailing whitespace" },
    MethodInfo { name: "split", signature: "split(delim)", doc: "Split by delimiter" },
    MethodInfo { name: "replace", signature: "replace(from, to)", doc: "Replace occurrences" },
    MethodInfo { name: "to_upper", signature: "to_upper()", doc: "Uppercase" },
    MethodInfo { name: "to_lower", signature: "to_lower()", doc: "Lowercase" },
    MethodInfo { name: "chars", signature: "chars()", doc: "List of characters" },
    MethodInfo { name: "char_len", signature: "char_len()", doc: "Character count (Unicode-aware)" },
    MethodInfo { name: "reverse", signature: "reverse()", doc: "Reversed copy" },
    MethodInfo { name: "find", signature: "find(sub)", doc: "Index of first occurrence" },
    MethodInfo { name: "repeat", signature: "repeat(n)", doc: "Repeat n times" },
    MethodInfo { name: "to_int", signature: "to_int()", doc: "Parse as integer" },
    MethodInfo { name: "to_float", signature: "to_float()", doc: "Parse as float" },
    MethodInfo { name: "to_string", signature: "to_string()", doc: "Convert to string" },
    MethodInfo { name: "pad_start", signature: "pad_start(width, char)", doc: "Pad start to width" },
    MethodInfo { name: "pad_end", signature: "pad_end(width, char)", doc: "Pad end to width" },
    MethodInfo { name: "strip_prefix", signature: "strip_prefix(prefix)", doc: "Remove prefix if present" },
    MethodInfo { name: "strip_suffix", signature: "strip_suffix(suffix)", doc: "Remove suffix if present" },
    // List methods
    MethodInfo { name: "push", signature: "push(val)", doc: "Append value" },
    MethodInfo { name: "pop", signature: "pop()", doc: "Remove last element" },
    MethodInfo { name: "first", signature: "first()", doc: "First element (Option)" },
    MethodInfo { name: "last", signature: "last()", doc: "Last element (Option)" },
    MethodInfo { name: "sort", signature: "sort()", doc: "Sorted copy" },
    MethodInfo { name: "sort_by", signature: "sort_by(fn)", doc: "Sort by key function" },
    MethodInfo { name: "flatten", signature: "flatten()", doc: "Flatten one level" },
    MethodInfo { name: "join", signature: "join(sep)", doc: "Join with separator" },
    MethodInfo { name: "enumerate", signature: "enumerate()", doc: "Index-value tuples" },
    MethodInfo { name: "zip", signature: "zip(other)", doc: "Zip with another list" },
    MethodInfo { name: "map", signature: "map(fn)", doc: "Apply function" },
    MethodInfo { name: "filter", signature: "filter(fn)", doc: "Keep matching elements" },
    MethodInfo { name: "fold", signature: "fold(init, fn)", doc: "Reduce with accumulator" },
    MethodInfo { name: "flat_map", signature: "flat_map(fn)", doc: "Map then flatten" },
    MethodInfo { name: "any", signature: "any(fn)", doc: "True if any match" },
    MethodInfo { name: "all", signature: "all(fn)", doc: "True if all match" },
    MethodInfo { name: "index", signature: "index(val)", doc: "Index of first occurrence" },
    MethodInfo { name: "count", signature: "count(val)", doc: "Count occurrences" },
    MethodInfo { name: "dedup", signature: "dedup()", doc: "Remove consecutive duplicates" },
    MethodInfo { name: "unique", signature: "unique()", doc: "Remove all duplicates" },
    MethodInfo { name: "sum", signature: "sum()", doc: "Sum of elements" },
    MethodInfo { name: "window", signature: "window(n)", doc: "Sliding windows of size n" },
    MethodInfo { name: "chunk", signature: "chunk(n)", doc: "Split into chunks of size n" },
    MethodInfo { name: "reduce", signature: "reduce(fn)", doc: "Reduce list with fn (no initial value)" },
    MethodInfo { name: "min", signature: "min()", doc: "Minimum element" },
    MethodInfo { name: "max", signature: "max()", doc: "Maximum element" },
    // Dict methods
    MethodInfo { name: "keys", signature: "keys()", doc: "List of keys" },
    MethodInfo { name: "values", signature: "values()", doc: "List of values" },
    MethodInfo { name: "entries", signature: "entries()", doc: "List of (key, value) tuples" },
    MethodInfo { name: "contains_key", signature: "contains_key(key)", doc: "Key exists" },
    MethodInfo { name: "get", signature: "get(key)", doc: "Get value by key" },
    MethodInfo { name: "insert", signature: "insert(key, val)", doc: "Insert entry" },
    MethodInfo { name: "remove", signature: "remove(key)", doc: "Remove entry (dict) or element (set)" },
    MethodInfo { name: "merge", signature: "merge(other)", doc: "Merge dicts" },
    MethodInfo { name: "update", signature: "update(other)", doc: "Merge dict (mutating)" },
    MethodInfo { name: "keys_of", signature: "keys_of(val)", doc: "Keys with matching value" },
    // Option/Result methods
    MethodInfo { name: "unwrap", signature: "unwrap()", doc: "Extract value or error" },
    MethodInfo { name: "unwrap_or", signature: "unwrap_or(default)", doc: "Unwrap or return default" },
    MethodInfo { name: "expect", signature: "expect(msg)", doc: "Unwrap or error" },
    MethodInfo { name: "is_some", signature: "is_some()", doc: "True if Some" },
    MethodInfo { name: "is_none", signature: "is_none()", doc: "True if None" },
    MethodInfo { name: "is_ok", signature: "is_ok()", doc: "True if Ok" },
    MethodInfo { name: "is_err", signature: "is_err()", doc: "True if Err" },
    MethodInfo { name: "map_err", signature: "map_err(fn)", doc: "Transform error" },
    MethodInfo { name: "and_then", signature: "and_then(fn)", doc: "Flat-map on Ok/Some" },
    MethodInfo { name: "or_else", signature: "or_else(fn)", doc: "Call fn on Err/None" },
    // Bytes methods
    MethodInfo { name: "to_list", signature: "to_list()", doc: "Convert to list" },
    MethodInfo { name: "to_str", signature: "to_str()", doc: "Decode as UTF-8" },
    MethodInfo { name: "to_hex", signature: "to_hex()", doc: "Hex-encoded string" },
    MethodInfo { name: "slice", signature: "slice(start, end)", doc: "Sub-slice" },
    MethodInfo { name: "bytes", signature: "bytes()", doc: "Byte representation" },
    // Task methods
    MethodInfo { name: "await", signature: "await", doc: "Wait for task" },
    MethodInfo { name: "is_finished", signature: "is_finished()", doc: "Check if done" },
    MethodInfo { name: "cancel", signature: "cancel()", doc: "Cancel task" },
    MethodInfo { name: "is_cancelled", signature: "is_cancelled()", doc: "Check if cancelled" },
    // Channel methods
    MethodInfo { name: "send", signature: "send(val)", doc: "Send value" },
    MethodInfo { name: "recv", signature: "recv()", doc: "Receive value" },
    MethodInfo { name: "try_recv", signature: "try_recv()", doc: "Non-blocking receive" },
    MethodInfo { name: "close", signature: "close()", doc: "Close channel" },
    // Set methods
    MethodInfo { name: "add", signature: "add(val)", doc: "Add element to set" },
    MethodInfo { name: "union", signature: "union(other)", doc: "Union of two sets" },
    MethodInfo { name: "intersection", signature: "intersection(other)", doc: "Intersection of two sets" },
    MethodInfo { name: "difference", signature: "difference(other)", doc: "Difference of two sets" },
    // Cell methods
    MethodInfo { name: "set", signature: "set(val)", doc: "Set cell value" },
];

// ---- Module members (shared by hover and completion) ----

struct ModuleMember {
    name: &'static str,
    signature: &'static str,
    doc: &'static str,
    is_const: bool,
}

fn module_members(module: &str) -> &'static [ModuleMember] {
    match module {
        "math" => MATH_MEMBERS,
        "json" => JSON_MEMBERS,
        "io" => IO_MEMBERS,
        "string" => STRING_MEMBERS,
        _ => &[],
    }
}

const MATH_MEMBERS: &[ModuleMember] = &[
    ModuleMember { name: "PI", signature: "math::PI", doc: "Pi constant (3.14159...)", is_const: true },
    ModuleMember { name: "E", signature: "math::E", doc: "Euler's number (2.71828...)", is_const: true },
    ModuleMember { name: "TAU", signature: "math::TAU", doc: "Tau (2π)", is_const: true },
    ModuleMember { name: "INF", signature: "math::INF", doc: "Positive infinity", is_const: true },
    ModuleMember { name: "NAN", signature: "math::NAN", doc: "Not-a-number", is_const: true },
    ModuleMember { name: "abs", signature: "math::abs(x)", doc: "Absolute value", is_const: false },
    ModuleMember { name: "min", signature: "math::min(a, b)", doc: "Minimum of arguments", is_const: false },
    ModuleMember { name: "max", signature: "math::max(a, b)", doc: "Maximum of arguments", is_const: false },
    ModuleMember { name: "floor", signature: "math::floor(x)", doc: "Floor (round down)", is_const: false },
    ModuleMember { name: "ceil", signature: "math::ceil(x)", doc: "Ceiling (round up)", is_const: false },
    ModuleMember { name: "round", signature: "math::round(x)", doc: "Round to nearest", is_const: false },
    ModuleMember { name: "sqrt", signature: "math::sqrt(x)", doc: "Square root", is_const: false },
    ModuleMember { name: "pow", signature: "math::pow(base, exp)", doc: "Exponentiation", is_const: false },
    ModuleMember { name: "clamp", signature: "math::clamp(x, lo, hi)", doc: "Clamp value to range", is_const: false },
    ModuleMember { name: "sin", signature: "math::sin(x)", doc: "Sine", is_const: false },
    ModuleMember { name: "cos", signature: "math::cos(x)", doc: "Cosine", is_const: false },
    ModuleMember { name: "tan", signature: "math::tan(x)", doc: "Tangent", is_const: false },
    ModuleMember { name: "atan2", signature: "math::atan2(y, x)", doc: "Two-argument arctangent", is_const: false },
    ModuleMember { name: "log", signature: "math::log(x)", doc: "Natural logarithm", is_const: false },
    ModuleMember { name: "log2", signature: "math::log2(x)", doc: "Base-2 logarithm", is_const: false },
    ModuleMember { name: "log10", signature: "math::log10(x)", doc: "Base-10 logarithm", is_const: false },
    ModuleMember { name: "is_nan", signature: "math::is_nan(x)", doc: "Check if NaN", is_const: false },
    ModuleMember { name: "is_inf", signature: "math::is_inf(x)", doc: "Check if infinite", is_const: false },
];

const JSON_MEMBERS: &[ModuleMember] = &[
    ModuleMember { name: "encode", signature: "json::encode(value) -> string", doc: "Value to JSON string", is_const: false },
    ModuleMember { name: "decode", signature: "json::decode(string) -> value", doc: "JSON string to value", is_const: false },
    ModuleMember { name: "pretty", signature: "json::pretty(value) -> string", doc: "Pretty-printed JSON string", is_const: false },
];

const IO_MEMBERS: &[ModuleMember] = &[
    ModuleMember { name: "print", signature: "io::print(...args)", doc: "Print without newline", is_const: false },
    ModuleMember { name: "println", signature: "io::println(...args)", doc: "Print with newline", is_const: false },
    ModuleMember { name: "eprintln", signature: "io::eprintln(...args)", doc: "Print to stderr with newline", is_const: false },
];

const STRING_MEMBERS: &[ModuleMember] = &[
    ModuleMember { name: "join", signature: "string::join(list, sep)", doc: "Join list elements into string with optional separator", is_const: false },
];

const MODULE_NAMES: &[&str] = &["math", "json", "io", "string"];

// ---- Type names (shared by hover and completion) ----

struct TypeInfo {
    name: &'static str,
    doc: &'static str,
}

const TYPES: &[TypeInfo] = &[
    TypeInfo { name: "int", doc: "Integer type" },
    TypeInfo { name: "float", doc: "Floating-point type" },
    TypeInfo { name: "bool", doc: "Boolean type" },
    TypeInfo { name: "string", doc: "String type" },
    TypeInfo { name: "bytes", doc: "Byte string type" },
    TypeInfo { name: "list", doc: "List type (e.g. list<int>)" },
    TypeInfo { name: "dict", doc: "Dictionary type (e.g. dict<string, int>)" },
    TypeInfo { name: "tuple", doc: "Tuple type" },
    TypeInfo { name: "set", doc: "Set type" },
    TypeInfo { name: "fn", doc: "Function type" },
    TypeInfo { name: "cell", doc: "Mutable reference cell type" },
    TypeInfo { name: "any", doc: "Any type (accepts all values)" },
    TypeInfo { name: "Option", doc: "Option type (e.g. Option<int>)" },
    TypeInfo { name: "Result", doc: "Result type (e.g. Result<int, string>)" },
];

// ---- Main ----

fn main() {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let init_result = InitializeResult {
        capabilities,
        server_info: Some(lsp_types::ServerInfo {
            name: "ion-lsp".to_string(),
            version: Some("0.1.0".to_string()),
        }),
    };

    let init_json = serde_json::to_value(init_result).unwrap();
    connection.initialize(init_json).unwrap();

    let mut documents: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap() {
                    break;
                }
                handle_request(&connection, &documents, req);
            }
            Message::Notification(not) => {
                handle_notification(&connection, &mut documents, not);
            }
            Message::Response(_) => {}
        }
    }

    io_threads.join().unwrap();
}

// ---- Request handling ----

fn handle_request(conn: &Connection, documents: &HashMap<Url, String>, req: Request) {
    if req.method == DocumentSymbolRequest::METHOD {
        let (id, params): (RequestId, DocumentSymbolParams) =
            req.extract(DocumentSymbolRequest::METHOD).unwrap();
        let uri = &params.text_document.uri;
        let symbols = if let Some(source) = documents.get(uri) {
            compute_symbols(source)
        } else {
            vec![]
        };
        let result = DocumentSymbolResponse::Nested(symbols);
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == GotoDefinition::METHOD {
        let (id, params): (RequestId, GotoDefinitionParams) =
            req.extract(GotoDefinition::METHOD).unwrap();
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let pos = params.text_document_position_params.position;
        let result = if let Some(source) = documents.get(&uri) {
            handle_goto_definition(source, &uri, pos)
        } else {
            None
        };
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == HoverRequest::METHOD {
        let (id, params): (RequestId, HoverParams) = req.extract(HoverRequest::METHOD).unwrap();
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let result = if let Some(source) = documents.get(uri) {
            handle_hover(source, pos)
        } else {
            None
        };
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == Completion::METHOD {
        let (id, params): (RequestId, CompletionParams) = req.extract(Completion::METHOD).unwrap();
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let result = if let Some(source) = documents.get(uri) {
            handle_completion(source, pos)
        } else {
            CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items: vec![],
            })
        };
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == References::METHOD {
        let (id, params): (RequestId, ReferenceParams) =
            req.extract(References::METHOD).unwrap();
        let uri = params.text_document_position.text_document.uri.clone();
        let pos = params.text_document_position.position;
        let include_decl = params.context.include_declaration;
        let result = documents
            .get(&uri)
            .map(|source| handle_references(source, &uri, pos, include_decl))
            .unwrap_or_default();
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == Rename::METHOD {
        let (id, params): (RequestId, RenameParams) = req.extract(Rename::METHOD).unwrap();
        let uri = params.text_document_position.text_document.uri.clone();
        let pos = params.text_document_position.position;
        let new_name = params.new_name;
        let resp = match documents.get(&uri) {
            Some(source) => match handle_rename(source, &uri, pos, &new_name) {
                Ok(edit) => Response::new_ok(id, serde_json::to_value(edit).unwrap()),
                Err(msg) => Response {
                    id,
                    result: None,
                    error: Some(ResponseError {
                        code: ErrorCode::InvalidParams as i32,
                        message: msg,
                        data: None,
                    }),
                },
            },
            None => Response::new_ok(id, serde_json::Value::Null),
        };
        conn.sender.send(Message::Response(resp)).unwrap();
    }
}

// ---- Go to Definition ----

fn handle_goto_definition(
    source: &str,
    uri: &Url,
    pos: Position,
) -> Option<GotoDefinitionResponse> {
    let word = word_at_position(source, pos.line, pos.character)?;
    let defs = collect_definitions(source);

    for def in &defs {
        if def.name == word {
            let loc = Location {
                uri: uri.clone(),
                range: Range {
                    start: Position::new(def.line, def.col),
                    end: Position::new(def.line, def.col + def.name.len() as u32),
                },
            };
            return Some(GotoDefinitionResponse::Scalar(loc));
        }
    }
    None
}

// ---- Hover ----

fn handle_hover(source: &str, pos: Position) -> Option<Hover> {
    let line_text = source.lines().nth(pos.line as usize)?;
    let col = pos.character as usize;
    let (start, end) = word_range_in_line(line_text, col)?;
    let word = &line_text[start..end];
    let range = Some(line_range(pos.line, line_text, start, end));
    let ctx = prefix_context_at(source, pos.line, pos.character);

    // Cursor sits on a module name immediately before `::name` — show module overview.
    if matches!(ctx, PrefixCtx::Plain)
        && line_text[end..].starts_with("::")
        && MODULE_NAMES.contains(&word)
    {
        return Some(markdown_hover(format_module_overview(word), range));
    }

    match &ctx {
        PrefixCtx::Method => {
            for m in METHODS {
                if m.name == word {
                    return Some(markdown_hover(format_method_doc(m), range));
                }
            }
            // Fall through — `await` also valid as keyword/method.
        }
        PrefixCtx::Module(module) => {
            if let Some(member) = module_members(module).iter().find(|m| m.name == word) {
                return Some(markdown_hover(format_module_member_doc(member), range));
            }
            // Fall through to other lookups.
        }
        PrefixCtx::Plain => {}
    }

    // Builtins (top-level functions like `len`, `range`, `int`, …).
    for bi in BUILTINS {
        if bi.name == word {
            return Some(markdown_hover(
                format!("```ion\n{}\n```\n{}", bi.signature, bi.description),
                range,
            ));
        }
    }

    // Type names (only when not already covered by a builtin with the same name).
    let known_builtin = BUILTINS.iter().any(|b| b.name == word);
    if !known_builtin {
        if let Some(t) = TYPES.iter().find(|t| t.name == word) {
            return Some(markdown_hover(
                format!("```ion\n{}\n```\n{}", t.name, t.doc),
                range,
            ));
        }
    }

    // User-defined functions/variables/parameters.
    let defs = collect_definitions(source);
    if let Some(def) = defs.iter().find(|d| d.name == word) {
        return Some(markdown_hover(format!("```ion\n{}\n```", def.detail), range));
    }

    // Keywords.
    let keyword_doc = match word {
        "let" => Some("Declare a variable. Use `mut` for mutable bindings.\n\n```ion\nlet x = 10;\nlet mut y = 0;\n```"),
        "fn" => Some("Declare a function.\n\n```ion\nfn add(a, b) { a + b }\n```"),
        "if" => Some("Conditional expression.\n\n```ion\nif x > 0 { \"positive\" } else { \"non-positive\" }\n```"),
        "else" => Some("Alternative branch of an `if` expression."),
        "match" => Some("Pattern matching expression.\n\n```ion\nmatch value {\n    Some(x) => x,\n    None => 0,\n}\n```"),
        "for" => Some("Iterate over a collection.\n\n```ion\nfor x in [1, 2, 3] { io::println(x); }\n```"),
        "while" => Some("Loop while condition is true.\n\n```ion\nwhile x < 10 { x += 1; }\n```"),
        "loop" => Some("Infinite loop. Use `break` to exit.\n\n```ion\nlet result = loop { if done { break 42; } };\n```"),
        "spawn" => Some("Spawn a concurrent task.\n\n```ion\nlet t = spawn compute(100);\nlet result = t.await;\n```"),
        "async" => Some("Structured concurrency scope.\n\n```ion\nlet result = async {\n    let t = spawn work();\n    t.await\n};\n```"),
        "select" => Some("Wait for the first of multiple async branches to complete.\n\n```ion\nselect {\n    val = ch.recv() => val,\n    _ = sleep(100) => 0,\n}\n```"),
        "await" => Some("Wait for a task or future to complete.\n\n```ion\nlet result = task.await;\n```"),
        "Some" => Some("`Option` variant containing a value.\n\n```ion\nSome(42)\n```"),
        "None" => Some("`Option` variant representing no value."),
        "Ok" => Some("`Result` variant representing success.\n\n```ion\nOk(42)\n```"),
        "Err" => Some("`Result` variant representing failure.\n\n```ion\nErr(\"something failed\")\n```"),
        "try" => Some("Begin a try/catch block.\n\n```ion\nlet result = try { risky() } catch e { fallback(e) };\n```"),
        "catch" => Some("Handle errors from a try block.\n\n```ion\ntry { risky() } catch e { io::println(e); }\n```"),
        "break" => Some("Exit a loop. Optionally return a value.\n\n```ion\nlet x = loop { break 42; };\n```"),
        "continue" => Some("Skip to the next iteration of a loop."),
        "return" => Some("Return a value from a function early.\n\n```ion\nfn check(x) { if x < 0 { return Err(\"negative\"); } Ok(x) }\n```"),
        "mut" => Some("Mark a binding as mutable.\n\n```ion\nlet mut count = 0;\ncount += 1;\n```"),
        "in" => Some("Used in `for` loops and membership tests.\n\n```ion\nfor x in [1, 2, 3] { io::println(x); }\n```"),
        "use" => Some("Import names from a module.\n\n```ion\nuse math::add;          // single import\nuse math::{add, PI};    // multiple imports\nuse math::*;            // glob import\n```"),
        "true" | "false" => Some("Boolean literal."),
        _ => None,
    };

    keyword_doc.map(|doc| markdown_hover(doc.to_string(), range))
}

fn markdown_hover(value: String, range: Option<Range>) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    }
}

fn format_method_doc(m: &MethodInfo) -> String {
    format!("```ion\n.{}\n```\n{}", m.signature, m.doc)
}

fn format_module_member_doc(m: &ModuleMember) -> String {
    let kind = if m.is_const { "constant" } else { "function" };
    format!("```ion\n{}\n```\n{} — {}", m.signature, kind, m.doc)
}

fn format_module_overview(module: &str) -> String {
    let members = module_members(module);
    let count = members.len();
    let summary = match module {
        "math" => "Mathematical constants and functions.",
        "json" => "JSON encoding and decoding.",
        "io" => "Standard input/output.",
        "string" => "String utilities.",
        _ => "Module.",
    };
    let mut out = format!("**module `{}`** — {}\n\n{} members:\n", module, summary, count);
    for m in members {
        out.push_str(&format!("- `{}`\n", m.signature));
    }
    out
}

// ---- Completion ----

fn handle_completion(source: &str, pos: Position) -> CompletionResponse {
    let mut items = Vec::new();

    // Check if we're after a dot (method completion)
    let line_text = source.lines().nth(pos.line as usize).unwrap_or("");
    let col = pos.character as usize;

    let is_dot_completion = col > 0 && line_text.as_bytes().get(col - 1) == Some(&b'.');

    // Check if we're after a `::` (module member completion)
    let module_prefix = {
        let before_cursor = &line_text[..col.min(line_text.len())];
        if let Some(idx) = before_cursor.rfind("::") {
            let prefix = before_cursor[..idx].trim();
            // Extract the last word as the module name
            let mod_name = prefix
                .rsplit(|c: char| c.is_whitespace() || c == '(' || c == ',' || c == '{')
                .next()
                .unwrap_or(prefix);
            if !mod_name.is_empty() {
                Some(mod_name.to_string())
            } else {
                None
            }
        } else {
            None
        }
    };

    // Check if we're in a type annotation position (after `:` in a let binding)
    let is_type_position = {
        let before_cursor = &line_text[..col.min(line_text.len())];
        let trimmed = before_cursor.trim_start();
        (trimmed.starts_with("let ") || trimmed.starts_with("let mut "))
            && before_cursor.contains(':')
            && !before_cursor.contains('=')
    };

    if let Some(ref mod_name) = module_prefix {
        for member in module_members(mod_name) {
            let kind = if member.is_const {
                CompletionItemKind::CONSTANT
            } else {
                CompletionItemKind::FUNCTION
            };
            items.push(CompletionItem {
                label: member.name.to_string(),
                kind: Some(kind),
                detail: Some(member.signature.to_string()),
                documentation: Some(lsp_types::Documentation::String(member.doc.to_string())),
                ..Default::default()
            });
        }
        return CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        });
    } else if is_type_position {
        for t in TYPES {
            items.push(CompletionItem {
                label: t.name.to_string(),
                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                documentation: Some(lsp_types::Documentation::String(t.doc.to_string())),
                ..Default::default()
            });
        }
    } else if is_dot_completion {
        for m in METHODS {
            items.push(CompletionItem {
                label: m.name.to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some(m.signature.to_string()),
                documentation: Some(lsp_types::Documentation::String(m.doc.to_string())),
                ..Default::default()
            });
        }
    } else {
        // Keywords
        for kw in KEYWORDS {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }

        // Builtins
        for bi in BUILTINS {
            items.push(CompletionItem {
                label: bi.name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(bi.signature.to_string()),
                documentation: Some(lsp_types::Documentation::String(bi.description.to_string())),
                ..Default::default()
            });
        }

        // User definitions
        let defs = collect_definitions(source);
        for def in &defs {
            let kind = match def.kind {
                DefKind::Function => CompletionItemKind::FUNCTION,
                DefKind::Variable => CompletionItemKind::VARIABLE,
            };
            items.push(CompletionItem {
                label: def.name.clone(),
                kind: Some(kind),
                detail: Some(def.detail.clone()),
                ..Default::default()
            });
        }
    }

    CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    })
}

// ---- References & Rename ----

/// Tokenize the source and return the location of every `Token::Ident` whose
/// text equals `name`. Range columns and lines are converted to LSP 0-based
/// coordinates. Tokens inside strings, comments, and other non-identifier
/// constructs are naturally excluded by the lexer.
fn find_identifier_occurrences(source: &str, name: &str) -> Vec<Range> {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for tok in tokens {
        if let Token::Ident(text) = &tok.token {
            if text == name {
                let line = tok.line.saturating_sub(1) as u32;
                let col = tok.col.saturating_sub(1) as u32;
                out.push(Range {
                    start: Position::new(line, col),
                    end: Position::new(line, col + name.len() as u32),
                });
            }
        }
    }
    out
}

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn handle_references(
    source: &str,
    uri: &Url,
    pos: Position,
    _include_decl: bool,
) -> Vec<Location> {
    // The LSP spec lets us omit the declaration when include_declaration is
    // false, but our textual scan gives us all occurrences uniformly and most
    // clients (VSCode, IntelliJ via LSP4IJ) expect the declaration in the
    // results regardless, so we always return everything.
    let Some(name) = word_at_position(source, pos.line, pos.character) else {
        return Vec::new();
    };
    if !is_valid_identifier(&name) {
        return Vec::new();
    }
    find_identifier_occurrences(source, &name)
        .into_iter()
        .map(|range| Location {
            uri: uri.clone(),
            range,
        })
        .collect()
}

fn handle_rename(
    source: &str,
    uri: &Url,
    pos: Position,
    new_name: &str,
) -> Result<WorkspaceEdit, String> {
    if !is_valid_identifier(new_name) {
        return Err(format!("'{new_name}' is not a valid Ion identifier"));
    }
    let Some(old_name) = word_at_position(source, pos.line, pos.character) else {
        return Err("no identifier under cursor".to_string());
    };
    if !is_valid_identifier(&old_name) {
        return Err(format!("'{old_name}' is not a renameable identifier"));
    }
    if new_name == old_name {
        return Ok(WorkspaceEdit::default());
    }
    let occurrences = find_identifier_occurrences(source, &old_name);
    if occurrences.is_empty() {
        return Err(format!("no occurrences of '{old_name}' found"));
    }
    let edits: Vec<TextEdit> = occurrences
        .into_iter()
        .map(|range| TextEdit {
            range,
            new_text: new_name.to_string(),
        })
        .collect();
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Ok(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

// ---- Notification handling ----

fn handle_notification(
    conn: &Connection,
    documents: &mut HashMap<Url, String>,
    not: LspNotification,
) {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params).unwrap();
            let uri = params.text_document.uri.clone();
            let text = params.text_document.text.clone();
            documents.insert(uri.clone(), text.clone());
            publish_diagnostics(conn, uri, &text);
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params).unwrap();
            let uri = params.text_document.uri.clone();
            if let Some(change) = params.content_changes.into_iter().last() {
                documents.insert(uri.clone(), change.text.clone());
                publish_diagnostics(conn, uri, &change.text);
            }
        }
        DidSaveTextDocument::METHOD => {}
        _ => {}
    }
}

// ---- Diagnostics ----

fn publish_diagnostics(conn: &Connection, uri: Url, source: &str) {
    let diagnostics = compute_diagnostics(source);
    let params = lsp_types::PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    let not = LspNotification::new(
        notification::PublishDiagnostics::METHOD.to_string(),
        serde_json::to_value(params).unwrap(),
    );
    conn.sender.send(Message::Notification(not)).unwrap();
}

fn compute_diagnostics(source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            add_error_diagnostic(&mut diagnostics, &err);
            return diagnostics;
        }
    };

    let mut parser = Parser::new(tokens);
    let output = parser.parse_program_recovering();
    for err in &output.errors {
        diagnostics.push(ion_error_to_diagnostic(err));
    }

    diagnostics
}

fn add_error_diagnostic(diagnostics: &mut Vec<Diagnostic>, err: &IonError) {
    diagnostics.push(ion_error_to_diagnostic(err));
    for extra in &err.additional {
        diagnostics.push(ion_error_to_diagnostic(extra));
    }
}

fn ion_error_to_diagnostic(err: &IonError) -> Diagnostic {
    let line = if err.line > 0 { err.line - 1 } else { 0 };
    let col = if err.col > 0 { err.col - 1 } else { 0 };

    Diagnostic {
        range: Range {
            start: Position::new(line as u32, col as u32),
            end: Position::new(line as u32, (col + 1) as u32),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("ion".to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

// ---- Document Symbols ----

fn compute_symbols(source: &str) -> Vec<DocumentSymbol> {
    let defs = collect_definitions(source);
    defs.iter().map(def_to_symbol).collect()
}

#[allow(deprecated)]
fn def_to_symbol(def: &Definition) -> DocumentSymbol {
    let kind = match def.kind {
        DefKind::Function => SymbolKind::FUNCTION,
        DefKind::Variable => SymbolKind::VARIABLE,
    };
    DocumentSymbol {
        name: def.name.clone(),
        detail: Some(def.detail.clone()),
        kind,
        tags: None,
        deprecated: None,
        range: Range {
            start: Position::new(def.line, 0),
            end: Position::new(def.line, 0),
        },
        selection_range: Range {
            start: Position::new(def.line, def.col),
            end: Position::new(def.line, def.col + def.name.len() as u32),
        },
        children: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_uri() -> Url {
        Url::parse("file:///tmp/test.ion").unwrap()
    }

    #[test]
    fn references_finds_all_occurrences_excluding_strings() {
        let src = "let count = 0;\ncount += 1;\nlet msg = \"count me out\";\n";
        // cursor on `count` in line 0
        let refs = handle_references(src, &fake_uri(), Position::new(0, 6), true);
        assert_eq!(refs.len(), 2, "expected 2 hits, got: {:?}", refs);
        assert_eq!(refs[0].range.start, Position::new(0, 4));
        assert_eq!(refs[1].range.start, Position::new(1, 0));
    }

    #[test]
    fn references_skips_comments() {
        let src = "let x = 1;\n// x is a counter\nx + 1\n";
        let refs = handle_references(src, &fake_uri(), Position::new(0, 4), true);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].range.start, Position::new(0, 4));
        assert_eq!(refs[1].range.start, Position::new(2, 0));
    }

    #[test]
    fn references_returns_empty_for_keyword_or_string() {
        let src = "let x = \"hello\";\n";
        // cursor on `let` keyword: word_at_position returns "let", but lexer
        // tokenizes it as Token::Let, not Token::Ident, so no occurrences.
        let refs = handle_references(src, &fake_uri(), Position::new(0, 1), true);
        assert!(refs.is_empty());
    }

    #[test]
    fn rename_rewrites_every_occurrence() {
        let src = "let count = 0;\ncount += 1;\n";
        let edit = handle_rename(src, &fake_uri(), Position::new(0, 6), "tally").unwrap();
        let changes = edit.changes.expect("changes map");
        let edits = changes.get(&fake_uri()).expect("edits for uri");
        assert_eq!(edits.len(), 2);
        for e in edits {
            assert_eq!(e.new_text, "tally");
        }
    }

    #[test]
    fn rename_rejects_invalid_new_name() {
        let src = "let x = 1;\n";
        let err = handle_rename(src, &fake_uri(), Position::new(0, 4), "1abc").unwrap_err();
        assert!(err.contains("not a valid"));
    }

    #[test]
    fn rename_same_name_is_noop() {
        let src = "let x = 1;\nx + 1\n";
        let edit = handle_rename(src, &fake_uri(), Position::new(0, 4), "x").unwrap();
        assert!(edit.changes.is_none() || edit.changes.as_ref().unwrap().is_empty());
    }

    #[test]
    fn rename_errors_when_cursor_not_on_identifier() {
        let src = "let x = 1;\n";
        // column 14 is past end of line
        let err = handle_rename(src, &fake_uri(), Position::new(0, 14), "y").unwrap_err();
        assert!(err.contains("no identifier"));
    }

    #[test]
    fn is_valid_identifier_basics() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("baz_99"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("9foo"));
        assert!(!is_valid_identifier("foo bar"));
        assert!(!is_valid_identifier("foo-bar"));
    }

    fn hover_text(src: &str, line: u32, character: u32) -> String {
        let h = handle_hover(src, Position::new(line, character))
            .expect("expected hover");
        match h.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markdown hover"),
        }
    }

    #[test]
    fn hover_method_after_dot() {
        let src = "let xs = [1, 2, 3];\nxs.push(4);\n";
        // cursor on `push`
        let text = hover_text(src, 1, 4);
        assert!(text.contains("push(val)"), "got: {}", text);
        assert!(text.contains("Append value"), "got: {}", text);
    }

    #[test]
    fn hover_module_member() {
        let src = "let r = math::sqrt(2.0);\n";
        // cursor on `sqrt`
        let text = hover_text(src, 0, 16);
        assert!(text.contains("math::sqrt"), "got: {}", text);
        assert!(text.contains("Square root"), "got: {}", text);
    }

    #[test]
    fn hover_module_constant() {
        let src = "let p = math::PI;\n";
        let text = hover_text(src, 0, 14);
        assert!(text.contains("math::PI"), "got: {}", text);
        assert!(text.contains("constant"), "got: {}", text);
    }

    #[test]
    fn hover_module_name_overview() {
        let src = "let r = math::sqrt(2.0);\n";
        // cursor on `math` (column 8 is the m)
        let text = hover_text(src, 0, 8);
        assert!(text.contains("module `math`"), "got: {}", text);
    }

    #[test]
    fn hover_let_with_initializer() {
        let src = "let count = 42;\ncount + 1;\n";
        // cursor on `count` in line 0
        let text = hover_text(src, 0, 6);
        assert!(text.contains("let count = 42"), "got: {}", text);
    }

    #[test]
    fn hover_let_mut_with_type() {
        let src = "let mut total: int = 0;\n";
        let text = hover_text(src, 0, 10);
        assert!(text.contains("let mut total: int = 0"), "got: {}", text);
    }

    #[test]
    fn hover_function_parameter() {
        let src = "fn double(x) {\n    x * 2\n}\n";
        // cursor on the `x` inside the body
        let text = hover_text(src, 1, 4);
        assert!(text.contains("(parameter) x"), "got: {}", text);
    }

    #[test]
    fn hover_type_name() {
        let src = "let x: Option<int> = Some(1);\n";
        // cursor on `Option`
        let text = hover_text(src, 0, 10);
        assert!(text.contains("Option"), "got: {}", text);
    }

    #[test]
    fn hover_returns_range() {
        let src = "let count = 0;\n";
        let h = handle_hover(src, Position::new(0, 6)).expect("hover");
        let r = h.range.expect("range");
        assert_eq!(r.start, Position::new(0, 4));
        assert_eq!(r.end, Position::new(0, 9));
    }

    #[test]
    fn prefix_context_dot() {
        let src = "xs.push(1)";
        assert_eq!(prefix_context_at(src, 0, 4), PrefixCtx::Method);
    }

    #[test]
    fn prefix_context_module() {
        let src = "math::sqrt(2.0)";
        assert_eq!(
            prefix_context_at(src, 0, 7),
            PrefixCtx::Module("math".into())
        );
    }

    #[test]
    fn prefix_context_plain() {
        let src = "let x = 1;";
        assert_eq!(prefix_context_at(src, 0, 4), PrefixCtx::Plain);
    }

    #[test]
    fn extract_let_initializer_basic() {
        assert_eq!(
            extract_let_initializer("let x = 42;"),
            Some("42".to_string())
        );
        assert_eq!(
            extract_let_initializer("let mut total: int = 0;"),
            Some("0".to_string())
        );
        assert_eq!(
            extract_let_initializer("let pair = (1, 2)"),
            Some("(1, 2)".to_string())
        );
        // Only treat top-level `=` (skip `==`, `!=`, etc.).
        assert_eq!(
            extract_let_initializer("let eq = a == b;"),
            Some("a == b".to_string())
        );
    }
}
