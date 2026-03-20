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
    /// `let [mut] pattern = expr;`
    Let { mutable: bool, pattern: Pattern, value: Expr },
    /// Expression statement (with trailing semicolon = discards value)
    ExprStmt { expr: Expr, has_semi: bool },
    /// `fn name(params) { body }`
    FnDecl { name: String, params: Vec<Param>, body: Vec<Stmt> },
    /// `for pattern in expr { body }`
    For { pattern: Pattern, iter: Expr, body: Vec<Stmt> },
    /// `while cond { body }`
    While { cond: Expr, body: Vec<Stmt> },
    /// `while let pattern = expr { body }`
    WhileLet { pattern: Pattern, expr: Expr, body: Vec<Stmt> },
    /// `loop { body }`
    Loop { body: Vec<Stmt> },
    /// `break [expr];`
    Break { value: Option<Expr> },
    /// `continue;`
    Continue,
    /// `return [expr];`
    Return { value: Option<Expr> },
    /// Assignment: `lhs = rhs;` or `lhs += rhs;`
    Assign { target: AssignTarget, op: AssignOp, value: Expr },
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
    None,
    Unit,

    // Variables
    Ident(String),

    // Constructors
    /// `Some(expr)`
    SomeExpr(Box<Expr>),
    /// `Ok(expr)`
    OkExpr(Box<Expr>),
    /// `Err(expr)`
    ErrExpr(Box<Expr>),

    // Collections
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `#{ "key": val, ... }` with optional spread entries
    Dict(Vec<DictEntry>),
    /// `(a, b, c)`
    Tuple(Vec<Expr>),
    /// `[expr for pattern in iter if cond]`
    ListComp { expr: Box<Expr>, pattern: Pattern, iter: Box<Expr>, cond: Option<Box<Expr>> },
    /// `#{ key: val for pattern in iter if cond }`
    DictComp { key: Box<Expr>, value: Box<Expr>, pattern: Pattern, iter: Box<Expr>, cond: Option<Box<Expr>> },

    // Operations
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    UnaryOp { op: UnaryOp, expr: Box<Expr> },
    /// `expr?`
    Try(Box<Expr>),
    /// `a |> b`
    PipeOp { left: Box<Expr>, right: Box<Expr> },

    // Access
    /// `expr.field`
    FieldAccess { expr: Box<Expr>, field: String },
    /// `expr[index]`
    Index { expr: Box<Expr>, index: Box<Expr> },
    /// `expr.method(args)`
    MethodCall { expr: Box<Expr>, method: String, args: Vec<CallArg> },

    // Functions
    /// `func(args)`
    Call { func: Box<Expr>, args: Vec<CallArg> },
    /// `|params| body`
    Lambda { params: Vec<String>, body: Box<Expr> },

    // Control flow (all are expressions)
    /// `if cond { then } [else { else_ }]`
    If { cond: Box<Expr>, then_body: Vec<Stmt>, else_body: Option<Vec<Stmt>> },
    /// `if let pattern = expr { then } [else { else_ }]`
    IfLet { pattern: Pattern, expr: Box<Expr>, then_body: Vec<Stmt>, else_body: Option<Vec<Stmt>> },
    /// `match expr { arms }`
    Match { expr: Box<Expr>, arms: Vec<MatchArm> },
    /// `{ stmts }`
    Block(Vec<Stmt>),
    /// `loop { body }` as expression (returns break value)
    LoopExpr(Vec<Stmt>),

    // Host type constructor: `TypeName { field: val, ... }` or `TypeName { ...spread, field: val }`
    StructConstruct { name: String, fields: Vec<(String, Expr)>, spread: Option<Box<Expr>> },

    // Enum variant access: `Enum::Variant` or `Enum::Variant(args)`
    EnumVariant { enum_name: String, variant: String },
    EnumVariantCall { enum_name: String, variant: String, args: Vec<Expr> },

    // Range
    Range { start: Box<Expr>, end: Box<Expr>, inclusive: bool },
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
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Neg, Not,
}
