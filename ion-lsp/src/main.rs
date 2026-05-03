use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ion_core::ast::{Param, StmtKind, UseImports};
use ion_core::error::IonError;
use ion_core::lexer::Lexer;
use ion_core::parser::Parser;
use ion_core::token::Token;
use serde::Deserialize;

use lsp_server::{
    Connection, ErrorCode, Message, Notification as LspNotification, Request, RequestId, Response,
    ResponseError,
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
    HoverProviderCapability, InitializeParams, InitializeResult, Location, MarkupContent,
    MarkupKind, OneOf, Position, Range, ReferenceParams, RenameParams, ServerCapabilities,
    SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
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
                match imports {
                    UseImports::Single(item) => {
                        let binding = item.binding().to_string();
                        let detail = match &item.alias {
                            Some(alias) => {
                                format!("use {}::{} as {}", module_path, item.name, alias)
                            }
                            None => format!("use {}::{}", module_path, item.name),
                        };
                        defs.push(Definition {
                            name: binding,
                            kind: DefKind::Variable,
                            line,
                            col,
                            detail,
                        });
                    }
                    UseImports::Names(items) => {
                        for item in items {
                            let binding = item.binding().to_string();
                            let detail = match &item.alias {
                                Some(alias) => {
                                    format!("use {}::{} as {}", module_path, item.name, alias)
                                }
                                None => format!("use {}::{}", module_path, item.name),
                            };
                            defs.push(Definition {
                                name: binding,
                                kind: DefKind::Variable,
                                line,
                                col,
                                detail,
                            });
                        }
                    }
                    UseImports::Glob => {
                        defs.push(Definition {
                            name: format!("{}::*", module_path),
                            kind: DefKind::Variable,
                            line,
                            col,
                            detail: format!("use {}::*", module_path),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_defs_from_expr(expr: &ion_core::ast::Expr, source: &str, defs: &mut Vec<Definition>) {
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
    let init = source
        .lines()
        .nth(line as usize)
        .and_then(extract_let_initializer);
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
            if next != b'='
                && !matches!(prev, b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/')
            {
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
    /// The word is preceded by `<path>::` (module member).
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
    // Look for `<ident>(::<ident>)*::` ending exactly at `start`.
    if start >= 2 && bytes[start - 1] == b':' && bytes[start - 2] == b':' {
        let mod_end = start - 2;
        let mut mod_start = mod_end;
        while mod_start > 0 && (is_ident_char(bytes[mod_start - 1]) || bytes[mod_start - 1] == b':')
        {
            mod_start -= 1;
        }
        if mod_start < mod_end {
            let path = &line_text[mod_start..mod_end];
            if is_valid_module_path(path) {
                return PrefixCtx::Module(path.to_string());
            }
        }
    }
    PrefixCtx::Plain
}

fn is_valid_module_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .split("::")
            .all(|part| !part.is_empty() && is_valid_identifier(part))
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

const KEYWORDS: &[&str] = &[
    "let", "mut", "fn", "if", "else", "while", "for", "loop", "break", "continue", "return",
    "match", "in", "true", "false", "None", "Some", "Ok", "Err", "async", "spawn", "select", "try",
    "catch", "use",
];

// ---- Documentation catalog ----

#[derive(Debug, Clone)]
struct BuiltinDoc {
    name: String,
    signature: String,
    doc: String,
}

#[derive(Debug, Clone)]
struct MethodDoc {
    name: String,
    signature: String,
    doc: String,
}

#[derive(Debug, Clone)]
struct TypeDoc {
    name: String,
    doc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MemberKind {
    Function,
    Constant,
    Method,
    Type,
    Builtin,
}

#[derive(Debug, Clone)]
struct MemberDoc {
    name: String,
    kind: MemberKind,
    signature: String,
    doc: String,
}

#[derive(Debug, Clone)]
struct ModuleDoc {
    name: String,
    path: String,
    summary: String,
    members: HashMap<String, MemberDoc>,
    modules: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct DocCatalog {
    builtins: HashMap<String, BuiltinDoc>,
    methods: HashMap<String, MethodDoc>,
    types: HashMap<String, TypeDoc>,
    modules: HashMap<String, ModuleDoc>,
}

impl DocCatalog {
    /// Build the catalog from the embedded `ion_core::STDLIB_DOCS_JSON`
    /// manifest. Single source of truth shared with the docs site.
    fn builtins() -> Self {
        let mut catalog = Self {
            builtins: HashMap::new(),
            methods: HashMap::new(),
            types: HashMap::new(),
            modules: HashMap::new(),
        };
        if let Err(err) = parse_doc_manifest(&mut catalog, ion_core::STDLIB_DOCS_JSON) {
            // The embedded manifest is part of the build; a parse failure is
            // a programmer bug, not a user-facing condition.
            panic!("ion-core::STDLIB_DOCS_JSON failed to parse: {err}");
        }
        catalog
    }

    fn for_workspace_roots(roots: &[PathBuf], env_paths: Option<OsString>) -> Self {
        let mut catalog = Self::builtins();
        let paths = discover_doc_manifest_paths(roots, env_paths);
        load_external_doc_paths(&mut catalog, paths);
        catalog
    }

    fn find_builtin(&self, name: &str) -> Option<&BuiltinDoc> {
        self.builtins.get(name)
    }

    fn find_method(&self, name: &str) -> Option<&MethodDoc> {
        self.methods.get(name)
    }

    fn find_type(&self, name: &str) -> Option<&TypeDoc> {
        self.types.get(name)
    }

    fn module(&self, path: &str) -> Option<&ModuleDoc> {
        self.modules.get(path)
    }

    fn member(&self, module_path: &str, member: &str) -> Option<&MemberDoc> {
        self.modules
            .get(module_path)
            .and_then(|module| module.members.get(member))
    }

    fn sorted_builtins(&self) -> Vec<&BuiltinDoc> {
        let mut docs: Vec<_> = self.builtins.values().collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }

    fn sorted_methods(&self) -> Vec<&MethodDoc> {
        let mut docs: Vec<_> = self.methods.values().collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }

    fn sorted_types(&self) -> Vec<&TypeDoc> {
        let mut docs: Vec<_> = self.types.values().collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }

    fn sorted_top_level_modules(&self) -> Vec<&ModuleDoc> {
        let mut docs: Vec<_> = self
            .modules
            .values()
            .filter(|module| !module.path.contains("::"))
            .collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }
}

#[derive(Debug, Deserialize)]
struct IonDocManifest {
    #[serde(rename = "ionDocVersion")]
    ion_doc_version: u32,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    modules: Vec<ManifestModule>,
}

#[derive(Debug, Deserialize)]
struct ManifestModule {
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    members: Vec<ManifestMember>,
    #[serde(default)]
    modules: Vec<ManifestModule>,
}

// receiver/variants/examples/since are deserialized so the schema validates
// strictly, but only the docs site renders them. Keep the fields here so the
// LSP and the site share one definition of the wire format.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ManifestMember {
    name: String,
    kind: MemberKind,
    signature: String,
    doc: String,
    #[serde(default)]
    receiver: Option<String>,
    #[serde(default)]
    methods: Vec<ManifestMember>,
    #[serde(default)]
    variants: Vec<String>,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    since: Option<String>,
}

fn load_external_doc_paths(catalog: &mut DocCatalog, paths: Vec<PathBuf>) {
    for path in paths {
        if let Err(err) = load_doc_manifest(catalog, &path) {
            eprintln!(
                "ion-lsp: warning: failed to load doc manifest {}: {}",
                path.display(),
                err
            );
        }
    }
}

fn load_doc_manifest(catalog: &mut DocCatalog, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    parse_doc_manifest(catalog, &text)
}

fn parse_doc_manifest(catalog: &mut DocCatalog, text: &str) -> Result<(), String> {
    let manifest: IonDocManifest = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if manifest.ion_doc_version != 1 && manifest.ion_doc_version != 2 {
        return Err(format!(
            "unsupported ionDocVersion {}; expected 1 or 2",
            manifest.ion_doc_version
        ));
    }
    // profile/homepage/repository/license/categories are surfaced by the
    // docs site; the LSP keeps them parsed-but-unused so unknown manifests
    // load cleanly.
    let _ = (
        manifest.profile.as_deref(),
        manifest.homepage.as_deref(),
        manifest.repository.as_deref(),
        manifest.license.as_deref(),
        &manifest.categories,
    );
    for module in manifest.modules {
        merge_manifest_module(catalog, None, module);
    }
    Ok(())
}

fn merge_manifest_module(
    catalog: &mut DocCatalog,
    parent_path: Option<&str>,
    manifest: ManifestModule,
) {
    let path = match parent_path {
        Some(parent) => format!("{}::{}", parent, manifest.name),
        None => manifest.name.clone(),
    };

    let module = catalog
        .modules
        .entry(path.clone())
        .or_insert_with(|| ModuleDoc {
            name: manifest.name.clone(),
            path: path.clone(),
            summary: String::new(),
            members: HashMap::new(),
            modules: HashMap::new(),
        });
    module.name = manifest.name.clone();
    if !manifest.summary.is_empty() {
        module.summary = manifest.summary;
    }

    for member in manifest.members {
        match member.kind {
            MemberKind::Function | MemberKind::Constant => {
                module.members.insert(
                    member.name.clone(),
                    MemberDoc {
                        name: member.name,
                        kind: member.kind,
                        signature: member.signature,
                        doc: member.doc,
                    },
                );
            }
            MemberKind::Method => {
                catalog.methods.insert(
                    member.name.clone(),
                    MethodDoc {
                        name: member.name,
                        signature: member.signature,
                        doc: member.doc,
                    },
                );
            }
            MemberKind::Type => {
                catalog.types.insert(
                    member.name.clone(),
                    TypeDoc {
                        name: member.name.clone(),
                        doc: member.doc,
                    },
                );
                for nested in member.methods {
                    catalog.methods.insert(
                        nested.name.clone(),
                        MethodDoc {
                            name: nested.name,
                            signature: nested.signature,
                            doc: nested.doc,
                        },
                    );
                }
            }
            MemberKind::Builtin => {
                catalog.builtins.insert(
                    member.name.clone(),
                    BuiltinDoc {
                        name: member.name,
                        signature: member.signature,
                        doc: member.doc,
                    },
                );
            }
        }
    }

    let child_modules = manifest.modules;
    for child in child_modules {
        let child_name = child.name.clone();
        let child_path = format!("{}::{}", path, child_name);
        catalog
            .modules
            .get_mut(&path)
            .expect("parent module exists")
            .modules
            .insert(child_name, child_path);
        merge_manifest_module(catalog, Some(&path), child);
    }
}

fn discover_doc_manifest_paths(roots: &[PathBuf], env_paths: Option<OsString>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        paths.extend(read_json_files(&root.join(".ion").join("ion-docs")));
        paths.extend(read_json_files(&root.join("ion-docs")));
    }
    if let Some(env_paths) = env_paths {
        for path in std::env::split_paths(&env_paths) {
            if path.is_dir() {
                paths.extend(read_json_files(&path));
            } else {
                paths.push(path);
            }
        }
    }
    paths
}

fn read_json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .collect();
    paths.sort();
    paths
}

fn workspace_roots_from_initialize(params: Option<&InitializeParams>) -> Vec<PathBuf> {
    if let Some(params) = params {
        if let Some(folders) = &params.workspace_folders {
            let roots: Vec<_> = folders
                .iter()
                .filter_map(|folder| folder.uri.to_file_path().ok())
                .collect();
            if !roots.is_empty() {
                return roots;
            }
        }
        #[allow(deprecated)]
        if let Some(root_uri) = &params.root_uri {
            if let Ok(path) = root_uri.to_file_path() {
                return vec![path];
            }
        }
    }
    std::env::current_dir()
        .map(|path| vec![path])
        .unwrap_or_default()
}

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
            trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let (initialize_id, init_params_value) = connection.initialize_start().unwrap();
    let init_params = serde_json::from_value::<InitializeParams>(init_params_value).ok();

    let init_result = InitializeResult {
        capabilities,
        server_info: Some(lsp_types::ServerInfo {
            name: "ion-lsp".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    connection
        .initialize_finish(initialize_id, serde_json::to_value(&init_result).unwrap())
        .unwrap();
    let workspace_roots = workspace_roots_from_initialize(init_params.as_ref());
    let catalog =
        DocCatalog::for_workspace_roots(&workspace_roots, std::env::var_os("ION_LSP_DOCS"));

    let mut documents: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap() {
                    break;
                }
                handle_request(&connection, &documents, &catalog, req);
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

fn handle_request(
    conn: &Connection,
    documents: &HashMap<Url, String>,
    catalog: &DocCatalog,
    req: Request,
) {
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
            handle_hover(source, pos, catalog)
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
            handle_completion(source, pos, catalog)
        } else {
            CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items: vec![],
            })
        };
        let resp = Response::new_ok(id, serde_json::to_value(result).unwrap());
        conn.sender.send(Message::Response(resp)).unwrap();
    } else if req.method == References::METHOD {
        let (id, params): (RequestId, ReferenceParams) = req.extract(References::METHOD).unwrap();
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

fn handle_hover(source: &str, pos: Position, catalog: &DocCatalog) -> Option<Hover> {
    let line_text = source.lines().nth(pos.line as usize)?;
    let col = pos.character as usize;
    let (start, end) = word_range_in_line(line_text, col)?;
    let word = &line_text[start..end];
    let range = Some(line_range(pos.line, line_text, start, end));
    let ctx = prefix_context_at(source, pos.line, pos.character);

    // Cursor sits on a module name immediately before `::name` — show module overview.
    if matches!(ctx, PrefixCtx::Plain)
        && line_text[end..].starts_with("::")
        && catalog.module(word).is_some()
    {
        if let Some(module) = catalog.module(word) {
            return Some(markdown_hover(format_module_overview(module), range));
        }
    }

    match &ctx {
        PrefixCtx::Method => {
            if let Some(method) = catalog.find_method(word) {
                return Some(markdown_hover(format_method_doc(method), range));
            }
            // Fall through — `await` also valid as keyword/method.
        }
        PrefixCtx::Module(module) => {
            if let Some(member) = catalog.member(module, word) {
                return Some(markdown_hover(format_module_member_doc(member), range));
            }
            // Fall through to other lookups.
        }
        PrefixCtx::Plain => {}
    }

    // Builtins (top-level functions like `len`, `range`, `int`, …).
    if let Some(builtin) = catalog.find_builtin(word) {
        return Some(markdown_hover(
            format!("```ion\n{}\n```\n{}", builtin.signature, builtin.doc),
            range,
        ));
    }

    // Type names (only when not already covered by a builtin with the same name).
    let known_builtin = catalog.find_builtin(word).is_some();
    if !known_builtin {
        if let Some(t) = catalog.find_type(word) {
            return Some(markdown_hover(
                format!("```ion\n{}\n```\n{}", t.name, t.doc),
                range,
            ));
        }
    }

    // User-defined functions/variables/parameters.
    let defs = collect_definitions(source);
    if let Some(def) = defs.iter().find(|d| d.name == word) {
        return Some(markdown_hover(
            format!("```ion\n{}\n```", def.detail),
            range,
        ));
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
        "use" => Some("Import names from a module. Imports may be aliased with `as`.\n\n```ion\nuse math::add;             // single import\nuse math::add as sum;      // single import with alias\nuse math::{add, PI};       // multiple imports\nuse math::{add as sum, PI};// multiple imports, some aliased\nuse math::*;               // glob import (cannot be aliased)\n```"),
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

fn format_method_doc(m: &MethodDoc) -> String {
    format!("```ion\n.{}\n```\n{}", m.signature, m.doc)
}

fn format_module_member_doc(m: &MemberDoc) -> String {
    let kind = match m.kind {
        MemberKind::Function => "function",
        MemberKind::Constant => "constant",
        MemberKind::Method | MemberKind::Type | MemberKind::Builtin => unreachable!(
            "method/type/builtin kinds are routed out of module.members in load_doc_manifest"
        ),
    };
    format!("```ion\n{}\n```\n{} — {}", m.signature, kind, m.doc)
}

fn format_module_overview(module: &ModuleDoc) -> String {
    let mut members: Vec<_> = module.members.values().collect();
    members.sort_by(|a, b| a.name.cmp(&b.name));
    let mut modules: Vec<_> = module.modules.keys().collect();
    modules.sort();
    let count = members.len() + modules.len();
    let summary = if module.summary.is_empty() {
        "Module."
    } else {
        &module.summary
    };
    let mut out = format!(
        "**module `{}`** — {}\n\n{} members:\n",
        module.path, summary, count
    );
    for child in modules {
        out.push_str(&format!("- `{}::{}`\n", module.path, child));
    }
    for m in members {
        out.push_str(&format!("- `{}`\n", m.signature));
    }
    out
}

// ---- Completion ----

fn handle_completion(source: &str, pos: Position, catalog: &DocCatalog) -> CompletionResponse {
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
        if let Some(module) = catalog.module(mod_name) {
            let mut child_modules: Vec<_> = module.modules.keys().collect();
            child_modules.sort();
            for child in child_modules {
                let child_doc = module
                    .modules
                    .get(child)
                    .and_then(|path| catalog.module(path));
                items.push(CompletionItem {
                    label: child.to_string(),
                    kind: Some(CompletionItemKind::MODULE),
                    detail: child_doc.map(|doc| format!("module {}", doc.path)),
                    documentation: child_doc
                        .filter(|doc| !doc.summary.is_empty())
                        .map(|doc| lsp_types::Documentation::String(doc.summary.clone())),
                    ..Default::default()
                });
            }

            let mut members: Vec<_> = module.members.values().collect();
            members.sort_by(|a, b| a.name.cmp(&b.name));
            for member in members {
                let kind = match member.kind {
                    MemberKind::Function => CompletionItemKind::FUNCTION,
                    MemberKind::Constant => CompletionItemKind::CONSTANT,
                    MemberKind::Method | MemberKind::Type | MemberKind::Builtin => {
                        unreachable!(
                            "method/type/builtin kinds are routed out of module.members in load_doc_manifest"
                        )
                    }
                };
                items.push(CompletionItem {
                    label: member.name.to_string(),
                    kind: Some(kind),
                    detail: Some(member.signature.to_string()),
                    documentation: Some(lsp_types::Documentation::String(member.doc.to_string())),
                    ..Default::default()
                });
            }
        }
        return CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        });
    } else if is_type_position {
        for t in catalog.sorted_types() {
            items.push(CompletionItem {
                label: t.name.to_string(),
                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                documentation: Some(lsp_types::Documentation::String(t.doc.to_string())),
                ..Default::default()
            });
        }
    } else if is_dot_completion {
        for m in catalog.sorted_methods() {
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
        for bi in catalog.sorted_builtins() {
            items.push(CompletionItem {
                label: bi.name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(bi.signature.to_string()),
                documentation: Some(lsp_types::Documentation::String(bi.doc.to_string())),
                ..Default::default()
            });
        }

        // Modules
        for module in catalog.sorted_top_level_modules() {
            items.push(CompletionItem {
                label: module.name.to_string(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some(format!("module {}", module.path)),
                documentation: if module.summary.is_empty() {
                    None
                } else {
                    Some(lsp_types::Documentation::String(module.summary.clone()))
                },
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

fn handle_references(source: &str, uri: &Url, pos: Position, _include_decl: bool) -> Vec<Location> {
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
            end: Position::new(def.line + 1, 0),
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

    fn test_catalog() -> DocCatalog {
        DocCatalog::builtins()
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
        let catalog = test_catalog();
        let h =
            handle_hover(src, Position::new(line, character), &catalog).expect("expected hover");
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
        let catalog = test_catalog();
        let h = handle_hover(src, Position::new(0, 6), &catalog).expect("hover");
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

    fn fixture_manifest() -> &'static str {
        r#"{
  "ionDocVersion": 1,
  "profile": "ivex.sensor-runtime",
  "modules": [
    {
      "name": "sensor",
      "summary": "Sensor runtime control plane, KEX sessions, jobs, artifacts, tunnels, and supervisor helpers.",
      "members": [
        {
          "name": "call",
          "kind": "function",
          "signature": "sensor::call(method, params?, options?) -> Result<value, dict>",
          "doc": "Calls a sensor-api method through the current or selected KEX session."
        },
        {
          "name": "version",
          "kind": "constant",
          "signature": "sensor::version",
          "doc": "Sensor runtime API version."
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
}"#
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ion-lsp-doc-test-{}-{}-{}",
            std::process::id(),
            name,
            nanos
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn catalog_with_fixture() -> DocCatalog {
        let root = temp_workspace("fixture");
        let docs = root.join(".ion").join("ion-docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("ivex.json"), fixture_manifest()).unwrap();
        DocCatalog::for_workspace_roots(&[root], None)
    }

    fn completion_items(
        catalog: &DocCatalog,
        source: &str,
        line: u32,
        character: u32,
    ) -> Vec<CompletionItem> {
        match handle_completion(source, Position::new(line, character), catalog) {
            CompletionResponse::List(list) => list.items,
            CompletionResponse::Array(items) => items,
        }
    }

    #[test]
    fn loads_fixture_manifest() {
        let catalog = catalog_with_fixture();
        let sensor = catalog.module("sensor").expect("sensor module");
        assert_eq!(sensor.summary, "Sensor runtime control plane, KEX sessions, jobs, artifacts, tunnels, and supervisor helpers.");
        assert!(sensor.members.contains_key("call"));
        assert!(catalog.module("sensor::session").is_some());
    }

    #[test]
    fn completion_includes_external_top_level_module() {
        let catalog = catalog_with_fixture();
        let items = completion_items(&catalog, "", 0, 0);
        let sensor = items
            .iter()
            .find(|item| item.label == "sensor")
            .expect("sensor completion");
        assert_eq!(sensor.kind, Some(CompletionItemKind::MODULE));
    }

    #[test]
    fn hover_external_module_member() {
        let catalog = catalog_with_fixture();
        let src = "sensor::call(\"status\");";
        let h = handle_hover(src, Position::new(0, 9), &catalog).expect("hover");
        let HoverContents::Markup(markup) = h.contents else {
            panic!("expected markdown hover");
        };
        assert!(
            markup.value.contains("sensor::call"),
            "got: {}",
            markup.value
        );
        assert!(
            markup.value.contains("Calls a sensor-api method"),
            "got: {}",
            markup.value
        );
    }

    #[test]
    fn completion_for_nested_external_module() {
        let catalog = catalog_with_fixture();
        let src = "sensor::session::";
        let items = completion_items(&catalog, src, 0, src.len() as u32);
        let set = items
            .iter()
            .find(|item| item.label == "set")
            .expect("set completion");
        assert_eq!(set.kind, Some(CompletionItemKind::FUNCTION));
        assert!(
            set.detail
                .as_ref()
                .is_some_and(|detail| detail.contains("sensor::session::set")),
            "got: {:?}",
            set.detail
        );
    }

    #[test]
    fn invalid_or_missing_manifest_does_not_panic() {
        let root = temp_workspace("invalid");
        let invalid = root.join("invalid.json");
        std::fs::write(&invalid, "{not json").unwrap();
        let missing = root.join("missing.json");
        let mut catalog = DocCatalog::builtins();
        load_external_doc_paths(&mut catalog, vec![invalid, missing]);
        assert!(catalog.find_builtin("len").is_some());
    }

    #[test]
    fn existing_stdlib_completions_still_work() {
        let catalog = test_catalog();
        let items = completion_items(&catalog, "", 0, 0);
        assert!(items.iter().any(|item| item.label == "len"));

        let module_items = completion_items(&catalog, "math::", 0, 6);
        assert!(module_items.iter().any(|item| item.label == "sqrt"));
    }

    #[test]
    fn v2_manifest_loads_methods_and_types() {
        let v2 = r#"{
  "ionDocVersion": 2,
  "homepage": "https://example.com",
  "repository": "https://github.com/example/pkg",
  "license": "MIT",
  "categories": ["serde", "encoding"],
  "modules": [
    {
      "name": "widget",
      "summary": "Widget toolkit.",
      "members": [
        {
          "name": "render",
          "kind": "function",
          "signature": "widget::render(w) -> string",
          "doc": "Render a widget to a string.",
          "examples": ["widget::render(button)"],
          "since": "0.1.0"
        },
        {
          "name": "Color",
          "kind": "type",
          "signature": "Color",
          "doc": "An RGB color.",
          "variants": ["Red", "Green", "Blue"],
          "methods": [
            {
              "name": "to_hex",
              "kind": "method",
              "signature": "Color.to_hex() -> string",
              "doc": "Render as `#rrggbb`.",
              "receiver": "Color"
            }
          ]
        },
        {
          "name": "tap",
          "kind": "method",
          "signature": "x.tap(fn) -> x",
          "doc": "Run `fn(x)` for side effects, return `x`.",
          "receiver": "any"
        }
      ]
    }
  ]
}"#;
        let mut catalog = DocCatalog::builtins();
        parse_doc_manifest(&mut catalog, v2).expect("v2 manifest parses");

        let widget = catalog.module("widget").expect("widget module");
        assert_eq!(widget.summary, "Widget toolkit.");
        assert!(widget.members.contains_key("render"));
        // Methods and types are routed out of module.members:
        assert!(!widget.members.contains_key("tap"));
        assert!(!widget.members.contains_key("Color"));
        // ...into the catalog-wide tables:
        assert!(catalog.find_method("tap").is_some());
        assert!(catalog.find_method("to_hex").is_some());
        assert!(catalog.find_type("Color").is_some());
    }

    #[test]
    fn v1_manifest_still_loads_under_v2_loader() {
        let mut catalog = DocCatalog::builtins();
        parse_doc_manifest(&mut catalog, fixture_manifest()).expect("v1 manifests remain accepted");
        assert!(catalog.module("sensor").is_some());
        assert!(catalog.module("sensor::session").is_some());
    }

    #[test]
    fn unsupported_manifest_version_is_rejected() {
        let v3 = r#"{"ionDocVersion": 3, "modules": []}"#;
        let mut catalog = DocCatalog::builtins();
        let err = parse_doc_manifest(&mut catalog, v3).expect_err("v3 should error");
        assert!(err.contains("ionDocVersion"));
    }
}
