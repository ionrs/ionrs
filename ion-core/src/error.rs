use std::fmt;

use redacted_error::{ErrorCode, Message, PublicError};

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct IonError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: usize,
    pub col: usize,
    /// Additional errors (for multi-error reporting from parser).
    pub additional: Vec<IonError>,
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
        #[cfg(not(debug_assertions))]
        {
            let _ = source;
            let mut out = format!("error: {}\n", self.public_message());
            for extra in &self.additional {
                out.push_str(&format!("error: {}\n", extra.public_message()));
            }
            return out;
        }

        #[cfg(debug_assertions)]
        {
            let mut out = Self::format_single(self, source);
            for extra in &self.additional {
                out.push('\n');
                out.push_str(&Self::format_single(extra, source));
            }
            out
        }
    }

    fn format_single(err: &IonError, source: &str) -> String {
        let mut out = String::new();
        // Header
        out.push_str(&format!(
            "\x1b[1;31merror[{}]\x1b[0m: {}\n",
            err.kind_str(),
            err.message
        ));
        // Source context
        if err.line > 0 {
            let lines: Vec<&str> = source.lines().collect();
            if err.line <= lines.len() {
                let line_str = lines[err.line - 1];
                let line_num = format!("{}", err.line);
                let padding = " ".repeat(line_num.len());
                out.push_str(&format!(" {} \x1b[34m|\x1b[0m\n", padding));
                out.push_str(&format!(" \x1b[34m{} |\x1b[0m {}\n", line_num, line_str));
                out.push_str(&format!(" {} \x1b[34m|\x1b[0m ", padding));
                if err.col > 0 && err.col <= line_str.len() + 1 {
                    out.push_str(&" ".repeat(err.col - 1));
                    out.push_str("\x1b[1;31m^\x1b[0m");
                }
                out.push('\n');
            }
        }
        // Suggestion hint
        if let Some(hint) = Self::suggest_hint(&err.kind, &err.message) {
            out.push_str(&format!(" \x1b[1;36mhelp\x1b[0m: {}\n", hint));
        }
        out
    }

    fn suggest_hint(kind: &ErrorKind, msg: &str) -> Option<&'static str> {
        match kind {
            ErrorKind::NameError => {
                if msg.contains(&*ion_str!("undefined variable")) {
                    Some(ion_static_str!(
                        "check spelling, or ensure the variable is declared with `let` before use"
                    ))
                } else {
                    None
                }
            }
            ErrorKind::TypeError => {
                if msg.contains(&*ion_str!("cannot assign to immutable")) {
                    Some(ion_static_str!(
                        "declare with `let mut` to allow reassignment"
                    ))
                } else if msg.contains(&*ion_str!("cannot add"))
                    || msg.contains(&*ion_str!("cannot subtract"))
                {
                    Some(ion_static_str!("Ion has no implicit type coercions \u{2014} convert explicitly with `int()`, `float()`, or `str()`"))
                } else if msg.contains(&*ion_str!("no method")) {
                    Some(ion_static_str!("use `.to_string()` to inspect the value's type, or check LANGUAGE.md for available methods"))
                } else {
                    None
                }
            }
            ErrorKind::ParseError => {
                if msg.contains(&*ion_str!("expected ';'")) {
                    Some(ion_static_str!("Ion requires semicolons after statements"))
                } else if msg.contains(&*ion_str!("expected '}'")) {
                    Some(ion_static_str!("check for unmatched `{` braces"))
                } else {
                    None
                }
            }
            ErrorKind::RuntimeError => {
                if msg.contains(&*ion_str!("division by zero")) {
                    Some(ion_static_str!(
                        "check the divisor before dividing, or use a try/catch block"
                    ))
                } else if msg.contains(&*ion_str!("stack overflow")) {
                    Some(ion_static_str!(
                        "check for infinite recursion, or increase the stack depth limit"
                    ))
                } else if msg.contains(&*ion_str!("index out of bounds")) {
                    Some(ion_static_str!(
                        "use `.len()` to check the collection size, or `.get()` for safe access"
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn kind_str(&self) -> &str {
        match &self.kind {
            ErrorKind::LexError => ion_static_str!("lex"),
            ErrorKind::ParseError => ion_static_str!("parse"),
            ErrorKind::RuntimeError => ion_static_str!("runtime"),
            ErrorKind::TypeError => ion_static_str!("type"),
            ErrorKind::NameError => ion_static_str!("name"),
            ErrorKind::PropagatedErr => ion_static_str!("propagated_err"),
            ErrorKind::PropagatedNone => ion_static_str!("propagated_none"),
        }
    }

    pub fn lex(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::LexError, message, line, col)
    }

    pub fn parse(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::ParseError, message, line, col)
    }

    pub fn runtime(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::RuntimeError, message, line, col)
    }

    pub fn type_err(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::TypeError, message, line, col)
    }

    pub fn name(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::NameError, message, line, col)
    }

    pub fn propagated_err(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self::new(ErrorKind::PropagatedErr, message, line, col)
    }

    pub fn propagated_none(line: usize, col: usize) -> Self {
        Self::new(ErrorKind::PropagatedNone, "", line, col)
    }

    fn new(kind: ErrorKind, message: impl Into<String>, line: usize, col: usize) -> Self {
        #[cfg(debug_assertions)]
        let message = message.into();

        #[cfg(not(debug_assertions))]
        let message = {
            let _ = message;
            Self::public_message_for_kind(&kind).into_string()
        };

        Self {
            kind,
            message,
            line,
            col,
            additional: Vec::new(),
        }
    }

    pub fn public_message(&self) -> Message {
        Self::public_message_for_kind(&self.kind)
    }

    fn public_message_for_kind(kind: &ErrorKind) -> Message {
        match kind {
            ErrorKind::LexError => redacted_error::message!("lex error"),
            ErrorKind::ParseError => redacted_error::message!("parse error"),
            ErrorKind::RuntimeError => redacted_error::message!("runtime error"),
            ErrorKind::TypeError => redacted_error::message!("type error"),
            ErrorKind::NameError => redacted_error::message!("name error"),
            ErrorKind::PropagatedErr => redacted_error::message!("propagated error"),
            ErrorKind::PropagatedNone => redacted_error::message!("propagated none"),
        }
    }
}

