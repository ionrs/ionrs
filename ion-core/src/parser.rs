use crate::ast::*;
use crate::error::IonError;
use crate::token::{SpannedToken, Token};

/// Result of parsing: partial AST + accumulated errors.
pub struct ParseOutput {
    pub program: Program,
    pub errors: Vec<IonError>,
}

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
    errors: Vec<IonError>,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    /// Parse the full program, recovering from errors at statement boundaries.
    /// Returns a `ParseOutput` containing both the partial AST and any errors.
    pub fn parse_program_recovering(&mut self) -> ParseOutput {
        let mut stmts = Vec::new();
        while !self.is_at_end() {
            let before = self.pos;
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                    // If no progress was made, force advance to prevent infinite loop
                    if self.pos == before {
                        self.advance();
                    }
                }
            }
        }
        ParseOutput {
            program: Program { stmts },
            errors: std::mem::take(&mut self.errors),
        }
    }

    /// Parse the full program, returning an error if any parse errors occurred.
    /// For multi-error reporting, use `parse_program_recovering` instead.
    pub fn parse_program(&mut self) -> Result<Program, IonError> {
        let output = self.parse_program_recovering();
        if output.errors.is_empty() {
            Ok(output.program)
        } else {
            let mut errors = output.errors;
            let mut first = errors.remove(0);
            first.additional = errors;
            Err(first)
        }
    }

    /// Advance past the current error to the next statement boundary.
    /// Respects brace nesting — stops at `}` that closes the current block.
    fn synchronize(&mut self) {
        let mut brace_depth = 0i32;
        while !self.is_at_end() {
            // If we just passed a semicolon at the same brace depth, we're at a new statement
            if self.pos > 0 && brace_depth == 0 {
                if let Token::Semicolon = &self.tokens[self.pos - 1].token {
                    return;
                }
            }
            match self.peek() {
                // Track brace depth to avoid skipping past a closing }
                Token::LBrace => {
                    brace_depth += 1;
                    self.advance();
                }
                Token::RBrace => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                        self.advance();
                    } else {
                        // This } closes the enclosing block — stop before it
                        return;
                    }
                }
                // Stop at tokens that typically begin a new statement (if at top level)
                Token::Let
                | Token::Fn
                | Token::For
                | Token::While
                | Token::If
                | Token::Return
                | Token::Match
                | Token::Loop
                | Token::Try
                | Token::Break
                | Token::Continue
                | Token::Use
                    if brace_depth == 0 =>
                {
                    return;
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    // --- Helpers ---

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn span(&self) -> Span {
        let t = &self.tokens[self.pos];
        Span {
            line: t.line,
            col: t.col,
        }
    }

    fn prev_span(&self) -> Span {
        if self.pos > 0 {
            let t = &self.tokens[self.pos - 1];
            Span {
                line: t.line,
                col: t.col,
            }
        } else {
            self.span()
        }
    }

    fn advance(&mut self) -> &SpannedToken {
        let tok = &self.tokens[self.pos];
        if !self.is_at_end() {
            self.pos += 1;
        }
        tok
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn check(&self, token: &Token) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(token)
    }

    fn eat(&mut self, expected: &Token) -> Result<(), IonError> {
        if self.check(expected) {
            self.advance();
            Ok(())
        } else {
            let s = self.span();
            Err(IonError::parse(
                format!(
                    "{}{:?}{}{:?}",
                    ion_str!("expected "),
                    expected,
                    ion_str!(", found "),
                    self.peek()
                ),
                s.line,
                s.col,
            ))
        }
    }

    fn eat_ident(&mut self) -> Result<String, IonError> {
        if let Token::Ident(name) = self.peek().clone() {
            self.advance();
            Ok(name)
        } else {
            let s = self.span();
            Err(IonError::parse(
                format!(
                    "{}{:?}",
                    ion_str!("expected identifier, found "),
                    self.peek()
                ),
                s.line,
                s.col,
            ))
        }
    }

    // --- Statement Parsing ---

    fn parse_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        match self.peek().clone() {
            Token::Let => self.parse_let_stmt(),
            Token::Fn => self.parse_fn_decl(),
            Token::For => self.parse_for_stmt(None),
            Token::While => self.parse_while_stmt(None),
            Token::Loop => self.parse_loop_stmt(None),
            Token::Label(name) => {
                self.advance();
                self.eat(&Token::Colon)?;
                match self.peek() {
                    Token::For => self.parse_for_stmt(Some(name)),
                    Token::While => self.parse_while_stmt(Some(name)),
                    Token::Loop => self.parse_loop_stmt(Some(name)),
                    other => {
                        let s = self.span();
                        Err(IonError::parse(
                            format!(
                                "{}{:?}",
                                ion_str!("expected loop after label, found "),
                                other,
                            ),
                            s.line,
                            s.col,
                        ))
                    }
                }
            }
            Token::Break => self.parse_break_stmt(),
            Token::Continue => self.parse_continue_stmt(span),
            Token::Return => self.parse_return_stmt(),
            Token::Use => self.parse_use_stmt(),
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    fn parse_optional_label(&mut self) -> Option<String> {
        if let Token::Label(name) = self.peek().clone() {
            self.advance();
            Some(name)
        } else {
            None
        }
    }

    fn parse_continue_stmt(&mut self, span: Span) -> Result<Stmt, IonError> {
        self.eat(&Token::Continue)?;
        let label = self.parse_optional_label();
        self.eat(&Token::Semicolon)?;
        Ok(Stmt {
            kind: StmtKind::Continue { label },
            span,
        })
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Let)?;
        let mutable = if self.check(&Token::Mut) {
            self.advance();
            true
        } else {
            false
        };
        let pattern = self.parse_pattern()?;
        let type_ann = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type_ann()?)
        } else {
            None
        };
        self.eat(&Token::Eq)?;
        let value = self.parse_expr()?;
        self.eat(&Token::Semicolon)?;
        Ok(Stmt {
            kind: StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            },
            span,
        })
    }

    fn parse_type_ann(&mut self) -> Result<TypeAnn, IonError> {
        // `fn` is a keyword token, so eat it specially
        if self.check(&Token::Fn) {
            self.advance();
            return Ok(TypeAnn::Simple("fn".to_string()));
        }
        let name = self.eat_ident()?;
        match name.as_str() {
            "Option" => {
                self.eat(&Token::Lt)?;
                let inner = self.parse_type_ann()?;
                self.eat(&Token::Gt)?;
                Ok(TypeAnn::Option(Box::new(inner)))
            }
            "Result" => {
                self.eat(&Token::Lt)?;
                let ok = self.parse_type_ann()?;
                self.eat(&Token::Comma)?;
                let err = self.parse_type_ann()?;
                self.eat(&Token::Gt)?;
                Ok(TypeAnn::Result(Box::new(ok), Box::new(err)))
            }
            "list" => {
                if self.check(&Token::Lt) {
                    self.advance();
                    let inner = self.parse_type_ann()?;
                    self.eat(&Token::Gt)?;
                    Ok(TypeAnn::List(Box::new(inner)))
                } else {
                    Ok(TypeAnn::Simple(name))
                }
            }
            "dict" => {
                if self.check(&Token::Lt) {
                    self.advance();
                    let key = self.parse_type_ann()?;
                    self.eat(&Token::Comma)?;
                    let val = self.parse_type_ann()?;
                    self.eat(&Token::Gt)?;
                    Ok(TypeAnn::Dict(Box::new(key), Box::new(val)))
                } else {
                    Ok(TypeAnn::Simple(name))
                }
            }
            _ => Ok(TypeAnn::Simple(name)),
        }
    }

    fn parse_fn_decl(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Fn)?;
        let name = self.eat_ident()?;
        self.eat(&Token::LParen)?;
        let params = self.parse_params()?;
        self.eat(&Token::RParen)?;
        self.eat(&Token::LBrace)?;
        let body = self.parse_block_stmts()?;
        self.eat(&Token::RBrace)?;
        Ok(Stmt {
            kind: StmtKind::FnDecl { name, params, body },
            span,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, IonError> {
        let mut params = Vec::new();
        while !self.check(&Token::RParen) {
            let name = self.eat_ident()?;
            let default = if self.check(&Token::Eq) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param { name, default });
            if !self.check(&Token::RParen) {
                self.eat(&Token::Comma)?;
            }
        }
        Ok(params)
    }

    fn parse_for_stmt(&mut self, label: Option<String>) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::For)?;
        let pattern = self.parse_pattern()?;
        self.eat(&Token::In)?;
        let iter = self.parse_expr()?;
        self.eat(&Token::LBrace)?;
        let body = self.parse_block_stmts()?;
        self.eat(&Token::RBrace)?;
        Ok(Stmt {
            kind: StmtKind::For {
                label,
                pattern,
                iter,
                body,
            },
            span,
        })
    }

    fn parse_while_stmt(&mut self, label: Option<String>) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::While)?;
        // while let ...
        if self.check(&Token::Let) {
            self.advance();
            let pattern = self.parse_pattern()?;
            self.eat(&Token::Eq)?;
            let expr = self.parse_expr()?;
            self.eat(&Token::LBrace)?;
            let body = self.parse_block_stmts()?;
            self.eat(&Token::RBrace)?;
            return Ok(Stmt {
                kind: StmtKind::WhileLet {
                    label,
                    pattern,
                    expr,
                    body,
                },
                span,
            });
        }
        let cond = self.parse_expr()?;
        self.eat(&Token::LBrace)?;
        let body = self.parse_block_stmts()?;
        self.eat(&Token::RBrace)?;
        Ok(Stmt {
            kind: StmtKind::While { label, cond, body },
            span,
        })
    }

    fn parse_loop_stmt(&mut self, label: Option<String>) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Loop)?;
        self.eat(&Token::LBrace)?;
        let body = self.parse_block_stmts()?;
        self.eat(&Token::RBrace)?;
        Ok(Stmt {
            kind: StmtKind::Loop { label, body },
            span,
        })
    }

    fn parse_break_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Break)?;
        let label = self.parse_optional_label();
        let value = if self.check(&Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.eat(&Token::Semicolon)?;
        Ok(Stmt {
            kind: StmtKind::Break { label, value },
            span,
        })
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Return)?;
        let value = if self.check(&Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.eat(&Token::Semicolon)?;
        Ok(Stmt {
            kind: StmtKind::Return { value },
            span,
        })
    }

    /// Parse `use path::name;` or `use path::{a, b};` or `use path::*;`
    fn parse_use_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        self.eat(&Token::Use)?;

        // Parse the module path: `a::b::...`
        let mut path = Vec::new();
        path.push(self.eat_ident()?);
        while self.check(&Token::ColonColon) {
            self.advance();
            // Check what follows ::
            if self.check(&Token::Star) {
                // use path::*
                self.advance();
                self.eat(&Token::Semicolon)?;
                return Ok(Stmt {
                    kind: StmtKind::Use {
                        path,
                        imports: UseImports::Glob,
                    },
                    span,
                });
            } else if self.check(&Token::LBrace) {
                // use path::{a, b, c}
                self.advance();
                let mut names = Vec::new();
                while !self.check(&Token::RBrace) && !self.is_at_end() {
                    names.push(self.eat_ident()?);
                    if !self.check(&Token::RBrace) {
                        self.eat(&Token::Comma)?;
                    }
                }
                self.eat(&Token::RBrace)?;
                self.eat(&Token::Semicolon)?;
                return Ok(Stmt {
                    kind: StmtKind::Use {
                        path,
                        imports: UseImports::Names(names),
                    },
                    span,
                });
            } else {
                // More path segments or final single name
                path.push(self.eat_ident()?);
            }
        }

        // `use path::name;` — last segment is the imported name
        self.eat(&Token::Semicolon)?;
        if path.len() < 2 {
            return Err(IonError::parse(
                ion_str!("use statement requires at least module::name"),
                span.line,
                span.col,
            ));
        }
        let name = path.pop().unwrap();
        Ok(Stmt {
            kind: StmtKind::Use {
                path,
                imports: UseImports::Single(name),
            },
            span,
        })
    }

    fn parse_expr_or_assign_stmt(&mut self) -> Result<Stmt, IonError> {
        let span = self.span();
        let expr = self.parse_expr()?;

        // Check for assignment
        let assign_op = match self.peek() {
            Token::Eq => Some(AssignOp::Eq),
            Token::PlusEq => Some(AssignOp::PlusEq),
            Token::MinusEq => Some(AssignOp::MinusEq),
            Token::StarEq => Some(AssignOp::StarEq),
            Token::SlashEq => Some(AssignOp::SlashEq),
            _ => None,
        };

        if let Some(op) = assign_op {
            self.advance();
            let target = self.expr_to_assign_target(&expr)?;
            let value = self.parse_expr()?;
            self.eat(&Token::Semicolon)?;
            Ok(Stmt {
                kind: StmtKind::Assign { target, op, value },
                span,
            })
        } else {
            let has_semi = self.check(&Token::Semicolon);
            if has_semi {
                self.advance();
            }
            Ok(Stmt {
                kind: StmtKind::ExprStmt { expr, has_semi },
                span,
            })
        }
    }

    fn expr_to_assign_target(&self, expr: &Expr) -> Result<AssignTarget, IonError> {
        match &expr.kind {
            ExprKind::Ident(name) => Ok(AssignTarget::Ident(name.clone())),
            ExprKind::Index { expr, index } => Ok(AssignTarget::Index(expr.clone(), index.clone())),
            ExprKind::FieldAccess { expr, field } => {
                Ok(AssignTarget::Field(expr.clone(), field.clone()))
            }
            _ => Err(IonError::parse(
                ion_str!("invalid assignment target").to_string(),
                expr.span.line,
                expr.span.col,
            )),
        }
    }

    fn parse_block_stmts(&mut self) -> Result<Vec<Stmt>, IonError> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let before = self.pos;
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                    // If no progress was made, force advance to prevent infinite loop
                    if self.pos == before {
                        self.advance();
                    }
                }
            }
        }
        Ok(stmts)
    }

    // --- Expression Parsing (Pratt) ---

    fn parse_expr(&mut self) -> Result<Expr, IonError> {
        self.parse_pipe()
    }

    fn parse_pipe(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_or()?;
        while self.check(&Token::Pipe) {
            self.advance();
            let right = self.parse_or()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::PipeOp {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_and()?;
        while self.check(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op: BinOp::Or,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_bitwise_or()?;
        while self.check(&Token::And) {
            self.advance();
            let right = self.parse_bitwise_or()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op: BinOp::And,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_bitwise_xor()?;
        while self.check(&Token::PipeSym) {
            self.advance();
            let right = self.parse_bitwise_xor()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op: BinOp::BitOr,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_bitwise_and()?;
        while self.check(&Token::Caret) {
            self.advance();
            let right = self.parse_bitwise_and()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op: BinOp::BitXor,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_equality()?;
        while self.check(&Token::Ampersand) {
            self.advance();
            let right = self.parse_equality()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op: BinOp::BitAnd,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Token::EqEq => BinOp::Eq,
                Token::BangEq => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_shift()?;
        loop {
            let op = match self.peek() {
                Token::Lt => BinOp::Lt,
                Token::Gt => BinOp::Gt,
                Token::LtEq => BinOp::Le,
                Token::GtEq => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_shift()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_range()?;
        loop {
            let op = match self.peek() {
                Token::Shl => BinOp::Shl,
                Token::Shr => BinOp::Shr,
                _ => break,
            };
            self.advance();
            let right = self.parse_range()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_range(&mut self) -> Result<Expr, IonError> {
        let left = self.parse_addition()?;
        match self.peek() {
            Token::DotDot => {
                self.advance();
                let right = self.parse_addition()?;
                let span = left.span;
                Ok(Expr {
                    kind: ExprKind::Range {
                        start: Box::new(left),
                        end: Box::new(right),
                        inclusive: false,
                    },
                    span,
                })
            }
            Token::DotDotEq => {
                self.advance();
                let right = self.parse_addition()?;
                let span = left.span;
                Ok(Expr {
                    kind: ExprKind::Range {
                        start: Box::new(left),
                        end: Box::new(right),
                        inclusive: true,
                    },
                    span,
                })
            }
            _ => Ok(left),
        }
    }

    fn parse_addition(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_multiplication(&mut self) -> Result<Expr, IonError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::BinOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, IonError> {
        let span = self.span();
        match self.peek() {
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr {
                    kind: ExprKind::UnaryOp {
                        op: UnaryOp::Neg,
                        expr: Box::new(expr),
                    },
                    span,
                })
            }
            Token::Bang => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr {
                    kind: ExprKind::UnaryOp {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    span,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, IonError> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.peek() {
                Token::Question => {
                    self.advance();
                    let span = expr.span;
                    expr = Expr {
                        kind: ExprKind::Try(Box::new(expr)),
                        span,
                    };
                }
                Token::Dot => {
                    self.advance();
                    // .await is special syntax
                    if self.check(&Token::Await) {
                        self.advance();
                        let span = expr.span;
                        expr = Expr {
                            kind: ExprKind::AwaitExpr(Box::new(expr)),
                            span,
                        };
                        continue;
                    }
                    let field = self.eat_ident()?;
                    if self.check(&Token::LParen) {
                        // Method call
                        self.advance();
                        let args = self.parse_call_args()?;
                        self.eat(&Token::RParen)?;
                        let span = expr.span;
                        expr = Expr {
                            kind: ExprKind::MethodCall {
                                expr: Box::new(expr),
                                method: field,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = expr.span;
                        expr = Expr {
                            kind: ExprKind::FieldAccess {
                                expr: Box::new(expr),
                                field,
                            },
                            span,
                        };
                    }
                }
                Token::LBracket => {
                    self.advance();
                    let span = expr.span;
                    // Check for [..end] or [..=end] or [..]
                    if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                        let inclusive = self.check(&Token::DotDotEq);
                        self.advance();
                        let end = if self.check(&Token::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        self.eat(&Token::RBracket)?;
                        expr = Expr {
                            kind: ExprKind::Slice {
                                expr: Box::new(expr),
                                start: None,
                                end,
                                inclusive,
                            },
                            span,
                        };
                        continue;
                    }
                    // Parse index/start using parse_addition (stops before ..)
                    let first = self.parse_addition()?;
                    if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                        let inclusive = self.check(&Token::DotDotEq);
                        self.advance();
                        let end = if self.check(&Token::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        self.eat(&Token::RBracket)?;
                        expr = Expr {
                            kind: ExprKind::Slice {
                                expr: Box::new(expr),
                                start: Some(Box::new(first)),
                                end,
                                inclusive,
                            },
                            span,
                        };
                    } else {
                        self.eat(&Token::RBracket)?;
                        expr = Expr {
                            kind: ExprKind::Index {
                                expr: Box::new(expr),
                                index: Box::new(first),
                            },
                            span,
                        };
                    }
                }
                Token::LParen => {
                    // Check if this is truly a call (the expr must be callable)
                    self.advance();
                    let args = self.parse_call_args()?;
                    self.eat(&Token::RParen)?;
                    let span = expr.span;
                    expr = Expr {
                        kind: ExprKind::Call {
                            func: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> Result<Vec<CallArg>, IonError> {
        let mut args = Vec::new();
        while !self.check(&Token::RParen) && !self.is_at_end() {
            // Check for named argument: `name: value`
            let arg = if let Token::Ident(name) = self.peek().clone() {
                if self.tokens.get(self.pos + 1).map(|t| &t.token) == Some(&Token::Colon) {
                    let name = name.clone();
                    self.advance(); // ident
                    self.advance(); // colon
                    let value = self.parse_expr()?;
                    CallArg {
                        name: Some(name),
                        value,
                    }
                } else {
                    let value = self.parse_expr()?;
                    CallArg { name: None, value }
                }
            } else {
                let value = self.parse_expr()?;
                CallArg { name: None, value }
            };
            args.push(arg);
            if !self.check(&Token::RParen) {
                self.eat(&Token::Comma)?;
            }
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, IonError> {
        let span = self.span();
        match self.peek().clone() {
            Token::Int(n) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Int(n),
                    span,
                })
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Float(n),
                    span,
                })
            }
            Token::True => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span,
                })
            }
            Token::False => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span,
                })
            }
            Token::Str(s) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Str(s),
                    span,
                })
            }
            Token::Bytes(b) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bytes(b),
                    span,
                })
            }
            Token::FStr(template) => {
                self.advance();
                let parts = self.parse_fstr_parts(&template, span)?;
                Ok(Expr {
                    kind: ExprKind::FStr(parts),
                    span,
                })
            }
            Token::None => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::None,
                    span,
                })
            }
            Token::Some => {
                self.advance();
                self.eat(&Token::LParen)?;
                let expr = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(Expr {
                    kind: ExprKind::SomeExpr(Box::new(expr)),
                    span,
                })
            }
            Token::Ok => {
                self.advance();
                self.eat(&Token::LParen)?;
                let expr = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(Expr {
                    kind: ExprKind::OkExpr(Box::new(expr)),
                    span,
                })
            }
            Token::Err => {
                self.advance();
                self.eat(&Token::LParen)?;
                let expr = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(Expr {
                    kind: ExprKind::ErrExpr(Box::new(expr)),
                    span,
                })
            }
            Token::LParen => {
                self.advance();
                // Check for closure: |
                // or tuple: (a, b, c)
                // or grouping: (expr)
                if self.check(&Token::RParen) {
                    self.advance();
                    return Ok(Expr {
                        kind: ExprKind::Unit,
                        span,
                    });
                }
                let first = self.parse_expr()?;
                if self.check(&Token::Comma) {
                    // Tuple
                    let mut items = vec![first];
                    while self.check(&Token::Comma) {
                        self.advance();
                        if self.check(&Token::RParen) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                    self.eat(&Token::RParen)?;
                    Ok(Expr {
                        kind: ExprKind::Tuple(items),
                        span,
                    })
                } else {
                    self.eat(&Token::RParen)?;
                    Ok(first) // grouping
                }
            }
            Token::LBracket => {
                self.advance();
                if self.check(&Token::RBracket) {
                    self.advance();
                    return Ok(Expr {
                        kind: ExprKind::List(vec![]),
                        span,
                    });
                }
                // Check for spread as first entry
                let first_entry = if self.check(&Token::DotDotDot) {
                    self.advance();
                    ListEntry::Spread(self.parse_expr()?)
                } else {
                    ListEntry::Elem(self.parse_expr()?)
                };
                // Check for list comprehension: [expr for pattern in iter]
                if let ListEntry::Elem(ref first) = first_entry {
                    if self.check(&Token::For) {
                        self.advance();
                        let pattern = self.parse_pattern()?;
                        self.eat(&Token::In)?;
                        let iter = self.parse_expr()?;
                        let cond = if self.check(&Token::If) {
                            self.advance();
                            Some(Box::new(self.parse_expr()?))
                        } else {
                            None
                        };
                        self.eat(&Token::RBracket)?;
                        return Ok(Expr {
                            kind: ExprKind::ListComp {
                                expr: Box::new(first.clone()),
                                pattern,
                                iter: Box::new(iter),
                                cond,
                            },
                            span,
                        });
                    }
                }
                let mut items = vec![first_entry];
                while self.check(&Token::Comma) {
                    self.advance();
                    if self.check(&Token::RBracket) {
                        break;
                    }
                    if self.check(&Token::DotDotDot) {
                        self.advance();
                        items.push(ListEntry::Spread(self.parse_expr()?));
                    } else {
                        items.push(ListEntry::Elem(self.parse_expr()?));
                    }
                }
                self.eat(&Token::RBracket)?;
                Ok(Expr {
                    kind: ExprKind::List(items),
                    span,
                })
            }
            Token::HashBrace => {
                self.advance();
                if self.check(&Token::RBrace) {
                    self.advance();
                    return Ok(Expr {
                        kind: ExprKind::Dict(vec![]),
                        span,
                    });
                }
                // Check for spread first entry
                if self.check(&Token::DotDotDot) {
                    return self.parse_dict_entries(span);
                }
                let first_key = self.parse_dict_key()?;
                self.eat(&Token::Colon)?;
                let first_val = self.parse_expr()?;
                // Check for dict comprehension: #{ key: val for pat in iter }
                if self.check(&Token::For) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.eat(&Token::In)?;
                    let iter = self.parse_expr()?;
                    let cond = if self.check(&Token::If) {
                        self.advance();
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    };
                    self.eat(&Token::RBrace)?;
                    return Ok(Expr {
                        kind: ExprKind::DictComp {
                            key: Box::new(first_key),
                            value: Box::new(first_val),
                            pattern,
                            iter: Box::new(iter),
                            cond,
                        },
                        span,
                    });
                }
                let mut entries = vec![DictEntry::KeyValue(first_key, first_val)];
                while self.check(&Token::Comma) {
                    self.advance();
                    if self.check(&Token::RBrace) {
                        break;
                    }
                    if self.check(&Token::DotDotDot) {
                        self.advance();
                        entries.push(DictEntry::Spread(self.parse_expr()?));
                    } else {
                        let key = self.parse_dict_key()?;
                        self.eat(&Token::Colon)?;
                        let value = self.parse_expr()?;
                        entries.push(DictEntry::KeyValue(key, value));
                    }
                }
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::Dict(entries),
                    span,
                })
            }
            Token::PipeSym => {
                // Lambda: |params| body
                self.advance();
                let mut params = Vec::new();
                if !self.check(&Token::PipeSym) {
                    params.push(self.eat_ident()?);
                    while self.check(&Token::Comma) {
                        self.advance();
                        params.push(self.eat_ident()?);
                    }
                }
                self.eat(&Token::PipeSym)?;
                let body = if self.check(&Token::LBrace) {
                    self.advance();
                    let stmts = self.parse_block_stmts()?;
                    self.eat(&Token::RBrace)?;
                    Expr {
                        kind: ExprKind::Block(stmts),
                        span,
                    }
                } else {
                    self.parse_expr()?
                };
                Ok(Expr {
                    kind: ExprKind::Lambda {
                        params,
                        body: Box::new(body),
                    },
                    span,
                })
            }
            Token::Or => {
                // Zero-arg lambda: || body
                self.advance();
                let body = if self.check(&Token::LBrace) {
                    self.advance();
                    let stmts = self.parse_block_stmts()?;
                    self.eat(&Token::RBrace)?;
                    Expr {
                        kind: ExprKind::Block(stmts),
                        span,
                    }
                } else {
                    self.parse_expr()?
                };
                Ok(Expr {
                    kind: ExprKind::Lambda {
                        params: Vec::new(),
                        body: Box::new(body),
                    },
                    span,
                })
            }
            Token::If => self.parse_if_expr(),
            Token::Match => self.parse_match_expr(),
            Token::Loop => {
                self.advance();
                self.eat(&Token::LBrace)?;
                let body = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::LoopExpr(body),
                    span,
                })
            }
            Token::Try => {
                self.advance();
                self.eat(&Token::LBrace)?;
                let body = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                self.eat(&Token::Catch)?;
                let var = match self.peek() {
                    Token::Ident(name) => {
                        let name = name.clone();
                        self.advance();
                        name
                    }
                    _ => {
                        return Err(IonError::parse(
                            ion_str!("expected identifier after 'catch'").to_string(),
                            self.span().line,
                            self.span().col,
                        ));
                    }
                };
                self.eat(&Token::LBrace)?;
                let handler = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::TryCatch { body, var, handler },
                    span,
                })
            }
            Token::Async => {
                self.advance();
                self.eat(&Token::LBrace)?;
                let body = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::AsyncBlock(body),
                    span,
                })
            }
            Token::Spawn => {
                self.advance();
                let expr = self.parse_expr()?;
                Ok(Expr {
                    kind: ExprKind::SpawnExpr(Box::new(expr)),
                    span,
                })
            }
            Token::Select => {
                self.advance();
                self.eat(&Token::LBrace)?;
                let mut branches = Vec::new();
                while !self.check(&Token::RBrace) {
                    let pattern = self.parse_pattern()?;
                    self.eat(&Token::Eq)?;
                    let future_expr = self.parse_expr()?;
                    self.eat(&Token::Arrow)?;
                    let body = self.parse_expr()?;
                    branches.push(SelectBranch {
                        pattern,
                        future_expr,
                        body,
                    });
                    if self.check(&Token::Comma) {
                        self.advance();
                    }
                }
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::SelectExpr(branches),
                    span,
                })
            }
            Token::LBrace => {
                self.advance();
                let stmts = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Ok(Expr {
                    kind: ExprKind::Block(stmts),
                    span,
                })
            }
            Token::Ident(name) => {
                self.advance();
                // Check for :: (enum variant or module path)
                if self.check(&Token::ColonColon) {
                    self.advance();
                    let second = self.eat_ident()?;
                    // If more :: segments follow, it's a module path: a::b::c::...
                    if self.check(&Token::ColonColon) {
                        let mut segments = vec![name, second];
                        while self.check(&Token::ColonColon) {
                            self.advance();
                            segments.push(self.eat_ident()?);
                        }
                        Ok(Expr {
                            kind: ExprKind::ModulePath(segments),
                            span,
                        })
                    }
                    // Two segments: could be enum variant or module path
                    // Enum variants have uppercase first char (e.g. Color::Red)
                    else if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        // Enum variant — check for call args
                        if self.check(&Token::LParen) {
                            self.advance();
                            let mut args = Vec::new();
                            while !self.check(&Token::RParen) && !self.is_at_end() {
                                args.push(self.parse_expr()?);
                                if !self.check(&Token::RParen) {
                                    self.eat(&Token::Comma)?;
                                }
                            }
                            self.eat(&Token::RParen)?;
                            Ok(Expr {
                                kind: ExprKind::EnumVariantCall {
                                    enum_name: name,
                                    variant: second,
                                    args,
                                },
                                span,
                            })
                        } else {
                            Ok(Expr {
                                kind: ExprKind::EnumVariant {
                                    enum_name: name,
                                    variant: second,
                                },
                                span,
                            })
                        }
                    }
                    // Two segments, lowercase first: module path (e.g. fs::read)
                    else {
                        Ok(Expr {
                            kind: ExprKind::ModulePath(vec![name, second]),
                            span,
                        })
                    }
                }
                // Check for struct construction: Name { ... }
                else if self.check(&Token::LBrace)
                    && name.chars().next().is_some_and(|c| c.is_uppercase())
                {
                    self.advance();
                    let mut fields = Vec::new();
                    let mut spread = None;
                    while !self.check(&Token::RBrace) && !self.is_at_end() {
                        if self.check(&Token::DotDotDot) {
                            self.advance();
                            spread = Some(Box::new(self.parse_expr()?));
                            if !self.check(&Token::RBrace) {
                                self.eat(&Token::Comma)?;
                            }
                            continue;
                        }
                        let field_name = self.eat_ident()?;
                        self.eat(&Token::Colon)?;
                        let field_value = self.parse_expr()?;
                        fields.push((field_name, field_value));
                        if !self.check(&Token::RBrace) {
                            self.eat(&Token::Comma)?;
                        }
                    }
                    self.eat(&Token::RBrace)?;
                    Ok(Expr {
                        kind: ExprKind::StructConstruct {
                            name,
                            fields,
                            spread,
                        },
                        span,
                    })
                } else {
                    Ok(Expr {
                        kind: ExprKind::Ident(name),
                        span,
                    })
                }
            }
            _ => {
                let s = self.span();
                Err(IonError::parse(
                    format!("{}{:?}", ion_str!("unexpected token: "), self.peek()),
                    s.line,
                    s.col,
                ))
            }
        }
    }

    /// Parse dict entries when the first token is `...` (spread).
    /// Parse a dict key: if it's an identifier followed by `:`, treat as string literal.
    fn parse_dict_key(&mut self) -> Result<Expr, IonError> {
        let span = self.span();
        if let Token::Ident(name) = self.peek().clone() {
            // Lookahead: if next token is `:`, this is a shorthand key
            if self.tokens.get(self.pos + 1).map(|t| &t.token) == Some(&Token::Colon) {
                self.advance(); // consume the identifier
                return Ok(Expr {
                    kind: ExprKind::Str(name),
                    span,
                });
            }
        }
        self.parse_expr()
    }

    fn parse_dict_entries(&mut self, span: Span) -> Result<Expr, IonError> {
        let mut entries = Vec::new();
        // First entry is a spread
        self.advance(); // consume `...`
        entries.push(DictEntry::Spread(self.parse_expr()?));
        while self.check(&Token::Comma) {
            self.advance();
            if self.check(&Token::RBrace) {
                break;
            }
            if self.check(&Token::DotDotDot) {
                self.advance();
                entries.push(DictEntry::Spread(self.parse_expr()?));
            } else {
                let key = self.parse_dict_key()?;
                self.eat(&Token::Colon)?;
                let value = self.parse_expr()?;
                entries.push(DictEntry::KeyValue(key, value));
            }
        }
        self.eat(&Token::RBrace)?;
        Ok(Expr {
            kind: ExprKind::Dict(entries),
            span,
        })
    }

    fn parse_if_expr(&mut self) -> Result<Expr, IonError> {
        let span = self.span();
        self.eat(&Token::If)?;

        // if let pattern = expr { ... }
        if self.check(&Token::Let) {
            self.advance();
            let pattern = self.parse_pattern()?;
            self.eat(&Token::Eq)?;
            let expr = self.parse_expr()?;
            self.eat(&Token::LBrace)?;
            let then_body = self.parse_block_stmts()?;
            self.eat(&Token::RBrace)?;
            let else_body = if self.check(&Token::Else) {
                self.advance();
                self.eat(&Token::LBrace)?;
                let stmts = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Some(stmts)
            } else {
                None
            };
            return Ok(Expr {
                kind: ExprKind::IfLet {
                    pattern,
                    expr: Box::new(expr),
                    then_body,
                    else_body,
                },
                span,
            });
        }

        let cond = self.parse_expr()?;
        self.eat(&Token::LBrace)?;
        let then_body = self.parse_block_stmts()?;
        self.eat(&Token::RBrace)?;
        let else_body = if self.check(&Token::Else) {
            self.advance();
            if self.check(&Token::If) {
                // else if
                let else_if = self.parse_if_expr()?;
                Some(vec![Stmt {
                    kind: StmtKind::ExprStmt {
                        expr: else_if,
                        has_semi: false,
                    },
                    span: self.prev_span(),
                }])
            } else {
                self.eat(&Token::LBrace)?;
                let stmts = self.parse_block_stmts()?;
                self.eat(&Token::RBrace)?;
                Some(stmts)
            }
        } else {
            None
        };
        Ok(Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then_body,
                else_body,
            },
            span,
        })
    }

    fn parse_match_expr(&mut self) -> Result<Expr, IonError> {
        let span = self.span();
        self.eat(&Token::Match)?;
        let expr = self.parse_expr()?;
        self.eat(&Token::LBrace)?;
        let mut arms = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            let guard = if self.check(&Token::If) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.eat(&Token::Arrow)?;
            let body = self.parse_expr()?;
            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });
            if !self.check(&Token::RBrace) {
                self.eat(&Token::Comma)?;
            }
        }
        self.eat(&Token::RBrace)?;
        Ok(Expr {
            kind: ExprKind::Match {
                expr: Box::new(expr),
                arms,
            },
            span,
        })
    }

    // --- Pattern Parsing ---

    fn parse_pattern(&mut self) -> Result<Pattern, IonError> {
        match self.peek().clone() {
            Token::Ident(name) if name == "_" => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            Token::Ident(name) => {
                self.advance();
                // Check for :: (enum variant)
                if self.check(&Token::ColonColon) {
                    self.advance();
                    let variant = self.eat_ident()?;
                    let fields = if self.check(&Token::LParen) {
                        self.advance();
                        let mut pats = Vec::new();
                        while !self.check(&Token::RParen) && !self.is_at_end() {
                            pats.push(self.parse_pattern()?);
                            if !self.check(&Token::RParen) {
                                self.eat(&Token::Comma)?;
                            }
                        }
                        self.eat(&Token::RParen)?;
                        EnumPatternFields::Positional(pats)
                    } else if self.check(&Token::LBrace) {
                        self.advance();
                        let mut fields = Vec::new();
                        while !self.check(&Token::RBrace) && !self.is_at_end() {
                            let field_name = self.eat_ident()?;
                            let pat = if self.check(&Token::Colon) {
                                self.advance();
                                Some(self.parse_pattern()?)
                            } else {
                                None
                            };
                            fields.push((field_name, pat));
                            if !self.check(&Token::RBrace) {
                                self.eat(&Token::Comma)?;
                            }
                        }
                        self.eat(&Token::RBrace)?;
                        EnumPatternFields::Named(fields)
                    } else {
                        EnumPatternFields::None
                    };
                    Ok(Pattern::EnumVariant {
                        enum_name: name,
                        variant,
                        fields,
                    })
                }
                // Check for struct pattern: Name { ... }
                else if self.check(&Token::LBrace)
                    && name.chars().next().is_some_and(|c| c.is_uppercase())
                {
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.check(&Token::RBrace) && !self.is_at_end() {
                        let field_name = self.eat_ident()?;
                        let pat = if self.check(&Token::Colon) {
                            self.advance();
                            Some(self.parse_pattern()?)
                        } else {
                            None
                        };
                        fields.push((field_name, pat));
                        if !self.check(&Token::RBrace) {
                            self.eat(&Token::Comma)?;
                        }
                    }
                    self.eat(&Token::RBrace)?;
                    Ok(Pattern::Struct { name, fields })
                } else {
                    Ok(Pattern::Ident(name))
                }
            }
            Token::Int(n) => {
                self.advance();
                Ok(Pattern::Int(n))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Pattern::Float(n))
            }
            Token::True => {
                self.advance();
                Ok(Pattern::Bool(true))
            }
            Token::False => {
                self.advance();
                Ok(Pattern::Bool(false))
            }
            Token::Str(s) => {
                self.advance();
                Ok(Pattern::Str(s))
            }
            Token::Bytes(b) => {
                self.advance();
                Ok(Pattern::Bytes(b))
            }
            Token::None => {
                self.advance();
                Ok(Pattern::None)
            }
            Token::Some => {
                self.advance();
                self.eat(&Token::LParen)?;
                let inner = self.parse_pattern()?;
                self.eat(&Token::RParen)?;
                Ok(Pattern::Some(Box::new(inner)))
            }
            Token::Ok => {
                self.advance();
                self.eat(&Token::LParen)?;
                let inner = self.parse_pattern()?;
                self.eat(&Token::RParen)?;
                Ok(Pattern::Ok(Box::new(inner)))
            }
            Token::Err => {
                self.advance();
                self.eat(&Token::LParen)?;
                let inner = self.parse_pattern()?;
                self.eat(&Token::RParen)?;
                Ok(Pattern::Err(Box::new(inner)))
            }
            Token::LParen => {
                self.advance();
                let mut pats = Vec::new();
                while !self.check(&Token::RParen) && !self.is_at_end() {
                    pats.push(self.parse_pattern()?);
                    if !self.check(&Token::RParen) {
                        self.eat(&Token::Comma)?;
                    }
                }
                self.eat(&Token::RParen)?;
                Ok(Pattern::Tuple(pats))
            }
            Token::LBracket => {
                self.advance();
                let mut pats = Vec::new();
                let mut rest = None;
                while !self.check(&Token::RBracket) && !self.is_at_end() {
                    if self.check(&Token::DotDotDot) {
                        self.advance();
                        let rest_name = self.eat_ident()?;
                        rest = Some(Box::new(Pattern::Ident(rest_name)));
                        if !self.check(&Token::RBracket) {
                            self.eat(&Token::Comma)?;
                        }
                        continue;
                    }
                    pats.push(self.parse_pattern()?);
                    if !self.check(&Token::RBracket) {
                        self.eat(&Token::Comma)?;
                    }
                }
                self.eat(&Token::RBracket)?;
                Ok(Pattern::List(pats, rest))
            }
            _ => {
                let s = self.span();
                Err(IonError::parse(
                    format!(
                        "{}{:?}",
                        ion_str!("unexpected token in pattern: "),
                        self.peek()
                    ),
                    s.line,
                    s.col,
                ))
            }
        }
    }

    // --- F-string parsing ---

    fn parse_fstr_parts(&self, template: &str, span: Span) -> Result<Vec<FStrPart>, IonError> {
        let mut parts = Vec::new();
        let mut chars = template.chars().peekable();
        let mut current = String::new();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                if !current.is_empty() {
                    parts.push(FStrPart::Literal(std::mem::take(&mut current)));
                }
                let mut expr_str = String::new();
                let mut depth = 1;
                for inner in chars.by_ref() {
                    if inner == '{' {
                        depth += 1;
                    } else if inner == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    expr_str.push(inner);
                }
                if depth != 0 {
                    return Err(IonError::parse(
                        ion_str!("unterminated expression in f-string").to_string(),
                        span.line,
                        span.col,
                    ));
                }
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                let tokens = lexer.tokenize()?;
                let mut parser = Parser::new(tokens);
                let expr = parser.parse_expr()?;
                if !parser.is_at_end() {
                    let s = parser.span();
                    return Err(IonError::parse(
                        format!(
                            "{}{:?}",
                            ion_str!("unexpected token in f-string expression: "),
                            parser.peek()
                        ),
                        span.line,
                        span.col + s.col.saturating_sub(1),
                    ));
                }
                parts.push(FStrPart::Expr(expr));
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            parts.push(FStrPart::Literal(current));
        }
        Ok(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Program {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn test_let_stmt() {
        let prog = parse("let x = 42;");
        assert_eq!(prog.stmts.len(), 1);
        assert!(matches!(
            &prog.stmts[0].kind,
            StmtKind::Let { mutable: false, .. }
        ));
    }

    #[test]
    fn test_let_mut() {
        let prog = parse("let mut x = 42;");
        assert!(matches!(
            &prog.stmts[0].kind,
            StmtKind::Let { mutable: true, .. }
        ));
    }

    #[test]
    fn test_fn_decl() {
        let prog = parse("fn add(a, b) { a + b }");
        assert!(matches!(&prog.stmts[0].kind, StmtKind::FnDecl { .. }));
    }

    #[test]
    fn test_if_expr() {
        let prog = parse("let x = if true { 1 } else { 2 };");
        assert!(matches!(&prog.stmts[0].kind, StmtKind::Let { .. }));
    }

    #[test]
    fn test_match_expr() {
        let prog = parse(r#"let x = match y { 1 => "one", _ => "other" };"#);
        assert!(matches!(&prog.stmts[0].kind, StmtKind::Let { .. }));
    }

    #[test]
    fn test_lambda() {
        let prog = parse("let f = |x| x + 1;");
        assert!(matches!(&prog.stmts[0].kind, StmtKind::Let { .. }));
    }

    #[test]
    fn test_dict() {
        let prog = parse(r#"let d = #{ "a": 1, "b": 2 };"#);
        assert!(matches!(&prog.stmts[0].kind, StmtKind::Let { .. }));
    }

    #[test]
    fn test_for_loop() {
        let prog = parse("for x in items { x; }");
        assert!(matches!(&prog.stmts[0].kind, StmtKind::For { .. }));
    }
}
