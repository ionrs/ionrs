use std::collections::HashMap;

use ion_core::ast::{Param, StmtKind};
use ion_core::error::IonError;
use ion_core::lexer::Lexer;
use ion_core::parser::Parser;

use lsp_server::{
    Connection, Message, Notification as LspNotification, Request, RequestId, Response,
};
use lsp_types::notification::{
    self, DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as NotificationTrait,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, Request as RequestTrait,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeResult, Location, MarkupContent, MarkupKind, Position,
    Range, ServerCapabilities, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
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
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let mut defs = Vec::new();
    collect_defs_from_stmts(&program.stmts, &mut defs);
    defs
}

fn collect_defs_from_stmts(stmts: &[ion_core::ast::Stmt], defs: &mut Vec<Definition>) {
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
                collect_defs_from_stmts(body, defs);
            }
            StmtKind::Let {
                pattern: ion_core::ast::Pattern::Ident(name),
                ..
            } => {
                defs.push(Definition {
                    name: name.clone(),
                    kind: DefKind::Variable,
                    line,
                    col,
                    detail: format!("let {}", name),
                });
            }
            StmtKind::For { body, .. } => collect_defs_from_stmts(body, defs),
            StmtKind::While { body, .. } => collect_defs_from_stmts(body, defs),
            StmtKind::WhileLet { body, .. } => collect_defs_from_stmts(body, defs),
            StmtKind::Loop { body } => collect_defs_from_stmts(body, defs),
            StmtKind::ExprStmt { expr, .. } => collect_defs_from_expr(expr, defs),
            _ => {}
        }
    }
}

