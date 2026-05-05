//! Source rewriter for replacing global (top-level) variable values.
//!
//! Gated behind the `rewrite` cargo feature. Given Ion source code, this
//! module locates a top-level `let [mut] NAME [: T] = VALUE;` binding and
//! returns the source with `VALUE` swapped for a caller-supplied fragment.
//! The rest of the file — formatting, comments, unrelated statements — is
//! preserved byte-for-byte.
//!
//! # Example
//!
//! ```
//! use ion_core::rewrite::replace_global;
//!
//! let src = "let threshold = 10;\nfn check(x) { x > threshold }\n";
//! let out = replace_global(src, "threshold", "42").unwrap();
//! assert_eq!(out, "let threshold = 42;\nfn check(x) { x > threshold }\n");
//! ```
//!
//! Only module-level `let` bindings are considered; identically-named
//! bindings inside function bodies, blocks, or expressions are skipped.

use crate::error::IonError;
use crate::lexer::Lexer;
use crate::token::{SpannedToken, Token};
use redacted_error::{message as m, ErrorCode, Message, PublicError};

/// Errors produced by the rewriter.
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone, thiserror::Error)]
pub enum RewriteError {
    /// The source failed to tokenize.
    #[cfg_attr(debug_assertions, error("{prefix} {0}", prefix = m!("lex error:")))]
    #[cfg_attr(not(debug_assertions), error("{}", m!("rewrite failed")))]
    Lex(IonError),
    /// No top-level `let NAME = ...;` binding exists for the given name.
    #[cfg_attr(
        debug_assertions,
        error("{prefix} {0}", prefix = m!("no top-level let binding found for:"))
    )]
    #[cfg_attr(not(debug_assertions), error("{}", m!("binding not found")))]
    NotFound(String),
    /// The let binding was found but its structure could not be parsed
    /// (e.g. truncated source, missing `=` or terminating `;`).
    #[cfg_attr(
        debug_assertions,
        error("{prefix} {0}", prefix = m!("malformed let binding:"))
    )]
    #[cfg_attr(not(debug_assertions), error("{}", m!("invalid rewrite input")))]
    Malformed(String),
    /// The rewritten source no longer parses as valid Ion.
    #[cfg_attr(
        debug_assertions,
        error("{prefix} {0}", prefix = m!("rewritten source is invalid:"))
    )]
    #[cfg_attr(not(debug_assertions), error("{}", m!("invalid replacement")))]
    InvalidReplacement(IonError),
}

impl ErrorCode for RewriteError {
    fn code(&self) -> Message {
        match self {
            RewriteError::Lex(_) => redacted_error::message!("rewrite.lex_error"),
            RewriteError::NotFound(_) => redacted_error::message!("rewrite.not_found"),
            RewriteError::Malformed(_) => redacted_error::message!("rewrite.malformed"),
            RewriteError::InvalidReplacement(_) => {
                redacted_error::message!("rewrite.invalid_replacement")
            }
        }
    }
}

impl PublicError for RewriteError {
    fn public_message(&self) -> Message {
        match self {
            RewriteError::Lex(_) => redacted_error::message!("rewrite failed"),
            RewriteError::NotFound(_) => redacted_error::message!("binding not found"),
            RewriteError::Malformed(_) => redacted_error::message!("invalid rewrite input"),
            RewriteError::InvalidReplacement(_) => redacted_error::message!("invalid replacement"),
        }
    }
}

redacted_error::impl_redacted_debug!(RewriteError);

