//! AST → Bytecode compiler for the Ion VM.

use crate::ast::*;
use crate::bytecode::{Chunk, FnProto, Op};
use crate::error::IonError;
use crate::value::{Value, FnChunkCache};

/// A local variable tracked at compile time for stack-slot resolution.
#[derive(Debug, Clone)]
struct Local {
    name: String,
    depth: usize,
}

pub struct Compiler {
    chunk: Chunk,
    /// Precompiled function body chunks, keyed by fn_id.
    pub fn_chunks: FnChunkCache,
    /// Whether the next expression is in tail position (for TCO).
    in_tail_position: bool,
    /// Compile-time local variable tracking for stack-slot resolution.
    locals: Vec<Local>,
    /// Current scope depth.
    scope_depth: usize,
    /// Whether locals must also be defined in env (needed when closures exist).
    needs_env_locals: bool,
    /// Pending break jump offsets to patch when loop ends.
    break_jumps: Vec<usize>,
    /// Loop start offset for continue jumps (used by while/loop).
    continue_target: Option<usize>,
    /// Whether we're inside a for-loop (break needs iterator cleanup, continue needs scope cleanup).
    in_for_loop: bool,
    /// Scope depth at the start of the current loop (for break/continue scope cleanup).
    loop_scope_depth: usize,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(),
            fn_chunks: FnChunkCache::new(),
            in_tail_position: false,
            locals: Vec::new(),
            scope_depth: 0,
            needs_env_locals: true, // conservative default for top-level
            break_jumps: Vec::new(),
            continue_target: None,
            in_for_loop: false,
            loop_scope_depth: 0,
        }
    }

    /// Check if a list of statements contains any closures (lambdas or inner fn decls).
    fn stmts_have_closures(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::FnDecl { body: _, .. } => {
                    // Inner fn decl itself is a closure; also check its body
                    return true;
                    // Note: we don't need to recurse — just the presence of a fn decl
                    // in the current scope means outer locals might be captured
                }
                StmtKind::ExprStmt { expr, .. } => {
                    if Self::expr_has_closures(expr) { return true; }
                }
                StmtKind::Let { value, .. } => {
                    if Self::expr_has_closures(value) { return true; }
                }
                StmtKind::For { body, iter, .. } => {
                    if Self::expr_has_closures(iter) { return true; }
                    if Self::stmts_have_closures(body) { return true; }
                }
                StmtKind::While { cond, body } => {
                    if Self::expr_has_closures(cond) { return true; }
                    if Self::stmts_have_closures(body) { return true; }
                }
                StmtKind::Loop { body } => {
                    if Self::stmts_have_closures(body) { return true; }
                }
                StmtKind::Return { value } => {
                    if let Some(e) = value { if Self::expr_has_closures(e) { return true; } }
                }
                StmtKind::Assign { value, .. } => {
                    if Self::expr_has_closures(value) { return true; }
                }
                StmtKind::WhileLet { expr, body, .. } => {
                    if Self::expr_has_closures(expr) { return true; }
                    if Self::stmts_have_closures(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    fn expr_has_closures(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Lambda { .. } => true,
            ExprKind::If { cond, then_body, else_body } => {
                Self::expr_has_closures(cond)
                    || Self::stmts_have_closures(then_body)
                    || else_body.as_ref().map_or(false, |b| Self::stmts_have_closures(b))
            }
            ExprKind::Block(stmts) => Self::stmts_have_closures(stmts),
            ExprKind::Call { func, args } => {
                Self::expr_has_closures(func)
                    || args.iter().any(|a| Self::expr_has_closures(&a.value))
            }
            ExprKind::MethodCall { expr, args, .. } => {
                Self::expr_has_closures(expr)
                    || args.iter().any(|a| Self::expr_has_closures(&a.value))
            }
            ExprKind::BinOp { left, right, .. } => {
                Self::expr_has_closures(left) || Self::expr_has_closures(right)
            }
            ExprKind::UnaryOp { expr, .. } => Self::expr_has_closures(expr),
            ExprKind::PipeOp { left, right } => {
                Self::expr_has_closures(left) || Self::expr_has_closures(right)
            }
            ExprKind::Match { expr, arms } => {
                Self::expr_has_closures(expr)
                    || arms.iter().any(|a| Self::expr_has_closures(&a.body))
            }
            ExprKind::List(items) => items.iter().any(|e| Self::expr_has_closures(e)),
            ExprKind::Tuple(items) => items.iter().any(|e| Self::expr_has_closures(e)),
            ExprKind::ListComp { expr, iter, cond, .. } => {
                Self::expr_has_closures(expr) || Self::expr_has_closures(iter)
                    || cond.as_ref().map_or(false, |c| Self::expr_has_closures(c))
            }
            ExprKind::IfLet { expr, then_body, else_body, .. } => {
                Self::expr_has_closures(expr)
                    || Self::stmts_have_closures(then_body)
                    || else_body.as_ref().map_or(false, |b| Self::stmts_have_closures(b))
            }
            _ => false,
        }
    }

    /// Try to constant-fold a binary operation on two literal operands.
    #[cfg(feature = "optimize")]
    fn try_fold_binop(left: &Expr, op: &BinOp, right: &Expr) -> Option<Value> {
        match (&left.kind, op, &right.kind) {
            // Int op Int
            (ExprKind::Int(a), BinOp::Add, ExprKind::Int(b)) => Some(Value::Int(a.wrapping_add(*b))),
            (ExprKind::Int(a), BinOp::Sub, ExprKind::Int(b)) => Some(Value::Int(a.wrapping_sub(*b))),
            (ExprKind::Int(a), BinOp::Mul, ExprKind::Int(b)) => Some(Value::Int(a.wrapping_mul(*b))),
            (ExprKind::Int(a), BinOp::Div, ExprKind::Int(b)) if *b != 0 => Some(Value::Int(a / b)),
            (ExprKind::Int(a), BinOp::Mod, ExprKind::Int(b)) if *b != 0 => Some(Value::Int(a % b)),
            (ExprKind::Int(a), BinOp::Eq, ExprKind::Int(b)) => Some(Value::Bool(a == b)),
            (ExprKind::Int(a), BinOp::Ne, ExprKind::Int(b)) => Some(Value::Bool(a != b)),
            (ExprKind::Int(a), BinOp::Lt, ExprKind::Int(b)) => Some(Value::Bool(a < b)),
            (ExprKind::Int(a), BinOp::Gt, ExprKind::Int(b)) => Some(Value::Bool(a > b)),
            (ExprKind::Int(a), BinOp::Le, ExprKind::Int(b)) => Some(Value::Bool(a <= b)),
            (ExprKind::Int(a), BinOp::Ge, ExprKind::Int(b)) => Some(Value::Bool(a >= b)),
            (ExprKind::Int(a), BinOp::BitAnd, ExprKind::Int(b)) => Some(Value::Int(a & b)),
            (ExprKind::Int(a), BinOp::BitOr, ExprKind::Int(b)) => Some(Value::Int(a | b)),
            (ExprKind::Int(a), BinOp::BitXor, ExprKind::Int(b)) => Some(Value::Int(a ^ b)),
            (ExprKind::Int(a), BinOp::Shl, ExprKind::Int(b)) => Some(Value::Int(a << (*b as u32))),
            (ExprKind::Int(a), BinOp::Shr, ExprKind::Int(b)) => Some(Value::Int(a >> (*b as u32))),
            // Float op Float
            (ExprKind::Float(a), BinOp::Add, ExprKind::Float(b)) => Some(Value::Float(a + b)),
            (ExprKind::Float(a), BinOp::Sub, ExprKind::Float(b)) => Some(Value::Float(a - b)),
            (ExprKind::Float(a), BinOp::Mul, ExprKind::Float(b)) => Some(Value::Float(a * b)),
            (ExprKind::Float(a), BinOp::Div, ExprKind::Float(b)) => Some(Value::Float(a / b)),
            (ExprKind::Float(a), BinOp::Mod, ExprKind::Float(b)) => Some(Value::Float(a % b)),
            // Int op Float / Float op Int
            (ExprKind::Int(a), BinOp::Add, ExprKind::Float(b)) => Some(Value::Float(*a as f64 + b)),
            (ExprKind::Float(a), BinOp::Add, ExprKind::Int(b)) => Some(Value::Float(a + *b as f64)),
            (ExprKind::Int(a), BinOp::Sub, ExprKind::Float(b)) => Some(Value::Float(*a as f64 - b)),
            (ExprKind::Float(a), BinOp::Sub, ExprKind::Int(b)) => Some(Value::Float(a - *b as f64)),
            (ExprKind::Int(a), BinOp::Mul, ExprKind::Float(b)) => Some(Value::Float(*a as f64 * b)),
            (ExprKind::Float(a), BinOp::Mul, ExprKind::Int(b)) => Some(Value::Float(a * *b as f64)),
            (ExprKind::Int(a), BinOp::Div, ExprKind::Float(b)) => Some(Value::Float(*a as f64 / b)),
            (ExprKind::Float(a), BinOp::Div, ExprKind::Int(b)) => Some(Value::Float(a / *b as f64)),
            // String concat
            (ExprKind::Str(a), BinOp::Add, ExprKind::Str(b)) => {
                let mut s = a.clone();
                s.push_str(b);
                Some(Value::Str(s))
            }
            // Bool logic
            (ExprKind::Bool(a), BinOp::And, ExprKind::Bool(b)) => Some(Value::Bool(*a && *b)),
            (ExprKind::Bool(a), BinOp::Or, ExprKind::Bool(b)) => Some(Value::Bool(*a || *b)),
            (ExprKind::Bool(a), BinOp::Eq, ExprKind::Bool(b)) => Some(Value::Bool(a == b)),
            (ExprKind::Bool(a), BinOp::Ne, ExprKind::Bool(b)) => Some(Value::Bool(a != b)),
            _ => None,
        }
    }

    /// Try to constant-fold a unary operation on a literal operand.
    #[cfg(feature = "optimize")]
    fn try_fold_unary(op: &UnaryOp, inner: &Expr) -> Option<Value> {
        match (op, &inner.kind) {
            (UnaryOp::Neg, ExprKind::Int(v)) => Some(Value::Int(-v)),
            (UnaryOp::Neg, ExprKind::Float(v)) => Some(Value::Float(-v)),
            (UnaryOp::Not, ExprKind::Bool(v)) => Some(Value::Bool(!v)),
            _ => None,
        }
    }

    /// Check if a statement is terminal (control never continues past it).
    #[cfg(feature = "optimize")]
    fn stmt_is_terminal(stmt: &Stmt) -> bool {
        matches!(&stmt.kind, StmtKind::Return { .. } | StmtKind::Break { .. } | StmtKind::Continue)
    }

    /// Resolve a local variable name to its slot index (searching innermost first).
    fn resolve_local(&self, name: &str) -> Option<usize> {
        for (i, local) in self.locals.iter().enumerate().rev() {
            if local.name == name {
                return Some(i);
            }
        }
        None
    }

    /// Add a local variable to the compile-time tracking.
    fn add_local(&mut self, name: String, _mutable: bool) {
        self.locals.push(Local { name, depth: self.scope_depth });
    }

    /// Begin a new compile-time scope and emit PushScope.
    fn begin_scope(&mut self, line: usize) {
        self.scope_depth += 1;
        self.chunk.emit_op(Op::PushScope, line);
    }

    /// Emit a variable read (GetLocalSlot or GetGlobal).
    fn emit_get_var(&mut self, name: &str, line: usize) {
        if let Some(slot) = self.resolve_local(name) {
            self.chunk.emit_op_u16(Op::GetLocalSlot, slot as u16, line);
        } else {
            let idx = self.chunk.add_constant(Value::Str(name.to_string()));
            self.chunk.emit_op_u16(Op::GetGlobal, idx, line);
        }
    }

    /// Define a new local variable. When closures exist, also stores in env.
    /// The value must be on top of the stack.
    fn emit_define_local(&mut self, name: &str, mutable: bool, line: usize) {
        self.add_local(name.to_string(), mutable);
        if self.needs_env_locals {
            // Dup value: one copy for env (closure capture), one for slot
            self.chunk.emit_op(Op::Dup, line);
            let idx = self.chunk.add_constant(Value::Str(name.to_string()));
            self.chunk.emit_op_u16(Op::DefineLocal, idx, line);
            self.chunk.emit(if mutable { 1 } else { 0 }, line);
        }
        self.chunk.emit_op_u8(Op::DefineLocalSlot, if mutable { 1 } else { 0 }, line);
    }

    /// Emit a variable write (SetLocalSlot or SetGlobal).
    fn emit_set_var(&mut self, name: &str, line: usize) {
        if let Some(slot) = self.resolve_local(name) {
            self.chunk.emit_op_u16(Op::SetLocalSlot, slot as u16, line);
        } else {
            let idx = self.chunk.add_constant(Value::Str(name.to_string()));
            self.chunk.emit_op_u16(Op::SetGlobal, idx, line);
        }
    }

    /// End the current compile-time scope, emit PopScope.
    fn end_scope(&mut self, line: usize) {
        while let Some(local) = self.locals.last() {
            if local.depth < self.scope_depth {
                break;
            }
            self.locals.pop();
        }
        self.scope_depth -= 1;
        self.chunk.emit_op(Op::PopScope, line);
    }

    pub fn compile_program(mut self, program: &Program) -> Result<(Chunk, FnChunkCache), IonError> {
        let len = program.stmts.len();
        for (i, stmt) in program.stmts.iter().enumerate() {
            let is_last = i == len - 1;
            match &stmt.kind {
                StmtKind::ExprStmt { expr, has_semi } => {
                    self.compile_expr(expr)?;
                    if is_last && !has_semi {
                        // Keep the value as the program result
                    } else {
                        self.chunk.emit_op(Op::Pop, stmt.span.line);
                    }
                }
                _ => {
                    self.compile_stmt(stmt)?;
                    if is_last {
                        // Statements produce Unit as the program result
                        self.chunk.emit_op(Op::Unit, stmt.span.line);
                    }
                }
            }
            #[cfg(feature = "optimize")]
            if !is_last && Self::stmt_is_terminal(stmt) {
                break;
            }
        }
        if program.stmts.is_empty() {
            self.chunk.emit_op(Op::Unit, 0);
        }
        self.chunk.emit_op(Op::Return, 0);
        #[cfg(feature = "optimize")]
        self.chunk.peephole_optimize();
        Ok((self.chunk, self.fn_chunks))
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), IonError> {
        let line = stmt.span.line;
        match &stmt.kind {
            StmtKind::Let { mutable, pattern, value } => {
                self.compile_expr(value)?;
                self.compile_let_pattern(pattern, *mutable, line)?;
            }
            StmtKind::ExprStmt { expr, .. } => {
                self.compile_expr(expr)?;
                self.chunk.emit_op(Op::Pop, line);
            }
            StmtKind::FnDecl { name, params, body } => {
                self.compile_fn_decl(name, params, body, line)?;
            }
            StmtKind::For { pattern, iter, body } => {
                self.compile_for(pattern, iter, body, line)?;
            }
            StmtKind::While { cond, body } => {
                self.compile_while(cond, body, line)?;
            }
            StmtKind::Loop { body } => {
                self.compile_loop(body, line)?;
            }
            StmtKind::Break { value } => {
                if let Some(expr) = value {
                    self.compile_expr(expr)?;
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }
                // Pop all scopes back to the loop's scope level
                for _ in self.loop_scope_depth..self.scope_depth {
                    self.chunk.emit_op(Op::PopScope, line);
                }
                if self.in_for_loop {
                    self.chunk.emit_op(Op::IterDrop, line);
                }
                let jump = self.chunk.emit_jump(Op::Jump, line);
                self.break_jumps.push(jump);
            }
            StmtKind::Continue => {
                if let Some(target) = self.continue_target {
                    // Pop all scopes back to the loop's scope level
                    for _ in self.loop_scope_depth..self.scope_depth {
                        self.chunk.emit_op(Op::PopScope, line);
                    }
                    if self.in_for_loop {
                        // For-loop: push Unit placeholder for IterNext
                        self.chunk.emit_op(Op::Unit, line);
                    }
                    let offset = self.chunk.len() - target + 3;
                    self.chunk.emit_op_u16(Op::Loop, offset as u16, line);
                } else {
                    self.chunk.emit_jump(Op::Jump, line);
                }
            }
            StmtKind::Return { value } => {
                if let Some(expr) = value {
                    let saved = self.in_tail_position;
                    self.in_tail_position = true;
                    self.compile_expr(expr)?;
                    self.in_tail_position = saved;
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }
                self.chunk.emit_op(Op::Return, line);
            }
            StmtKind::Assign { target, op, value } => {
                self.compile_assign(target, op, value, line)?;
                self.chunk.emit_op(Op::Pop, line); // discard assignment result
            }
            StmtKind::WhileLet { pattern, expr, body } => {
                let saved_breaks = std::mem::take(&mut self.break_jumps);
                let saved_in_for = self.in_for_loop;
                let saved_loop_depth = self.loop_scope_depth;
                self.in_for_loop = false;
                self.loop_scope_depth = self.scope_depth;
                let saved_continue = self.continue_target.take();

                let loop_start = self.chunk.len();
                self.continue_target = Some(loop_start);

                // Evaluate expression
                self.compile_expr(expr)?;

                // Test pattern
                self.chunk.emit_op(Op::Dup, line); // keep value for binding
                self.compile_pattern_test(pattern, line)?;

                let exit_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true

                // Pattern matched — bind and execute body
                self.begin_scope(line);
                self.compile_pattern_bind(pattern, line)?;
                for stmt in body {
                    self.compile_stmt(stmt)?;
                    #[cfg(feature = "optimize")]
                    if Self::stmt_is_terminal(stmt) { break; }
                }
                self.end_scope(line);

                let offset = self.chunk.len() - loop_start + 3;
                self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

                self.chunk.patch_jump(exit_jump);
                self.chunk.emit_op(Op::Pop, line); // pop false
                self.chunk.emit_op(Op::Pop, line); // pop the duped value

                for jump in &self.break_jumps {
                    self.chunk.patch_jump(*jump);
                }
                self.break_jumps = saved_breaks;
                self.continue_target = saved_continue;
                self.in_for_loop = saved_in_for;
                self.loop_scope_depth = saved_loop_depth;
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), IonError> {
        let line = expr.span.line;
        let col = expr.span.col;
        // Save tail position — only Call, If, Block, Match, IfLet propagate it
        let was_tail = self.in_tail_position;
        self.in_tail_position = false;
        match &expr.kind {
            ExprKind::Int(n) => {
                self.chunk.emit_constant(Value::Int(*n), line);
            }
            ExprKind::Float(n) => {
                self.chunk.emit_constant(Value::Float(*n), line);
            }
            ExprKind::Bool(b) => {
                self.chunk.emit_op(if *b { Op::True } else { Op::False }, line);
            }
            ExprKind::Str(s) => {
                self.chunk.emit_constant(Value::Str(s.clone()), line);
            }
            ExprKind::Bytes(b) => {
                self.chunk.emit_constant(Value::Bytes(b.clone()), line);
            }
            ExprKind::Unit => {
                self.chunk.emit_op(Op::Unit, line);
            }
            ExprKind::None => {
                self.chunk.emit_op(Op::None, line);
            }
            ExprKind::SomeExpr(inner) => {
                self.compile_expr(inner)?;
                self.chunk.emit_op(Op::WrapSome, line);
            }
            ExprKind::OkExpr(inner) => {
                self.compile_expr(inner)?;
                self.chunk.emit_op(Op::WrapOk, line);
            }
            ExprKind::ErrExpr(inner) => {
                self.compile_expr(inner)?;
                self.chunk.emit_op(Op::WrapErr, line);
            }

            ExprKind::Ident(name) => {
                if let Some(slot) = self.resolve_local(name) {
                    self.chunk.emit_op_u16(Op::GetLocalSlot, slot as u16, line);
                } else {
                    let idx = self.chunk.add_constant(Value::Str(name.clone()));
                    self.chunk.emit_op_u16(Op::GetGlobal, idx, line);
                }
            }

            ExprKind::BinOp { left, op, right } => {
                // Constant folding: evaluate at compile time if both sides are literals
                #[cfg(feature = "optimize")]
                let folded = Self::try_fold_binop(left, op, right);
                #[cfg(not(feature = "optimize"))]
                let folded: Option<Value> = None;
                if let Some(val) = folded {
                    self.chunk.emit_constant(val, line);
                } else {
                    match op {
                        BinOp::And => {
                            self.compile_expr(left)?;
                            let jump = self.chunk.emit_jump(Op::And, line);
                            self.chunk.emit_op(Op::Pop, line);
                            self.compile_expr(right)?;
                            self.chunk.patch_jump(jump);
                        }
                        BinOp::Or => {
                            self.compile_expr(left)?;
                            let jump = self.chunk.emit_jump(Op::Or, line);
                            self.chunk.emit_op(Op::Pop, line);
                            self.compile_expr(right)?;
                            self.chunk.patch_jump(jump);
                        }
                        _ => {
                            self.compile_expr(left)?;
                            self.compile_expr(right)?;
                            match op {
                                BinOp::Add => self.chunk.emit_op_span(Op::Add, line, col),
                                BinOp::Sub => self.chunk.emit_op_span(Op::Sub, line, col),
                                BinOp::Mul => self.chunk.emit_op_span(Op::Mul, line, col),
                                BinOp::Div => self.chunk.emit_op_span(Op::Div, line, col),
                                BinOp::Mod => self.chunk.emit_op_span(Op::Mod, line, col),
                                BinOp::Eq => self.chunk.emit_op(Op::Eq, line),
                                BinOp::Ne => self.chunk.emit_op(Op::NotEq, line),
                                BinOp::Lt => self.chunk.emit_op(Op::Lt, line),
                                BinOp::Gt => self.chunk.emit_op(Op::Gt, line),
                                BinOp::Le => self.chunk.emit_op(Op::LtEq, line),
                                BinOp::Ge => self.chunk.emit_op(Op::GtEq, line),
                                BinOp::BitAnd => self.chunk.emit_op(Op::BitAnd, line),
                                BinOp::BitOr => self.chunk.emit_op(Op::BitOr, line),
                                BinOp::BitXor => self.chunk.emit_op(Op::BitXor, line),
                                BinOp::Shl => self.chunk.emit_op(Op::Shl, line),
                                BinOp::Shr => self.chunk.emit_op(Op::Shr, line),
                                _ => unreachable!(),
                            }
                        }
                    }
                }
            }

            ExprKind::UnaryOp { op, expr: inner } => {
                #[cfg(feature = "optimize")]
                let folded = Self::try_fold_unary(op, inner);
                #[cfg(not(feature = "optimize"))]
                let folded: Option<Value> = None;
                if let Some(val) = folded {
                    self.chunk.emit_constant(val, line);
                } else {
                    self.compile_expr(inner)?;
                    match op {
                        UnaryOp::Neg => self.chunk.emit_op_span(Op::Neg, line, col),
                        UnaryOp::Not => self.chunk.emit_op_span(Op::Not, line, col),
                    }
                }
            }

            ExprKind::If { cond, then_body, else_body } => {
                // Condition is not in tail position (already cleared)
                self.compile_expr(cond)?;
                let then_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop condition
                self.begin_scope(line);
                // Both branches inherit tail position
                self.in_tail_position = was_tail;
                self.compile_block_expr(then_body, line)?;
                self.end_scope(line);
                let else_jump = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(then_jump);
                self.chunk.emit_op(Op::Pop, line); // pop condition
                if let Some(else_stmts) = else_body {
                    self.begin_scope(line);
                    self.in_tail_position = was_tail;
                    self.compile_block_expr(else_stmts, line)?;
                    self.end_scope(line);
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }
                self.chunk.patch_jump(else_jump);
            }

            ExprKind::Block(stmts) => {
                self.begin_scope(line);
                self.in_tail_position = was_tail;
                self.compile_block_expr(stmts, line)?;
                self.end_scope(line);
            }

            ExprKind::Call { func, args } => {
                // Sub-expressions are not in tail position (already cleared above)
                self.compile_expr(func)?;
                for arg in args {
                    self.compile_expr(&arg.value)?;
                }
                #[cfg(feature = "optimize")]
                let op = if was_tail { Op::TailCall } else { Op::Call };
                #[cfg(not(feature = "optimize"))]
                let op = Op::Call;
                self.chunk.emit_op_u8_span(op, args.len() as u8, line, col);
            }

            ExprKind::List(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.chunk.emit_op_u16(Op::BuildList, items.len() as u16, line);
            }

            ExprKind::Tuple(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.chunk.emit_op_u16(Op::BuildTuple, items.len() as u16, line);
            }

            ExprKind::Dict(entries) => {
                let has_spread = entries.iter().any(|e| matches!(e, DictEntry::Spread(_)));
                if has_spread {
                    // Build empty dict, then insert/merge entries one by one
                    self.chunk.emit_op_u16(Op::BuildDict, 0, line);
                    for entry in entries {
                        match entry {
                            DictEntry::KeyValue(k, v) => {
                                self.compile_expr(k)?;
                                self.compile_expr(v)?;
                                self.chunk.emit_op(Op::DictInsert, line);
                            }
                            DictEntry::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.chunk.emit_op(Op::DictMerge, line);
                            }
                        }
                    }
                } else {
                    // Fast path: no spreads, use BuildDict directly
                    let count = entries.len() as u16;
                    for entry in entries {
                        if let DictEntry::KeyValue(k, v) = entry {
                            self.compile_expr(k)?;
                            self.compile_expr(v)?;
                        }
                    }
                    self.chunk.emit_op_u16(Op::BuildDict, count, line);
                }
            }

            ExprKind::FieldAccess { expr: inner, field } => {
                self.compile_expr(inner)?;
                let idx = self.chunk.add_constant(Value::Str(field.clone()));
                self.chunk.emit_op_u16_span(Op::GetField, idx, line, col);
            }

            ExprKind::Index { expr: inner, index } => {
                self.compile_expr(inner)?;
                self.compile_expr(index)?;
                self.chunk.emit_op_span(Op::GetIndex, line, col);
            }

            ExprKind::Slice { expr: inner, start, end, inclusive } => {
                self.compile_expr(inner)?;
                let mut flags: u8 = 0;
                if let Some(s) = start {
                    self.compile_expr(s)?;
                    flags |= 1; // has_start
                }
                if let Some(e) = end {
                    self.compile_expr(e)?;
                    flags |= 2; // has_end
                }
                if *inclusive {
                    flags |= 4; // inclusive
                }
                self.chunk.emit_op_u8(Op::Slice, flags, line);
            }

            ExprKind::MethodCall { expr: inner, method, args } => {
                self.compile_expr(inner)?;
                for arg in args {
                    self.compile_expr(&arg.value)?;
                }
                let idx = self.chunk.add_constant(Value::Str(method.clone()));
                self.chunk.emit_op_u16_span(Op::MethodCall, idx, line, col);
                self.chunk.emit_span(args.len() as u8, line, col);
            }

            ExprKind::Lambda { params, body } => {
                // Build lambda body as a single expression statement for tree-walk fallback
                let body_stmt = Stmt {
                    kind: StmtKind::ExprStmt { expr: *body.clone(), has_semi: false },
                    span: expr.span,
                };
                // Precompile lambda body
                let mut fn_compiler = Compiler::new();
                fn_compiler.in_tail_position = true;
                fn_compiler.needs_env_locals = Self::expr_has_closures(body);
                // Pre-register parameters as locals
                for p in params {
                    fn_compiler.add_local(p.clone(), false);
                }
                fn_compiler.compile_expr(body)?;
                fn_compiler.chunk.emit_op(Op::Return, line);
                #[cfg(feature = "optimize")]
                fn_compiler.chunk.peephole_optimize();
                let compiled_chunk = fn_compiler.chunk;
                self.fn_chunks.extend(fn_compiler.fn_chunks);

                let fn_value = Value::Fn(crate::value::IonFn::new(
                    "<lambda>".to_string(),
                    params.iter().map(|n| crate::ast::Param {
                        name: n.clone(),
                        default: None,
                    }).collect(),
                    vec![body_stmt],
                    std::collections::HashMap::new(),
                ));
                // Associate precompiled chunk with fn_id
                if let Value::Fn(ref ion_fn) = fn_value {
                    self.fn_chunks.insert(ion_fn.fn_id, compiled_chunk);
                }
                let fn_idx = self.chunk.add_constant(fn_value);
                self.chunk.emit_op_u16(Op::Closure, fn_idx, line);
            }

            ExprKind::FStr(parts) => {
                for part in parts {
                    match part {
                        FStrPart::Literal(s) => {
                            self.chunk.emit_constant(Value::Str(s.clone()), line);
                        }
                        FStrPart::Expr(expr) => {
                            self.compile_expr(expr)?;
                        }
                    }
                }
                self.chunk.emit_op_u16(Op::BuildFString, parts.len() as u16, line);
            }

            ExprKind::PipeOp { left, right } => {
                // Desugar: left |> right(args)  →  right(left, args)
                // Compile func first, then piped value as first arg, then other args
                match &right.kind {
                    ExprKind::Call { func, args } => {
                        self.compile_expr(func)?;
                        self.compile_expr(left)?; // piped value = first arg
                        for arg in args {
                            self.compile_expr(&arg.value)?;
                        }
                        self.chunk.emit_op_u8(Op::Call, (args.len() + 1) as u8, line);
                    }
                    _ => {
                        // bare function: left |> func  →  func(left)
                        self.compile_expr(right)?;
                        self.compile_expr(left)?;
                        self.chunk.emit_op_u8(Op::Call, 1, line);
                    }
                }
            }

            ExprKind::Try(inner) => {
                self.compile_expr(inner)?;
                self.chunk.emit_op(Op::Try, line);
            }

            ExprKind::Range { start, end, inclusive } => {
                self.compile_expr(start)?;
                self.compile_expr(end)?;
                self.chunk.emit_op_u8(Op::BuildRange, if *inclusive { 1 } else { 0 }, line);
            }

            ExprKind::LoopExpr(body) => {
                self.compile_loop(body, line)?;
            }

            ExprKind::Match { expr: subject, arms } => {
                self.compile_match(subject, arms, line)?;
            }

            ExprKind::ListComp { expr: item_expr, pattern, iter, cond } => {
                self.compile_list_comp(item_expr, pattern, iter, cond.as_deref(), line)?;
            }

            ExprKind::DictComp { key, value, pattern, iter, cond } => {
                self.compile_dict_comp(key, value, pattern, iter, cond.as_deref(), line)?;
            }

            ExprKind::IfLet { pattern, expr: inner, then_body, else_body } => {
                // Evaluate the expression (not in tail position — already cleared)
                self.compile_expr(inner)?;

                // Test pattern
                self.chunk.emit_op(Op::Dup, line); // keep value for binding
                self.compile_pattern_test(pattern, line)?;

                let else_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true

                // Pattern matched — bind variables in new scope
                self.begin_scope(line);
                self.compile_pattern_bind(pattern, line)?;
                self.in_tail_position = was_tail;
                self.compile_block_expr(then_body, line)?;
                self.end_scope(line);

                let end_jump = self.chunk.emit_jump(Op::Jump, line);

                self.chunk.patch_jump(else_jump);
                self.chunk.emit_op(Op::Pop, line); // pop false
                self.chunk.emit_op(Op::Pop, line); // pop the duped value

                if let Some(else_stmts) = else_body {
                    self.begin_scope(line);
                    self.in_tail_position = was_tail;
                    self.compile_block_expr(else_stmts, line)?;
                    self.end_scope(line);
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }

                self.chunk.patch_jump(end_jump);
            }

            // Features that fall back to tree-walk for now
            ExprKind::StructConstruct { name, fields, spread } => {
                if let Some(spread_expr) = spread {
                    self.compile_expr(spread_expr)?;
                    for (fname, fexpr) in fields {
                        self.chunk.emit_constant(Value::Str(fname.clone()), line);
                        self.compile_expr(fexpr)?;
                    }
                    let type_idx = self.chunk.add_constant(Value::Str(name.clone()));
                    let field_count = (0x8000 | fields.len()) as u16;
                    self.chunk.emit_op(Op::ConstructStruct, line);
                    self.chunk.emit((type_idx >> 8) as u8, line);
                    self.chunk.emit((type_idx & 0xff) as u8, line);
                    self.chunk.emit((field_count >> 8) as u8, line);
                    self.chunk.emit((field_count & 0xff) as u8, line);
                } else {
                    for (fname, fexpr) in fields {
                        self.chunk.emit_constant(Value::Str(fname.clone()), line);
                        self.compile_expr(fexpr)?;
                    }
                    let type_idx = self.chunk.add_constant(Value::Str(name.clone()));
                    let count = fields.len() as u16;
                    self.chunk.emit_op(Op::ConstructStruct, line);
                    self.chunk.emit((type_idx >> 8) as u8, line);
                    self.chunk.emit((type_idx & 0xff) as u8, line);
                    self.chunk.emit((count >> 8) as u8, line);
                    self.chunk.emit((count & 0xff) as u8, line);
                }
            }
            ExprKind::EnumVariant { enum_name, variant } => {
                let enum_idx = self.chunk.add_constant(Value::Str(enum_name.clone()));
                let variant_idx = self.chunk.add_constant(Value::Str(variant.clone()));
                self.chunk.emit_op(Op::ConstructEnum, line);
                self.chunk.emit((enum_idx >> 8) as u8, line);
                self.chunk.emit((enum_idx & 0xff) as u8, line);
                self.chunk.emit((variant_idx >> 8) as u8, line);
                self.chunk.emit((variant_idx & 0xff) as u8, line);
                self.chunk.emit(0u8, line);
            }
            ExprKind::EnumVariantCall { enum_name, variant, args } => {
                for arg in args {
                    self.compile_expr(arg)?;
                }
                let enum_idx = self.chunk.add_constant(Value::Str(enum_name.clone()));
                let variant_idx = self.chunk.add_constant(Value::Str(variant.clone()));
                self.chunk.emit_op(Op::ConstructEnum, line);
                self.chunk.emit((enum_idx >> 8) as u8, line);
                self.chunk.emit((enum_idx & 0xff) as u8, line);
                self.chunk.emit((variant_idx >> 8) as u8, line);
                self.chunk.emit((variant_idx & 0xff) as u8, line);
                self.chunk.emit(args.len() as u8, line);
            }

            #[cfg(feature = "concurrency")]
            ExprKind::AsyncBlock(_) | ExprKind::SpawnExpr(_) |
            ExprKind::AwaitExpr(_) | ExprKind::SelectExpr(_) => {
                return Err(IonError::runtime(
                    "concurrency not supported in bytecode VM".to_string(),
                    line, 0,
                ));
            }
            #[cfg(not(feature = "concurrency"))]
            ExprKind::AsyncBlock(_) | ExprKind::SpawnExpr(_) |
            ExprKind::AwaitExpr(_) | ExprKind::SelectExpr(_) => {
                return Err(IonError::runtime(
                    "concurrency not available".to_string(),
                    line, 0,
                ));
            }
        }
        self.in_tail_position = was_tail;
        Ok(())
    }

    fn compile_block_expr(&mut self, stmts: &[Stmt], line: usize) -> Result<(), IonError> {
        if stmts.is_empty() {
            self.chunk.emit_op(Op::Unit, line);
            return Ok(());
        }
        let len = stmts.len();
        let saved_tail = self.in_tail_position;
        for (i, stmt) in stmts.iter().enumerate() {
            let is_last = i == len - 1;
            // Only the last expression (without semicolon) inherits tail position
            if !is_last {
                self.in_tail_position = false;
            } else {
                self.in_tail_position = saved_tail;
            }
            match &stmt.kind {
                StmtKind::ExprStmt { expr, has_semi } => {
                    if is_last && *has_semi {
                        self.in_tail_position = false;
                    }
                    self.compile_expr(expr)?;
                    if is_last && !has_semi {
                        // Keep value
                    } else {
                        self.chunk.emit_op(Op::Pop, stmt.span.line);
                    }
                }
                _ => {
                    self.in_tail_position = false;
                    self.compile_stmt(stmt)?;
                    if is_last {
                        self.chunk.emit_op(Op::Unit, stmt.span.line);
                    }
                }
            }
            // Dead code elimination: skip remaining statements after terminal
            #[cfg(feature = "optimize")]
            if !is_last && Self::stmt_is_terminal(stmt) {
                break;
            }
        }
        self.in_tail_position = saved_tail;
        Ok(())
    }

    fn compile_let_pattern(&mut self, pattern: &Pattern, mutable: bool, line: usize) -> Result<(), IonError> {
        match pattern {
            Pattern::Ident(name) => {
                self.emit_define_local(name, mutable, line);
            }
            Pattern::Tuple(pats) => {
                // Value is on stack. Destructure it.
                for (i, pat) in pats.iter().enumerate() {
                    self.chunk.emit_op(Op::Dup, line);
                    self.chunk.emit_constant(Value::Int(i as i64), line);
                    self.chunk.emit_op(Op::GetIndex, line);
                    self.compile_let_pattern(pat, mutable, line)?;
                }
                self.chunk.emit_op(Op::Pop, line); // pop the original tuple
            }
            Pattern::List(pats, rest) => {
                for (i, pat) in pats.iter().enumerate() {
                    self.chunk.emit_op(Op::Dup, line);
                    self.chunk.emit_constant(Value::Int(i as i64), line);
                    self.chunk.emit_op(Op::GetIndex, line);
                    self.compile_let_pattern(pat, mutable, line)?;
                }
                if let Some(rest_pat) = rest {
                    self.chunk.emit_op(Op::Dup, line);
                    self.chunk.emit_constant(Value::Int(pats.len() as i64), line);
                    self.chunk.emit_op_u8(Op::Slice, 1, line); // has_start only
                    self.compile_let_pattern(rest_pat, mutable, line)?;
                }
                self.chunk.emit_op(Op::Pop, line);
            }
            Pattern::Wildcard => {
                self.chunk.emit_op(Op::Pop, line);
            }
            _ => {
                return Err(IonError::runtime(
                    "complex pattern not yet supported in bytecode VM let".to_string(),
                    line, 0,
                ));
            }
        }
        Ok(())
    }

    fn compile_fn_decl(&mut self, name: &str, params: &[Param], body: &[Stmt], line: usize) -> Result<(), IonError> {
        // Compile function body into a separate chunk
        let mut fn_compiler = Compiler::new();
        fn_compiler.in_tail_position = true;
        // Only dual-define locals if body contains closures
        fn_compiler.needs_env_locals = Self::stmts_have_closures(body);
        // Pre-register parameters as locals (they'll be pushed by the VM)
        for param in params {
            fn_compiler.add_local(param.name.clone(), false);
        }
        fn_compiler.compile_block_expr(body, line)?;
        fn_compiler.chunk.emit_op(Op::Return, line);
        #[cfg(feature = "optimize")]
        fn_compiler.chunk.peephole_optimize();
        let compiled_chunk = fn_compiler.chunk;
        // Collect any nested function chunks
        self.fn_chunks.extend(fn_compiler.fn_chunks);

        let fn_value = Value::Fn(crate::value::IonFn::new(
            name.to_string(),
            params.to_vec(),
            body.to_vec(), // Keep AST body for tree-walk fallback
            std::collections::HashMap::new(),
        ));
        // Extract fn_id to associate with precompiled chunk
        if let Value::Fn(ref ion_fn) = fn_value {
            self.fn_chunks.insert(ion_fn.fn_id, compiled_chunk);
        }

        // Define the function in the current scope
        self.chunk.emit_constant(fn_value, line);
        self.emit_define_local(name, false, line);
        Ok(())
    }

    #[allow(dead_code)]
    fn compile_lambda(&mut self, params: &[String], body: &Expr, line: usize) -> Result<FnProto, IonError> {
        let mut fn_compiler = Compiler::new();
        fn_compiler.compile_expr(body)?;
        fn_compiler.chunk.emit_op(Op::Return, line);
        #[cfg(feature = "optimize")]
        fn_compiler.chunk.peephole_optimize();
        Ok(FnProto {
            name: "<lambda>".to_string(),
            arity: params.len(),
            chunk: fn_compiler.chunk,
            param_names: params.to_vec(),
            has_defaults: vec![false; params.len()],
        })
    }

    fn compile_for(&mut self, pattern: &Pattern, iter: &Expr, body: &[Stmt], line: usize) -> Result<(), IonError> {
        // Save outer loop context
        let saved_breaks = std::mem::take(&mut self.break_jumps);
        let saved_continue = self.continue_target.take();
        let saved_in_for = self.in_for_loop;
        let saved_loop_depth = self.loop_scope_depth;
        self.in_for_loop = true;
        self.loop_scope_depth = self.scope_depth;

        // Evaluate the iterator expression
        self.compile_expr(iter)?;

        // Convert to iterable (the VM will handle this)
        self.chunk.emit_op(Op::IterInit, line);

        let loop_start = self.chunk.len();
        self.continue_target = Some(loop_start);

        // Get next item or jump to end
        let exit_jump = self.chunk.emit_jump(Op::IterNext, line);

        // Bind pattern
        self.begin_scope(line);
        self.compile_let_pattern(pattern, false, line)?;

        // Execute body (with dead code elimination)
        for stmt in body {
            self.compile_stmt(stmt)?;
            #[cfg(feature = "optimize")]
            if Self::stmt_is_terminal(stmt) { break; }
        }
        self.end_scope(line);

        // Push placeholder for IterNext to pop on next iteration
        self.chunk.emit_op(Op::Unit, line);

        // Loop back
        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        // Pop the iterator placeholder
        self.chunk.emit_op(Op::Pop, line);

        // Patch all break jumps to after the loop
        for jump in &self.break_jumps {
            self.chunk.patch_jump(*jump);
        }

        // Restore outer loop context
        self.break_jumps = saved_breaks;
        self.continue_target = saved_continue;
        self.in_for_loop = saved_in_for;
        self.loop_scope_depth = saved_loop_depth;
        Ok(())
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt], line: usize) -> Result<(), IonError> {
        let saved_breaks = std::mem::take(&mut self.break_jumps);
        let saved_continue = self.continue_target.take();
        let saved_in_for = self.in_for_loop;
        let saved_loop_depth = self.loop_scope_depth;
        self.in_for_loop = false;
        self.loop_scope_depth = self.scope_depth;

        let loop_start = self.chunk.len();
        self.continue_target = Some(loop_start);

        self.compile_expr(cond)?;
        let exit_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
        self.chunk.emit_op(Op::Pop, line); // pop condition

        self.begin_scope(line);
        for stmt in body {
            self.compile_stmt(stmt)?;
            #[cfg(feature = "optimize")]
            if Self::stmt_is_terminal(stmt) { break; }
        }
        self.end_scope(line);

        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        self.chunk.emit_op(Op::Pop, line); // pop condition

        for jump in &self.break_jumps {
            self.chunk.patch_jump(*jump);
        }
        self.break_jumps = saved_breaks;
        self.continue_target = saved_continue;
        self.in_for_loop = saved_in_for;
        self.loop_scope_depth = saved_loop_depth;
        Ok(())
    }

    fn compile_loop(&mut self, body: &[Stmt], line: usize) -> Result<(), IonError> {
        let saved_breaks = std::mem::take(&mut self.break_jumps);
        let saved_continue = self.continue_target.take();
        let saved_in_for = self.in_for_loop;
        let saved_loop_depth = self.loop_scope_depth;
        self.in_for_loop = false;
        self.loop_scope_depth = self.scope_depth;

        let loop_start = self.chunk.len();
        self.continue_target = Some(loop_start);

        self.begin_scope(line);
        for stmt in body {
            self.compile_stmt(stmt)?;
            #[cfg(feature = "optimize")]
            if Self::stmt_is_terminal(stmt) { break; }
        }
        self.end_scope(line);

        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        for jump in &self.break_jumps {
            self.chunk.patch_jump(*jump);
        }
        self.break_jumps = saved_breaks;
        self.continue_target = saved_continue;
        self.in_for_loop = saved_in_for;
        self.loop_scope_depth = saved_loop_depth;
        Ok(())
    }

    fn compile_assign(&mut self, target: &AssignTarget, op: &AssignOp, value: &Expr, line: usize) -> Result<(), IonError> {
        match target {
            AssignTarget::Ident(name) => {
                match op {
                    AssignOp::Eq => {
                        self.compile_expr(value)?;
                    }
                    AssignOp::PlusEq | AssignOp::MinusEq |
                    AssignOp::StarEq | AssignOp::SlashEq => {
                        self.emit_get_var(name, line);
                        self.compile_expr(value)?;
                        match op {
                            AssignOp::PlusEq => self.chunk.emit_op(Op::Add, line),
                            AssignOp::MinusEq => self.chunk.emit_op(Op::Sub, line),
                            AssignOp::StarEq => self.chunk.emit_op(Op::Mul, line),
                            AssignOp::SlashEq => self.chunk.emit_op(Op::Div, line),
                            _ => unreachable!(),
                        }
                    }
                }
                self.emit_set_var(name, line);
            }
            AssignTarget::Index(obj_expr, index_expr) => {
                // For index assignment, we need to:
                // 1. Get the container, 2. Modify it, 3. Write it back
                // This only works when obj_expr is an Ident (variable)
                let var_name = match &obj_expr.kind {
                    ExprKind::Ident(name) => name.clone(),
                    _ => return Err(IonError::runtime(
                        "index assignment only supported on variables".to_string(), line, 0,
                    )),
                };

                // Get the container
                self.compile_expr(obj_expr)?;
                self.compile_expr(index_expr)?;

                // Compute new value
                match op {
                    AssignOp::Eq => {
                        self.compile_expr(value)?;
                    }
                    _ => {
                        // Get old value for compound assignment
                        self.compile_expr(obj_expr)?;
                        self.compile_expr(index_expr)?;
                        self.chunk.emit_op(Op::GetIndex, line);
                        self.compile_expr(value)?;
                        match op {
                            AssignOp::PlusEq => self.chunk.emit_op(Op::Add, line),
                            AssignOp::MinusEq => self.chunk.emit_op(Op::Sub, line),
                            AssignOp::StarEq => self.chunk.emit_op(Op::Mul, line),
                            AssignOp::SlashEq => self.chunk.emit_op(Op::Div, line),
                            _ => unreachable!(),
                        }
                    }
                }

                // Stack: [..., obj, index, new_value]
                self.chunk.emit_op(Op::SetIndex, line);
                // SetIndex returns the modified container — write it back
                self.emit_set_var(&var_name, line);
            }
            AssignTarget::Field(obj_expr, field) => {
                let var_name = match &obj_expr.kind {
                    ExprKind::Ident(name) => name.clone(),
                    _ => return Err(IonError::runtime(
                        "field assignment only supported on variables".to_string(), line, 0,
                    )),
                };

                self.compile_expr(obj_expr)?;

                match op {
                    AssignOp::Eq => {
                        self.compile_expr(value)?;
                    }
                    _ => {
                        self.chunk.emit_op(Op::Dup, line);
                        let get_idx = self.chunk.add_constant(Value::Str(field.clone()));
                        self.chunk.emit_op_u16(Op::GetField, get_idx, line);
                        self.compile_expr(value)?;
                        match op {
                            AssignOp::PlusEq => self.chunk.emit_op(Op::Add, line),
                            AssignOp::MinusEq => self.chunk.emit_op(Op::Sub, line),
                            AssignOp::StarEq => self.chunk.emit_op(Op::Mul, line),
                            AssignOp::SlashEq => self.chunk.emit_op(Op::Div, line),
                            _ => unreachable!(),
                        }
                    }
                }

                // Stack: [..., obj, new_value]
                let field_idx = self.chunk.add_constant(Value::Str(field.clone()));
                self.chunk.emit_op_u16(Op::SetField, field_idx, line);
                // SetField returns the modified container — write it back
                self.emit_set_var(&var_name, line);
            }
        }
        Ok(())
    }

    /// Compile a function body to a standalone chunk (for VM-native function execution).
    pub fn compile_fn_body(mut self, params: &[Param], body: &[Stmt], line: usize) -> Result<Chunk, IonError> {
        self.in_tail_position = true;
        self.needs_env_locals = Self::stmts_have_closures(body);
        // Pre-register parameters as locals
        for param in params {
            self.add_local(param.name.clone(), false);
        }
        self.compile_block_expr(body, line)?;
        self.chunk.emit_op(Op::Return, line);
        Ok(self.chunk)
    }

    fn compile_match(&mut self, subject: &Expr, arms: &[MatchArm], line: usize) -> Result<(), IonError> {
        let was_tail = self.in_tail_position;
        // Store subject in a hidden temp variable (not in tail position)
        self.begin_scope(line);
        self.in_tail_position = false;
        self.compile_expr(subject)?;
        let tmp_name = "__match_subject__";
        self.emit_define_local(tmp_name, false, line);
        let subject_slot = self.locals.len() - 1;

        let mut end_jumps = Vec::new();

        for arm in arms {
            // Load subject for pattern test
            self.chunk.emit_op_u16(Op::GetLocalSlot, subject_slot as u16, line);

            // Emit pattern test — consumes subject copy, pushes bool
            self.compile_pattern_test(&arm.pattern, line)?;

            // If guard exists, test it too (only if pattern matched)
            if let Some(guard) = &arm.guard {
                let skip_guard = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true
                self.compile_expr(guard)?;
                let after_guard = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(skip_guard);
                // false stays on stack — jump lands here
                self.chunk.patch_jump(after_guard);
            }

            let next_arm = self.chunk.emit_jump(Op::JumpIfFalse, line);
            self.chunk.emit_op(Op::Pop, line); // pop true

            // Bind pattern variables in new scope
            self.begin_scope(line);
            self.chunk.emit_op_u16(Op::GetLocalSlot, subject_slot as u16, line);
            self.compile_pattern_bind(&arm.pattern, line)?;

            // Compile arm body — inherits tail position
            self.in_tail_position = was_tail;
            self.compile_expr(&arm.body)?;
            self.end_scope(line);

            end_jumps.push(self.chunk.emit_jump(Op::Jump, line));

            self.chunk.patch_jump(next_arm);
            self.chunk.emit_op(Op::Pop, line); // pop false
        }

        // No arm matched — push Unit
        self.chunk.emit_op(Op::Unit, line);

        for j in end_jumps {
            self.chunk.patch_jump(j);
        }

        self.end_scope(line); // pop the match subject scope
        Ok(())
    }

    /// Compile a pattern test: consumes the value on stack, pushes bool.
    fn compile_pattern_test(&mut self, pattern: &Pattern, line: usize) -> Result<(), IonError> {
        match pattern {
            Pattern::Wildcard | Pattern::Ident(_) => {
                self.chunk.emit_op(Op::Pop, line); // consume value
                self.chunk.emit_op(Op::True, line); // always matches
            }
            Pattern::Int(n) => {
                self.chunk.emit_constant(Value::Int(*n), line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::Float(n) => {
                self.chunk.emit_constant(Value::Float(*n), line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::Bool(b) => {
                self.chunk.emit_op(if *b { Op::True } else { Op::False }, line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::Str(s) => {
                self.chunk.emit_constant(Value::Str(s.clone()), line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::Bytes(b) => {
                self.chunk.emit_constant(Value::Bytes(b.clone()), line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::None => {
                // Check if value is Option(None)
                self.chunk.emit_op(Op::None, line);
                self.chunk.emit_op(Op::Eq, line);
            }
            Pattern::Some(inner) => {
                // Test: is it Some(x)? Use MatchArm opcode for complex patterns
                // For now, test structurally: use a simpler encoding
                // We'll use the MatchBegin/MatchArm opcodes repurposed:
                // Actually, let's just emit inline checks.
                // Stack has value. We need to check if it's Some(_) and test inner.
                self.chunk.emit_op_u8(Op::MatchBegin, 1, line); // 1 = test Some
                let fail_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true
                // Now unwrap the Some and test inner pattern
                self.chunk.emit_op_u8(Op::MatchArm, 1, line); // 1 = unwrap Some
                self.compile_pattern_test(inner, line)?;
                let end = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(fail_jump);
                // false stays
                self.chunk.patch_jump(end);
            }
            Pattern::Ok(inner) => {
                self.chunk.emit_op_u8(Op::MatchBegin, 2, line); // 2 = test Ok
                let fail_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line);
                self.chunk.emit_op_u8(Op::MatchArm, 2, line); // 2 = unwrap Ok
                self.compile_pattern_test(inner, line)?;
                let end = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(fail_jump);
                self.chunk.patch_jump(end);
            }
            Pattern::Err(inner) => {
                self.chunk.emit_op_u8(Op::MatchBegin, 3, line); // 3 = test Err
                let fail_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line);
                self.chunk.emit_op_u8(Op::MatchArm, 3, line); // 3 = unwrap Err
                self.compile_pattern_test(inner, line)?;
                let end = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(fail_jump);
                self.chunk.patch_jump(end);
            }
            Pattern::Tuple(pats) => {
                // Check: is it a tuple of the right length, and do all sub-patterns match?
                self.chunk.emit_op_u8(Op::MatchBegin, 4, line); // 4 = test Tuple
                self.chunk.emit(pats.len() as u8, line); // expected length
                let fail_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true
                // Test each element
                for (i, pat) in pats.iter().enumerate() {
                    // Load the subject again and index into it
                    self.chunk.emit_op_u8(Op::MatchArm, 4, line); // 4 = get tuple element
                    self.chunk.emit(i as u8, line);
                    self.compile_pattern_test(pat, line)?;
                    let sub_fail = self.chunk.emit_jump(Op::JumpIfFalse, line);
                    self.chunk.emit_op(Op::Pop, line); // pop true, continue
                    if i == pats.len() - 1 {
                        // All matched
                        self.chunk.emit_op(Op::True, line);
                    }
                    // Patch sub_fail to push false and skip remaining
                    let sub_end = self.chunk.emit_jump(Op::Jump, line);
                    self.chunk.patch_jump(sub_fail);
                    // false stays on stack
                    self.chunk.patch_jump(sub_end);
                }
                if pats.is_empty() {
                    self.chunk.emit_op(Op::True, line);
                }
                let end = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(fail_jump);
                // false stays
                self.chunk.patch_jump(end);
            }
            Pattern::List(pats, rest) => {
                // Check: is it a list with at least pats.len() elements (or exact if no rest)?
                let has_rest = rest.is_some();
                self.chunk.emit_op_u8(Op::MatchBegin, 5, line); // 5 = test List
                self.chunk.emit(pats.len() as u8, line); // min/exact length
                self.chunk.emit(if has_rest { 1 } else { 0 }, line); // has_rest flag
                let fail_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop true
                // Test each element pattern
                for (i, pat) in pats.iter().enumerate() {
                    self.chunk.emit_op_u8(Op::MatchArm, 5, line); // 5 = get list element
                    self.chunk.emit(i as u8, line);
                    self.compile_pattern_test(pat, line)?;
                    let sub_fail = self.chunk.emit_jump(Op::JumpIfFalse, line);
                    self.chunk.emit_op(Op::Pop, line); // pop true
                    if i == pats.len() - 1 {
                        self.chunk.emit_op(Op::True, line);
                    }
                    let sub_end = self.chunk.emit_jump(Op::Jump, line);
                    self.chunk.patch_jump(sub_fail);
                    self.chunk.patch_jump(sub_end);
                }
                if pats.is_empty() {
                    self.chunk.emit_op(Op::True, line);
                }
                let end = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(fail_jump);
                self.chunk.patch_jump(end);
            }
            _ => {
                // For complex patterns (EnumVariant, Struct), fall back
                return Err(IonError::runtime(
                    "complex pattern not yet supported in bytecode VM match".to_string(),
                    line, 0,
                ));
            }
        }
        Ok(())
    }

    /// Bind pattern variables: consumes value on stack.
    fn compile_pattern_bind(&mut self, pattern: &Pattern, line: usize) -> Result<(), IonError> {
        match pattern {
            Pattern::Wildcard => {
                self.chunk.emit_op(Op::Pop, line);
            }
            Pattern::Ident(name) => {
                self.emit_define_local(name, false, line);
            }
            Pattern::Int(_) | Pattern::Float(_) | Pattern::Bool(_) |
            Pattern::Str(_) | Pattern::Bytes(_) | Pattern::None => {
                self.chunk.emit_op(Op::Pop, line); // no bindings for literals
            }
            Pattern::Some(inner) => {
                // Unwrap the Some value
                self.chunk.emit_op_u8(Op::MatchArm, 1, line); // unwrap Some
                self.compile_pattern_bind(inner, line)?;
            }
            Pattern::Ok(inner) => {
                self.chunk.emit_op_u8(Op::MatchArm, 2, line); // unwrap Ok
                self.compile_pattern_bind(inner, line)?;
            }
            Pattern::Err(inner) => {
                self.chunk.emit_op_u8(Op::MatchArm, 3, line); // unwrap Err
                self.compile_pattern_bind(inner, line)?;
            }
            Pattern::Tuple(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    self.chunk.emit_op(Op::Dup, line); // dup tuple
                    self.chunk.emit_constant(Value::Int(i as i64), line);
                    self.chunk.emit_op(Op::GetIndex, line);
                    self.compile_pattern_bind(pat, line)?;
                }
                self.chunk.emit_op(Op::Pop, line); // pop tuple
            }
            Pattern::List(pats, rest) => {
                // Bind each element
                for (i, pat) in pats.iter().enumerate() {
                    self.chunk.emit_op(Op::Dup, line); // dup list
                    self.chunk.emit_constant(Value::Int(i as i64), line);
                    self.chunk.emit_op(Op::GetIndex, line);
                    self.compile_pattern_bind(pat, line)?;
                }
                // If there's a rest pattern, bind the remaining elements
                if let Some(rest_pat) = rest {
                    self.chunk.emit_op(Op::Dup, line); // dup list
                    // Slice from pats.len() to end
                    self.chunk.emit_constant(Value::Int(pats.len() as i64), line);
                    // Use Slice with has_start only
                    self.chunk.emit_op_u8(Op::Slice, 1, line); // flags: has_start=1
                    self.compile_pattern_bind(rest_pat, line)?;
                }
                self.chunk.emit_op(Op::Pop, line); // pop list
            }
            _ => {
                return Err(IonError::runtime(
                    "complex pattern binding not yet supported in bytecode VM".to_string(),
                    line, 0,
                ));
            }
        }
        Ok(())
    }

    fn compile_list_comp(&mut self, item_expr: &Expr, pattern: &Pattern, iter: &Expr, cond: Option<&Expr>, line: usize) -> Result<(), IonError> {
        // Build an empty list, then iterate and append
        self.chunk.emit_op_u16(Op::BuildList, 0, line); // empty list on stack

        // Evaluate iterator
        self.compile_expr(iter)?;
        self.chunk.emit_op(Op::IterInit, line);

        let loop_start = self.chunk.len();
        let exit_jump = self.chunk.emit_jump(Op::IterNext, line);

        // Bind pattern in scope
        self.begin_scope(line);
        self.compile_let_pattern(pattern, false, line)?;

        // If there's a condition, check it
        if let Some(cond_expr) = cond {
            self.compile_expr(cond_expr)?;
            let skip_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
            self.chunk.emit_op(Op::Pop, line); // pop true

            // Compile item expression and append
            self.compile_expr(item_expr)?;
            self.chunk.emit_op(Op::ListAppend, line);

            let after = self.chunk.emit_jump(Op::Jump, line);
            self.chunk.patch_jump(skip_jump);
            self.chunk.emit_op(Op::Pop, line); // pop false
            self.chunk.patch_jump(after);
        } else {
            // Compile item expression and append
            self.compile_expr(item_expr)?;
            self.chunk.emit_op(Op::ListAppend, line);
        }

        self.end_scope(line);

        // Push placeholder for IterNext to pop on next iteration
        self.chunk.emit_op(Op::Unit, line);

        // Loop back
        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        self.chunk.emit_op(Op::Pop, line); // pop exhausted iterator placeholder
        // List is still on stack
        Ok(())
    }

    fn compile_dict_comp(&mut self, key_expr: &Expr, value_expr: &Expr, pattern: &Pattern, iter: &Expr, cond: Option<&Expr>, line: usize) -> Result<(), IonError> {
        // Build an empty dict, then iterate and insert
        self.chunk.emit_op_u16(Op::BuildDict, 0, line);

        self.compile_expr(iter)?;
        self.chunk.emit_op(Op::IterInit, line);

        let loop_start = self.chunk.len();
        let exit_jump = self.chunk.emit_jump(Op::IterNext, line);

        self.begin_scope(line);
        self.compile_let_pattern(pattern, false, line)?;

        if let Some(cond_expr) = cond {
            self.compile_expr(cond_expr)?;
            let skip_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
            self.chunk.emit_op(Op::Pop, line);

            self.compile_expr(key_expr)?;
            self.compile_expr(value_expr)?;
            self.chunk.emit_op(Op::DictInsert, line);

            let after = self.chunk.emit_jump(Op::Jump, line);
            self.chunk.patch_jump(skip_jump);
            self.chunk.emit_op(Op::Pop, line);
            self.chunk.patch_jump(after);
        } else {
            self.compile_expr(key_expr)?;
            self.compile_expr(value_expr)?;
            self.chunk.emit_op(Op::DictInsert, line);
        }

        self.end_scope(line);

        // Push placeholder for IterNext to pop on next iteration
        self.chunk.emit_op(Op::Unit, line);

        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        self.chunk.emit_op(Op::Pop, line);
        Ok(())
    }
}
