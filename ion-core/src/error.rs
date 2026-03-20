use std::fmt;

#[derive(Debug, Clone)]
pub struct IonError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorKind {
    LexError,
    ParseError,
    RuntimeError,
    TypeError,
    NameError,
    PropagatedErr,
    PropagatedNone,
}

impl IonError {
    pub fn lex(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::LexError, message: message.into(), line, col }
    }

    pub fn parse(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::ParseError, message: message.into(), line, col }
    }

    pub fn runtime(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::RuntimeError, message: message.into(), line, col }
    }

    pub fn type_err(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::TypeError, message: message.into(), line, col }
    }

    pub fn name(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::NameError, message: message.into(), line, col }
    }

    pub fn propagated_err(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::PropagatedErr, message: message.into(), line, col }
    }

    pub fn propagated_none(line: usize, col: usize) -> Self {
        Self { kind: ErrorKind::PropagatedNone, message: String::new(), line, col }
    }
}

impl fmt::Display for IonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.kind {
            ErrorKind::LexError => ion_str!("LexError"),
            ErrorKind::ParseError => ion_str!("ParseError"),
            ErrorKind::RuntimeError => ion_str!("RuntimeError"),
            ErrorKind::TypeError => ion_str!("TypeError"),
            ErrorKind::NameError => ion_str!("NameError"),
            ErrorKind::PropagatedErr => ion_str!("PropagatedErr"),
            ErrorKind::PropagatedNone => ion_str!("PropagatedNone"),
        };
        write!(f, "{} at {}:{}: {}", kind, self.line, self.col, self.message)
    }
}

impl std::error::Error for IonError {}
