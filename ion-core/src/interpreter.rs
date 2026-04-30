use crate::ast::*;
use crate::env::Env;
use crate::error::{ErrorKind, IonError};
use crate::host_types::TypeRegistry;
use crate::stdlib::OutputHandler;
use crate::value::{IonFn, Value};
use indexmap::IndexMap;
use std::sync::Arc;

/// Control flow signals that escape normal evaluation.
enum Signal {
    Return(Value),
    Break { label: Option<String>, value: Value },
    Continue { label: Option<String> },
}

/// Returns true if a break/continue signal carrying `sig_label` should be
/// caught by a loop carrying `loop_label`. Unlabeled signals are caught by
/// the innermost loop regardless of that loop's own label (Rust semantics).
fn signal_targets_loop(sig_label: &Option<String>, loop_label: &Option<String>) -> bool {
    match sig_label {
        None => true,
        Some(want) => loop_label.as_deref() == Some(want.as_str()),
    }
}

fn unmatched_label_msg(keyword: &str, label: Option<String>) -> String {
    match label {
        Some(name) => format!("{keyword} with unknown label '{name}"),
        None => format!("{keyword} outside of loop"),
    }
}

type IonResult = Result<Value, IonError>;
type SignalResult = Result<Value, SignalOrError>;

enum SignalOrError {
    Signal(Signal),
    Error(IonError),
}

impl From<IonError> for SignalOrError {
    fn from(e: IonError) -> Self {
        SignalOrError::Error(e)
    }
}

impl From<Signal> for SignalOrError {
    fn from(s: Signal) -> Self {
        SignalOrError::Signal(s)
    }
}

#[derive(Clone)]
pub struct Limits {
    pub max_call_depth: usize,
    pub max_loop_iters: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_call_depth: 512,
            max_loop_iters: 1_000_000,
        }
    }
}