/// Replace the value of the top-level `let NAME = ...;` binding with
/// `new_value_src` (an Ion source fragment).
///
/// The replacement string is inserted verbatim between `=` and `;`. Callers
/// are responsible for producing a well-formed Ion expression; the result
/// is parsed at the end and [`RewriteError::InvalidReplacement`] is
/// returned if the rewrite produces syntactically invalid source.
///
/// Returns [`RewriteError::NotFound`] if no module-level binding for
/// `name` exists. The first matching binding is replaced if there are
/// multiple (which would itself be a program error, but is tolerated).
pub fn replace_global(
    source: &str,
    name: &str,
    new_value_src: &str,
) -> Result<String, RewriteError> {
    let tokens = Lexer::new(source).tokenize().map_err(RewriteError::Lex)?;
    let line_starts = line_start_offsets(source);

    let (value_start, value_end) = find_global_value_span(&tokens, name)?;

    let start_byte = byte_offset_of(&tokens[value_start], &line_starts);
    let end_byte = byte_offset_of(&tokens[value_end], &line_starts);

    let mut out = String::with_capacity(source.len() + new_value_src.len());
    out.push_str(&source[..start_byte]);
    out.push_str(new_value_src);
    out.push_str(&source[end_byte..]);

    // Validate the rewritten source round-trips through the lexer and
    // parser. This is cheap and catches most malformed replacements.
    let new_tokens = Lexer::new(&out)
        .tokenize()
        .map_err(RewriteError::InvalidReplacement)?;
    crate::parser::Parser::new(new_tokens)
        .parse_program()
        .map_err(RewriteError::InvalidReplacement)?;

    Ok(out)
}

