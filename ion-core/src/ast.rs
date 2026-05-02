/// Represents a location in source for error reporting.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

/// Top-level program: a list of statements.
#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    /// `let [mut] pattern [: type] = expr;`
    Let {
        mutable: bool,
        pattern: Pattern,
        type_ann: Option<TypeAnn>,
        value: Expr,
    },
    /// Expression statement (with trailing semicolon = discards value)
    ExprStmt { expr: Expr, has_semi: bool },
    /// `fn name(params) { body }`
    FnDecl {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    /// `['label:] for pattern in expr { body }`
    For {
        label: Option<String>,
        pattern: Pattern,
        iter: Expr,
        body: Vec<Stmt>,
    },
    /// `['label:] while cond { body }`
    While {
        label: Option<String>,
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// `['label:] while let pattern = expr { body }`
    WhileLet {
        label: Option<String>,
        pattern: Pattern,
        expr: Expr,
        body: Vec<Stmt>,
    },
    /// `['label:] loop { body }`
    Loop {
        label: Option<String>,
        body: Vec<Stmt>,
    },
    /// `break ['label] [expr];`
    Break {
        label: Option<String>,
        value: Option<Expr>,
    },
    /// `continue ['label];`
    Continue { label: Option<String> },
    /// `use module::{name1, name2}` or `use module::*`
    Use {
        path: Vec<String>,
        imports: UseImports,
    },
    /// `return [expr];`
    Return { value: Option<Expr> },
    /// Assignment: `lhs = rhs;` or `lhs += rhs;`
    Assign {
        target: AssignTarget,
        op: AssignOp,
        value: Expr,
    },
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Ident(String),
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
}

#[derive(Debug, Clone, Copy)]
pub enum AssignOp {
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    // Literals
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    /// f-string: list of parts (literal strings and expressions)
    FStr(Vec<FStrPart>),
    Bytes(Vec<u8>),
    None,
    Unit,

    // Variables
    Ident(String),
    /// `module::path::name` — module path access
    ModulePath(Vec<String>),

    // Constructors
    /// `Some(expr)`
    SomeExpr(Box<Expr>),
    /// `Ok(expr)`
    OkExpr(Box<Expr>),
    /// `Err(expr)`
    ErrExpr(Box<Expr>),

    // Collections
    /// `[a, b, ...c]` with optional spread entries
    List(Vec<ListEntry>),
    /// `#{ "key": val, ... }` with optional spread entries
    Dict(Vec<DictEntry>),
    /// `(a, b, c)`
    Tuple(Vec<Expr>),
    /// `[expr for pattern in iter if cond]`
    ListComp {
        expr: Box<Expr>,
        pattern: Pattern,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
    },
    /// `#{ key: val for pattern in iter if cond }`
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        pattern: Pattern,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
    },

    // Operations
    BinOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    /// `expr?`
    Try(Box<Expr>),
    /// `a |> b`
    PipeOp {
        left: Box<Expr>,
        right: Box<Expr>,
    },

    // Access
    /// `expr.field`
    FieldAccess {
        expr: Box<Expr>,
        field: String,
    },
    /// `expr[index]`
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    /// `expr[start..end]`, `expr[..end]`, `expr[start..]`
    Slice {
        expr: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
    },
    /// `expr.method(args)`
    MethodCall {
        expr: Box<Expr>,
        method: String,
        args: Vec<CallArg>,
    },

    // Functions
    /// `func(args)`
    Call {
        func: Box<Expr>,
        args: Vec<CallArg>,
    },
    /// `|params| body`
    Lambda {
        params: Vec<String>,
        body: Box<Expr>,
    },

    // Control flow (all are expressions)
    /// `if cond { then } [else { else_ }]`
    If {
        cond: Box<Expr>,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    /// `if let pattern = expr { then } [else { else_ }]`
    IfLet {
        pattern: Pattern,
        expr: Box<Expr>,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    /// `match expr { arms }`
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `{ stmts }`
    Block(Vec<Stmt>),
    /// `loop { body }` as expression (returns break value)
    LoopExpr(Vec<Stmt>),
    /// `try { body } catch ident { handler }`
    TryCatch {
        body: Vec<Stmt>,
        var: String,
        handler: Vec<Stmt>,
    },

    // Host type constructor: `TypeName { field: val, ... }` or `TypeName { ...spread, field: val }`
    StructConstruct {
        name: String,
        fields: Vec<(String, Expr)>,
        spread: Option<Box<Expr>>,
    },

    // Enum variant access: `Enum::Variant` or `Enum::Variant(args)`
    EnumVariant {
        enum_name: String,
        variant: String,
    },
    EnumVariantCall {
        enum_name: String,
        variant: String,
        args: Vec<Expr>,
    },

    // Range
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },

    // Concurrency
    /// `async { body }` — structured concurrency scope
    AsyncBlock(Vec<Stmt>),
    /// `spawn expr` — launch a child task, returns Task handle
    SpawnExpr(Box<Expr>),
    /// `expr.await` — wait for a task/future result
    AwaitExpr(Box<Expr>),
    /// `select { branch => expr, ... }` — race multiple async expressions
    SelectExpr(Vec<SelectBranch>),
}

/// A list entry: either a single element or a spread `...expr`.
#[derive(Debug, Clone)]
pub enum ListEntry {
    Elem(Expr),
    Spread(Expr),
}

/// A dict entry: either a key-value pair or a spread `...expr`.
#[derive(Debug, Clone)]
pub enum DictEntry {
    KeyValue(Expr, Expr),
    Spread(Expr),
}

#[derive(Debug, Clone)]
pub enum FStrPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Expr,
}