pub struct Interpreter {
    pub env: Env,
    pub limits: Limits,
    pub types: TypeRegistry,
    call_depth: usize,
    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
    nursery: Option<crate::async_rt::Nursery>,
    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
    pub(crate) cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Self::with_output(crate::stdlib::missing_output_handler())
    }

    pub fn with_output(output: Arc<dyn OutputHandler>) -> Self {
        let mut env = Env::new();
        register_builtins(&mut env, output);
        Self {
            env,
            limits: Limits::default(),
            types: TypeRegistry::new(),
            call_depth: 0,
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            nursery: None,
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            cancel_flag: None,
        }
    }

    pub fn eval_program(&mut self, program: &Program) -> IonResult {
        match self.eval_stmts(&program.stmts) {
            Ok(v) => Ok(v),
            Err(SignalOrError::Error(e)) if e.kind == ErrorKind::PropagatedErr => {
                Ok(Value::Result(Err(Box::new(Value::Str(e.message.clone())))))
            }
            Err(SignalOrError::Error(e)) if e.kind == ErrorKind::PropagatedNone => {
                Ok(Value::Option(None))
            }
            Err(SignalOrError::Error(e)) => Err(e),
            Err(SignalOrError::Signal(Signal::Return(v))) => Ok(v),
            Err(SignalOrError::Signal(Signal::Break { label, .. })) => {
                Err(IonError::runtime(unmatched_label_msg("break", label), 0, 0))
            }
            Err(SignalOrError::Signal(Signal::Continue { label })) => Err(IonError::runtime(
                unmatched_label_msg("continue", label),
                0,
                0,
            )),
        }
    }

    /// Create an interpreter with a pre-existing environment (for VM hybrid mode).
    pub fn with_env(env: Env) -> Self {
        Self {
            env,
            limits: Limits::default(),
            types: TypeRegistry::new(),
            call_depth: 0,
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            nursery: None,
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            cancel_flag: None,
        }
    }

    /// Take ownership of the environment (for VM hybrid mode).
    pub fn take_env(self) -> Env {
        self.env
    }

    /// Evaluate a block of statements, returning the last value (public for VM).
    pub fn eval_block(&mut self, stmts: &[Stmt]) -> IonResult {
        match self.eval_stmts(stmts) {
            Ok(v) => Ok(v),
            Err(SignalOrError::Error(e)) => Err(e),
            Err(SignalOrError::Signal(Signal::Return(v))) => Ok(v),
            Err(SignalOrError::Signal(Signal::Break { value, .. })) => Ok(value),
            Err(SignalOrError::Signal(Signal::Continue { .. })) => Ok(Value::Unit),
        }
    }

    /// Evaluate a single expression (public for VM).
    pub fn eval_single_expr(&mut self, expr: &Expr) -> IonResult {
        match self.eval_expr(expr) {
            Ok(v) => Ok(v),
            Err(SignalOrError::Error(e)) => Err(e),
            Err(SignalOrError::Signal(Signal::Return(v))) => Ok(v),
            Err(SignalOrError::Signal(Signal::Break { value, .. })) => Ok(value),
            Err(SignalOrError::Signal(Signal::Continue { .. })) => Ok(Value::Unit),
        }
    }

    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
    fn check_cancelled(&self, line: usize, col: usize) -> Result<(), SignalOrError> {
        if let Some(flag) = &self.cancel_flag {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(IonError::runtime(ion_str!("task cancelled"), line, col).into());
            }
        }
        Ok(())
    }

    fn eval_stmts(&mut self, stmts: &[Stmt]) -> SignalResult {
        let mut last = Value::Unit;
        for (i, stmt) in stmts.iter().enumerate() {
            let is_last = i == stmts.len() - 1;
            match &stmt.kind {
                StmtKind::ExprStmt { expr, has_semi } => {
                    let val = self.eval_expr(expr)?;
                    if is_last && !has_semi {
                        last = val;
                    } else {
                        last = Value::Unit;
                    }
                }
                _ => {
                    self.eval_stmt(stmt)?;
                    last = Value::Unit;
                }
            }
        }
        Ok(last)
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> SignalResult {
        match &stmt.kind {
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            } => {
                let val = self.eval_expr(value)?;
                if let Some(ann) = type_ann {
                    Self::check_type_ann(&val, ann, stmt.span)?;
                }
                self.bind_pattern(pattern, &val, *mutable, stmt.span)?;
                Ok(Value::Unit)
            }
            StmtKind::FnDecl { name, params, body } => {
                let captures = self.env.capture();
                let func = Value::Fn(IonFn::new(
                    name.clone(),
                    params.clone(),
                    body.clone(),
                    captures,
                ));
                self.env.define(name.clone(), func, false);
                Ok(Value::Unit)
            }
            StmtKind::ExprStmt { expr, .. } => {
                self.eval_expr(expr)?;
                Ok(Value::Unit)
            }
            StmtKind::For {
                label,
                pattern,
                iter,
                body,
            } => {
                let iter_val = self.eval_expr(iter)?;
                let items = self.value_to_iter(&iter_val, iter.span)?;
                for item in items {
                    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
                    self.check_cancelled(stmt.span.line, stmt.span.col)?;
                    self.env.push_scope();
                    self.bind_pattern(pattern, &item, false, iter.span)?;
                    match self.eval_stmts(body) {
                        Ok(_) => {}
                        Err(SignalOrError::Signal(Signal::Break {
                            label: sig_label,
                            value,
                        })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                break;
                            }
                            return Err(Signal::Break {
                                label: sig_label,
                                value,
                            }
                            .into());
                        }
                        Err(SignalOrError::Signal(Signal::Continue { label: sig_label })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                continue;
                            }
                            return Err(Signal::Continue { label: sig_label }.into());
                        }
                        Err(e) => {
                            self.env.pop_scope();
                            return Err(e);
                        }
                    }
                    self.env.pop_scope();
                }
                Ok(Value::Unit)
            }
            StmtKind::While { label, cond, body } => {
                let mut iters = 0usize;
                loop {
                    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
                    self.check_cancelled(stmt.span.line, stmt.span.col)?;
                    let c = self.eval_expr(cond)?;
                    if !c.is_truthy() {
                        break;
                    }
                    iters += 1;
                    if iters > self.limits.max_loop_iters {
                        return Err(IonError::runtime(
                            ion_str!("maximum loop iterations exceeded").to_string(),
                            stmt.span.line,
                            stmt.span.col,
                        )
                        .into());
                    }
                    self.env.push_scope();
                    match self.eval_stmts(body) {
                        Ok(_) => {}
                        Err(SignalOrError::Signal(Signal::Break {
                            label: sig_label,
                            value,
                        })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                break;
                            }
                            return Err(Signal::Break {
                                label: sig_label,
                                value,
                            }
                            .into());
                        }
                        Err(SignalOrError::Signal(Signal::Continue { label: sig_label })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                continue;
                            }
                            return Err(Signal::Continue { label: sig_label }.into());
                        }
                        Err(e) => {
                            self.env.pop_scope();
                            return Err(e);
                        }
                    }
                    self.env.pop_scope();
                }
                Ok(Value::Unit)
            }
            StmtKind::WhileLet {
                label,
                pattern,
                expr,
                body,
            } => {
                let mut iters = 0usize;
                loop {
                    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
                    self.check_cancelled(stmt.span.line, stmt.span.col)?;
                    let val = self.eval_expr(expr)?;
                    if !self.pattern_matches(pattern, &val) {
                        break;
                    }
                    iters += 1;
                    if iters > self.limits.max_loop_iters {
                        return Err(IonError::runtime(
                            ion_str!("maximum loop iterations exceeded").to_string(),
                            stmt.span.line,
                            stmt.span.col,
                        )
                        .into());
                    }
                    self.env.push_scope();
                    self.bind_pattern(pattern, &val, false, expr.span)?;
                    match self.eval_stmts(body) {
                        Ok(_) => {}
                        Err(SignalOrError::Signal(Signal::Break {
                            label: sig_label,
                            value,
                        })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                break;
                            }
                            return Err(Signal::Break {
                                label: sig_label,
                                value,
                            }
                            .into());
                        }
                        Err(SignalOrError::Signal(Signal::Continue { label: sig_label })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                continue;
                            }
                            return Err(Signal::Continue { label: sig_label }.into());
                        }
                        Err(e) => {
                            self.env.pop_scope();
                            return Err(e);
                        }
                    }
                    self.env.pop_scope();
                }
                Ok(Value::Unit)
            }
            StmtKind::Loop { label, body } => {
                let mut iters = 0usize;
                let result = loop {
                    iters += 1;
                    if iters > self.limits.max_loop_iters {
                        return Err(IonError::runtime(
                            ion_str!("maximum loop iterations exceeded").to_string(),
                            stmt.span.line,
                            stmt.span.col,
                        )
                        .into());
                    }
                    self.env.push_scope();
                    match self.eval_stmts(body) {
                        Ok(_) => {}
                        Err(SignalOrError::Signal(Signal::Break {
                            label: sig_label,
                            value,
                        })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                break value;
                            }
                            return Err(Signal::Break {
                                label: sig_label,
                                value,
                            }
                            .into());
                        }
                        Err(SignalOrError::Signal(Signal::Continue { label: sig_label })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, label) {
                                continue;
                            }
                            return Err(Signal::Continue { label: sig_label }.into());
                        }
                        Err(e) => {
                            self.env.pop_scope();
                            return Err(e);
                        }
                    }
                    self.env.pop_scope();
                };
                Ok(result)
            }
            StmtKind::Break { label, value } => {
                let v = match value {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Unit,
                };
                Err(Signal::Break {
                    label: label.clone(),
                    value: v,
                }
                .into())
            }
            StmtKind::Continue { label } => Err(Signal::Continue {
                label: label.clone(),
            }
            .into()),
            StmtKind::Return { value } => {
                let v = match value {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Unit,
                };
                Err(Signal::Return(v).into())
            }
            StmtKind::Assign { target, op, value } => {
                let rhs = self.eval_expr(value)?;
                match target {
                    AssignTarget::Ident(name) => {
                        let final_val = match op {
                            AssignOp::Eq => rhs,
                            _ => {
                                let lhs = self
                                    .env
                                    .get(name)
                                    .ok_or_else(|| {
                                        IonError::name(
                                            format!("{}{}", ion_str!("undefined variable: "), name),
                                            stmt.span.line,
                                            stmt.span.col,
                                        )
                                    })?
                                    .clone();
                                self.apply_compound_op(*op, &lhs, &rhs, stmt.span)?
                            }
                        };
                        self.env
                            .set(name, final_val)
                            .map_err(|msg| IonError::runtime(msg, stmt.span.line, stmt.span.col))?;
                    }
                    AssignTarget::Index(obj_expr, index_expr) => {
                        let var_name = match &obj_expr.kind {
                            ExprKind::Ident(name) => name.clone(),
                            _ => {
                                return Err(IonError::runtime(
                                    ion_str!("index assignment only supported on variables"),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                                .into())
                            }
                        };
                        let mut container = self
                            .env
                            .get(&var_name)
                            .ok_or_else(|| {
                                IonError::name(
                                    format!("{}{}", ion_str!("undefined variable: "), var_name),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                            })?
                            .clone();
                        let index = self.eval_expr(index_expr)?;
                        let final_val = match op {
                            AssignOp::Eq => rhs,
                            _ => {
                                let old = self.index_access(&container, &index, stmt.span)?;
                                // index_access returns Option-wrapped values; unwrap for compound assign
                                let old = match old {
                                    Value::Option(Some(v)) => *v,
                                    other => other,
                                };
                                self.apply_compound_op(*op, &old, &rhs, stmt.span)?
                            }
                        };
                        match (&mut container, &index) {
                            (Value::List(items), Value::Int(i)) => {
                                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                                if idx >= items.len() {
                                    return Err(IonError::runtime(
                                        format!(
                                            "{}{}{}",
                                            ion_str!("index "),
                                            i,
                                            ion_str!(" out of range")
                                        ),
                                        stmt.span.line,
                                        stmt.span.col,
                                    )
                                    .into());
                                }
                                items[idx] = final_val;
                            }
                            (Value::Dict(map), Value::Str(key)) => {
                                map.insert(key.clone(), final_val);
                            }
                            _ => {
                                return Err(IonError::type_err(
                                    format!(
                                        "{}{}",
                                        ion_str!("cannot set index on "),
                                        container.type_name()
                                    ),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                                .into())
                            }
                        }
                        self.env
                            .set(&var_name, container)
                            .map_err(|msg| IonError::runtime(msg, stmt.span.line, stmt.span.col))?;
                    }
                    AssignTarget::Field(obj_expr, field) => {
                        let var_name = match &obj_expr.kind {
                            ExprKind::Ident(name) => name.clone(),
                            _ => {
                                return Err(IonError::runtime(
                                    ion_str!("field assignment only supported on variables"),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                                .into())
                            }
                        };
                        let mut container = self
                            .env
                            .get(&var_name)
                            .ok_or_else(|| {
                                IonError::name(
                                    format!("{}{}", ion_str!("undefined variable: "), var_name),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                            })?
                            .clone();
                        let final_val = match op {
                            AssignOp::Eq => rhs,
                            _ => {
                                let old = self.field_access(&container, field, stmt.span)?;
                                self.apply_compound_op(*op, &old, &rhs, stmt.span)?
                            }
                        };
                        match &mut container {
                            Value::Dict(map) => {
                                map.insert(field.clone(), final_val);
                            }
                            Value::HostStruct { fields, .. } => {
                                if fields.contains_key(field.as_str()) {
                                    fields.insert(field.clone(), final_val);
                                } else {
                                    return Err(IonError::runtime(
                                        format!(
                                            "{}{}{}",
                                            ion_str!("field '"),
                                            field,
                                            ion_str!("' not found")
                                        ),
                                        stmt.span.line,
                                        stmt.span.col,
                                    )
                                    .into());
                                }
                            }
                            _ => {
                                return Err(IonError::type_err(
                                    format!(
                                        "{}{}",
                                        ion_str!("cannot set field on "),
                                        container.type_name()
                                    ),
                                    stmt.span.line,
                                    stmt.span.col,
                                )
                                .into())
                            }
                        }
                        self.env
                            .set(&var_name, container)
                            .map_err(|msg| IonError::runtime(msg, stmt.span.line, stmt.span.col))?;
                    }
                }
                Ok(Value::Unit)
            }
            StmtKind::Use { path, imports } => {
                // Resolve the module dict by walking the path segments
                let root = self.env.get(&path[0]).ok_or_else(|| {
                    SignalOrError::Error(IonError::name(
                        format!("{}{}", ion_str!("undefined module: "), &path[0]),
                        stmt.span.line,
                        stmt.span.col,
                    ))
                })?;
                let mut module_val = root.clone();
                for seg in &path[1..] {
                    match &module_val {
                        Value::Dict(map) => {
                            module_val = map.get(seg).cloned().ok_or_else(|| {
                                SignalOrError::Error(IonError::name(
                                    format!(
                                        "{}{}{}{}",
                                        ion_str!("'"),
                                        seg,
                                        ion_str!("' not found in module "),
                                        &path[0]
                                    ),
                                    stmt.span.line,
                                    stmt.span.col,
                                ))
                            })?;
                        }
                        _ => {
                            return Err(IonError::type_err(
                                format!(
                                    "{}{}{}",
                                    ion_str!("'"),
                                    seg,
                                    ion_str!("' is not a module")
                                ),
                                stmt.span.line,
                                stmt.span.col,
                            )
                            .into())
                        }
                    }
                }
                // Now import from module_val (which should be a dict)
                match imports {
                    UseImports::Glob => {
                        if let Value::Dict(map) = &module_val {
                            for (name, val) in map {
                                self.env.define(name.clone(), val.clone(), false);
                            }
                        } else {
                            return Err(IonError::type_err(
                                ion_str!("use target is not a module"),
                                stmt.span.line,
                                stmt.span.col,
                            )
                            .into());
                        }
                    }
                    UseImports::Names(names) => {
                        if let Value::Dict(map) = &module_val {
                            for name in names {
                                let val = map.get(name).ok_or_else(|| {
                                    SignalOrError::Error(IonError::name(
                                        format!(
                                            "{}{}{}",
                                            ion_str!("'"),
                                            name,
                                            ion_str!("' not found in module")
                                        ),
                                        stmt.span.line,
                                        stmt.span.col,
                                    ))
                                })?;
                                self.env.define(name.clone(), val.clone(), false);
                            }
                        } else {
                            return Err(IonError::type_err(
                                ion_str!("use target is not a module"),
                                stmt.span.line,
                                stmt.span.col,
                            )
                            .into());
                        }
                    }
                    UseImports::Single(name) => {
                        if let Value::Dict(map) = &module_val {
                            let val = map.get(name).ok_or_else(|| {
                                SignalOrError::Error(IonError::name(
                                    format!(
                                        "{}{}{}",
                                        ion_str!("'"),
                                        name,
                                        ion_str!("' not found in module")
                                    ),
                                    stmt.span.line,
                                    stmt.span.col,
                                ))
                            })?;
                            self.env.define(name.clone(), val.clone(), false);
                        } else {
                            return Err(IonError::type_err(
                                ion_str!("use target is not a module"),
                                stmt.span.line,
                                stmt.span.col,
                            )
                            .into());
                        }
                    }
                }
                Ok(Value::Unit)
            }
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> SignalResult {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(n) => Ok(Value::Float(*n)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Bytes(b) => Ok(Value::Bytes(b.clone())),
            ExprKind::None => Ok(Value::Option(None)),
            ExprKind::Unit => Ok(Value::Unit),

            ExprKind::FStr(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        FStrPart::Literal(s) => result.push_str(s),
                        FStrPart::Expr(e) => {
                            let val = self.eval_expr(e)?;
                            result.push_str(&val.to_string());
                        }
                    }
                }
                Ok(Value::Str(result))
            }

            ExprKind::Ident(name) => self.env.get(name).cloned().ok_or_else(|| {
                IonError::name(
                    format!("{}{}", ion_str!("undefined variable: "), name),
                    span.line,
                    span.col,
                )
                .into()
            }),

            ExprKind::ModulePath(segments) => {
                // Resolve a::b::c by walking dict fields from the root module
                let root = self.env.get(&segments[0]).ok_or_else(|| {
                    SignalOrError::Error(IonError::name(
                        format!("{}{}", ion_str!("undefined module: "), &segments[0]),
                        span.line,
                        span.col,
                    ))
                })?;
                let mut current = root.clone();
                for seg in &segments[1..] {
                    match &current {
                        Value::Dict(map) => {
                            current = map.get(seg).cloned().ok_or_else(|| {
                                SignalOrError::Error(IonError::name(
                                    format!(
                                        "{}{}{}{}",
                                        ion_str!("'"),
                                        seg,
                                        ion_str!("' not found in module "),
                                        &segments[0]
                                    ),
                                    span.line,
                                    span.col,
                                ))
                            })?;
                        }
                        _ => {
                            return Err(IonError::type_err(
                                format!(
                                    "{}{}{}",
                                    ion_str!("cannot access '"),
                                    seg,
                                    ion_str!("' on non-module value")
                                ),
                                span.line,
                                span.col,
                            )
                            .into())
                        }
                    }
                }
                Ok(current)
            }

            ExprKind::SomeExpr(e) => {
                let val = self.eval_expr(e)?;
                Ok(Value::Option(Some(Box::new(val))))
            }
            ExprKind::OkExpr(e) => {
                let val = self.eval_expr(e)?;
                Ok(Value::Result(Ok(Box::new(val))))
            }
            ExprKind::ErrExpr(e) => {
                let val = self.eval_expr(e)?;
                Ok(Value::Result(Err(Box::new(val))))
            }

            ExprKind::List(items) => {
                let mut vals = Vec::new();
                for entry in items {
                    match entry {
                        ListEntry::Elem(expr) => vals.push(self.eval_expr(expr)?),
                        ListEntry::Spread(expr) => match self.eval_expr(expr)? {
                            Value::List(sub) => vals.extend(sub),
                            other => {
                                return Err(IonError::type_err(
                                    format!(
                                        "{}{}",
                                        ion_str!("spread requires a list, got "),
                                        other.type_name()
                                    ),
                                    span.line,
                                    span.col,
                                )
                                .into())
                            }
                        },
                    }
                }
                Ok(Value::List(vals))
            }
            ExprKind::Dict(entries) => {
                let mut map = IndexMap::new();
                for entry in entries {
                    match entry {
                        DictEntry::KeyValue(k, v) => {
                            let key = self.eval_expr(k)?;
                            let key_str = match key {
                                Value::Str(s) => s,
                                _ => {
                                    return Err(IonError::type_err(
                                        ion_str!("dict keys must be strings").to_string(),
                                        span.line,
                                        span.col,
                                    )
                                    .into())
                                }
                            };
                            let val = self.eval_expr(v)?;
                            map.insert(key_str, val);
                        }
                        DictEntry::Spread(expr) => {
                            let val = self.eval_expr(expr)?;
                            match val {
                                Value::Dict(other) => {
                                    for (k, v) in other {
                                        map.insert(k, v);
                                    }
                                }
                                _ => {
                                    return Err(IonError::type_err(
                                        ion_str!("spread requires a dict").to_string(),
                                        span.line,
                                        span.col,
                                    )
                                    .into())
                                }
                            }
                        }
                    }
                }
                Ok(Value::Dict(map))
            }
            ExprKind::Tuple(items) => {
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval_expr(item)?);
                }
                Ok(Value::Tuple(vals))
            }

            ExprKind::ListComp {
                expr,
                pattern,
                iter,
                cond,
            } => {
                let iter_val = self.eval_expr(iter)?;
                let items = self.value_to_iter(&iter_val, span)?;
                let mut result = Vec::new();
                for item in items {
                    self.env.push_scope();
                    self.bind_pattern(pattern, &item, false, span)?;
                    let include = if let Some(c) = cond {
                        let v = self.eval_expr(c)?;
                        v.is_truthy()
                    } else {
                        true
                    };
                    if include {
                        result.push(self.eval_expr(expr)?);
                    }
                    self.env.pop_scope();
                }
                Ok(Value::List(result))
            }
            ExprKind::DictComp {
                key,
                value,
                pattern,
                iter,
                cond,
            } => {
                let iter_val = self.eval_expr(iter)?;
                let items = self.value_to_iter(&iter_val, span)?;
                let mut map = IndexMap::new();
                for item in items {
                    self.env.push_scope();
                    self.bind_pattern(pattern, &item, false, span)?;
                    let include = if let Some(c) = cond {
                        let v = self.eval_expr(c)?;
                        v.is_truthy()
                    } else {
                        true
                    };
                    if include {
                        let k = self.eval_expr(key)?;
                        let k_str = match k {
                            Value::Str(s) => s,
                            _ => {
                                return Err(IonError::type_err(
                                    ion_str!("dict comp keys must be strings").to_string(),
                                    span.line,
                                    span.col,
                                )
                                .into())
                            }
                        };
                        let v = self.eval_expr(value)?;
                        map.insert(k_str, v);
                    }
                    self.env.pop_scope();
                }
                Ok(Value::Dict(map))
            }

            ExprKind::BinOp { left, op, right } => {
                // Short-circuit for && and ||
                if matches!(op, BinOp::And) {
                    let l = self.eval_expr(left)?;
                    if !l.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                    let r = self.eval_expr(right)?;
                    return Ok(Value::Bool(r.is_truthy()));
                }
                if matches!(op, BinOp::Or) {
                    let l = self.eval_expr(left)?;
                    if l.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                    let r = self.eval_expr(right)?;
                    return Ok(Value::Bool(r.is_truthy()));
                }
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binop(*op, &l, &r, span)
            }

            ExprKind::UnaryOp { op, expr } => {
                let val = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match val {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(n) => Ok(Value::Float(-n)),
                        _ => Err(IonError::type_err(
                            format!("{}{}", ion_str!("cannot negate "), val.type_name()),
                            span.line,
                            span.col,
                        )
                        .into()),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!val.is_truthy())),
                }
            }

            ExprKind::Try(inner) => {
                let val = self.eval_expr(inner)?;
                match val {
                    Value::Result(Ok(v)) => Ok(*v),
                    Value::Result(Err(e)) => {
                        Err(IonError::propagated_err(e.to_string(), span.line, span.col).into())
                    }
                    Value::Option(Some(v)) => Ok(*v),
                    Value::Option(None) => {
                        Err(IonError::propagated_none(span.line, span.col).into())
                    }
                    _ => Err(IonError::type_err(
                        format!(
                            "{}{}",
                            ion_str!("? applied to non-Result/Option: "),
                            val.type_name()
                        ),
                        span.line,
                        span.col,
                    )
                    .into()),
                }
            }

            ExprKind::PipeOp { left, right } => {
                let lval = self.eval_expr(left)?;
                // right should be a Call — insert lval as first argument
                match &right.kind {
                    ExprKind::Call { func, args } => {
                        let mut new_args = vec![CallArg {
                            name: None,
                            value: Expr {
                                kind: ExprKind::Int(0),
                                span, // placeholder
                            },
                        }];
                        new_args.extend(args.iter().cloned());
                        let func_val = self.eval_expr(func)?;
                        let mut arg_vals = vec![lval];
                        for arg in args {
                            arg_vals.push(self.eval_expr(&arg.value)?);
                        }
                        self.call_value(&func_val, &arg_vals, span)
                    }
                    ExprKind::Ident(_) => {
                        // Bare function name, call with lval as only arg
                        let func_val = self.eval_expr(right)?;
                        self.call_value(&func_val, &[lval], span)
                    }
                    _ => Err(IonError::runtime(
                        ion_str!("right side of |> must be a function call").to_string(),
                        span.line,
                        span.col,
                    )
                    .into()),
                }
            }

            ExprKind::FieldAccess { expr, field } => {
                let val = self.eval_expr(expr)?;
                self.field_access(&val, field, span)
            }

            ExprKind::Index { expr, index } => {
                let val = self.eval_expr(expr)?;
                let idx = self.eval_expr(index)?;
                self.index_access(&val, &idx, span)
            }

            ExprKind::Slice {
                expr,
                start,
                end,
                inclusive,
            } => {
                let val = self.eval_expr(expr)?;
                let s = match start {
                    Some(e) => Some(self.eval_expr(e)?),
                    None => None,
                };
                let e = match end {
                    Some(e) => Some(self.eval_expr(e)?),
                    None => None,
                };
                self.slice_access(&val, s.as_ref(), e.as_ref(), *inclusive, span)
            }

            ExprKind::MethodCall { expr, method, args } => {
                let receiver = self.eval_expr(expr)?;
                let mut arg_vals = Vec::new();
                for arg in args {
                    arg_vals.push(self.eval_expr(&arg.value)?);
                }
                self.method_call(&receiver, method, &arg_vals, span)
            }

            ExprKind::Call { func, args } => {
                let func_val = self.eval_expr(func)?;
                let has_named = args.iter().any(|a| a.name.is_some());
                if has_named {
                    let mut evaluated: Vec<(Option<String>, Value)> = Vec::new();
                    for arg in args {
                        evaluated.push((arg.name.clone(), self.eval_expr(&arg.value)?));
                    }
                    self.call_with_named(&func_val, evaluated, span)
                } else {
                    let mut arg_vals = Vec::new();
                    for arg in args {
                        arg_vals.push(self.eval_expr(&arg.value)?);
                    }
                    self.call_value(&func_val, &arg_vals, span)
                }
            }

            ExprKind::Lambda { params, body } => {
                let captures = self.env.capture();
                let fn_params: Vec<Param> = params
                    .iter()
                    .map(|p| Param {
                        name: p.clone(),
                        default: None,
                    })
                    .collect();
                // Wrap body expr into a block with one ExprStmt
                let body_stmts = vec![Stmt {
                    kind: StmtKind::ExprStmt {
                        expr: (**body).clone(),
                        has_semi: false,
                    },
                    span,
                }];
                Ok(Value::Fn(IonFn::new(
                    ion_str!("<lambda>").to_string(),
                    fn_params,
                    body_stmts,
                    captures,
                )))
            }

            ExprKind::If {
                cond,
                then_body,
                else_body,
            } => {
                let c = self.eval_expr(cond)?;
                self.env.push_scope();
                let result = if c.is_truthy() {
                    self.eval_stmts(then_body)
                } else if let Some(else_stmts) = else_body {
                    self.eval_stmts(else_stmts)
                } else {
                    Ok(Value::Unit)
                };
                self.env.pop_scope();
                result
            }

            ExprKind::IfLet {
                pattern,
                expr,
                then_body,
                else_body,
            } => {
                let val = self.eval_expr(expr)?;
                if self.pattern_matches(pattern, &val) {
                    self.env.push_scope();
                    self.bind_pattern(pattern, &val, false, span)?;
                    let result = self.eval_stmts(then_body);
                    self.env.pop_scope();
                    result
                } else if let Some(else_stmts) = else_body {
                    self.env.push_scope();
                    let result = self.eval_stmts(else_stmts);
                    self.env.pop_scope();
                    result
                } else {
                    Ok(Value::Unit)
                }
            }

            ExprKind::Match { expr, arms } => {
                let val = self.eval_expr(expr)?;
                for arm in arms {
                    if self.pattern_matches(&arm.pattern, &val) {
                        self.env.push_scope();
                        self.bind_pattern(&arm.pattern, &val, false, span)?;
                        if let Some(guard) = &arm.guard {
                            let guard_val = self.eval_expr(guard)?;
                            if !guard_val.is_truthy() {
                                self.env.pop_scope();
                                continue;
                            }
                        }
                        let result = self.eval_expr(&arm.body);
                        self.env.pop_scope();
                        return result;
                    }
                }
                Err(IonError::runtime(
                    ion_str!("non-exhaustive match").to_string(),
                    span.line,
                    span.col,
                )
                .into())
            }

            ExprKind::Block(stmts) => {
                self.env.push_scope();
                let result = self.eval_stmts(stmts);
                self.env.pop_scope();
                result
            }

            ExprKind::LoopExpr(body) => {
                let no_label: Option<String> = None;
                let result = loop {
                    self.env.push_scope();
                    match self.eval_stmts(body) {
                        Ok(_) => {}
                        Err(SignalOrError::Signal(Signal::Break {
                            label: sig_label,
                            value,
                        })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, &no_label) {
                                break value;
                            }
                            return Err(Signal::Break {
                                label: sig_label,
                                value,
                            }
                            .into());
                        }
                        Err(SignalOrError::Signal(Signal::Continue { label: sig_label })) => {
                            self.env.pop_scope();
                            if signal_targets_loop(&sig_label, &no_label) {
                                continue;
                            }
                            return Err(Signal::Continue { label: sig_label }.into());
                        }
                        Err(e) => {
                            self.env.pop_scope();
                            return Err(e);
                        }
                    }
                    self.env.pop_scope();
                };
                Ok(result)
            }

            ExprKind::TryCatch { body, var, handler } => {
                self.env.push_scope();
                let result = self.eval_stmts(body);
                self.env.pop_scope();
                match result {
                    Ok(v) => Ok(v),
                    Err(SignalOrError::Signal(s)) => {
                        // Signals (return/break/continue) pass through — not errors
                        Err(SignalOrError::Signal(s))
                    }
                    Err(SignalOrError::Error(e)) => {
                        // Catch the error: bind error message to `var`, run handler
                        self.env.push_scope();
                        self.env
                            .define(var.clone(), Value::Str(e.message.clone()), false);
                        let handler_result = self.eval_stmts(handler);
                        self.env.pop_scope();
                        handler_result
                    }
                }
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let s = self.eval_expr(start)?;
                let e = self.eval_expr(end)?;
                match (&s, &e) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Range {
                        start: *a,
                        end: *b,
                        inclusive: *inclusive,
                    }),
                    _ => Err(IonError::type_err(
                        ion_str!("range requires integer bounds").to_string(),
                        span.line,
                        span.col,
                    )
                    .into()),
                }
            }

            ExprKind::StructConstruct {
                name,
                fields,
                spread,
            } => {
                let mut field_map = IndexMap::new();
                if let Some(spread_expr) = spread {
                    let spread_val = self.eval_expr(spread_expr)?;
                    match spread_val {
                        Value::HostStruct { fields: sf, .. } => {
                            for (k, v) in sf {
                                field_map.insert(k, v);
                            }
                        }
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("spread in struct constructor requires a struct")
                                    .to_string(),
                                span.line,
                                span.col,
                            )
                            .into())
                        }
                    }
                }
                for (fname, fexpr) in fields {
                    let val = self.eval_expr(fexpr)?;
                    field_map.insert(fname.clone(), val);
                }
                self.types
                    .construct_struct(name, field_map)
                    .map_err(|msg| IonError::runtime(msg, span.line, span.col).into())
            }
            ExprKind::EnumVariant { enum_name, variant } => self
                .types
                .construct_enum(enum_name, variant, vec![])
                .map_err(|msg| IonError::runtime(msg, span.line, span.col).into()),
            ExprKind::EnumVariantCall {
                enum_name,
                variant,
                args,
            } => {
                let mut vals = Vec::new();
                for arg in args {
                    vals.push(self.eval_expr(arg)?);
                }
                self.types
                    .construct_enum(enum_name, variant, vals)
                    .map_err(|msg| IonError::runtime(msg, span.line, span.col).into())
            }

            // Concurrency
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            ExprKind::AsyncBlock(body) => self.eval_async_block(body, span),
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            ExprKind::SpawnExpr(expr) => self.eval_spawn(expr, span),
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            ExprKind::AwaitExpr(expr) => self.eval_await(expr, span),
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            ExprKind::SelectExpr(branches) => self.eval_select(branches, span),

            #[cfg(not(all(feature = "concurrency", not(feature = "async-runtime"))))]
            ExprKind::AsyncBlock(_)
            | ExprKind::SpawnExpr(_)
            | ExprKind::AwaitExpr(_)
            | ExprKind::SelectExpr(_) => Err(IonError::runtime(
                ion_str!("concurrency features require the 'concurrency' cargo feature")
                    .to_string(),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    // --- Helpers ---

    fn eval_binop(&self, op: BinOp, l: &Value, r: &Value, span: Span) -> SignalResult {
        match op {
            BinOp::Add => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                (Value::Bytes(a), Value::Bytes(b)) => {
                    let mut result = a.clone();
                    result.extend(b);
                    Ok(Value::Bytes(result))
                }
                _ => Err(self.type_mismatch_err(ion_str!("+"), l, r, span)),
            },
            BinOp::Sub => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
                _ => Err(self.type_mismatch_err(ion_str!("-"), l, r, span)),
            },
            BinOp::Mul => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
                (Value::Str(s), Value::Int(n)) | (Value::Int(n), Value::Str(s)) => {
                    Ok(Value::Str(s.repeat(*n as usize)))
                }
                _ => Err(self.type_mismatch_err(ion_str!("*"), l, r, span)),
            },
            BinOp::Div => match (l, r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Err(IonError::runtime(
                            ion_str!("division by zero").to_string(),
                            span.line,
                            span.col,
                        )
                        .into())
                    } else {
                        Ok(Value::Int(a / b))
                    }
                }
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
                _ => Err(self.type_mismatch_err(ion_str!("/"), l, r, span)),
            },
            BinOp::Mod => match (l, r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Err(IonError::runtime(
                            ion_str!("modulo by zero").to_string(),
                            span.line,
                            span.col,
                        )
                        .into())
                    } else {
                        Ok(Value::Int(a % b))
                    }
                }
                _ => Err(self.type_mismatch_err(ion_str!("%"), l, r, span)),
            },
            BinOp::Eq => Ok(Value::Bool(l == r)),
            BinOp::Ne => Ok(Value::Bool(l != r)),
            BinOp::Lt => self.compare_values(l, r, span, |o| o == std::cmp::Ordering::Less),
            BinOp::Gt => self.compare_values(l, r, span, |o| o == std::cmp::Ordering::Greater),
            BinOp::Le => self.compare_values(l, r, span, |o| o != std::cmp::Ordering::Greater),
            BinOp::Ge => self.compare_values(l, r, span, |o| o != std::cmp::Ordering::Less),
            BinOp::And | BinOp::Or => unreachable!(), // handled in eval_expr
            BinOp::BitAnd => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a & b)),
                _ => Err(self.type_mismatch_err(ion_str!("&"), l, r, span)),
            },
            BinOp::BitOr => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a | b)),
                _ => Err(self.type_mismatch_err(ion_str!("|"), l, r, span)),
            },
            BinOp::BitXor => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a ^ b)),
                _ => Err(self.type_mismatch_err(ion_str!("^"), l, r, span)),
            },
            BinOp::Shl => match (l, r) {
                (Value::Int(a), Value::Int(b)) if (0..64).contains(b) => Ok(Value::Int(a << b)),
                (Value::Int(_), Value::Int(b)) => Err(IonError::runtime(
                    format!("shift count {} is out of range 0..64", b),
                    span.line,
                    span.col,
                )
                .into()),
                _ => Err(self.type_mismatch_err(ion_str!("<<"), l, r, span)),
            },
            BinOp::Shr => match (l, r) {
                (Value::Int(a), Value::Int(b)) if (0..64).contains(b) => Ok(Value::Int(a >> b)),
                (Value::Int(_), Value::Int(b)) => Err(IonError::runtime(
                    format!("shift count {} is out of range 0..64", b),
                    span.line,
                    span.col,
                )
                .into()),
                _ => Err(self.type_mismatch_err(ion_str!(">>"), l, r, span)),
            },
        }
    }

    fn compare_values(
        &self,
        l: &Value,
        r: &Value,
        span: Span,
        f: impl Fn(std::cmp::Ordering) -> bool,
    ) -> SignalResult {
        let ord = match (l, r) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::Int(a), Value::Float(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            _ => return Err(self.type_mismatch_err(ion_str!("compare"), l, r, span)),
        };
        Ok(Value::Bool(f(ord)))
    }

    fn type_mismatch_err(
        &self,
        op: impl std::fmt::Display,
        l: &Value,
        r: &Value,
        span: Span,
    ) -> SignalOrError {
        IonError::type_err(
            format!(
                "{}{}{}{}{}{}",
                ion_str!("cannot apply '"),
                op,
                ion_str!("' to "),
                l.type_name(),
                ion_str!(" and "),
                r.type_name(),
            ),
            span.line,
            span.col,
        )
        .into()
    }

    fn apply_compound_op(
        &self,
        op: AssignOp,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> SignalResult {
        match op {
            AssignOp::PlusEq => self.eval_binop(BinOp::Add, lhs, rhs, span),
            AssignOp::MinusEq => self.eval_binop(BinOp::Sub, lhs, rhs, span),
            AssignOp::StarEq => self.eval_binop(BinOp::Mul, lhs, rhs, span),
            AssignOp::SlashEq => self.eval_binop(BinOp::Div, lhs, rhs, span),
            AssignOp::Eq => unreachable!(),
        }
    }

    fn field_access(&self, val: &Value, field: &str, span: Span) -> SignalResult {
        match val {
            Value::Dict(map) => Ok(match map.get(field) {
                Some(v) => v.clone(),
                None => Value::Option(None),
            }),
            Value::HostStruct { fields, .. } => Ok(match fields.get(field) {
                Some(v) => v.clone(),
                None => {
                    return Err(IonError::type_err(
                        format!(
                            "{}{}{}",
                            ion_str!("no field '"),
                            field,
                            ion_str!("' on struct")
                        ),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }),
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot access field on "), val.type_name()),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn index_access(&self, val: &Value, idx: &Value, span: Span) -> SignalResult {
        match (val, idx) {
            (Value::List(items), Value::Int(i)) => {
                let index = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(index).cloned().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("list index out of bounds").to_string(),
                        span.line,
                        span.col,
                    )
                    .into()
                })
            }
            (Value::Dict(map), Value::Str(key)) => Ok(match map.get(key.as_str()) {
                Some(v) => v.clone(),
                None => Value::Option(None),
            }),
            (Value::Bytes(bytes), Value::Int(i)) => {
                let index = if *i < 0 { bytes.len() as i64 + i } else { *i } as usize;
                bytes
                    .get(index)
                    .map(|&b| Value::Int(b as i64))
                    .ok_or_else(|| {
                        IonError::runtime(
                            ion_str!("bytes index out of bounds").to_string(),
                            span.line,
                            span.col,
                        )
                        .into()
                    })
            }
            (Value::Str(s), Value::Int(i)) => {
                let index = if *i < 0 {
                    s.chars().count() as i64 + i
                } else {
                    *i
                } as usize;
                s.chars()
                    .nth(index)
                    .map(|c| Value::Str(c.to_string()))
                    .ok_or_else(|| {
                        IonError::runtime(
                            ion_str!("string index out of bounds").to_string(),
                            span.line,
                            span.col,
                        )
                        .into()
                    })
            }
            (Value::Tuple(items), Value::Int(i)) => {
                let index = *i as usize;
                items.get(index).cloned().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("tuple index out of bounds").to_string(),
                        span.line,
                        span.col,
                    )
                    .into()
                })
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("cannot index "),
                    val.type_name(),
                    ion_str!(" with "),
                    idx.type_name(),
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn slice_access(
        &self,
        val: &Value,
        start: Option<&Value>,
        end: Option<&Value>,
        inclusive: bool,
        span: Span,
    ) -> SignalResult {
        let get_idx = |v: Option<&Value>, default: i64| -> Result<i64, SignalOrError> {
            match v {
                Some(Value::Int(n)) => Ok(*n),
                None => Ok(default),
                Some(other) => Err(IonError::type_err(
                    format!(
                        "{}{}",
                        ion_str!("slice index must be int, got "),
                        other.type_name()
                    ),
                    span.line,
                    span.col,
                )
                .into()),
            }
        };

        match val {
            Value::List(items) => {
                let len = items.len() as i64;
                let s = get_idx(start, 0)?;
                let e = get_idx(end, len)?;
                let s = s.max(0).min(len) as usize;
                let e = if inclusive {
                    (e + 1).max(0).min(len) as usize
                } else {
                    e.max(0).min(len) as usize
                };
                Ok(Value::List(items[s..e].to_vec()))
            }
            Value::Str(string) => {
                let chars: Vec<char> = string.chars().collect();
                let len = chars.len() as i64;
                let s = get_idx(start, 0)?;
                let e = get_idx(end, len)?;
                let s = s.max(0).min(len) as usize;
                let e = if inclusive {
                    (e + 1).max(0).min(len) as usize
                } else {
                    e.max(0).min(len) as usize
                };
                Ok(Value::Str(chars[s..e].iter().collect()))
            }
            Value::Bytes(bytes) => {
                let len = bytes.len() as i64;
                let s = get_idx(start, 0)?;
                let e = get_idx(end, len)?;
                let s = s.max(0).min(len) as usize;
                let e = if inclusive {
                    (e + 1).max(0).min(len) as usize
                } else {
                    e.max(0).min(len) as usize
                };
                Ok(Value::Bytes(bytes[s..e].to_vec()))
            }
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot slice "), val.type_name()),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn method_call(
        &mut self,
        receiver: &Value,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        // Universal methods available on all types
        if method == "to_string" {
            return Ok(Value::Str(format!("{}", receiver)));
        }
        match receiver {
            Value::List(items) => self.list_method(items, method, args, span),
            Value::Tuple(items) => self.tuple_method(items, method, args, span),
            Value::Str(s) => self.string_method(s, method, args, span),
            Value::Bytes(b) => self.bytes_method(b, method, args, span),
            Value::Dict(map) => match method {
                "map" => {
                    let func = &args[0];
                    let mut result = indexmap::IndexMap::new();
                    for (k, v) in map {
                        let mapped =
                            self.call_value(func, &[Value::Str(k.clone()), v.clone()], span)?;
                        result.insert(k.clone(), mapped);
                    }
                    Ok(Value::Dict(result))
                }
                "filter" => {
                    let func = &args[0];
                    let mut result = indexmap::IndexMap::new();
                    for (k, v) in map {
                        let keep =
                            self.call_value(func, &[Value::Str(k.clone()), v.clone()], span)?;
                        if keep.is_truthy() {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                    Ok(Value::Dict(result))
                }
                _ => self.dict_method(map, method, args, span),
            },
            Value::Set(items) => self.set_method(items, method, args, span),
            Value::Option(opt) => self.option_method(opt.clone(), method, args, span),
            Value::Result(res) => self.result_method(res.clone(), method, args, span),
            Value::Range {
                start,
                end,
                inclusive,
            } => match method {
                "len" => Ok(Value::Int(Value::range_len(*start, *end, *inclusive))),
                "contains" => {
                    let val = args[0]
                        .as_int()
                        .ok_or_else(|| {
                            IonError::type_err(
                                ion_str!("range.contains requires int"),
                                span.line,
                                span.col,
                            )
                        })
                        .map_err(SignalOrError::from)?;
                    let in_range = if *inclusive {
                        val >= *start && val <= *end
                    } else {
                        val >= *start && val < *end
                    };
                    Ok(Value::Bool(in_range))
                }
                "to_list" => Ok(Value::List(Value::range_to_list(*start, *end, *inclusive))),
                // For other list-like methods, materialize and delegate
                _ => {
                    let items = Value::range_to_list(*start, *end, *inclusive);
                    self.list_method(&items, method, args, span)
                }
            },
            Value::Cell(cell) => self.cell_method(cell, method, args, span),
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            Value::Task(handle) => self.task_method(handle, method, args, span),
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            Value::Channel(ch) => self.channel_method(ch, method, args, span),
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on "),
                    receiver.type_name(),
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn list_method(
        &mut self,
        items: &[Value],
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "push" => {
                let mut new_list = items.to_vec();
                new_list.push(args[0].clone());
                Ok(Value::List(new_list))
            }
            "pop" => {
                if items.is_empty() {
                    Ok(Value::Tuple(vec![Value::List(vec![]), Value::Option(None)]))
                } else {
                    let mut new_list = items.to_vec();
                    let popped = new_list.pop().unwrap();
                    Ok(Value::Tuple(vec![
                        Value::List(new_list),
                        Value::Option(Some(Box::new(popped))),
                    ]))
                }
            }
            "map" => {
                let func = &args[0];
                let mut result = Vec::new();
                for item in items {
                    result.push(self.call_value(func, std::slice::from_ref(item), span)?);
                }
                Ok(Value::List(result))
            }
            "filter" => {
                let func = &args[0];
                let mut result = Vec::new();
                for item in items {
                    let keep = self.call_value(func, std::slice::from_ref(item), span)?;
                    if keep.is_truthy() {
                        result.push(item.clone());
                    }
                }
                Ok(Value::List(result))
            }
            "fold" => {
                let mut acc = args[0].clone();
                let func = &args[1];
                for item in items {
                    acc = self.call_value(func, &[acc, item.clone()], span)?;
                }
                Ok(acc)
            }
            "flat_map" => {
                let func = &args[0];
                let mut result = Vec::new();
                for item in items {
                    let mapped = self.call_value(func, std::slice::from_ref(item), span)?;
                    match mapped {
                        Value::List(sub) => result.extend(sub),
                        other => result.push(other),
                    }
                }
                Ok(Value::List(result))
            }
            "any" => {
                let func = &args[0];
                for item in items {
                    let v = self.call_value(func, std::slice::from_ref(item), span)?;
                    if v.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "all" => {
                let func = &args[0];
                for item in items {
                    let v = self.call_value(func, std::slice::from_ref(item), span)?;
                    if !v.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "first" => Ok(match items.first() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            "last" => Ok(match items.last() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            "reverse" => {
                let mut rev = items.to_vec();
                rev.reverse();
                Ok(Value::List(rev))
            }
            "sort" => {
                if !items.is_empty() {
                    let first_type = std::mem::discriminant(&items[0]);
                    for item in items.iter().skip(1) {
                        if std::mem::discriminant(item) != first_type {
                            return Err(IonError::type_err(
                                ion_str!("sort() requires all elements to be the same type")
                                    .to_string(),
                                span.line,
                                span.col,
                            )
                            .into());
                        }
                    }
                }
                let mut sorted = items.to_vec();
                sorted.sort_by(|a, b| match (a, b) {
                    (Value::Int(x), Value::Int(y)) => x.cmp(y),
                    (Value::Float(x), Value::Float(y)) => {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::Str(x), Value::Str(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                });
                Ok(Value::List(sorted))
            }
            "sort_by" => {
                let func = &args[0];
                let mut sorted = items.to_vec();
                let mut err: Option<SignalOrError> = None;
                let func_clone = func.clone();
                sorted.sort_by(|a, b| {
                    if err.is_some() {
                        return std::cmp::Ordering::Equal;
                    }
                    match self.call_value(&func_clone, &[a.clone(), b.clone()], span) {
                        Ok(Value::Int(n)) => {
                            if n < 0 {
                                std::cmp::Ordering::Less
                            } else if n > 0 {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Equal
                            }
                        }
                        Ok(_) => {
                            err = Some(
                                IonError::type_err(
                                    ion_str!("sort_by comparator must return an int").to_string(),
                                    span.line,
                                    span.col,
                                )
                                .into(),
                            );
                            std::cmp::Ordering::Equal
                        }
                        Err(e) => {
                            err = Some(e);
                            std::cmp::Ordering::Equal
                        }
                    }
                });
                if let Some(e) = err {
                    return Err(e);
                }
                Ok(Value::List(sorted))
            }
            "flatten" => {
                let mut result = Vec::new();
                for item in items {
                    if let Value::List(inner) = item {
                        result.extend(inner.iter().cloned());
                    } else {
                        result.push(item.clone());
                    }
                }
                Ok(Value::List(result))
            }
            "zip" => {
                if let Value::List(other) = &args[0] {
                    let result: Vec<Value> = items
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| Value::Tuple(vec![a.clone(), b.clone()]))
                        .collect();
                    Ok(Value::List(result))
                } else {
                    Err(IonError::type_err(
                        ion_str!("zip requires a list argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "contains" => {
                let target = &args[0];
                Ok(Value::Bool(items.iter().any(|v| v == target)))
            }
            "join" => {
                let sep = if args.is_empty() {
                    String::new()
                } else {
                    args[0]
                        .as_str()
                        .ok_or_else(|| {
                            IonError::type_err(
                                ion_str!("join separator must be a string").to_string(),
                                span.line,
                                span.col,
                            )
                        })?
                        .to_string()
                };
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                Ok(Value::Str(parts.join(&sep)))
            }
            "enumerate" => Ok(Value::List(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                    .collect(),
            )),
            "is_empty" => Ok(Value::Bool(items.is_empty())),
            "index" => {
                let target = &args[0];
                Ok(match items.iter().position(|v| v == target) {
                    Some(i) => Value::Option(Some(Box::new(Value::Int(i as i64)))),
                    None => Value::Option(None),
                })
            }
            "count" => {
                let target = &args[0];
                Ok(Value::Int(
                    items.iter().filter(|v| *v == target).count() as i64
                ))
            }
            "slice" => {
                let start = args.first().and_then(|a| a.as_int()).unwrap_or(0) as usize;
                let end = args
                    .get(1)
                    .and_then(|a| a.as_int())
                    .map(|n| n as usize)
                    .unwrap_or(items.len());
                let start = start.min(items.len());
                let end = end.min(items.len());
                Ok(Value::List(items[start..end].to_vec()))
            }
            "dedup" => {
                let mut result: Vec<Value> = Vec::new();
                for item in items {
                    if result.last() != Some(item) {
                        result.push(item.clone());
                    }
                }
                Ok(Value::List(result))
            }
            "unique" => {
                let mut seen = Vec::new();
                let mut result = Vec::new();
                for item in items {
                    if !seen.contains(item) {
                        seen.push(item.clone());
                        result.push(item.clone());
                    }
                }
                Ok(Value::List(result))
            }
            "min" => {
                if items.is_empty() {
                    return Ok(Value::Option(None));
                }
                let mut min = &items[0];
                for item in items.iter().skip(1) {
                    match (min, item) {
                        (Value::Int(a), Value::Int(b)) => {
                            if b < a {
                                min = item;
                            }
                        }
                        (Value::Float(a), Value::Float(b)) => {
                            if b < a {
                                min = item;
                            }
                        }
                        (Value::Str(a), Value::Str(b)) => {
                            if b < a {
                                min = item;
                            }
                        }
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("min() requires homogeneous comparable elements")
                                    .to_string(),
                                span.line,
                                span.col,
                            )
                            .into())
                        }
                    }
                }
                Ok(Value::Option(Some(Box::new(min.clone()))))
            }
            "max" => {
                if items.is_empty() {
                    return Ok(Value::Option(None));
                }
                let mut max = &items[0];
                for item in items.iter().skip(1) {
                    match (max, item) {
                        (Value::Int(a), Value::Int(b)) => {
                            if b > a {
                                max = item;
                            }
                        }
                        (Value::Float(a), Value::Float(b)) => {
                            if b > a {
                                max = item;
                            }
                        }
                        (Value::Str(a), Value::Str(b)) => {
                            if b > a {
                                max = item;
                            }
                        }
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("max() requires homogeneous comparable elements")
                                    .to_string(),
                                span.line,
                                span.col,
                            )
                            .into())
                        }
                    }
                }
                Ok(Value::Option(Some(Box::new(max.clone()))))
            }
            "sum" => {
                let mut int_sum: i64 = 0;
                let mut float_sum: f64 = 0.0;
                let mut has_float = false;
                for item in items {
                    match item {
                        Value::Int(n) => int_sum += n,
                        Value::Float(f) => {
                            has_float = true;
                            float_sum += f;
                        }
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("sum() requires numeric elements").to_string(),
                                span.line,
                                span.col,
                            )
                            .into())
                        }
                    }
                }
                if has_float {
                    Ok(Value::Float(float_sum + int_sum as f64))
                } else {
                    Ok(Value::Int(int_sum))
                }
            }
            "window" => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("window requires int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })? as usize;
                if n == 0 {
                    return Err(IonError::runtime(
                        ion_str!("window size must be > 0").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let result: Vec<Value> =
                    items.windows(n).map(|w| Value::List(w.to_vec())).collect();
                Ok(Value::List(result))
            }
            "chunk" => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("chunk requires int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })? as usize;
                if n == 0 {
                    return Err(IonError::type_err(
                        ion_str!("chunk size must be > 0").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let result: Vec<Value> = items.chunks(n).map(|c| Value::List(c.to_vec())).collect();
                Ok(Value::List(result))
            }
            "reduce" => {
                if items.is_empty() {
                    return Err(IonError::type_err(
                        ion_str!("reduce on empty list").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let func = &args[0];
                let mut acc = items[0].clone();
                for item in items.iter().skip(1) {
                    acc = self.call_value(func, &[acc, item.clone()], span)?;
                }
                Ok(acc)
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on list")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn set_method(
        &self,
        items: &[Value],
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "contains" => {
                let target = &args[0];
                Ok(Value::Bool(items.iter().any(|v| v == target)))
            }
            "is_empty" => Ok(Value::Bool(items.is_empty())),
            "add" => {
                let val = &args[0];
                let mut new = items.to_vec();
                if !new.iter().any(|v| v == val) {
                    new.push(val.clone());
                }
                Ok(Value::Set(new))
            }
            "remove" => {
                let val = &args[0];
                let new: Vec<Value> = items.iter().filter(|v| *v != val).cloned().collect();
                Ok(Value::Set(new))
            }
            "union" => {
                if let Value::Set(other) = &args[0] {
                    let mut new = items.to_vec();
                    for v in other {
                        if !new.iter().any(|x| x == v) {
                            new.push(v.clone());
                        }
                    }
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("union requires a set argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "intersection" => {
                if let Value::Set(other) = &args[0] {
                    let new: Vec<Value> = items
                        .iter()
                        .filter(|v| other.iter().any(|x| x == *v))
                        .cloned()
                        .collect();
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("intersection requires a set argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "difference" => {
                if let Value::Set(other) = &args[0] {
                    let new: Vec<Value> = items
                        .iter()
                        .filter(|v| !other.iter().any(|x| x == *v))
                        .cloned()
                        .collect();
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("difference requires a set argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "to_list" => Ok(Value::List(items.to_vec())),
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on set")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn cell_method(
        &mut self,
        cell: &std::sync::Arc<std::sync::Mutex<Value>>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "get" => {
                let inner = cell.lock().unwrap();
                Ok(inner.clone())
            }
            "set" => {
                if args.is_empty() {
                    return Err(IonError::runtime(
                        ion_str!("cell.set() requires 1 argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let mut inner = cell.lock().unwrap();
                *inner = args[0].clone();
                Ok(Value::Unit)
            }
            "update" => {
                if args.is_empty() {
                    return Err(IonError::runtime(
                        ion_str!("cell.update() requires a function argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let current = { cell.lock().unwrap().clone() };
                let new_val = self.call_value(&args[0], &[current], span)?;
                let mut inner = cell.lock().unwrap();
                *inner = new_val.clone();
                Ok(new_val)
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on cell"),
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn tuple_method(
        &self,
        items: &[Value],
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "contains" => {
                let target = &args[0];
                Ok(Value::Bool(items.iter().any(|v| v == target)))
            }
            "to_list" => Ok(Value::List(items.to_vec())),
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on tuple")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn string_method(&self, s: &str, method: &str, args: &[Value], span: Span) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(s.len() as i64)),
            "contains" => match &args[0] {
                Value::Str(sub) => Ok(Value::Bool(s.contains(sub.as_str()))),
                Value::Int(code) => {
                    let ch = char::from_u32(*code as u32).ok_or_else(|| {
                        IonError::type_err(
                            ion_str!("invalid char code").to_string(),
                            span.line,
                            span.col,
                        )
                    })?;
                    Ok(Value::Bool(s.contains(ch)))
                }
                _ => Err(IonError::type_err(
                    ion_str!("contains requires string or int argument").to_string(),
                    span.line,
                    span.col,
                )
                .into()),
            },
            "starts_with" => {
                let sub = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("starts_with requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Bool(s.starts_with(sub)))
            }
            "ends_with" => {
                let sub = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("ends_with requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Bool(s.ends_with(sub)))
            }
            "trim" => Ok(Value::Str(s.trim().to_string())),
            "to_upper" => Ok(Value::Str(s.to_uppercase())),
            "to_lower" => Ok(Value::Str(s.to_lowercase())),
            "split" => {
                let delim = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("split requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let parts: Vec<Value> = s.split(delim).map(|p| Value::Str(p.to_string())).collect();
                Ok(Value::List(parts))
            }
            "replace" => {
                let from = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("replace requires string arguments").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let to = args[1].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("replace requires string arguments").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Str(s.replace(from, to)))
            }
            "chars" => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Ok(Value::List(chars))
            }
            "char_len" => Ok(Value::Int(s.chars().count() as i64)),
            "is_empty" => Ok(Value::Bool(s.is_empty())),
            "trim_start" => Ok(Value::Str(s.trim_start().to_string())),
            "trim_end" => Ok(Value::Str(s.trim_end().to_string())),
            "repeat" => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("repeat requires int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Str(s.repeat(n as usize)))
            }
            "find" => {
                let sub = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("find requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(match s.find(sub) {
                    Some(byte_idx) => {
                        let char_idx = s[..byte_idx].chars().count();
                        Value::Option(Some(Box::new(Value::Int(char_idx as i64))))
                    }
                    None => Value::Option(None),
                })
            }
            "to_int" => Ok(match s.trim().parse::<i64>() {
                std::result::Result::Ok(n) => Value::Result(Ok(Box::new(Value::Int(n)))),
                std::result::Result::Err(e) => {
                    Value::Result(Err(Box::new(Value::Str(e.to_string()))))
                }
            }),
            "to_float" => Ok(match s.trim().parse::<f64>() {
                std::result::Result::Ok(f) => Value::Result(Ok(Box::new(Value::Float(f)))),
                std::result::Result::Err(e) => {
                    Value::Result(Err(Box::new(Value::Str(e.to_string()))))
                }
            }),
            "bytes" => {
                let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
                Ok(Value::List(bytes))
            }
            "strip_prefix" => {
                let pre = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("strip_prefix requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Str(s.strip_prefix(pre).unwrap_or(s).to_string()))
            }
            "strip_suffix" => {
                let suf = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("strip_suffix requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Str(s.strip_suffix(suf).unwrap_or(s).to_string()))
            }
            "pad_start" => {
                let width = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("pad_start requires int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })? as usize;
                let ch = args
                    .get(1)
                    .and_then(|a| a.as_str())
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let char_len = s.chars().count();
                if char_len >= width {
                    Ok(Value::Str(s.to_string()))
                } else {
                    let pad: String = std::iter::repeat_n(ch, width - char_len).collect();
                    Ok(Value::Str(format!("{}{}", pad, s)))
                }
            }
            "pad_end" => {
                let width = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("pad_end requires int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })? as usize;
                let ch = args
                    .get(1)
                    .and_then(|a| a.as_str())
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let char_len = s.chars().count();
                if char_len >= width {
                    Ok(Value::Str(s.to_string()))
                } else {
                    let pad: String = std::iter::repeat_n(ch, width - char_len).collect();
                    Ok(Value::Str(format!("{}{}", s, pad)))
                }
            }
            "reverse" => Ok(Value::Str(s.chars().rev().collect())),
            "slice" => {
                let chars: Vec<char> = s.chars().collect();
                let char_count = chars.len();
                let start = args.first().and_then(|a| a.as_int()).unwrap_or(0) as usize;
                let end = args
                    .get(1)
                    .and_then(|a| a.as_int())
                    .map(|n| n as usize)
                    .unwrap_or(char_count);
                let start = start.min(char_count);
                let end = end.min(char_count);
                Ok(Value::Str(chars[start..end].iter().collect()))
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on string")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn bytes_method(&self, bytes: &[u8], method: &str, args: &[Value], span: Span) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(bytes.len() as i64)),
            "is_empty" => Ok(Value::Bool(bytes.is_empty())),
            "contains" => {
                let byte = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("bytes.contains() requires an int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Bool(bytes.contains(&(byte as u8))))
            }
            "slice" => {
                let start = args.first().and_then(|a| a.as_int()).unwrap_or(0) as usize;
                let end = args
                    .get(1)
                    .and_then(|a| a.as_int())
                    .map(|n| n as usize)
                    .unwrap_or(bytes.len());
                let start = start.min(bytes.len());
                let end = end.min(bytes.len());
                Ok(Value::Bytes(bytes[start..end].to_vec()))
            }
            "to_list" => Ok(Value::List(
                bytes.iter().map(|&b| Value::Int(b as i64)).collect(),
            )),
            "to_str" => match std::str::from_utf8(bytes) {
                std::result::Result::Ok(s) => {
                    Ok(Value::Result(Ok(Box::new(Value::Str(s.to_string())))))
                }
                std::result::Result::Err(e) => {
                    Ok(Value::Result(Err(Box::new(Value::Str(format!("{}", e))))))
                }
            },
            "to_hex" => {
                let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                Ok(Value::Str(hex))
            }
            "find" => {
                let needle = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("bytes.find() requires an int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let pos = bytes.iter().position(|&b| b == needle as u8);
                Ok(match pos {
                    Some(i) => Value::Option(Some(Box::new(Value::Int(i as i64)))),
                    None => Value::Option(None),
                })
            }
            "reverse" => {
                let mut rev = bytes.to_vec();
                rev.reverse();
                Ok(Value::Bytes(rev))
            }
            "push" => {
                let byte = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("bytes.push() requires an int argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let mut new = bytes.to_vec();
                new.push(byte as u8);
                Ok(Value::Bytes(new))
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on "),
                    ion_str!("bytes"),
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn dict_method(
        &self,
        map: &IndexMap<String, Value>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "len" => Ok(Value::Int(map.len() as i64)),
            "keys" => Ok(Value::List(
                map.keys().map(|k| Value::Str(k.clone())).collect(),
            )),
            "values" => Ok(Value::List(map.values().cloned().collect())),
            "entries" => Ok(Value::List(
                map.iter()
                    .map(|(k, v)| Value::Tuple(vec![Value::Str(k.clone()), v.clone()]))
                    .collect(),
            )),
            "contains_key" => {
                let key = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("contains_key requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Bool(map.contains_key(key)))
            }
            "get" => {
                let key = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("get requires string argument").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(match map.get(key) {
                    Some(v) => Value::Option(Some(Box::new(v.clone()))),
                    None => Value::Option(None),
                })
            }
            "insert" => {
                if args.len() < 2 {
                    return Err(IonError::runtime(
                        ion_str!("insert requires 2 arguments: key, value").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let key = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("insert requires string key").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let mut new_map = map.clone();
                new_map.insert(key.to_string(), args[1].clone());
                Ok(Value::Dict(new_map))
            }
            "remove" => {
                let key = args[0].as_str().ok_or_else(|| {
                    IonError::type_err(
                        ion_str!("remove requires string key").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                let mut new_map = map.clone();
                new_map.shift_remove(key);
                Ok(Value::Dict(new_map))
            }
            "merge" => {
                if let Value::Dict(other) = &args[0] {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        ion_str!("merge requires a dict argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "is_empty" => Ok(Value::Bool(map.is_empty())),
            "update" => {
                if let Value::Dict(other) = &args[0] {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        ion_str!("update requires a dict argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            "keys_of" => {
                let target = &args[0];
                let keys: Vec<Value> = map
                    .iter()
                    .filter(|(_, v)| *v == target)
                    .map(|(k, _)| Value::Str(k.clone()))
                    .collect();
                Ok(Value::List(keys))
            }
            "zip" => {
                if let Value::Dict(other) = &args[0] {
                    let mut result = indexmap::IndexMap::new();
                    for (k, v) in map {
                        if let Some(ov) = other.get(k) {
                            result.insert(k.clone(), Value::Tuple(vec![v.clone(), ov.clone()]));
                        }
                    }
                    Ok(Value::Dict(result))
                } else {
                    Err(IonError::type_err(
                        ion_str!("zip requires a dict argument").to_string(),
                        span.line,
                        span.col,
                    )
                    .into())
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on dict")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn option_method(
        &mut self,
        opt: Option<Box<Value>>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "is_some" => Ok(Value::Bool(opt.is_some())),
            "is_none" => Ok(Value::Bool(opt.is_none())),
            "unwrap" => match opt {
                Some(v) => Ok(*v),
                None => {
                    Err(
                        IonError::runtime(ion_str!("called unwrap on None"), span.line, span.col)
                            .into(),
                    )
                }
            },
            "unwrap_or" => match opt {
                Some(v) => Ok(*v),
                None => Ok(args[0].clone()),
            },
            "expect" => match opt {
                Some(v) => Ok(*v),
                None => {
                    let default_msg = ion_str!("expect failed");
                    let msg = args[0].as_str().unwrap_or(&default_msg);
                    Err(IonError::runtime(msg.to_string(), span.line, span.col).into())
                }
            },
            "map" => {
                let func = args[0].clone();
                match opt {
                    Some(v) => {
                        let result = self.call_value(&func, &[*v], span)?;
                        Ok(Value::Option(Some(Box::new(result))))
                    }
                    None => Ok(Value::Option(None)),
                }
            }
            "and_then" => {
                let func = args[0].clone();
                match opt {
                    Some(v) => self.call_value(&func, &[*v], span),
                    None => Ok(Value::Option(None)),
                }
            }
            "or_else" => {
                let func = args[0].clone();
                match opt {
                    Some(v) => Ok(Value::Option(Some(v))),
                    None => self.call_value(&func, &[], span),
                }
            }
            "unwrap_or_else" => {
                let func = args[0].clone();
                match opt {
                    Some(v) => Ok(*v),
                    None => self.call_value(&func, &[], span),
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on Option")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn result_method(
        &mut self,
        res: Result<Box<Value>, Box<Value>>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "is_ok" => Ok(Value::Bool(res.is_ok())),
            "is_err" => Ok(Value::Bool(res.is_err())),
            "unwrap" => match res {
                Ok(v) => Ok(*v),
                Err(e) => Err(IonError::runtime(
                    format!("{}{}", ion_str!("called unwrap on Err: "), e),
                    span.line,
                    span.col,
                )
                .into()),
            },
            "unwrap_or" => match res {
                Ok(v) => Ok(*v),
                Err(_) => Ok(args[0].clone()),
            },
            "expect" => match res {
                Ok(v) => Ok(*v),
                Err(e) => {
                    let default_msg = ion_str!("expect failed");
                    let msg = args[0].as_str().unwrap_or(&default_msg);
                    Err(IonError::runtime(format!("{}: {}", msg, e), span.line, span.col).into())
                }
            },
            "map" => {
                let func = args[0].clone();
                match res {
                    Ok(v) => {
                        let result = self.call_value(&func, &[*v], span)?;
                        Ok(Value::Result(Ok(Box::new(result))))
                    }
                    Err(e) => Ok(Value::Result(Err(e))),
                }
            }
            "map_err" => {
                let func = args[0].clone();
                match res {
                    Ok(v) => Ok(Value::Result(Ok(v))),
                    Err(e) => {
                        let result = self.call_value(&func, &[*e], span)?;
                        Ok(Value::Result(Err(Box::new(result))))
                    }
                }
            }
            "and_then" => {
                let func = args[0].clone();
                match res {
                    Ok(v) => self.call_value(&func, &[*v], span),
                    Err(e) => Ok(Value::Result(Err(e))),
                }
            }
            "or_else" => {
                let func = args[0].clone();
                match res {
                    Ok(v) => Ok(Value::Result(Ok(v))),
                    Err(e) => self.call_value(&func, &[*e], span),
                }
            }
            "unwrap_or_else" => {
                let func = args[0].clone();
                match res {
                    Ok(v) => Ok(*v),
                    Err(e) => self.call_value(&func, &[*e], span),
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on Result")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn call_value(&mut self, func: &Value, args: &[Value], span: Span) -> SignalResult {
        match func {
            Value::Fn(ion_fn) => {
                if self.call_depth >= self.limits.max_call_depth {
                    return Err(IonError::runtime(
                        ion_str!("maximum call depth exceeded").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                self.call_depth += 1;
                self.env.push_scope();
                let result = (|| {
                    // Load captures
                    for (name, val) in &ion_fn.captures {
                        self.env.define(name.clone(), val.clone(), false);
                    }
                    // Bind parameters
                    for (i, param) in ion_fn.params.iter().enumerate() {
                        let val = if i < args.len() {
                            args[i].clone()
                        } else if let Some(default) = &param.default {
                            self.eval_expr(default)?
                        } else {
                            return Err(IonError::runtime(
                                format!(
                                    "{}{}{}{}{}{}",
                                    ion_str!("function '"),
                                    ion_fn.name,
                                    ion_str!("' expected "),
                                    ion_fn.params.len(),
                                    ion_str!(" arguments, got "),
                                    args.len(),
                                ),
                                span.line,
                                span.col,
                            )
                            .into());
                        };
                        self.env.define(param.name.clone(), val, false);
                    }
                    self.eval_stmts(&ion_fn.body)
                })();
                self.env.pop_scope();
                self.call_depth -= 1;
                match result {
                    Ok(v) => Ok(v),
                    Err(SignalOrError::Signal(Signal::Return(v))) => Ok(v),
                    Err(SignalOrError::Signal(Signal::Break { label, .. })) => Err(
                        IonError::runtime(unmatched_label_msg("break", label), span.line, span.col)
                            .into(),
                    ),
                    Err(SignalOrError::Signal(Signal::Continue { label })) => {
                        Err(IonError::runtime(
                            unmatched_label_msg("continue", label),
                            span.line,
                            span.col,
                        )
                        .into())
                    }
                    Err(SignalOrError::Error(e)) => {
                        // Convert ? propagation into values at function boundary
                        if e.kind == ErrorKind::PropagatedErr {
                            Ok(Value::Result(Err(Box::new(Value::Str(e.message.clone())))))
                        } else if e.kind == ErrorKind::PropagatedNone {
                            Ok(Value::Option(None))
                        } else {
                            Err(e.into())
                        }
                    }
                }
            }
            #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
            Value::BuiltinFn(ref name, _) if name == "timeout" => self.builtin_timeout(args, span),
            Value::BuiltinFn(_, func) => {
                func(args).map_err(|msg| IonError::runtime(msg, span.line, span.col).into())
            }
            Value::BuiltinClosure(_, func) => func
                .call(args)
                .map_err(|msg| IonError::runtime(msg, span.line, span.col).into()),
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure(_, _) => Err(IonError::runtime(
                ion_str!(
                        "async host function cannot be called by the synchronous evaluator; use eval_async"
                )
                .to_string(),
                span.line,
                span.col,
            )
            .into()),
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("not callable: "), func.type_name()),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
    fn builtin_timeout(&mut self, args: &[Value], span: Span) -> SignalResult {
        if args.len() < 2 {
            return Err(IonError::runtime(
                ion_str!("timeout(ms, fn) requires 2 arguments").to_string(),
                span.line,
                span.col,
            )
            .into());
        }
        let ms = args[0].as_int().ok_or_else(|| {
            SignalOrError::Error(IonError::runtime(
                ion_str!("timeout: first argument must be int (ms)").to_string(),
                span.line,
                span.col,
            ))
        })?;
        let func = args[1].clone();
        // Call the function synchronously but with a timeout via spawn + join_timeout
        let captured_env = self.env.capture();
        let limits = self.limits.clone();
        let types = self.types.clone();
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = crate::async_rt::spawn_task_with_cancel(cancel_flag, move |flag| {
            let mut child = Interpreter::new();
            child.limits = limits;
            child.types = types;
            child.cancel_flag = Some(flag);
            for (name, val) in captured_env {
                child.env.define(name, val, false);
            }
            child
                .call_value(&func, &[], crate::ast::Span { line: 0, col: 0 })
                .map_err(|e| match e {
                    SignalOrError::Error(err) => err,
                    SignalOrError::Signal(_) => {
                        IonError::runtime(ion_str!("unexpected signal in timeout"), 0, 0)
                    }
                })
        });
        match task.join_timeout(std::time::Duration::from_millis(ms as u64)) {
            Some(Ok(val)) => Ok(Value::Option(Some(Box::new(val)))),
            Some(Err(e)) => Err(e.into()),
            None => {
                // Timer expired — signal cancellation so the runaway
                // task terminates at the next statement boundary.
                task.cancel();
                Ok(Value::Option(None))
            }
        }
    }

    fn call_with_named(
        &mut self,
        func: &Value,
        named_args: Vec<(Option<String>, Value)>,
        span: Span,
    ) -> SignalResult {
        match func {
            Value::Fn(ion_fn) => {
                // Reorder named args to match parameter positions
                let mut ordered = vec![None; ion_fn.params.len()];
                let mut pos_idx = 0;
                for (name, val) in named_args {
                    if let Some(name) = name {
                        // Find param by name
                        let param_idx = ion_fn
                            .params
                            .iter()
                            .position(|p| p.name == name)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    format!(
                                        "{}{}{}{}",
                                        ion_str!("unknown parameter '"),
                                        name,
                                        ion_str!("' for function '"),
                                        ion_fn.name,
                                    ),
                                    span.line,
                                    span.col,
                                )
                            })?;
                        ordered[param_idx] = Some(val);
                    } else {
                        // Positional arg — fill next empty slot
                        while pos_idx < ordered.len() && ordered[pos_idx].is_some() {
                            pos_idx += 1;
                        }
                        if pos_idx < ordered.len() {
                            ordered[pos_idx] = Some(val);
                            pos_idx += 1;
                        }
                    }
                }
                // Keep as Option<Value> so we can distinguish "not provided" from "provided Unit"
                let opt_args: Vec<Option<Value>> = ordered;
                // Use call_value with the reordered args, but handle defaults specially
                if self.call_depth >= self.limits.max_call_depth {
                    return Err(IonError::runtime(
                        ion_str!("maximum call depth exceeded").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                self.call_depth += 1;
                self.env.push_scope();
                let result = (|| {
                    for (name, val) in &ion_fn.captures {
                        self.env.define(name.clone(), val.clone(), false);
                    }
                    for (i, param) in ion_fn.params.iter().enumerate() {
                        let val = if i < opt_args.len() && opt_args[i].is_some() {
                            opt_args[i].clone().unwrap()
                        } else if let Some(default) = &param.default {
                            self.eval_expr(default)?
                        } else {
                            return Err(IonError::runtime(
                                format!(
                                    "{}{}{}",
                                    ion_str!("missing argument '"),
                                    param.name,
                                    ion_str!("'"),
                                ),
                                span.line,
                                span.col,
                            )
                            .into());
                        };
                        self.env.define(param.name.clone(), val, false);
                    }
                    self.eval_stmts(&ion_fn.body)
                })();
                self.env.pop_scope();
                self.call_depth -= 1;
                match result {
                    Ok(v) => Ok(v),
                    Err(SignalOrError::Signal(Signal::Return(v))) => Ok(v),
                    Err(SignalOrError::Error(e)) if e.kind == ErrorKind::PropagatedErr => {
                        Ok(Value::Result(Err(Box::new(Value::Str(e.message.clone())))))
                    }
                    Err(SignalOrError::Error(e)) if e.kind == ErrorKind::PropagatedNone => {
                        Ok(Value::Option(None))
                    }
                    Err(e) => Err(e),
                }
            }
            _ => {
                // For builtins, just pass positional values
                let args: Vec<Value> = named_args.into_iter().map(|(_, v)| v).collect();
                self.call_value(func, &args, span)
            }
        }
    }

    fn check_type_ann(val: &Value, ann: &TypeAnn, span: Span) -> Result<(), SignalOrError> {
        let matches = match ann {
            TypeAnn::Simple(name) => match name.as_str() {
                "int" => matches!(val, Value::Int(_)),
                "float" => matches!(val, Value::Float(_)),
                "bool" => matches!(val, Value::Bool(_)),
                "string" => matches!(val, Value::Str(_)),
                "bytes" => matches!(val, Value::Bytes(_)),
                "list" => matches!(val, Value::List(_)),
                "dict" => matches!(val, Value::Dict(_)),
                "tuple" => matches!(val, Value::Tuple(_)),
                "set" => matches!(val, Value::Set(_)),
                "fn" => match val {
                    Value::Fn(_) | Value::BuiltinFn(_, _) | Value::BuiltinClosure(_, _) => true,
                    #[cfg(feature = "async-runtime")]
                    Value::AsyncBuiltinClosure(_, _) => true,
                    _ => false,
                },
                "cell" => matches!(val, Value::Cell(_)),
                "any" => true,
                _ => true, // unknown types pass (forward compatibility)
            },
            TypeAnn::Option(_) => matches!(val, Value::Option(_)),
            TypeAnn::Result(_, _) => matches!(val, Value::Result(_)),
            TypeAnn::List(_) => matches!(val, Value::List(_)),
            TypeAnn::Dict(_, _) => matches!(val, Value::Dict(_)),
        };
        if !matches {
            return Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("type mismatch: expected "),
                    Self::type_ann_name(ann),
                    ion_str!(", got "),
                    val.type_name()
                ),
                span.line,
                span.col,
            )
            .into());
        }
        Ok(())
    }

    fn type_ann_name(ann: &TypeAnn) -> String {
        match ann {
            TypeAnn::Simple(name) => name.clone(),
            TypeAnn::Option(inner) => format!("Option<{}>", Self::type_ann_name(inner)),
            TypeAnn::Result(ok, err) => {
                format!(
                    "Result<{}, {}>",
                    Self::type_ann_name(ok),
                    Self::type_ann_name(err)
                )
            }
            TypeAnn::List(inner) => format!("list<{}>", Self::type_ann_name(inner)),
            TypeAnn::Dict(k, v) => {
                format!(
                    "dict<{}, {}>",
                    Self::type_ann_name(k),
                    Self::type_ann_name(v)
                )
            }
        }
    }

    fn value_to_iter(&self, val: &Value, span: Span) -> Result<Vec<Value>, SignalOrError> {
        match val {
            Value::List(items) => Ok(items.clone()),
            Value::Set(items) => Ok(items.clone()),
            Value::Tuple(items) => Ok(items.clone()),
            Value::Dict(map) => Ok(map
                .iter()
                .map(|(k, v)| Value::Tuple(vec![Value::Str(k.clone()), v.clone()]))
                .collect()),
            Value::Str(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            Value::Bytes(bytes) => Ok(bytes.iter().map(|&b| Value::Int(b as i64)).collect()),
            Value::Range {
                start,
                end,
                inclusive,
            } => Ok(Value::range_to_list(*start, *end, *inclusive)),
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot iterate over "), val.type_name()),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    // --- Pattern Matching ---

    fn pattern_matches(&self, pattern: &Pattern, val: &Value) -> bool {
        match (pattern, val) {
            (Pattern::Wildcard, _) => true,
            (Pattern::Ident(_), _) => true,
            (Pattern::Int(a), Value::Int(b)) => a == b,
            (Pattern::Float(a), Value::Float(b)) => a == b,
            (Pattern::Bool(a), Value::Bool(b)) => a == b,
            (Pattern::Str(a), Value::Str(b)) => a == b,
            (Pattern::Bytes(a), Value::Bytes(b)) => a == b,
            (Pattern::None, Value::Option(None)) => true,
            (Pattern::Some(p), Value::Option(Some(v))) => self.pattern_matches(p, v),
            (Pattern::Ok(p), Value::Result(Ok(v))) => self.pattern_matches(p, v),
            (Pattern::Err(p), Value::Result(Err(v))) => self.pattern_matches(p, v),
            (Pattern::Tuple(pats), Value::Tuple(vals)) => {
                pats.len() == vals.len()
                    && pats
                        .iter()
                        .zip(vals)
                        .all(|(p, v)| self.pattern_matches(p, v))
            }
            (Pattern::List(pats, rest), Value::List(vals)) => {
                if rest.is_some() {
                    vals.len() >= pats.len()
                        && pats
                            .iter()
                            .zip(vals)
                            .all(|(p, v)| self.pattern_matches(p, v))
                } else {
                    pats.len() == vals.len()
                        && pats
                            .iter()
                            .zip(vals)
                            .all(|(p, v)| self.pattern_matches(p, v))
                }
            }
            (
                Pattern::EnumVariant {
                    enum_name,
                    variant,
                    fields,
                },
                Value::HostEnum {
                    enum_name: en,
                    variant: v,
                    data,
                },
            ) => {
                if enum_name != en || variant != v {
                    return false;
                }
                match fields {
                    EnumPatternFields::None => data.is_empty(),
                    EnumPatternFields::Positional(pats) => {
                        pats.len() == data.len()
                            && pats
                                .iter()
                                .zip(data)
                                .all(|(p, v)| self.pattern_matches(p, v))
                    }
                    EnumPatternFields::Named(_) => false, // named fields not applicable to enum data
                }
            }
            (
                Pattern::Struct { name, fields },
                Value::HostStruct {
                    type_name,
                    fields: val_fields,
                },
            ) => {
                if name != type_name {
                    return false;
                }
                fields.iter().all(|(fname, fpat)| {
                    match val_fields.get(fname) {
                        Some(v) => match fpat {
                            Some(p) => self.pattern_matches(p, v),
                            None => true, // just binding, always matches
                        },
                        None => false,
                    }
                })
            }
            _ => false,
        }
    }

    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        val: &Value,
        mutable: bool,
        span: Span,
    ) -> Result<(), SignalOrError> {
        match (pattern, val) {
            (Pattern::Wildcard, _) => Ok(()),
            (Pattern::Ident(name), _) => {
                self.env.define(name.clone(), val.clone(), mutable);
                Ok(())
            }
            (
                Pattern::Int(_)
                | Pattern::Float(_)
                | Pattern::Bool(_)
                | Pattern::Str(_)
                | Pattern::Bytes(_)
                | Pattern::None,
                _,
            ) => Ok(()),
            (Pattern::Some(p), Value::Option(Some(v))) => self.bind_pattern(p, v, mutable, span),
            (Pattern::Ok(p), Value::Result(Ok(v))) => self.bind_pattern(p, v, mutable, span),
            (Pattern::Err(p), Value::Result(Err(v))) => self.bind_pattern(p, v, mutable, span),
            (Pattern::Tuple(pats), Value::Tuple(vals)) => {
                for (p, v) in pats.iter().zip(vals) {
                    self.bind_pattern(p, v, mutable, span)?;
                }
                Ok(())
            }
            (Pattern::List(pats, rest), Value::List(vals)) => {
                for (p, v) in pats.iter().zip(vals) {
                    self.bind_pattern(p, v, mutable, span)?;
                }
                if let Some(rest_pat) = rest {
                    let rest_vals = vals[pats.len()..].to_vec();
                    self.bind_pattern(rest_pat, &Value::List(rest_vals), mutable, span)?;
                }
                Ok(())
            }
            (Pattern::EnumVariant { fields, .. }, Value::HostEnum { data, .. }) => match fields {
                EnumPatternFields::None => Ok(()),
                EnumPatternFields::Positional(pats) => {
                    for (p, v) in pats.iter().zip(data) {
                        self.bind_pattern(p, v, mutable, span)?;
                    }
                    Ok(())
                }
                EnumPatternFields::Named(_) => Ok(()),
            },
            (
                Pattern::Struct { fields, .. },
                Value::HostStruct {
                    fields: val_fields, ..
                },
            ) => {
                for (fname, fpat) in fields {
                    if let Some(v) = val_fields.get(fname) {
                        match fpat {
                            Some(p) => self.bind_pattern(p, v, mutable, span)?,
                            None => self.env.define(fname.clone(), v.clone(), mutable),
                        }
                    }
                }
                Ok(())
            }
            _ => Err(IonError::runtime(
                ion_str!("pattern match failed in binding").to_string(),
                span.line,
                span.col,
            )
            .into()),
        }
    }
}

#[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
impl Interpreter {
    fn eval_async_block(&mut self, body: &[Stmt], _span: Span) -> SignalResult {
        use crate::async_rt::Nursery;

        // Save and set nursery for this scope
        let prev_nursery = self.nursery.take();
        self.nursery = Some(Nursery::new());

        self.env.push_scope();
        let result = self.eval_stmts(body);
        self.env.pop_scope();

        // Join all spawned tasks (structured concurrency)
        let nursery = self.nursery.take().unwrap();
        self.nursery = prev_nursery;

        if let Err(e) = nursery.join_all() {
            return Err(e.into());
        }

        result
    }

    fn eval_spawn(&mut self, expr: &Expr, span: Span) -> SignalResult {
        use std::sync::Arc;

        // Require being inside an async block
        if self.nursery.is_none() {
            return Err(IonError::runtime(
                ion_str!("spawn is only allowed inside async {}").to_string(),
                span.line,
                span.col,
            )
            .into());
        }

        // Capture current environment for the spawned task
        let captured_env = self.env.capture();
        let expr_clone = expr.clone();
        let limits = self.limits.clone();
        let types = self.types.clone();

        let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_handle: Arc<dyn crate::async_rt::TaskHandle> =
            crate::async_rt::spawn_task_with_cancel(cancel_flag, move |flag| {
                let mut child = Interpreter::new();
                child.limits = limits;
                child.types = types;
                child.cancel_flag = Some(flag);
                // Load captured environment
                for (name, val) in captured_env {
                    child.env.define(name, val, false);
                }
                // Evaluate the expression
                let program = crate::ast::Program {
                    stmts: vec![crate::ast::Stmt {
                        kind: crate::ast::StmtKind::ExprStmt {
                            expr: expr_clone,
                            has_semi: false,
                        },
                        span: crate::ast::Span { line: 0, col: 0 },
                    }],
                };
                child.eval_program(&program)
            });

        // Register with nursery
        if let Some(nursery) = &mut self.nursery {
            nursery.spawn(task_handle.clone());
        }

        Ok(Value::Task(task_handle))
    }

    fn eval_await(&mut self, expr: &Expr, span: Span) -> SignalResult {
        let val = self.eval_expr(expr)?;
        match val {
            Value::Task(handle) => handle.join().map_err(SignalOrError::Error),
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot await "), val.type_name()),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn eval_select(&mut self, branches: &[crate::ast::SelectBranch], span: Span) -> SignalResult {
        use std::sync::Arc;

        // Spawn all branch futures as tasks
        let mut tasks: Vec<(usize, Arc<dyn crate::async_rt::TaskHandle>)> = Vec::new();
        for (i, branch) in branches.iter().enumerate() {
            let captured_env = self.env.capture();
            let expr_clone = branch.future_expr.clone();
            let limits = self.limits.clone();
            let types = self.types.clone();

            let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let handle = crate::async_rt::spawn_task_with_cancel(cancel_flag, move |flag| {
                let mut child = Interpreter::new();
                child.limits = limits;
                child.types = types;
                child.cancel_flag = Some(flag);
                for (name, val) in captured_env {
                    child.env.define(name, val, false);
                }
                let program = crate::ast::Program {
                    stmts: vec![crate::ast::Stmt {
                        kind: crate::ast::StmtKind::ExprStmt {
                            expr: expr_clone,
                            has_semi: false,
                        },
                        span: crate::ast::Span { line: 0, col: 0 },
                    }],
                };
                child.eval_program(&program)
            });
            tasks.push((i, handle));
        }

        // Wait for the first task to complete (condvar-based, no polling)
        let handles: Vec<_> = tasks.iter().map(|(_, h)| h.clone()).collect();
        let (winner_idx, result) = crate::async_rt::wait_any(&handles);

        // Signal cancellation to losing branches so they terminate at
        // the next statement boundary instead of burning CPU.
        for (i, h) in handles.iter().enumerate() {
            if i != winner_idx {
                h.cancel();
            }
        }

        let result = result?;
        let branch = &branches[tasks[winner_idx].0];
        self.env.push_scope();
        self.bind_pattern(&branch.pattern, &result, false, span)?;
        let body_result = self.eval_expr(&branch.body);
        self.env.pop_scope();
        body_result
    }

    fn task_method(
        &self,
        handle: &std::sync::Arc<dyn crate::async_rt::TaskHandle>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        match method {
            "is_finished" => Ok(Value::Bool(handle.is_finished())),
            "cancel" => {
                handle.cancel();
                Ok(Value::Unit)
            }
            "is_cancelled" => Ok(Value::Bool(handle.is_cancelled())),
            "await_timeout" => {
                let ms = args.first().and_then(|v| v.as_int()).ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("await_timeout requires int (ms)").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                match handle.join_timeout(std::time::Duration::from_millis(ms as u64)) {
                    Some(result) => {
                        let val = result.map_err(SignalOrError::Error)?;
                        Ok(Value::Option(Some(Box::new(val))))
                    }
                    None => Ok(Value::Option(None)),
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on Task")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn channel_method(
        &self,
        ch: &crate::async_rt::ChannelEnd,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> SignalResult {
        use crate::async_rt::ChannelEnd;
        match (ch, method) {
            (ChannelEnd::Sender(tx), "send") => {
                if args.is_empty() {
                    return Err(IonError::runtime(
                        ion_str!("send requires a value").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                tx.send(args[0].clone()).map_err(|e| {
                    IonError::runtime(
                        format!("{}{}", ion_str!("channel send failed: "), e.message),
                        span.line,
                        span.col,
                    )
                })?;
                Ok(Value::Unit)
            }
            (ChannelEnd::Sender(tx), "close") => {
                tx.close();
                Ok(Value::Unit)
            }
            (ChannelEnd::Receiver(rx), "recv") => match rx.recv() {
                Some(v) => Ok(Value::Option(Some(Box::new(v)))),
                None => Ok(Value::Option(None)),
            },
            (ChannelEnd::Receiver(rx), "try_recv") => match rx.try_recv() {
                Some(v) => Ok(Value::Option(Some(Box::new(v)))),
                None => Ok(Value::Option(None)),
            },
            (ChannelEnd::Receiver(rx), "recv_timeout") => {
                if args.is_empty() {
                    return Err(IonError::runtime(
                        ion_str!("recv_timeout requires a timeout in ms").to_string(),
                        span.line,
                        span.col,
                    )
                    .into());
                }
                let ms = args[0].as_int().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("recv_timeout requires int (ms)").to_string(),
                        span.line,
                        span.col,
                    )
                })?;
                match rx.recv_timeout(std::time::Duration::from_millis(ms as u64)) {
                    Some(v) => Ok(Value::Option(Some(Box::new(v)))),
                    None => Ok(Value::Option(None)),
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("no method '"),
                    method,
                    ion_str!("' on Channel")
                ),
                span.line,
                span.col,
            )
            .into()),
        }
    }
}

pub fn register_builtins(env: &mut Env, output: Arc<dyn OutputHandler>) {
    // I/O: moved to io:: module
    // Math: moved to math:: module
    // JSON/Msgpack: moved to json:: module

    env.define(
        ion_str!("len").to_string(),
        Value::BuiltinFn(ion_str!("len").to_string(), |args| {
            if args.is_empty() {
                return Err(ion_str!("len() requires 1 argument"));
            }
            match &args[0] {
                Value::List(items) => Ok(Value::Int(items.len() as i64)),
                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                Value::Dict(map) => Ok(Value::Int(map.len() as i64)),
                Value::Bytes(b) => Ok(Value::Int(b.len() as i64)),
                _ => Err(format!(
                    "{}{}",
                    ion_str!("len() not supported for "),
                    args[0].type_name()
                )),
            }
        }),
        false,
    );
    env.define(
        ion_str!("range").to_string(),
        Value::BuiltinFn(ion_str!("range").to_string(), |args| match args.len() {
            1 => {
                let n = args[0].as_int().ok_or(ion_str!("range requires int"))?;
                Ok(Value::Range {
                    start: 0,
                    end: n,
                    inclusive: false,
                })
            }
            2 => {
                let start = args[0].as_int().ok_or(ion_str!("range requires int"))?;
                let end = args[1].as_int().ok_or(ion_str!("range requires int"))?;
                Ok(Value::Range {
                    start,
                    end,
                    inclusive: false,
                })
            }
            _ => Err(ion_str!("range takes 1 or 2 arguments").to_string()),
        }),
        false,
    );
    env.define(
        ion_str!("set"),
        Value::BuiltinFn(ion_str!("set"), |args| {
            if args.is_empty() {
                return Ok(Value::Set(vec![]));
            }
            match &args[0] {
                Value::List(items) => {
                    let mut unique = Vec::new();
                    for v in items {
                        if !unique.iter().any(|x| x == v) {
                            unique.push(v.clone());
                        }
                    }
                    Ok(Value::Set(unique))
                }
                _ => Err(ion_str!("set() requires a list argument")),
            }
        }),
        false,
    );
    env.define(
        ion_str!("cell"),
        Value::BuiltinFn(ion_str!("cell"), |args| {
            if args.len() != 1 {
                return Err(ion_str!("cell() takes 1 argument"));
            }
            Ok(Value::Cell(std::sync::Arc::new(std::sync::Mutex::new(
                args[0].clone(),
            ))))
        }),
        false,
    );
    env.define(
        ion_str!("type_of").to_string(),
        Value::BuiltinFn(ion_str!("type_of").to_string(), |args| {
            if args.is_empty() {
                return Err(ion_str!("type_of() requires 1 argument"));
            }
            Ok(Value::Str(args[0].type_name().to_string()))
        }),
        false,
    );
    // json_encode, json_decode, abs, min, max → moved to json:: / math:: modules
    env.define(
        ion_str!("str").to_string(),
        Value::BuiltinFn(ion_str!("str").to_string(), |args| {
            if args.len() != 1 {
                return Err(ion_str!("str takes 1 argument"));
            }
            Ok(Value::Str(args[0].to_string()))
        }),
        false,
    );
    env.define(
        ion_str!("int").to_string(),
        Value::BuiltinFn(ion_str!("int").to_string(), |args| {
            if args.len() != 1 {
                return Err(ion_str!("int takes 1 argument"));
            }
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Float(n) => Ok(Value::Int(*n as i64)),
                Value::Str(s) => s.parse::<i64>().map(Value::Int).map_err(|_| {
                    format!(
                        "{}{}{}",
                        ion_str!("cannot convert '"),
                        s,
                        ion_str!("' to int")
                    )
                }),
                Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                _ => Err(format!(
                    "{}{}",
                    ion_str!("cannot convert "),
                    args[0].type_name()
                )),
            }
        }),
        false,
    );
    env.define(
        ion_str!("float").to_string(),
        Value::BuiltinFn(ion_str!("float").to_string(), |args| {
            if args.len() != 1 {
                return Err(ion_str!("float takes 1 argument"));
            }
            match &args[0] {
                Value::Float(n) => Ok(Value::Float(*n)),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                Value::Str(s) => s.parse::<f64>().map(Value::Float).map_err(|_| {
                    format!(
                        "{}{}{}",
                        ion_str!("cannot convert '"),
                        s,
                        ion_str!("' to float")
                    )
                }),
                _ => Err(format!(
                    "{}{}",
                    ion_str!("cannot convert "),
                    args[0].type_name()
                )),
            }
        }),
        false,
    );
    // floor, ceil, round, pow, sqrt, clamp → moved to math:: module
    // join → moved to str:: module
    // json_encode_pretty, msgpack_encode, msgpack_decode → moved to json:: module
    env.define(
        ion_str!("enumerate").to_string(),
        Value::BuiltinFn(ion_str!("enumerate").to_string(), |args| {
            if args.len() != 1 {
                return Err(ion_str!("enumerate takes 1 argument"));
            }
            match &args[0] {
                Value::List(items) => Ok(Value::List(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                        .collect(),
                )),
                Value::Str(s) => Ok(Value::List(
                    s.chars()
                        .enumerate()
                        .map(|(i, c)| {
                            Value::Tuple(vec![Value::Int(i as i64), Value::Str(c.to_string())])
                        })
                        .collect(),
                )),
                Value::Dict(map) => Ok(Value::List(
                    map.iter()
                        .enumerate()
                        .map(|(i, (k, v))| {
                            Value::Tuple(vec![
                                Value::Int(i as i64),
                                Value::Tuple(vec![Value::Str(k.clone()), v.clone()]),
                            ])
                        })
                        .collect(),
                )),
                _ => Err(format!(
                    "{}{}",
                    ion_str!("enumerate() not supported for "),
                    args[0].type_name()
                )),
            }
        }),
        false,
    );

    env.define(
        ion_str!("bytes").to_string(),
        Value::BuiltinFn(ion_str!("bytes").to_string(), |args| match args.first() {
            Some(Value::List(items)) => {
                let mut bytes = Vec::with_capacity(items.len());
                for item in items {
                    let n = item
                        .as_int()
                        .ok_or_else(|| ion_str!("bytes() list items must be ints"))?;
                    if !(0..=255).contains(&n) {
                        return Err(format!("{}{}", ion_str!("byte value out of range: "), n));
                    }
                    bytes.push(n as u8);
                }
                Ok(Value::Bytes(bytes))
            }
            Some(Value::Str(s)) => Ok(Value::Bytes(s.as_bytes().to_vec())),
            Some(Value::Int(n)) if *n >= 0 && *n <= 10_000_000 => {
                Ok(Value::Bytes(vec![0u8; *n as usize]))
            }
            Some(Value::Int(n)) => Err(format!(
                "bytes(n): size {} is out of range (0..10_000_000)",
                n
            )),
            None => Ok(Value::Bytes(Vec::new())),
            _ => Err(format!(
                "{}{}",
                ion_str!("bytes() not supported for "),
                args[0].type_name()
            )),
        }),
        false,
    );
    env.define(
        ion_str!("bytes_from_hex").to_string(),
        Value::BuiltinFn(ion_str!("bytes_from_hex").to_string(), |args| {
            if args.len() != 1 {
                return Err(ion_str!("bytes_from_hex takes 1 argument"));
            }
            let s = args[0]
                .as_str()
                .ok_or_else(|| ion_str!("bytes_from_hex requires a string"))?;
            if !s.is_ascii() {
                return Err(ion_str!("bytes_from_hex requires an ASCII hex string"));
            }
            if s.len() % 2 != 0 {
                return Err(ion_str!("hex string must have even length").to_string());
            }
            let mut bytes = Vec::with_capacity(s.len() / 2);
            for i in (0..s.len()).step_by(2) {
                let byte = u8::from_str_radix(&s[i..i + 2], 16)
                    .map_err(|_| format!("{}{}", ion_str!("invalid hex: "), &s[i..i + 2]))?;
                bytes.push(byte);
            }
            Ok(Value::Bytes(bytes))
        }),
        false,
    );

    env.define(
        ion_str!("assert").to_string(),
        Value::BuiltinFn(ion_str!("assert").to_string(), |args| {
            if args.is_empty() {
                return Err(ion_str!("assert requires at least 1 argument").to_string());
            }
            let condition = match &args[0] {
                Value::Bool(b) => *b,
                _ => {
                    return Err(format!(
                        "{}{}",
                        ion_str!("assert condition must be bool, got "),
                        args[0].type_name()
                    ))
                }
            };
            if !condition {
                let msg = if args.len() > 1 {
                    args[1].to_string()
                } else {
                    ion_str!("assertion failed").to_string()
                };
                return Err(msg);
            }
            Ok(Value::Unit)
        }),
        false,
    );

    env.define(
        ion_str!("assert_eq").to_string(),
        Value::BuiltinFn(ion_str!("assert_eq").to_string(), |args| {
            if args.len() < 2 {
                return Err(ion_str!("assert_eq requires at least 2 arguments").to_string());
            }
            if args[0] != args[1] {
                let msg = if args.len() > 2 {
                    format!(
                        "{}{}{}{}{}",
                        args[2],
                        ion_str!(": expected "),
                        args[0],
                        ion_str!(", got "),
                        args[1]
                    )
                } else {
                    format!(
                        "{}{}{}{}",
                        ion_str!("assertion failed: expected "),
                        args[0],
                        ion_str!(", got "),
                        args[1]
                    )
                };
                return Err(msg);
            }
            Ok(Value::Unit)
        }),
        false,
    );

    #[cfg(all(feature = "concurrency", not(feature = "async-runtime")))]
    {
        env.define(
            ion_str!("sleep").to_string(),
            Value::BuiltinFn(ion_str!("sleep").to_string(), |args| {
                let ms = args
                    .first()
                    .and_then(|v| v.as_int())
                    .ok_or(ion_str!("sleep requires int (ms)"))?;
                crate::async_rt::sleep(std::time::Duration::from_millis(ms as u64));
                Ok(Value::Unit)
            }),
            false,
        );
        env.define(
            ion_str!("timeout").to_string(),
            Value::BuiltinFn(ion_str!("timeout").to_string(), |_args| {
                // Actual implementation is in call_value (needs interpreter context)
                Err(ion_str!("timeout: internal error (should not reach here)"))
            }),
            false,
        );
        env.define(
            ion_str!("channel").to_string(),
            Value::BuiltinFn(ion_str!("channel").to_string(), |args| {
                let buffer = if args.is_empty() {
                    16
                } else {
                    args[0]
                        .as_int()
                        .ok_or(ion_str!("channel buffer size must be int"))?
                        as usize
                };
                let (tx, rx) = crate::async_rt::create_channel(buffer);
                Ok(Value::Tuple(vec![tx, rx]))
            }),
            false,
        );
    }

    // Register stdlib modules (math, json, io)
    crate::stdlib::register_stdlib_with_output(env, output);
}