impl fmt::Display for IonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(not(debug_assertions))]
        {
            return f.write_str(self.public_message().as_str());
        }

        #[cfg(debug_assertions)]
        {
            let kind = match &self.kind {
                ErrorKind::LexError => ion_str!("LexError"),
                ErrorKind::ParseError => ion_str!("ParseError"),
                ErrorKind::RuntimeError => ion_str!("RuntimeError"),
                ErrorKind::TypeError => ion_str!("TypeError"),
                ErrorKind::NameError => ion_str!("NameError"),
                ErrorKind::PropagatedErr => ion_str!("PropagatedErr"),
                ErrorKind::PropagatedNone => ion_str!("PropagatedNone"),
            };
            write!(
                f,
                "{} at {}:{}: {}",
                kind, self.line, self.col, self.message
            )
        }
    }
}

impl std::error::Error for IonError {}

pub fn type_conversion_failed_message() -> String {
    redacted_error::message_string!("type conversion failed")
}

impl ErrorCode for IonError {
    fn code(&self) -> Message {
        match &self.kind {
            ErrorKind::LexError => redacted_error::message!("ion.lex_error"),
            ErrorKind::ParseError => redacted_error::message!("ion.parse_error"),
            ErrorKind::RuntimeError => redacted_error::message!("ion.runtime_error"),
            ErrorKind::TypeError => redacted_error::message!("ion.type_error"),
            ErrorKind::NameError => redacted_error::message!("ion.name_error"),
            ErrorKind::PropagatedErr => redacted_error::message!("ion.propagated_error"),
            ErrorKind::PropagatedNone => redacted_error::message!("ion.propagated_none"),
        }
    }
}

impl PublicError for IonError {
    fn public_message(&self) -> Message {
        IonError::public_message(self)
    }
}

redacted_error::impl_redacted_debug!(IonError);
