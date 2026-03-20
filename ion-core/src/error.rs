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
    /// Format error with source context showing the offending line.
    pub fn format_with_source(&self, source: &str) -> String {
        let mut out = String::new();
        // Header
        out.push_str(&format!("\x1b[1;31merror[{}]\x1b[0m: {}\n", self.kind_str(), self.message));
        // Source context
        if self.line > 0 {
            let lines: Vec<&str> = source.lines().collect();
            if self.line <= lines.len() {
                let line_str = lines[self.line - 1];
                let line_num = format!("{}", self.line);
                let padding = " ".repeat(line_num.len());
                out.push_str(&format!(" {} \x1b[34m|\x1b[0m\n", padding));
                out.push_str(&format!(" \x1b[34m{} |\x1b[0m {}\n", line_num, line_str));
                out.push_str(&format!(" {} \x1b[34m|\x1b[0m ", padding));
                if self.col > 0 && self.col <= line_str.len() + 1 {
                    out.push_str(&" ".repeat(self.col - 1));
                    out.push_str("\x1b[1;31m^\x1b[0m");
                }
                out.push('\n');
            }
        }
        out
    }

    fn kind_str(&self) -> &str {
        match &self.kind {
            ErrorKind::LexError => "lex",
            ErrorKind::ParseError => "parse",
            ErrorKind::RuntimeError => "runtime",
            ErrorKind::TypeError => "type",
            ErrorKind::NameError => "name",
            ErrorKind::PropagatedErr => "propagated_err",
            ErrorKind::PropagatedNone => "propagated_none",
        }
    }

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