/// A branch in a `select {}` expression.
#[derive(Debug, Clone)]
pub struct SelectBranch {
    pub pattern: Pattern,
    pub future_expr: Expr,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Ident(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    None,
    /// `Some(pattern)`
    Some(Box<Pattern>),
    /// `Ok(pattern)`
    Ok(Box<Pattern>),
    /// `Err(pattern)`
    Err(Box<Pattern>),
    /// `(a, b, c)`
    Tuple(Vec<Pattern>),
    /// `[a, b, ...rest]`
    List(Vec<Pattern>, Option<Box<Pattern>>),
    /// `EnumName::Variant` or `EnumName::Variant(patterns)` or `EnumName::Variant { fields }`
    EnumVariant {
        enum_name: String,
        variant: String,
        fields: EnumPatternFields,
    },
    /// `StructName { field1, field2: pattern }`
    Struct {
        name: String,
        fields: Vec<(String, Option<Pattern>)>,
    },
}

#[derive(Debug, Clone)]
pub enum EnumPatternFields {
    None,
    Positional(Vec<Pattern>),
    Named(Vec<(String, Option<Pattern>)>),
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// Optional type annotation for `let` bindings.
/// Inner/generic types are parsed but only the outer type is checked at runtime
/// (e.g. `list<int>` checks that the value is a list, not that elements are ints).
#[derive(Debug, Clone)]
pub enum TypeAnn {
    Simple(String),                     // int, float, bool, string, list, dict, set, etc.
    Option(Box<TypeAnn>),               // Option<T>
    Result(Box<TypeAnn>, Box<TypeAnn>), // Result<T, E>
    List(Box<TypeAnn>),                 // list<T>
    Dict(Box<TypeAnn>, Box<TypeAnn>),   // dict<K, V>
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// A single name imported by a `use` statement, optionally aliased.
///
/// `name` is the member to look up in the module dict; `alias` (when set)
/// is the local binding name introduced into scope.
#[derive(Debug, Clone)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

impl ImportItem {
    /// The local binding name: alias if present, otherwise the original name.
    pub fn binding(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// What to import from a module path.
#[derive(Debug, Clone)]
pub enum UseImports {
    /// `use path::*` — import all names. Glob imports cannot be aliased.
    Glob,
    /// `use path::{a, b as c}` — import specific names, each optionally aliased.
    Names(Vec<ImportItem>),
    /// `use path::name` or `use path::name as alias` — import a single name.
    Single(ImportItem),
}
