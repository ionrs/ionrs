//! AST → Bytecode compiler for the Ion VM.

use crate::ast::*;
use crate::bytecode::{Chunk, FnProto, Op};
use crate::error::IonError;
use crate::value::Value;

pub struct Compiler {
    chunk: Chunk,
}

impl Compiler {
    pub fn new() -> Self {
        Self { chunk: Chunk::new() }
    }

    pub fn compile_program(mut self, program: &Program) -> Result<Chunk, IonError> {
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
        }
        if program.stmts.is_empty() {
            self.chunk.emit_op(Op::Unit, 0);
        }
        self.chunk.emit_op(Op::Return, 0);
        Ok(self.chunk)
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
                // Break is handled via jump patching — emit placeholder
                // The loop compilation will patch this
                self.chunk.emit_jump(Op::Jump, line);
            }
            StmtKind::Continue => {
                self.chunk.emit_jump(Op::Jump, line);
            }
            StmtKind::Return { value } => {
                if let Some(expr) = value {
                    self.compile_expr(expr)?;
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }
                self.chunk.emit_op(Op::Return, line);
            }
            StmtKind::Assign { target, op, value } => {
                self.compile_assign(target, op, value, line)?;
            }
            StmtKind::WhileLet { .. } => {
                return Err(IonError::runtime(
                    "while-let not yet supported in bytecode VM".to_string(), line, 0,
                ));
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), IonError> {
        let line = expr.span.line;
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
                let idx = self.chunk.add_constant(Value::Str(name.clone()));
                self.chunk.emit_op_u16(Op::GetLocal, idx, line);
            }

            ExprKind::BinOp { left, op, right } => {
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
                            BinOp::Add => self.chunk.emit_op(Op::Add, line),
                            BinOp::Sub => self.chunk.emit_op(Op::Sub, line),
                            BinOp::Mul => self.chunk.emit_op(Op::Mul, line),
                            BinOp::Div => self.chunk.emit_op(Op::Div, line),
                            BinOp::Mod => self.chunk.emit_op(Op::Mod, line),
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

            ExprKind::UnaryOp { op, expr: inner } => {
                self.compile_expr(inner)?;
                match op {
                    UnaryOp::Neg => self.chunk.emit_op(Op::Neg, line),
                    UnaryOp::Not => self.chunk.emit_op(Op::Not, line),
                }
            }

            ExprKind::If { cond, then_body, else_body } => {
                self.compile_expr(cond)?;
                let then_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
                self.chunk.emit_op(Op::Pop, line); // pop condition
                self.chunk.emit_op(Op::PushScope, line);
                self.compile_block_expr(then_body, line)?;
                self.chunk.emit_op(Op::PopScope, line);
                let else_jump = self.chunk.emit_jump(Op::Jump, line);
                self.chunk.patch_jump(then_jump);
                self.chunk.emit_op(Op::Pop, line); // pop condition
                if let Some(else_stmts) = else_body {
                    self.chunk.emit_op(Op::PushScope, line);
                    self.compile_block_expr(else_stmts, line)?;
                    self.chunk.emit_op(Op::PopScope, line);
                } else {
                    self.chunk.emit_op(Op::Unit, line);
                }
                self.chunk.patch_jump(else_jump);
            }

            ExprKind::Block(stmts) => {
                self.chunk.emit_op(Op::PushScope, line);
                self.compile_block_expr(stmts, line)?;
                self.chunk.emit_op(Op::PopScope, line);
            }

            ExprKind::Call { func, args } => {
                self.compile_expr(func)?;
                for arg in args {
                    self.compile_expr(&arg.value)?;
                }
                self.chunk.emit_op_u8(Op::Call, args.len() as u8, line);
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
                let mut count = 0u16;
                for entry in entries {
                    match entry {
                        DictEntry::KeyValue(k, v) => {
                            self.compile_expr(k)?;
                            self.compile_expr(v)?;
                            count += 1;
                        }
                        DictEntry::Spread(expr) => {
                            // For spread, we push a sentinel + the dict
                            // The VM will handle merging
                            self.compile_expr(expr)?;
                            // TODO: handle spread in VM
                            count += 1;
                        }
                    }
                }
                self.chunk.emit_op_u16(Op::BuildDict, count, line);
            }

            ExprKind::FieldAccess { expr: inner, field } => {
                self.compile_expr(inner)?;
                let idx = self.chunk.add_constant(Value::Str(field.clone()));
                self.chunk.emit_op_u16(Op::GetField, idx, line);
            }

            ExprKind::Index { expr: inner, index } => {
                self.compile_expr(inner)?;
                self.compile_expr(index)?;
                self.chunk.emit_op(Op::GetIndex, line);
            }

            ExprKind::MethodCall { expr: inner, method, args } => {
                self.compile_expr(inner)?;
                for arg in args {
                    self.compile_expr(&arg.value)?;
                }
                let idx = self.chunk.add_constant(Value::Str(method.clone()));
                self.chunk.emit_op_u16(Op::MethodCall, idx, line);
                self.chunk.emit(args.len() as u8, line);
            }

            ExprKind::Lambda { params, body } => {
                let fn_proto = self.compile_lambda(params, body, line)?;
                let idx = self.chunk.add_constant(Value::Str(fn_proto.name.clone()));
                // Store function prototype as a constant
                let proto_idx = self.chunk.add_constant(Value::Fn(crate::value::IonFn {
                    name: fn_proto.name,
                    params: fn_proto.param_names.iter().map(|n| crate::ast::Param {
                        name: n.clone(),
                        default: None,
                    }).collect(),
                    body: vec![],  // VM uses bytecode, not AST body
                    captures: std::collections::HashMap::new(),
                }));
                self.chunk.emit_op_u16(Op::Constant, proto_idx, line);
                let _ = idx; // name index unused for now
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

            // Features that fall back to tree-walk for now
            ExprKind::IfLet { .. } |
            ExprKind::ListComp { .. } |
            ExprKind::DictComp { .. } |
            ExprKind::StructConstruct { .. } |
            ExprKind::EnumVariant { .. } |
            ExprKind::EnumVariantCall { .. } => {
                return Err(IonError::runtime(
                    format!("expression not yet supported in bytecode VM"),
                    line, 0,
                ));
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
        Ok(())
    }

    fn compile_block_expr(&mut self, stmts: &[Stmt], line: usize) -> Result<(), IonError> {
        if stmts.is_empty() {
            self.chunk.emit_op(Op::Unit, line);
            return Ok(());
        }
        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            let is_last = i == len - 1;
            match &stmt.kind {
                StmtKind::ExprStmt { expr, has_semi } => {
                    self.compile_expr(expr)?;
                    if is_last && !has_semi {
                        // Keep value
                    } else {
                        self.chunk.emit_op(Op::Pop, stmt.span.line);
                    }
                }
                _ => {
                    self.compile_stmt(stmt)?;
                    if is_last {
                        self.chunk.emit_op(Op::Unit, stmt.span.line);
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_let_pattern(&mut self, pattern: &Pattern, mutable: bool, line: usize) -> Result<(), IonError> {
        match pattern {
            Pattern::Ident(name) => {
                let idx = self.chunk.add_constant(Value::Str(name.clone()));
                self.chunk.emit_op_u16(Op::DefineLocal, idx, line);
                self.chunk.emit(if mutable { 1 } else { 0 }, line);
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
        // Compile body as block expression
        fn_compiler.compile_block_expr(body, line)?;
        fn_compiler.chunk.emit_op(Op::Return, line);

        let fn_value = Value::Fn(crate::value::IonFn {
            name: name.to_string(),
            params: params.to_vec(),
            body: body.to_vec(), // Keep AST body for tree-walk fallback
            captures: std::collections::HashMap::new(),
        });

        // Define the function in the current scope
        self.chunk.emit_constant(fn_value, line);
        let name_idx = self.chunk.add_constant(Value::Str(name.to_string()));
        self.chunk.emit_op_u16(Op::DefineLocal, name_idx, line);
        self.chunk.emit(0, line); // immutable
        Ok(())
    }

    fn compile_lambda(&mut self, params: &[String], _body: &Expr, _line: usize) -> Result<FnProto, IonError> {
        Ok(FnProto {
            name: "<lambda>".to_string(),
            arity: params.len(),
            chunk: Chunk::new(),
            param_names: params.to_vec(),
            has_defaults: vec![false; params.len()],
        })
    }

    fn compile_for(&mut self, pattern: &Pattern, iter: &Expr, body: &[Stmt], line: usize) -> Result<(), IonError> {
        // Evaluate the iterator expression
        self.compile_expr(iter)?;

        // Convert to iterable (the VM will handle this)
        self.chunk.emit_op(Op::IterInit, line);

        let loop_start = self.chunk.len();
        // Get next item or jump to end
        let exit_jump = self.chunk.emit_jump(Op::IterNext, line);

        // Bind pattern
        self.chunk.emit_op(Op::PushScope, line);
        self.compile_let_pattern(pattern, false, line)?;

        // Execute body
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.chunk.emit_op(Op::PopScope, line);

        // Loop back
        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        // Pop the iterator
        self.chunk.emit_op(Op::Pop, line);
        Ok(())
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt], line: usize) -> Result<(), IonError> {
        let loop_start = self.chunk.len();

        self.compile_expr(cond)?;
        let exit_jump = self.chunk.emit_jump(Op::JumpIfFalse, line);
        self.chunk.emit_op(Op::Pop, line); // pop condition

        self.chunk.emit_op(Op::PushScope, line);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.chunk.emit_op(Op::PopScope, line);

        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);

        self.chunk.patch_jump(exit_jump);
        self.chunk.emit_op(Op::Pop, line); // pop condition
        Ok(())
    }

    fn compile_loop(&mut self, body: &[Stmt], line: usize) -> Result<(), IonError> {
        let loop_start = self.chunk.len();

        self.chunk.emit_op(Op::PushScope, line);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.chunk.emit_op(Op::PopScope, line);

        let offset = self.chunk.len() - loop_start + 3;
        self.chunk.emit_op_u16(Op::Loop, offset as u16, line);
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
                        let idx = self.chunk.add_constant(Value::Str(name.clone()));
                        self.chunk.emit_op_u16(Op::GetLocal, idx, line);
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
                let idx = self.chunk.add_constant(Value::Str(name.clone()));
                self.chunk.emit_op_u16(Op::SetLocal, idx, line);
            }
            _ => {
                return Err(IonError::runtime(
                    "complex assignment target not yet supported in bytecode VM".to_string(),
                    line, 0,
                ));
            }
        }
        Ok(())
    }

    fn compile_match(&mut self, _subject: &Expr, _arms: &[MatchArm], line: usize) -> Result<(), IonError> {
        // For now, fall back to unsupported
        Err(IonError::runtime(
            "match not yet supported in bytecode VM".to_string(),
            line, 0,
        ))
    }
}
