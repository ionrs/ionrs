use std::collections::HashMap;

use ion_core::ast::{Program, StmtKind};
use ion_core::error::IonError;
use ion_core::lexer::Lexer;
use ion_core::parser::Parser;

use lsp_server::{Connection, Message, Notification as LspNotification, Request, RequestId, Response};
use lsp_types::notification::{
    self, DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as NotificationTrait,
};
use lsp_types::request::{DocumentSymbolRequest, Request as RequestTrait};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    InitializeResult, Position, Range, ServerCapabilities, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

fn main() {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
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
    }
}

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
        DidSaveTextDocument::METHOD => {
            // Re-publish on save (in case of external changes)
        }
        _ => {}
    }
}

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

    // Try lexing
    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            add_error_diagnostic(&mut diagnostics, &err);
            return diagnostics;
        }
    };

    // Try parsing
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

fn compute_symbols(source: &str) -> Vec<DocumentSymbol> {
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

    extract_symbols(&program)
}

#[allow(deprecated)] // DocumentSymbol::deprecated field
fn extract_symbols(program: &Program) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    for stmt in &program.stmts {
        match &stmt.kind {
            StmtKind::FnDecl { name, params, body: _ } => {
                let line = if stmt.span.line > 0 { stmt.span.line - 1 } else { 0 };
                let detail = format!(
                    "({})",
                    params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                );
                symbols.push(DocumentSymbol {
                    name: name.clone(),
                    detail: Some(detail),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    range: Range {
                        start: Position::new(line as u32, 0),
                        end: Position::new(line as u32, 0),
                    },
                    selection_range: Range {
                        start: Position::new(line as u32, 0),
                        end: Position::new(line as u32, name.len() as u32 + 3),
                    },
                    children: None,
                });
            }
            StmtKind::Let { pattern, .. } => {
                let line = if stmt.span.line > 0 { stmt.span.line - 1 } else { 0 };
                // Extract simple variable name from pattern
                let var_name = extract_pattern_name(pattern);
                if let Some(var_name) = var_name {
                    symbols.push(DocumentSymbol {
                        name: var_name.clone(),
                        detail: None,
                        kind: SymbolKind::VARIABLE,
                        tags: None,
                        deprecated: None,
                        range: Range {
                            start: Position::new(line as u32, 0),
                            end: Position::new(line as u32, 0),
                        },
                        selection_range: Range {
                            start: Position::new(line as u32, 0),
                            end: Position::new(line as u32, var_name.len() as u32),
                        },
                        children: None,
                    });
                }
            }
            _ => {}
        }
    }

    symbols
}

fn extract_pattern_name(pattern: &ion_core::ast::Pattern) -> Option<String> {
    match pattern {
        ion_core::ast::Pattern::Ident(name) => Some(name.clone()),
        _ => None,
    }
}
