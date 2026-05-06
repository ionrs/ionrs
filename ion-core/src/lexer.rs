use crate::error::IonError;
use crate::token::{SpannedToken, Token};

pub struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>, IonError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.token == Token::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn peek(&self) -> u8 {
        if self.pos < self.source.len() {
            self.source[self.pos]
        } else {
            0
        }
    }

    fn peek_at(&self, offset: usize) -> u8 {
        let idx = self.pos + offset;
        if idx < self.source.len() {
            self.source[idx]
        } else {
            0
        }
    }

    fn advance(&mut self) -> u8 {
        let ch = self.peek();
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        self.pos += 1;
        ch
    }

    /// Consume a full UTF-8 character from the source and push it to the string.
    fn push_utf8_char(&mut self, s: &mut String) {
        let b = self.advance();
        let width = if b < 0x80 {
            s.push(b as char);
            return;
        } else if b < 0xE0 {
            2
        } else if b < 0xF0 {
            3
        } else {
            4
        };
        let mut bytes = vec![b];
        for _ in 1..width {
            if self.pos < self.source.len() {
                bytes.push(self.advance());
            }
        }
        if let Ok(ch) = std::str::from_utf8(&bytes) {
            s.push_str(ch);
        } else {
            // Invalid UTF-8: push replacement character
            s.push(char::REPLACEMENT_CHARACTER);
        }
    }

    /// Parse `\u{XXXX}` Unicode escape (the `u` has already been consumed).
    fn parse_unicode_escape(&mut self, s: &mut String) -> Result<(), IonError> {
        if self.pos >= self.source.len() || self.peek() != b'{' {
            return Err(IonError::lex(
                ion_str!("expected '{' after \\u"),
                self.line,
                self.col,
            ));
        }
        self.advance(); // consume '{'
        let mut hex = String::new();
        while self.pos < self.source.len() && self.peek() != b'}' {
            hex.push(self.advance() as char);
            if hex.len() > 6 {
                return Err(IonError::lex(
                    ion_str!("unicode escape too long (max 6 hex digits)"),
                    self.line,
                    self.col,
                ));
            }
        }
        if self.pos >= self.source.len() {
            return Err(IonError::lex(
                ion_str!("unterminated unicode escape"),
                self.line,
                self.col,
            ));
        }
        self.advance(); // consume '}'
        if hex.is_empty() {
            return Err(IonError::lex(
                ion_str!("empty unicode escape"),
                self.line,
                self.col,
            ));
        }
        let code_point = u32::from_str_radix(&hex, 16)
            .map_err(|_| IonError::lex(ion_str!("invalid unicode escape"), self.line, self.col))?;
        let ch = char::from_u32(code_point).ok_or_else(|| {
            IonError::lex(ion_str!("invalid unicode code point"), self.line, self.col)
        })?;
        s.push(ch);
        Ok(())
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self.pos < self.source.len() && self.peek().is_ascii_whitespace() {
                self.advance();
            }
            if self.peek() == b'/' && self.peek_at(1) == b'/' {
                while self.pos < self.source.len() && self.peek() != b'\n' {
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn spanned(&self, token: Token, line: usize, col: usize) -> SpannedToken {
        SpannedToken { token, line, col }
    }

    fn next_token(&mut self) -> Result<SpannedToken, IonError> {
        self.skip_whitespace_and_comments();

        let line = self.line;
        let col = self.col;

        if self.pos >= self.source.len() {
            return Ok(self.spanned(Token::Eof, line, col));
        }

        let ch = self.peek();

        // Numbers
        if ch.is_ascii_digit() {
            return self.lex_number(line, col);
        }

        // Triple-quoted strings (must check before single-quoted)
        // ch == peek() since pos hasn't advanced yet, so check peek_at(1) and peek_at(2)
        if ch == b'"' && self.peek_at(1) == b'"' && self.peek_at(2) == b'"' {
            return self.lex_triple_string(line, col, false);
        }

        // Triple-quoted f-strings
        if ch == b'f'
            && self.peek_at(1) == b'"'
            && self.peek_at(2) == b'"'
            && self.peek_at(3) == b'"'
        {
            self.advance(); // consume 'f'
            return self.lex_triple_string(line, col, true);
        }

        // Strings
        if ch == b'"' {
            return self.lex_string(line, col, false);
        }

        // f-strings
        if ch == b'f' && self.peek_at(1) == b'"' {
            self.advance(); // consume 'f'
            return self.lex_string(line, col, true);
        }

        // byte strings
        if ch == b'b' && self.peek_at(1) == b'"' {
            self.advance(); // consume 'b'
            return self.lex_bytes(line, col);
        }

        // Identifiers and keywords
        if ch.is_ascii_alphabetic() || ch == b'_' {
            return self.lex_ident(line, col);
        }

        // Loop labels: 'name
        if ch == b'\'' {
            return self.lex_label(line, col);
        }

        // Operators and punctuation
        self.advance();
        match ch {
            b'(' => Ok(self.spanned(Token::LParen, line, col)),
            b')' => Ok(self.spanned(Token::RParen, line, col)),
            b'{' => Ok(self.spanned(Token::LBrace, line, col)),
            b'}' => Ok(self.spanned(Token::RBrace, line, col)),
            b'[' => Ok(self.spanned(Token::LBracket, line, col)),
            b']' => Ok(self.spanned(Token::RBracket, line, col)),
            b',' => Ok(self.spanned(Token::Comma, line, col)),
            b';' => Ok(self.spanned(Token::Semicolon, line, col)),
            b'?' => Ok(self.spanned(Token::Question, line, col)),
            b'#' => {
                if self.peek() == b'{' {
                    self.advance();
                    Ok(self.spanned(Token::HashBrace, line, col))
                } else {
                    Err(IonError::lex(
                        ion_format!("{}{}", ion_str!("unexpected character: "), '#'),
                        line,
                        col,
                    ))
                }
            }
            b'+' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::PlusEq, line, col))
                } else {
                    Ok(self.spanned(Token::Plus, line, col))
                }
            }
            b'-' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::MinusEq, line, col))
                } else {
                    Ok(self.spanned(Token::Minus, line, col))
                }
            }
            b'*' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::StarEq, line, col))
                } else {
                    Ok(self.spanned(Token::Star, line, col))
                }
            }
            b'/' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::SlashEq, line, col))
                } else {
                    Ok(self.spanned(Token::Slash, line, col))
                }
            }
            b'%' => Ok(self.spanned(Token::Percent, line, col)),
            b'=' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::EqEq, line, col))
                } else if self.peek() == b'>' {
                    self.advance();
                    Ok(self.spanned(Token::Arrow, line, col))
                } else {
                    Ok(self.spanned(Token::Eq, line, col))
                }
            }
            b'!' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::BangEq, line, col))
                } else {
                    Ok(self.spanned(Token::Bang, line, col))
                }
            }
            b'<' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::LtEq, line, col))
                } else if self.peek() == b'<' {
                    self.advance();
                    Ok(self.spanned(Token::Shl, line, col))
                } else {
                    Ok(self.spanned(Token::Lt, line, col))
                }
            }
            b'>' => {
                if self.peek() == b'=' {
                    self.advance();
                    Ok(self.spanned(Token::GtEq, line, col))
                } else if self.peek() == b'>' {
                    self.advance();
                    Ok(self.spanned(Token::Shr, line, col))
                } else {
                    Ok(self.spanned(Token::Gt, line, col))
                }
            }
            b'&' => {
                if self.peek() == b'&' {
                    self.advance();
                    Ok(self.spanned(Token::And, line, col))
                } else {
                    Ok(self.spanned(Token::Ampersand, line, col))
                }
            }
            b'^' => Ok(self.spanned(Token::Caret, line, col)),
            b'|' => {
                if self.peek() == b'|' {
                    self.advance();
                    Ok(self.spanned(Token::Or, line, col))
                } else if self.peek() == b'>' {
                    self.advance();
                    Ok(self.spanned(Token::Pipe, line, col))
                } else {
                    Ok(self.spanned(Token::PipeSym, line, col))
                }
            }
            b'.' => {
                if self.peek() == b'.' {
                    self.advance();
                    if self.peek() == b'.' {
                        self.advance();
                        Ok(self.spanned(Token::DotDotDot, line, col))
                    } else if self.peek() == b'=' {
                        self.advance();
                        Ok(self.spanned(Token::DotDotEq, line, col))
                    } else {
                        Ok(self.spanned(Token::DotDot, line, col))
                    }
                } else {
                    Ok(self.spanned(Token::Dot, line, col))
                }
            }
            b':' => {
                if self.peek() == b':' {
                    self.advance();
                    Ok(self.spanned(Token::ColonColon, line, col))
                } else {
                    Ok(self.spanned(Token::Colon, line, col))
                }
            }
            _ => Err(IonError::lex(
                ion_format!("{}{}", ion_str!("unexpected character: "), ch as char),
                line,
                col,
            )),
        }
    }

    fn lex_number(&mut self, line: usize, col: usize) -> Result<SpannedToken, IonError> {
        let start = self.pos;
        let mut is_float = false;

        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.advance();
        }
        if self.peek() == b'.' && self.peek_at(1) != b'.' {
            is_float = true;
            self.advance();
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.advance();
            }
        }

        let text: String = self.source[start..self.pos]
            .iter()
            .filter(|&&b| b != b'_')
            .map(|&b| b as char)
            .collect();

        if is_float {
            let val: f64 = text
                .parse()
                .map_err(|_| IonError::lex(ion_str!("invalid float literal"), line, col))?;
            Ok(self.spanned(Token::Float(val), line, col))
        } else {
            let val: i64 = text
                .parse()
                .map_err(|_| IonError::lex(ion_str!("invalid integer literal"), line, col))?;
            Ok(self.spanned(Token::Int(val), line, col))
        }
    }

    fn lex_string(
        &mut self,
        line: usize,
        col: usize,
        is_fstr: bool,
    ) -> Result<SpannedToken, IonError> {
        self.advance(); // consume opening "
        let mut s = String::new();
        let mut interp_depth = 0u32; // track {} nesting in f-string interpolation

        while self.pos < self.source.len() {
            let ch = self.peek();

            // In f-string interpolation mode, track braces and allow quotes
            if is_fstr && interp_depth > 0 {
                if ch == b'"' {
                    // Skip over nested string inside interpolation
                    self.advance();
                    s.push('"');
                    while self.pos < self.source.len() && self.peek() != b'"' {
                        if self.peek() == b'\\' {
                            self.advance();
                            s.push('\\');
                            if self.pos < self.source.len() {
                                self.push_utf8_char(&mut s);
                            }
                        } else {
                            self.push_utf8_char(&mut s);
                        }
                    }
                    if self.pos < self.source.len() {
                        self.advance(); // closing "
                        s.push('"');
                    }
                    continue;
                } else if ch == b'{' {
                    interp_depth += 1;
                    self.advance();
                    s.push('{');
                    continue;
                } else if ch == b'}' {
                    interp_depth -= 1;
                    self.advance();
                    s.push('}');
                    continue;
                } else {
                    self.push_utf8_char(&mut s);
                    continue;
                }
            }

            // Outside interpolation: " terminates the string
            if ch == b'"' {
                break;
            }

            if ch == b'\\' {
                self.advance();
                match self.peek() {
                    b'n' => {
                        self.advance();
                        s.push('\n');
                    }
                    b't' => {
                        self.advance();
                        s.push('\t');
                    }
                    b'r' => {
                        self.advance();
                        s.push('\r');
                    }
                    b'\\' => {
                        self.advance();
                        s.push('\\');
                    }
                    b'"' => {
                        self.advance();
                        s.push('"');
                    }
                    b'{' => {
                        self.advance();
                        s.push('{');
                    }
                    b'}' => {
                        self.advance();
                        s.push('}');
                    }
                    b'u' => {
                        self.advance(); // consume 'u'
                        self.parse_unicode_escape(&mut s)?;
                    }
                    _ => {
                        return Err(IonError::lex(
                            ion_str!("invalid escape sequence"),
                            self.line,
                            self.col,
                        ));
                    }
                }
            } else if is_fstr && ch == b'{' {
                // Enter interpolation mode
                interp_depth = 1;
                self.advance();
                s.push('{');
            } else {
                self.push_utf8_char(&mut s);
            }
        }

        if self.pos >= self.source.len() {
            return Err(IonError::lex(ion_str!("unterminated string"), line, col));
        }
        self.advance(); // consume closing "

        if is_fstr {
            Ok(self.spanned(Token::FStr(s), line, col))
        } else {
            Ok(self.spanned(Token::Str(s), line, col))
        }
    }

    fn lex_triple_string(
        &mut self,
        line: usize,
        col: usize,
        is_fstr: bool,
    ) -> Result<SpannedToken, IonError> {
        // consume opening """
        self.advance(); // first "
        self.advance(); // second "
        self.advance(); // third "
                        // Skip a leading newline immediately after """
        if self.pos < self.source.len() && self.peek() == b'\n' {
            self.advance();
        }
        let mut s = String::new();

        while self.pos < self.source.len() {
            if self.peek() == b'"'
                && self.pos + 1 < self.source.len()
                && self.source[self.pos + 1] == b'"'
                && self.pos + 2 < self.source.len()
                && self.source[self.pos + 2] == b'"'
            {
                // consume closing """
                self.advance();
                self.advance();
                self.advance();
                if is_fstr {
                    return Ok(self.spanned(Token::FStr(s), line, col));
                } else {
                    return Ok(self.spanned(Token::Str(s), line, col));
                }
            }
            let ch = self.peek();
            if ch == b'\n' {
                self.advance();
                s.push('\n');
            } else if ch == b'\\' {
                self.advance();
                match self.peek() {
                    b'n' => {
                        self.advance();
                        s.push('\n');
                    }
                    b't' => {
                        self.advance();
                        s.push('\t');
                    }
                    b'r' => {
                        self.advance();
                        s.push('\r');
                    }
                    b'\\' => {
                        self.advance();
                        s.push('\\');
                    }
                    b'"' => {
                        self.advance();
                        s.push('"');
                    }
                    b'{' => {
                        self.advance();
                        s.push('{');
                    }
                    b'}' => {
                        self.advance();
                        s.push('}');
                    }
                    b'u' => {
                        self.advance(); // consume 'u'
                        self.parse_unicode_escape(&mut s)?;
                    }
                    _ => {
                        return Err(IonError::lex(
                            ion_str!("invalid escape sequence"),
                            self.line,
                            self.col,
                        ));
                    }
                }
            } else {
                self.push_utf8_char(&mut s);
            }
        }

        Err(IonError::lex(
            ion_str!("unterminated triple-quoted string"),
            line,
            col,
        ))
    }

    fn lex_bytes(&mut self, line: usize, col: usize) -> Result<SpannedToken, IonError> {
        self.advance(); // consume opening "
        let mut bytes = Vec::new();

        while self.pos < self.source.len() && self.peek() != b'"' {
            let ch = self.peek();
            if ch == b'\\' {
                self.advance();
                match self.peek() {
                    b'n' => {
                        self.advance();
                        bytes.push(b'\n');
                    }
                    b't' => {
                        self.advance();
                        bytes.push(b'\t');
                    }
                    b'r' => {
                        self.advance();
                        bytes.push(b'\r');
                    }
                    b'\\' => {
                        self.advance();
                        bytes.push(b'\\');
                    }
                    b'"' => {
                        self.advance();
                        bytes.push(b'"');
                    }
                    b'0' => {
                        self.advance();
                        bytes.push(0);
                    }
                    b'x' => {
                        self.advance(); // consume 'x'
                        let hi = self.advance();
                        let lo = self.advance();
                        let val = hex_digit(hi).ok_or_else(|| {
                            IonError::lex(
                                ion_str!("invalid hex escape in byte string"),
                                self.line,
                                self.col,
                            )
                        })? << 4
                            | hex_digit(lo).ok_or_else(|| {
                                IonError::lex(
                                    ion_str!("invalid hex escape in byte string"),
                                    self.line,
                                    self.col,
                                )
                            })?;
                        bytes.push(val);
                    }
                    _ => {
                        return Err(IonError::lex(
                            ion_str!("invalid escape sequence in byte string"),
                            self.line,
                            self.col,
                        ));
                    }
                }
            } else {
                self.advance();
                bytes.push(ch);
            }
        }

        if self.pos >= self.source.len() {
            return Err(IonError::lex(
                ion_str!("unterminated byte string"),
                line,
                col,
            ));
        }
        self.advance(); // consume closing "

        Ok(self.spanned(Token::Bytes(bytes), line, col))
    }

    fn lex_label(&mut self, line: usize, col: usize) -> Result<SpannedToken, IonError> {
        self.advance(); // consume '
        let start = self.pos;
        if !(self.peek().is_ascii_alphabetic() || self.peek() == b'_') {
            return Err(IonError::lex(
                ion_str!("expected label name after '"),
                line,
                col,
            ));
        }
        while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
            self.advance();
        }
        let text = std::str::from_utf8(&self.source[start..self.pos]).unwrap();
        Ok(self.spanned(Token::Label(text.to_string()), line, col))
    }

    fn lex_ident(&mut self, line: usize, col: usize) -> Result<SpannedToken, IonError> {
        let start = self.pos;
        while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
            self.advance();
        }
        let bytes = &self.source[start..self.pos];
        let text = std::str::from_utf8(bytes).unwrap();
        let token = keyword_token(bytes).unwrap_or_else(|| Token::Ident(text.to_string()));
        Ok(self.spanned(token, line, col))
    }
}