fn collect_defs_from_expr(expr: &ion_core::ast::Expr, defs: &mut Vec<Definition>) {
    use ion_core::ast::ExprKind;
    match &expr.kind {
        ExprKind::If {
            then_body,
            else_body,
            ..
        } => {
            collect_defs_from_stmts(then_body, defs);
            if let Some(eb) = else_body {
                collect_defs_from_stmts(eb, defs);
            }
        }
        ExprKind::IfLet {
            then_body,
            else_body,
            ..
        } => {
            collect_defs_from_stmts(then_body, defs);
            if let Some(eb) = else_body {
                collect_defs_from_stmts(eb, defs);
            }
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                collect_defs_from_expr(&arm.body, defs);
            }
        }
        ExprKind::Block(stmts) => collect_defs_from_stmts(stmts, defs),
        ExprKind::TryCatch { body, handler, .. } => {
            collect_defs_from_stmts(body, defs);
            collect_defs_from_stmts(handler, defs);
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

// ---- Builtins ----

struct BuiltinInfo {
    name: &'static str,
    signature: &'static str,
    description: &'static str,
}

const BUILTINS: &[BuiltinInfo] = &[
    BuiltinInfo {
        name: "print",
        signature: "print(args...)",
        description: "Print without newline",
    },
    BuiltinInfo {
        name: "println",
        signature: "println(args...)",
        description: "Print with newline",
    },
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
        name: "abs",
        signature: "abs(x)",
        description: "Absolute value",
    },
    BuiltinInfo {
        name: "min",
        signature: "min(a, b, ...)",
        description: "Minimum of arguments",
    },
    BuiltinInfo {
        name: "max",
        signature: "max(a, b, ...)",
        description: "Maximum of arguments",
    },
    BuiltinInfo {
        name: "floor",
        signature: "floor(x)",
        description: "Floor (rounds down)",
    },
    BuiltinInfo {
        name: "ceil",
        signature: "ceil(x)",
        description: "Ceiling (rounds up)",
    },
    BuiltinInfo {
        name: "round",
        signature: "round(x)",
        description: "Round to nearest",
    },
    BuiltinInfo {
        name: "sqrt",
        signature: "sqrt(x)",
        description: "Square root",
    },
    BuiltinInfo {
        name: "pow",
        signature: "pow(base, exp)",
        description: "Exponentiation",
    },
    BuiltinInfo {
        name: "clamp",
        signature: "clamp(val, min, max)",
        description: "Clamp value to range [min, max]",
    },
    BuiltinInfo {
        name: "join",
        signature: "join(list, separator)",
        description: "Join list elements into a string with separator",
    },
    BuiltinInfo {
        name: "json_encode",
        signature: "json_encode(value)",
        description: "Value to JSON string",
    },
    BuiltinInfo {
        name: "json_decode",
        signature: "json_decode(string)",
        description: "JSON string to value",
    },
    BuiltinInfo {
        name: "json_encode_pretty",
        signature: "json_encode_pretty(value)",
        description: "Pretty-printed JSON",
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
        name: "sort_by",
        signature: "sort_by(list, fn)",
        description: "Sort list by key function",
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
    BuiltinInfo {
        name: "msgpack_encode",
        signature: "msgpack_encode(value)",
        description: "Encode value to MessagePack bytes (requires msgpack feature)",
    },
    BuiltinInfo {
        name: "msgpack_decode",
        signature: "msgpack_decode(bytes)",
        description: "Decode MessagePack bytes to value (requires msgpack feature)",
    },
];

const KEYWORDS: &[&str] = &[
    "let", "mut", "fn", "if", "else", "while", "for", "loop", "break", "continue", "return",
    "match", "in", "true", "false", "None", "Some", "Ok", "Err", "async", "spawn", "select", "try",
    "catch",
];

// ---- Main ----

fn main() {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
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
    let word = word_at_position(source, pos.line, pos.character)?;

    // Check builtins first
    for bi in BUILTINS {
        if bi.name == word {
            let content = format!("```ion\n{}\n```\n{}", bi.signature, bi.description);
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    // Check user-defined functions/variables
    let defs = collect_definitions(source);
    for def in &defs {
        if def.name == word {
            let content = match def.kind {
                DefKind::Function => format!("```ion\n{}\n```", def.detail),
                DefKind::Variable => format!("```ion\n{}\n```", def.detail),
            };
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    // Check keywords
    let keyword_doc = match word.as_str() {
        "let" => Some("Declare a variable. Use `mut` for mutable bindings.\n\n```ion\nlet x = 10;\nlet mut y = 0;\n```"),
        "fn" => Some("Declare a function.\n\n```ion\nfn add(a, b) { a + b }\n```"),
        "if" => Some("Conditional expression.\n\n```ion\nif x > 0 { \"positive\" } else { \"non-positive\" }\n```"),
        "match" => Some("Pattern matching expression.\n\n```ion\nmatch value {\n    Some(x) => x,\n    None => 0,\n}\n```"),
        "for" => Some("Iterate over a collection.\n\n```ion\nfor x in [1, 2, 3] { println(x); }\n```"),
        "while" => Some("Loop while condition is true.\n\n```ion\nwhile x < 10 { x += 1; }\n```"),
        "loop" => Some("Infinite loop. Use `break` to exit.\n\n```ion\nlet result = loop { if done { break 42; } };\n```"),
        "spawn" => Some("Spawn a concurrent task (requires `concurrency` feature).\n\n```ion\nlet t = spawn compute(100);\nlet result = t.await;\n```"),
        "async" => Some("Structured concurrency scope.\n\n```ion\nlet result = async {\n    let t = spawn work();\n    t.await\n};\n```"),
        "Some" => Some("`Option` variant containing a value.\n\n```ion\nSome(42)\n```"),
        "None" => Some("`Option` variant representing no value."),
        "Ok" => Some("`Result` variant representing success.\n\n```ion\nOk(42)\n```"),
        "Err" => Some("`Result` variant representing failure.\n\n```ion\nErr(\"something failed\")\n```"),
        "try" => Some("Begin a try/catch block.\n\n```ion\nlet result = try { risky() } catch e { fallback(e) };\n```"),
        "catch" => Some("Handle errors from a try block.\n\n```ion\ntry { risky() } catch e { println(e); }\n```"),
        "break" => Some("Exit a loop. Optionally return a value.\n\n```ion\nlet x = loop { break 42; };\n```"),
        "continue" => Some("Skip to the next iteration of a loop."),
        "return" => Some("Return a value from a function early.\n\n```ion\nfn check(x) { if x < 0 { return Err(\"negative\"); } Ok(x) }\n```"),
        "mut" => Some("Mark a binding as mutable.\n\n```ion\nlet mut count = 0;\ncount += 1;\n```"),
        "in" => Some("Used in `for` loops and membership tests.\n\n```ion\nfor x in [1, 2, 3] { println(x); }\n```"),
        _ => None,
    };

    keyword_doc.map(|doc| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

// ---- Completion ----

fn handle_completion(source: &str, pos: Position) -> CompletionResponse {
    let mut items = Vec::new();

    // Check if we're after a dot (method completion)
    let line_text = source.lines().nth(pos.line as usize).unwrap_or("");
    let col = pos.character as usize;

    let is_dot_completion = col > 0 && line_text.as_bytes().get(col - 1) == Some(&b'.');

    // Check if we're in a type annotation position (after `:` in a let binding)
    let is_type_position = {
        let before_cursor = &line_text[..col.min(line_text.len())];
        let trimmed = before_cursor.trim_start();
        (trimmed.starts_with("let ") || trimmed.starts_with("let mut "))
            && before_cursor.contains(':')
            && !before_cursor.contains('=')
    };

    if is_type_position {
        let type_names = [
            ("int", "Integer type"),
            ("float", "Floating-point type"),
            ("bool", "Boolean type"),
            ("string", "String type"),
            ("bytes", "Byte string type"),
            ("list", "List type (e.g. list<int>)"),
            ("dict", "Dictionary type (e.g. dict<string, int>)"),
            ("tuple", "Tuple type"),
            ("set", "Set type"),
            ("fn", "Function type"),
            ("cell", "Mutable reference cell type"),
            ("any", "Any type (accepts all values)"),
            ("Option", "Option type (e.g. Option<int>)"),
            ("Result", "Result type (e.g. Result<int, string>)"),
        ];
        for (name, doc) in type_names {
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                ..Default::default()
            });
        }
    } else if is_dot_completion {
        // Provide method completions
        let methods = [
            // String methods
            ("len", "len()", "Length", CompletionItemKind::METHOD),
            (
                "is_empty",
                "is_empty()",
                "True if empty",
                CompletionItemKind::METHOD,
            ),
            (
                "contains",
                "contains(sub)",
                "Contains substring/element",
                CompletionItemKind::METHOD,
            ),
            (
                "starts_with",
                "starts_with(prefix)",
                "Starts with prefix",
                CompletionItemKind::METHOD,
            ),
            (
                "ends_with",
                "ends_with(suffix)",
                "Ends with suffix",
                CompletionItemKind::METHOD,
            ),
            (
                "trim",
                "trim()",
                "Strip whitespace",
                CompletionItemKind::METHOD,
            ),
            (
                "split",
                "split(delim)",
                "Split by delimiter",
                CompletionItemKind::METHOD,
            ),
            (
                "replace",
                "replace(from, to)",
                "Replace occurrences",
                CompletionItemKind::METHOD,
            ),
            (
                "to_upper",
                "to_upper()",
                "Uppercase",
                CompletionItemKind::METHOD,
            ),
            (
                "to_lower",
                "to_lower()",
                "Lowercase",
                CompletionItemKind::METHOD,
            ),
            (
                "chars",
                "chars()",
                "List of characters",
                CompletionItemKind::METHOD,
            ),
            (
                "reverse",
                "reverse()",
                "Reversed copy",
                CompletionItemKind::METHOD,
            ),
            (
                "find",
                "find(sub)",
                "Index of first occurrence",
                CompletionItemKind::METHOD,
            ),
            (
                "repeat",
                "repeat(n)",
                "Repeat n times",
                CompletionItemKind::METHOD,
            ),
            (
                "to_int",
                "to_int()",
                "Parse as integer",
                CompletionItemKind::METHOD,
            ),
            (
                "to_float",
                "to_float()",
                "Parse as float",
                CompletionItemKind::METHOD,
            ),
            // List methods
            (
                "push",
                "push(val)",
                "Append value",
                CompletionItemKind::METHOD,
            ),
            (
                "pop",
                "pop()",
                "Remove last element",
                CompletionItemKind::METHOD,
            ),
            (
                "first",
                "first()",
                "First element (Option)",
                CompletionItemKind::METHOD,
            ),
            (
                "last",
                "last()",
                "Last element (Option)",
                CompletionItemKind::METHOD,
            ),
            ("sort", "sort()", "Sorted copy", CompletionItemKind::METHOD),
            (
                "flatten",
                "flatten()",
                "Flatten one level",
                CompletionItemKind::METHOD,
            ),
            (
                "join",
                "join(sep)",
                "Join with separator",
                CompletionItemKind::METHOD,
            ),
            (
                "enumerate",
                "enumerate()",
                "Index-value tuples",
                CompletionItemKind::METHOD,
            ),
            (
                "zip",
                "zip(other)",
                "Zip with another list",
                CompletionItemKind::METHOD,
            ),
            (
                "map",
                "map(fn)",
                "Apply function",
                CompletionItemKind::METHOD,
            ),
            (
                "filter",
                "filter(fn)",
                "Keep matching elements",
                CompletionItemKind::METHOD,
            ),
            (
                "fold",
                "fold(init, fn)",
                "Reduce with accumulator",
                CompletionItemKind::METHOD,
            ),
            (
                "flat_map",
                "flat_map(fn)",
                "Map then flatten",
                CompletionItemKind::METHOD,
            ),
            (
                "any",
                "any(fn)",
                "True if any match",
                CompletionItemKind::METHOD,
            ),
            (
                "all",
                "all(fn)",
                "True if all match",
                CompletionItemKind::METHOD,
            ),
            // Dict methods
            ("keys", "keys()", "List of keys", CompletionItemKind::METHOD),
            (
                "values",
                "values()",
                "List of values",
                CompletionItemKind::METHOD,
            ),
            (
                "entries",
                "entries()",
                "List of (key, value) tuples",
                CompletionItemKind::METHOD,
            ),
            (
                "contains_key",
                "contains_key(key)",
                "Key exists",
                CompletionItemKind::METHOD,
            ),
            (
                "get",
                "get(key)",
                "Get value by key",
                CompletionItemKind::METHOD,
            ),
            (
                "insert",
                "insert(key, val)",
                "Insert entry",
                CompletionItemKind::METHOD,
            ),
            (
                "remove",
                "remove(key)",
                "Remove entry",
                CompletionItemKind::METHOD,
            ),
            (
                "merge",
                "merge(other)",
                "Merge dicts",
                CompletionItemKind::METHOD,
            ),
            // New list/string/dict methods
            (
                "index",
                "index(val)",
                "Index of first occurrence",
                CompletionItemKind::METHOD,
            ),
            (
                "count",
                "count(val)",
                "Count occurrences",
                CompletionItemKind::METHOD,
            ),
            (
                "dedup",
                "dedup()",
                "Remove consecutive duplicates",
                CompletionItemKind::METHOD,
            ),
            (
                "unique",
                "unique()",
                "Remove all duplicates",
                CompletionItemKind::METHOD,
            ),
            (
                "sum",
                "sum()",
                "Sum of elements",
                CompletionItemKind::METHOD,
            ),
            (
                "window",
                "window(n)",
                "Sliding windows of size n",
                CompletionItemKind::METHOD,
            ),
            (
                "sort_by",
                "sort_by(fn)",
                "Sort by key function",
                CompletionItemKind::METHOD,
            ),
            (
                "to_string",
                "to_string()",
                "Convert to string",
                CompletionItemKind::METHOD,
            ),
            (
                "pad_start",
                "pad_start(width, char)",
                "Pad start to width",
                CompletionItemKind::METHOD,
            ),
            (
                "pad_end",
                "pad_end(width, char)",
                "Pad end to width",
                CompletionItemKind::METHOD,
            ),
            (
                "strip_prefix",
                "strip_prefix(prefix)",
                "Remove prefix if present",
                CompletionItemKind::METHOD,
            ),
            (
                "strip_suffix",
                "strip_suffix(suffix)",
                "Remove suffix if present",
                CompletionItemKind::METHOD,
            ),
            (
                "char_len",
                "char_len()",
                "Character count (Unicode-aware)",
                CompletionItemKind::METHOD,
            ),
            (
                "bytes",
                "bytes()",
                "Byte representation",
                CompletionItemKind::METHOD,
            ),
            (
                "update",
                "update(other)",
                "Merge dict (mutating)",
                CompletionItemKind::METHOD,
            ),
            (
                "keys_of",
                "keys_of(val)",
                "Keys with matching value",
                CompletionItemKind::METHOD,
            ),
            // Option/Result methods
            (
                "unwrap",
                "unwrap()",
                "Extract value or error",
                CompletionItemKind::METHOD,
            ),
            (
                "unwrap_or",
                "unwrap_or(default)",
                "Unwrap or return default",
                CompletionItemKind::METHOD,
            ),
            (
                "expect",
                "expect(msg)",
                "Unwrap or error",
                CompletionItemKind::METHOD,
            ),
            (
                "is_some",
                "is_some()",
                "True if Some",
                CompletionItemKind::METHOD,
            ),
            (
                "is_none",
                "is_none()",
                "True if None",
                CompletionItemKind::METHOD,
            ),
            ("is_ok", "is_ok()", "True if Ok", CompletionItemKind::METHOD),
            (
                "is_err",
                "is_err()",
                "True if Err",
                CompletionItemKind::METHOD,
            ),
            (
                "map_err",
                "map_err(fn)",
                "Transform error",
                CompletionItemKind::METHOD,
            ),
            (
                "and_then",
                "and_then(fn)",
                "Flat-map on Ok/Some",
                CompletionItemKind::METHOD,
            ),
            (
                "or_else",
                "or_else(fn)",
                "Call fn on Err/None",
                CompletionItemKind::METHOD,
            ),
            // Bytes methods
            (
                "to_list",
                "to_list()",
                "Convert to list",
                CompletionItemKind::METHOD,
            ),
            (
                "to_str",
                "to_str()",
                "Decode as UTF-8",
                CompletionItemKind::METHOD,
            ),
            (
                "to_hex",
                "to_hex()",
                "Hex-encoded string",
                CompletionItemKind::METHOD,
            ),
            (
                "slice",
                "slice(start, end)",
                "Sub-slice",
                CompletionItemKind::METHOD,
            ),
            // Task methods
            (
                "await",
                "await",
                "Wait for task",
                CompletionItemKind::METHOD,
            ),
            (
                "is_finished",
                "is_finished()",
                "Check if done",
                CompletionItemKind::METHOD,
            ),
            (
                "cancel",
                "cancel()",
                "Cancel task",
                CompletionItemKind::METHOD,
            ),
            (
                "is_cancelled",
                "is_cancelled()",
                "Check if cancelled",
                CompletionItemKind::METHOD,
            ),
            // Channel methods
            (
                "send",
                "send(val)",
                "Send value",
                CompletionItemKind::METHOD,
            ),
            (
                "recv",
                "recv()",
                "Receive value",
                CompletionItemKind::METHOD,
            ),
            (
                "try_recv",
                "try_recv()",
                "Non-blocking receive",
                CompletionItemKind::METHOD,
            ),
            (
                "close",
                "close()",
                "Close channel",
                CompletionItemKind::METHOD,
            ),
            // List methods: chunk, reduce, min, max
            (
                "chunk",
                "chunk(n)",
                "Split into chunks of size n",
                CompletionItemKind::METHOD,
            ),
            (
                "reduce",
                "reduce(fn)",
                "Reduce list with fn (no initial value)",
                CompletionItemKind::METHOD,
            ),
            (
                "min",
                "min()",
                "Minimum element",
                CompletionItemKind::METHOD,
            ),
            (
                "max",
                "max()",
                "Maximum element",
                CompletionItemKind::METHOD,
            ),
            // String methods: trim_start, trim_end
            (
                "trim_start",
                "trim_start()",
                "Trim leading whitespace",
                CompletionItemKind::METHOD,
            ),
            (
                "trim_end",
                "trim_end()",
                "Trim trailing whitespace",
                CompletionItemKind::METHOD,
            ),
            // Set methods
            (
                "add",
                "add(val)",
                "Add element to set",
                CompletionItemKind::METHOD,
            ),
            (
                "remove",
                "remove(val)",
                "Remove element from set",
                CompletionItemKind::METHOD,
            ),
            (
                "union",
                "union(other)",
                "Union of two sets",
                CompletionItemKind::METHOD,
            ),
            (
                "intersection",
                "intersection(other)",
                "Intersection of two sets",
                CompletionItemKind::METHOD,
            ),
            (
                "difference",
                "difference(other)",
                "Difference of two sets",
                CompletionItemKind::METHOD,
            ),
            // Cell methods
            (
                "set",
                "set(val)",
                "Set cell value",
                CompletionItemKind::METHOD,
            ),
        ];

        for (label, detail, doc, kind) in methods {
            items.push(CompletionItem {
                label: label.to_string(),
                kind: Some(kind),
                detail: Some(detail.to_string()),
                documentation: Some(lsp_types::Documentation::String(doc.to_string())),
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
    match parser.parse_program() {
        Ok(_) => {}
        Err(err) => {
            add_error_diagnostic(&mut diagnostics, &err);
        }
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