/// Return `(value_start_tok_idx, terminating_semi_tok_idx)` for the first
/// top-level `let [mut] name [: T] = ... ;` in the token stream.
fn find_global_value_span(
    tokens: &[SpannedToken],
    name: &str,
) -> Result<(usize, usize), RewriteError> {
    let mut i = 0;
    let mut depth: i32 = 0;
    while i < tokens.len() {
        match &tokens[i].token {
            Token::LBrace | Token::LBracket | Token::LParen | Token::HashBrace => {
                depth += 1;
            }
            Token::RBrace | Token::RBracket | Token::RParen => {
                depth -= 1;
            }
            Token::Let if depth == 0 => {
                if let Some(span) = try_match_let(tokens, i, name)? {
                    return Ok(span);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(RewriteError::NotFound(name.to_string()))
}

/// If `tokens[let_idx]` begins a `let [mut] name [: T] = ... ;` targeting
/// `name`, return `(value_first_tok_idx, terminating_semi_idx)`. Returns
/// `Ok(None)` if the let is structurally valid but binds a different name.
fn try_match_let(
    tokens: &[SpannedToken],
    let_idx: usize,
    name: &str,
) -> Result<Option<(usize, usize)>, RewriteError> {
    let mut j = let_idx + 1;
    if matches!(tokens.get(j).map(|t| &t.token), Some(Token::Mut)) {
        j += 1;
    }
    // The binding pattern. Only simple `ident` bindings are candidates
    // for rewriting — destructuring patterns (`let (a, b) = ...`) are
    // skipped even if they bind the target name.
    let ident_matches = match tokens.get(j).map(|t| &t.token) {
        Some(Token::Ident(n)) => n == name,
        _ => return Ok(None),
    };
    j += 1;
    // Optional `: TypeAnn`.
    if matches!(tokens.get(j).map(|t| &t.token), Some(Token::Colon)) {
        j += 1;
        // Type annotations never contain `=` or `;`, so scan to the `=`.
        while let Some(tok) = tokens.get(j) {
            match tok.token {
                Token::Eq | Token::Semicolon | Token::Eof => break,
                _ => j += 1,
            }
        }
    }
    // Require `=`.
    if !matches!(tokens.get(j).map(|t| &t.token), Some(Token::Eq)) {
        return Err(RewriteError::Malformed(format!(
            "expected `=` after `let {}`",
            name
        )));
    }
    let value_start = j + 1;
    if value_start >= tokens.len()
        || matches!(tokens[value_start].token, Token::Eof | Token::Semicolon)
    {
        return Err(RewriteError::Malformed(
            "expected expression after `=`".to_string(),
        ));
    }
    // Walk the value, tracking local bracket depth, until we find the
    // terminating `;` at depth 0.
    let mut k = value_start;
    let mut local: i32 = 0;
    loop {
        match tokens.get(k).map(|t| &t.token) {
            Some(Token::LBrace)
            | Some(Token::LBracket)
            | Some(Token::LParen)
            | Some(Token::HashBrace) => local += 1,
            Some(Token::RBrace) | Some(Token::RBracket) | Some(Token::RParen) => local -= 1,
            Some(Token::Semicolon) if local == 0 => {
                if !ident_matches {
                    return Ok(None);
                }
                return Ok(Some((value_start, k)));
            }
            Some(Token::Eof) | None => {
                return Err(RewriteError::Malformed(
                    "unterminated `let` binding (missing `;`)".to_string(),
                ));
            }
            _ => {}
        }
        k += 1;
    }
}

/// Precompute the byte offset at which each 1-based line begins.
/// `line_start_offsets(src)[line - 1]` is the byte position of the first
/// character on `line`.
fn line_start_offsets(source: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(source.len() / 40 + 1);
    starts.push(0);
    for (i, b) in source.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convert a `SpannedToken`'s `(line, col)` back to a byte offset into the
/// original source. The lexer emits byte-counted columns (both col and its
/// internal byte position advance in lockstep), so this is exact.
fn byte_offset_of(tok: &SpannedToken, line_starts: &[usize]) -> usize {
    let line = tok.line.max(1);
    let idx = (line - 1).min(line_starts.len() - 1);
    line_starts[idx] + tok.col.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_simple_int() {
        let src = "let x = 1;";
        let out = replace_global(src, "x", "42").unwrap();
        assert_eq!(out, "let x = 42;");
    }

    #[test]
    fn preserves_surrounding_code() {
        let src = "fn pre() { 1 }\nlet threshold = 10;\nfn post() { threshold }\n";
        let out = replace_global(src, "threshold", "99").unwrap();
        assert_eq!(
            out,
            "fn pre() { 1 }\nlet threshold = 10;\nfn post() { threshold }\n"
                .replace("= 10", "= 99")
        );
    }

    #[test]
    fn handles_mutable_global() {
        let src = "let mut counter = 0;";
        let out = replace_global(src, "counter", "100").unwrap();
        assert_eq!(out, "let mut counter = 100;");
    }

    #[test]
    fn handles_type_annotation() {
        let src = "let name: string = \"old\";";
        let out = replace_global(src, "name", "\"new\"").unwrap();
        assert_eq!(out, "let name: string = \"new\";");
    }

    #[test]
    fn handles_list_value() {
        let src = "let xs = [1, 2, 3];";
        let out = replace_global(src, "xs", "[4, 5, 6, 7]").unwrap();
        assert_eq!(out, "let xs = [4, 5, 6, 7];");
    }

    #[test]
    fn handles_dict_value_with_nested_semicolons_impossible_but_braces_ok() {
        let src = "let cfg = #{\"a\": 1, \"b\": [2, 3]};";
        let out = replace_global(src, "cfg", "#{\"a\": 9}").unwrap();
        assert_eq!(out, "let cfg = #{\"a\": 9};");
    }

    #[test]
    fn skips_bindings_inside_function_bodies() {
        let src = "fn f() { let x = 1; x }\nlet x = 99;";
        let out = replace_global(src, "x", "7").unwrap();
        assert_eq!(out, "fn f() { let x = 1; x }\nlet x = 7;");
    }

    #[test]
    fn not_found_returns_error() {
        let src = "let y = 1;";
        let err = replace_global(src, "missing", "0").unwrap_err();
        assert!(matches!(err, RewriteError::NotFound(_)));
    }

    #[test]
    fn rejects_invalid_replacement() {
        let src = "let x = 1;";
        let err = replace_global(src, "x", "}{ not valid").unwrap_err();
        assert!(matches!(err, RewriteError::InvalidReplacement(_)));
    }

    #[test]
    fn preserves_trailing_newline_and_comments() {
        let src = "// config\nlet port = 8080; // default\n";
        let out = replace_global(src, "port", "9090").unwrap();
        assert_eq!(out, "// config\nlet port = 9090; // default\n");
    }

    #[test]
    fn first_top_level_binding_wins() {
        // Ion doesn't really allow duplicate top-level lets, but the
        // rewriter still needs deterministic behavior if it encounters
        // them in some generated source.
        let src = "let x = 1;\nlet x = 2;";
        let out = replace_global(src, "x", "9").unwrap();
        assert_eq!(out, "let x = 9;\nlet x = 2;");
    }
}