fn keyword_token(bytes: &[u8]) -> Option<Token> {
    match bytes.len() {
        2 => match (bytes[0], bytes[1]) {
            (102, 110) => Some(Token::Fn),
            (105, 102) => Some(Token::If),
            (105, 110) => Some(Token::In),
            (97, 115) => Some(Token::As),
            (79, 107) => Some(Token::Ok),
            _ => None,
        },
        3 => match (bytes[0], bytes[1], bytes[2]) {
            (108, 101, 116) => Some(Token::Let),
            (109, 117, 116) => Some(Token::Mut),
            (102, 111, 114) => Some(Token::For),
            (116, 114, 121) => Some(Token::Try),
            (69, 114, 114) => Some(Token::Err),
            (117, 115, 101) => Some(Token::Use),
            _ => None,
        },
        4 => match (bytes[0], bytes[1], bytes[2], bytes[3]) {
            (101, 108, 115, 101) => Some(Token::Else),
            (108, 111, 111, 112) => Some(Token::Loop),
            (116, 114, 117, 101) => Some(Token::True),
            (78, 111, 110, 101) => Some(Token::None),
            (83, 111, 109, 101) => Some(Token::Some),
            _ => None,
        },
        5 => match (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]) {
            (109, 97, 116, 99, 104) => Some(Token::Match),
            (119, 104, 105, 108, 101) => Some(Token::While),
            (98, 114, 101, 97, 107) => Some(Token::Break),
            (102, 97, 108, 115, 101) => Some(Token::False),
            (97, 115, 121, 110, 99) => Some(Token::Async),
            (115, 112, 97, 119, 110) => Some(Token::Spawn),
            (97, 119, 97, 105, 116) => Some(Token::Await),
            (99, 97, 116, 99, 104) => Some(Token::Catch),
            _ => None,
        },
        6 => match (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]) {
            (114, 101, 116, 117, 114, 110) => Some(Token::Return),
            (115, 101, 108, 101, 99, 116) => Some(Token::Select),
            _ => None,
        },
        8 => match (
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ) {
            (99, 111, 110, 116, 105, 110, 117, 101) => Some(Token::Continue),
            _ => None,
        },
        _ => None,
    }
}

