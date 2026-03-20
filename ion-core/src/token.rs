#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Int(i64),
    Float(f64),
    Str(String),
    FStr(String), // f"..." interpolated string (raw template)
    True,
    False,

    // Identifiers
    Ident(String),

    // Keywords
    Let,
    Mut,
    Fn,
    Match,
    If,
    Else,
    For,
    While,
    Loop,
    Break,
    Continue,
    Return,
    In,
    As,
    None,
    Some,
    Ok,
    Err,
    Async,
    Spawn,
    Await,
    Select,

    // Delimiters
    LParen,   // (
    RParen,   // )
    LBrace,   // {
    RBrace,   // }
    LBracket, // [
    RBracket, // ]
    HashBrace, // #{

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,       // =
    EqEq,     // ==
    BangEq,   // !=
    Lt,       // <
    Gt,       // >
    LtEq,     // <=
    GtEq,     // >=
    And,      // &&
    Or,       // ||
    Bang,     // !
    PlusEq,   // +=
    MinusEq,  // -=
    StarEq,   // *=
    SlashEq,  // /=
    Pipe,     // |>
    Question, // ?
    DotDot,   // ..
    DotDotEq, // ..=
    Dot,      // .
    DotDotDot, // ...

    // Punctuation
    Comma,
    Colon,
    Semicolon,
    Arrow, // =>
    PipeSym, // | (for closures)
    ColonColon, // ::

    // Special
    Eof,
}

#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub line: usize,
    pub col: usize,
}