fn hex_digit(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<Token> {
        Lexer::new(src)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.token)
            .collect()
    }

    #[test]
    fn test_basic_tokens() {
        let tokens = lex("let x = 42;");
        assert_eq!(
            tokens,
            vec![
                Token::Let,
                Token::Ident("x".into()),
                Token::Eq,
                Token::Int(42),
                Token::Semicolon,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_string() {
        let tokens = lex(r#""hello world""#);
        assert_eq!(tokens[0], Token::Str("hello world".into()));
    }

    #[test]
    fn test_fstring() {
        let tokens = lex(r#"f"hi {name}""#);
        assert_eq!(tokens[0], Token::FStr("hi {name}".into()));
    }

    #[test]
    fn test_hash_brace() {
        let tokens = lex("#{ }");
        assert_eq!(tokens[0], Token::HashBrace);
    }

    #[test]
    fn test_operators() {
        let tokens = lex("|> .. ... => :: ? !=");
        assert_eq!(
            tokens,
            vec![
                Token::Pipe,
                Token::DotDot,
                Token::DotDotDot,
                Token::Arrow,
                Token::ColonColon,
                Token::Question,
                Token::BangEq,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_float() {
        let tokens = lex("3.5");
        assert_eq!(tokens[0], Token::Float(3.5));
    }

    #[test]
    fn test_comments() {
        let tokens = lex("let x = 1; // comment\nlet y = 2;");
        assert_eq!(
            tokens,
            vec![
                Token::Let,
                Token::Ident("x".into()),
                Token::Eq,
                Token::Int(1),
                Token::Semicolon,
                Token::Let,
                Token::Ident("y".into()),
                Token::Eq,
                Token::Int(2),
                Token::Semicolon,
                Token::Eof,
            ]
        );
    }
}
