//! Stack-based virtual machine for executing Ion bytecode.

use indexmap::IndexMap;

use crate::bytecode::{Chunk, Op};
use crate::env::Env;
use crate::error::IonError;
use crate::host_types::TypeRegistry;
use crate::stdlib::{OutputHandler, OutputStream};
use crate::value::Value;
use std::sync::Arc;

/// A local variable stored in a stack slot.
#[derive(Debug, Clone)]
struct LocalSlot {
    value: Value,
    mutable: bool,
}

/// An exception handler for try/catch blocks.
#[derive(Debug, Clone)]
struct ExceptionHandler {
    /// IP to jump to (start of catch block).
    catch_ip: usize,
    /// Stack depth to restore on catch.
    stack_depth: usize,
    /// Local frames depth to restore on catch.
    local_frames_depth: usize,
    /// Locals depth to restore on catch.
    locals_depth: usize,
}

/// The Ion virtual machine.
pub struct Vm {
    /// Value stack.
    stack: Vec<Value>,
    /// Environment for variable bindings (globals and fallback).
    env: Env,
    /// Instruction pointer.
    ip: usize,
    /// Iterator stack for for-loops.
    iterators: Vec<Box<dyn Iterator<Item = Value>>>,
    /// Compilation cache: fn_id -> compiled bytecode chunk.
    fn_cache: std::collections::HashMap<u64, crate::bytecode::Chunk>,
    /// Pending tail call: (func, args) to be executed by the trampoline.
    pending_tail_call: Option<(Value, Vec<Value>)>,
    /// Stack-slot local variables (fast indexed access).
    locals: Vec<LocalSlot>,
    /// Scope boundaries in the locals array (each entry is the locals.len() at scope start).
    local_frames: Vec<usize>,
    /// Base offset for the current function's locals (slot indices are relative to this).
    locals_base: usize,
    /// Host type registry for struct/enum construction.
    types: TypeRegistry,
    /// Exception handler stack for try/catch.
    exception_handlers: Vec<ExceptionHandler>,
    /// Host output handler for bytecode print operations.
    output: Arc<dyn OutputHandler>,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(256),
            env: Env::new(),
            ip: 0,
            iterators: Vec::new(),
            fn_cache: std::collections::HashMap::new(),
            pending_tail_call: None,
            locals: Vec::with_capacity(64),
            local_frames: Vec::with_capacity(16),
            locals_base: 0,
            types: TypeRegistry::default(),
            exception_handlers: Vec::new(),
            output: crate::stdlib::missing_output_handler(),
        }
    }

    /// Create a VM with an existing environment (for engine integration).
    pub fn with_env(env: Env) -> Self {
        Self::with_env_and_output(env, crate::stdlib::missing_output_handler())
    }

    /// Create a VM with an existing environment and output handler.
    pub fn with_env_and_output(env: Env, output: Arc<dyn OutputHandler>) -> Self {
        Self {
            stack: Vec::with_capacity(256),
            env,
            ip: 0,
            iterators: Vec::new(),
            fn_cache: std::collections::HashMap::new(),
            pending_tail_call: None,
            locals: Vec::with_capacity(64),
            local_frames: Vec::with_capacity(16),
            locals_base: 0,
            types: TypeRegistry::default(),
            exception_handlers: Vec::new(),
            output,
        }
    }

    /// Set the host output handler for bytecode print operations.
    pub fn set_output_handler(&mut self, output: Arc<dyn OutputHandler>) {
        self.output = output;
    }

    /// Set the type registry for host type construction.
    pub fn set_types(&mut self, types: TypeRegistry) {
        self.types = types;
    }

    /// Get a reference to the environment.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Get a mutable reference to the environment.
    pub fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    /// Pre-populate the function cache with precompiled chunks from the compiler.
    pub fn preload_fn_chunks(&mut self, chunks: crate::value::FnChunkCache) {
        self.fn_cache.extend(chunks);
    }

    /// Execute a compiled chunk, returning the final value.
    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, IonError> {
        self.ip = 0;
        self.stack.clear();
        match self.run_chunk(chunk) {
            Ok(v) => Ok(v),
            Err(e) if e.kind == crate::error::ErrorKind::PropagatedErr => {
                Ok(Value::Result(Err(Box::new(Value::Str(e.message.clone())))))
            }
            Err(e) if e.kind == crate::error::ErrorKind::PropagatedNone => Ok(Value::Option(None)),
            Err(e) => Err(e),
        }
    }

    /// Run a chunk without resetting state (used for recursive function calls).
    fn run_chunk(&mut self, chunk: &Chunk) -> Result<Value, IonError> {
        while self.ip < chunk.code.len() {
            let op_byte = chunk.code[self.ip];
            let line = chunk.lines[self.ip];
            let col = chunk.cols[self.ip];
            self.ip += 1;

            let op = match self.decode_op(op_byte, line, col) {
                Ok(op) => op,
                Err(e) => {
                    if let Some(handler) = self.exception_handlers.pop() {
                        self.stack.truncate(handler.stack_depth);
                        self.locals.truncate(handler.locals_depth);
                        self.local_frames.truncate(handler.local_frames_depth);
                        self.stack.push(Value::Str(e.message.clone()));
                        self.ip = handler.catch_ip;
                        continue;
                    }
                    return Err(e);
                }
            };

            // Handle TryBegin/TryEnd before dispatch_instruction
            match op {
                Op::TryBegin => {
                    let offset = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let catch_ip = self.ip + offset;
                    self.exception_handlers.push(ExceptionHandler {
                        catch_ip,
                        stack_depth: self.stack.len(),
                        local_frames_depth: self.local_frames.len(),
                        locals_depth: self.locals.len(),
                    });
                    continue;
                }
                Op::TryEnd => {
                    let offset = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    self.exception_handlers.pop();
                    self.ip += offset; // jump over catch block
                    continue;
                }
                _ => {}
            }

            match self.dispatch_instruction(op, chunk, line, col) {
                Ok(Some(val)) => return Ok(val),
                Ok(None) => {}
                Err(e) => {
                    // Check if we're inside a try block
                    if e.kind != crate::error::ErrorKind::PropagatedErr
                        && e.kind != crate::error::ErrorKind::PropagatedNone
                    {
                        if let Some(handler) = self.exception_handlers.pop() {
                            self.stack.truncate(handler.stack_depth);
                            self.locals.truncate(handler.locals_depth);
                            self.local_frames.truncate(handler.local_frames_depth);
                            self.stack.push(Value::Str(e.message.clone()));
                            self.ip = handler.catch_ip;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
        Ok(self.stack.pop().unwrap_or(Value::Unit))
    }

    /// Dispatch a single instruction. Returns Ok(Some(val)) for Return, Ok(None) to continue.
    fn dispatch_instruction(
        &mut self,
        op: Op,
        chunk: &Chunk,
        line: usize,
        col: usize,
    ) -> Result<Option<Value>, IonError> {
        match op {
            Op::Constant => {
                let idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let val = chunk.constants[idx].clone();
                self.stack.push(val);
            }

            Op::True => self.stack.push(Value::Bool(true)),
            Op::False => self.stack.push(Value::Bool(false)),
            Op::Unit => self.stack.push(Value::Unit),
            Op::None => self.stack.push(Value::Option(None)),

            // --- Arithmetic ---
            Op::Add => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(self.op_add(a, b, line, col)?);
            }
            Op::Sub => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(self.op_sub(a, b, line, col)?);
            }
            Op::Mul => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(self.op_mul(a, b, line, col)?);
            }
            Op::Div => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(self.op_div(a, b, line, col)?);
            }
            Op::Mod => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(self.op_mod(a, b, line, col)?);
            }
            Op::Neg => {
                let val = self.pop(line, col)?;
                self.stack.push(self.op_neg(val, line, col)?);
            }

            // --- Bitwise ---
            Op::BitAnd => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x & y)),
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{} and {}",
                                ion_str!("'&' expects int, got "),
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }
            Op::BitOr => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x | y)),
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{} and {}",
                                ion_str!("'|' expects int, got "),
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }
            Op::BitXor => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x ^ y)),
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{} and {}",
                                ion_str!("'^' expects int, got "),
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }
            Op::Shl => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) if (0..64).contains(&y) => {
                        self.stack.push(Value::Int(x << y))
                    }
                    (Value::Int(_), Value::Int(y)) => {
                        return Err(IonError::runtime(
                            ion_format!("shift count {} is out of range 0..64", y),
                            line,
                            col,
                        ));
                    }
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{} and {}",
                                ion_str!("'<<' expects int, got "),
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }
            Op::Shr => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) if (0..64).contains(&y) => {
                        self.stack.push(Value::Int(x >> y))
                    }
                    (Value::Int(_), Value::Int(y)) => {
                        return Err(IonError::runtime(
                            ion_format!("shift count {} is out of range 0..64", y),
                            line,
                            col,
                        ));
                    }
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{} and {}",
                                ion_str!("'>>' expects int, got "),
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }

            // --- Comparison ---
            Op::Eq => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(Value::Bool(a == b));
            }
            Op::NotEq => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack.push(Value::Bool(a != b));
            }
            Op::Lt => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack
                    .push(Value::Bool(self.compare_lt(&a, &b, line, col)?));
            }
            Op::Gt => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack
                    .push(Value::Bool(self.compare_lt(&b, &a, line, col)?));
            }
            Op::LtEq => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack
                    .push(Value::Bool(!self.compare_lt(&b, &a, line, col)?));
            }
            Op::GtEq => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                self.stack
                    .push(Value::Bool(!self.compare_lt(&a, &b, line, col)?));
            }

            // --- Logic ---
            Op::Not => {
                let val = self.pop(line, col)?;
                self.stack.push(Value::Bool(!val.is_truthy()));
            }
            Op::And => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                if !self.is_top_truthy(line, col)? {
                    self.ip += offset; // short-circuit: keep falsy value
                }
                // If truthy, fall through — Pop will remove it, then eval right
            }
            Op::Or => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                if self.is_top_truthy(line, col)? {
                    self.ip += offset; // short-circuit: keep truthy value
                }
            }

            // --- Variables ---
            Op::DefineLocal => {
                let name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let mutable = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let sym = self.const_to_sym(&chunk.constants[name_idx], line, col)?;
                let val = self.pop(line, col)?;
                self.env.define_sym(sym, val, mutable);
            }
            Op::GetLocal => {
                let name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let sym = self.const_to_sym(&chunk.constants[name_idx], line, col)?;
                let val = self.env.get_sym(sym).cloned().ok_or_else(|| {
                    let name = self.env.resolve(sym);
                    IonError::name(
                        format!("{}{}", ion_str!("undefined variable: "), name),
                        line,
                        col,
                    )
                })?;
                self.stack.push(val);
            }
            Op::SetLocal => {
                let name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let sym = self.const_to_sym(&chunk.constants[name_idx], line, col)?;
                let val = self.pop(line, col)?;
                self.env
                    .set_sym(sym, val.clone())
                    .map_err(|e| IonError::runtime(e, line, col))?;
                self.stack.push(val); // assignment is an expression
            }
            Op::GetGlobal => {
                let name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let sym = self.const_to_sym(&chunk.constants[name_idx], line, col)?;
                let val = self.env.get_sym_or_global(sym).cloned().ok_or_else(|| {
                    let name = self.env.resolve(sym);
                    IonError::name(
                        format!("{}{}", ion_str!("undefined variable: "), name),
                        line,
                        col,
                    )
                })?;
                self.stack.push(val);
            }
            Op::SetGlobal => {
                let name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let sym = self.const_to_sym(&chunk.constants[name_idx], line, col)?;
                let val = self.pop(line, col)?;
                self.env
                    .set_sym(sym, val.clone())
                    .map_err(|e| IonError::runtime(e, line, col))?;
                self.stack.push(val);
            }

            // --- Stack-slot locals (fast path) ---
            Op::DefineLocalSlot => {
                let mutable = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let val = self.pop(line, col)?;
                self.locals.push(LocalSlot {
                    value: val,
                    mutable,
                });
            }
            Op::ImportGlob => {
                let module = self.pop(line, col)?;
                let Value::Dict(map) = module else {
                    return Err(IonError::type_err(
                        ion_str!("use target is not a module"),
                        line,
                        col,
                    ));
                };
                for (name, value) in map {
                    let sym = self.env.intern(&name);
                    self.env.define_sym(sym, value, false);
                }
            }
            Op::GetLocalSlot => {
                let slot = self.locals_base + chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let val = self.locals[slot].value.clone();
                self.stack.push(val);
            }
            Op::SetLocalSlot => {
                let slot = self.locals_base + chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let val = self.pop(line, col)?;
                if !self.locals[slot].mutable {
                    return Err(IonError::runtime(
                        ion_str!("cannot assign to immutable variable"),
                        line,
                        col,
                    ));
                }
                self.locals[slot].value = val.clone();
                self.stack.push(val); // assignment is an expression
            }

            // --- Control flow ---
            Op::Jump => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                self.ip += offset;
            }
            Op::JumpIfFalse => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                if !self.is_top_truthy(line, col)? {
                    self.ip += offset;
                }
            }
            Op::Loop => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                self.ip -= offset;
            }

            // --- Functions ---
            Op::Call => {
                let arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                self.call_function(arg_count, line, col)?;
            }
            Op::CallNamed => {
                let arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                let named_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                // Read named arg metadata: (position, name)
                let mut named_map: Vec<(usize, String)> = Vec::with_capacity(named_count);
                for _ in 0..named_count {
                    let pos = chunk.read_u8(self.ip) as usize;
                    self.ip += 1;
                    let name_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    if let Value::Str(name) = &chunk.constants[name_idx] {
                        named_map.push((pos, name.clone()));
                    }
                }
                self.call_function_named(arg_count, &named_map, line, col)?;
            }
            Op::TailCall => {
                let arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                // Extract func and args for trampoline
                let args_start = self.stack.len() - arg_count;
                let func_idx = args_start - 1;
                let func = self.stack[func_idx].clone();
                let args: Vec<Value> = self.stack[args_start..].to_vec();
                self.stack.truncate(func_idx);
                self.pending_tail_call = Some((func, args));
                return Ok(Some(Value::Unit)); // value is unused; caller checks pending_tail_call
            }
            Op::Return => {
                // Return the top of stack value
                let val = if self.stack.is_empty() {
                    Value::Unit
                } else {
                    self.pop(line, col)?
                };
                return Ok(Some(val));
            }

            // --- Stack ---
            Op::Pop => {
                self.pop(line, col)?;
            }
            Op::Dup => {
                let val = self.peek(line, col)?;
                self.stack.push(val);
            }

            // --- Composite types ---
            Op::BuildList => {
                let count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let start = self.stack.len() - count;
                let items: Vec<Value> = self.stack.drain(start..).collect();
                self.stack.push(Value::List(items));
            }
            Op::BuildTuple => {
                let count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let start = self.stack.len() - count;
                let items: Vec<Value> = self.stack.drain(start..).collect();
                self.stack.push(Value::Tuple(items));
            }
            Op::BuildDict => {
                let count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let mut map = IndexMap::new();
                // Pop count key-value pairs (pushed in order)
                let start = self.stack.len() - count * 2;
                let items: Vec<Value> = self.stack.drain(start..).collect();
                for pair in items.chunks(2) {
                    let key = match &pair[0] {
                        Value::Str(s) => s.clone(),
                        other => other.to_string(),
                    };
                    map.insert(key, pair[1].clone());
                }
                self.stack.push(Value::Dict(map));
            }

            // --- Field/index access ---
            Op::GetField => {
                let field_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let field = self.const_as_str(&chunk.constants[field_idx], line, col)?;
                let obj = self.pop(line, col)?;
                self.stack.push(self.get_field(obj, &field, line, col)?);
            }
            Op::GetIndex => {
                let index = self.pop(line, col)?;
                let obj = self.pop(line, col)?;
                self.stack.push(self.get_index(obj, index, line, col)?);
            }
            Op::SetField => {
                let field_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let field = self.const_as_str(&chunk.constants[field_idx], line, col)?;
                let value = self.pop(line, col)?;
                let obj = self.pop(line, col)?;
                let result = self.set_field(obj, &field, value, line, col)?;
                self.stack.push(result);
            }
            Op::SetIndex => {
                let value = self.pop(line, col)?;
                let index = self.pop(line, col)?;
                let obj = self.pop(line, col)?;
                let result = self.set_index(obj, index, value, line, col)?;
                self.stack.push(result);
            }
            Op::MethodCall => {
                let method_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                let method = self.const_as_str(&chunk.constants[method_idx], line, col)?;
                // Stack: [..., receiver, arg0, arg1, ...]
                let start = self.stack.len() - arg_count;
                let args: Vec<Value> = self.stack.drain(start..).collect();
                let receiver = self.pop(line, col)?;
                let result = self.call_method(receiver, &method, &args, line, col)?;
                self.stack.push(result);
            }

            // --- Closures ---
            Op::Closure => {
                let fn_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let val = chunk.constants[fn_idx].clone();
                // Attach captures from current env
                if let Value::Fn(mut f) = val {
                    f.captures = self.env.capture();
                    self.stack.push(Value::Fn(f));
                } else {
                    self.stack.push(val);
                }
            }

            // --- Option/Result ---
            Op::WrapSome => {
                let val = self.pop(line, col)?;
                self.stack.push(Value::Option(Some(Box::new(val))));
            }
            Op::WrapOk => {
                let val = self.pop(line, col)?;
                self.stack.push(Value::Result(Ok(Box::new(val))));
            }
            Op::WrapErr => {
                let val = self.pop(line, col)?;
                self.stack.push(Value::Result(Err(Box::new(val))));
            }
            Op::Try => {
                let val = self.pop(line, col)?;
                match val {
                    Value::Option(Some(v)) => self.stack.push(*v),
                    Value::Option(None) => {
                        return Err(IonError::propagated_none(line, 0));
                    }
                    Value::Result(Ok(v)) => self.stack.push(*v),
                    Value::Result(Err(e)) => {
                        return Err(IonError::propagated_err(e.to_string(), line, col));
                    }
                    other => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{}",
                                ion_str!("? operator requires Option or Result, got "),
                                other.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }

            // --- Scope ---
            Op::PushScope => {
                self.env.push_scope();
                self.local_frames.push(self.locals.len());
            }
            Op::PopScope => {
                self.env.pop_scope();
                if let Some(base) = self.local_frames.pop() {
                    self.locals.truncate(base);
                }
            }

            // --- String ---
            Op::BuildFString => {
                let count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let start = self.stack.len() - count;
                let parts: Vec<Value> = self.stack.drain(start..).collect();
                let mut s = String::with_capacity(count * 8);
                for part in &parts {
                    use std::fmt::Write;
                    let _ = write!(s, "{}", part);
                }
                self.stack.push(Value::Str(s));
            }

            // --- Pipe ---
            Op::Pipe => {
                let _arg_count = chunk.read_u8(self.ip);
                self.ip += 1;
                // Pipe is handled by the compiler rewriting to Call
                return Err(IonError::runtime(
                    ion_str!("pipe opcode should not be executed directly"),
                    line,
                    col,
                ));
            }

            // --- Pattern matching ---
            Op::MatchBegin => {
                // u8: kind (1=Some, 2=Ok, 3=Err, 4=Tuple, 5=List, 6=HostStruct, 7=HostEnum)
                let kind = chunk.read_u8(self.ip);
                self.ip += 1;
                let val = self.pop(line, col)?;
                let result = match kind {
                    1 => matches!(val, Value::Option(Some(_))),
                    2 => matches!(val, Value::Result(Ok(_))),
                    3 => matches!(val, Value::Result(Err(_))),
                    4 => {
                        let expected_len = chunk.read_u8(self.ip) as usize;
                        self.ip += 1;
                        match &val {
                            Value::Tuple(items) => items.len() == expected_len,
                            _ => false,
                        }
                    }
                    5 => {
                        let min_len = chunk.read_u8(self.ip) as usize;
                        self.ip += 1;
                        let has_rest = chunk.read_u8(self.ip) != 0;
                        self.ip += 1;
                        match &val {
                            Value::List(items) => {
                                if has_rest {
                                    items.len() >= min_len
                                } else {
                                    items.len() == min_len
                                }
                            }
                            _ => false,
                        }
                    }
                    6 => {
                        let type_idx = chunk.read_u16(self.ip) as usize;
                        self.ip += 2;
                        let expected = chunk
                            .constants
                            .get(type_idx)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    ion_str!("type constant index out of bounds"),
                                    line,
                                    col,
                                )
                            })
                            .and_then(|value| self.const_as_str(value, line, col))?;
                        let want = crate::hash::h(&expected);
                        matches!(&val, Value::HostStruct { type_hash, .. } if *type_hash == want)
                    }
                    7 => {
                        let enum_idx = chunk.read_u16(self.ip) as usize;
                        self.ip += 2;
                        let variant_idx = chunk.read_u16(self.ip) as usize;
                        self.ip += 2;
                        let expected_arity = chunk.read_u8(self.ip) as usize;
                        self.ip += 1;
                        let expected_enum = chunk
                            .constants
                            .get(enum_idx)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    ion_str!("enum constant index out of bounds"),
                                    line,
                                    col,
                                )
                            })
                            .and_then(|value| self.const_as_str(value, line, col))?;
                        let expected_variant = chunk
                            .constants
                            .get(variant_idx)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    ion_str!("variant constant index out of bounds"),
                                    line,
                                    col,
                                )
                            })
                            .and_then(|value| self.const_as_str(value, line, col))?;
                        let want_enum = crate::hash::h(&expected_enum);
                        let want_variant = crate::hash::h(&expected_variant);
                        matches!(
                            &val,
                            Value::HostEnum {
                                enum_hash,
                                variant_hash,
                                data,
                            } if *enum_hash == want_enum
                                && *variant_hash == want_variant
                                && data.len() == expected_arity
                        )
                    }
                    _ => false,
                };
                // Push value back (needed for unwrap) and then bool
                self.stack.push(val);
                self.stack.push(Value::Bool(result));
            }
            Op::MatchArm => {
                // u8: kind (1=unwrap Some, 2=unwrap Ok, 3=unwrap Err, 4=get tuple element, 5=get list element, 6=get struct field, 7=get enum data)
                let kind = chunk.read_u8(self.ip);
                self.ip += 1;
                match kind {
                    1 => {
                        // Unwrap Some: pop Option(Some(v)), push v
                        let val = self.pop(line, col)?;
                        match val {
                            Value::Option(Some(v)) => self.stack.push(*v),
                            other => self.stack.push(other),
                        }
                    }
                    2 => {
                        let val = self.pop(line, col)?;
                        match val {
                            Value::Result(Ok(v)) => self.stack.push(*v),
                            other => self.stack.push(other),
                        }
                    }
                    3 => {
                        let val = self.pop(line, col)?;
                        match val {
                            Value::Result(Err(v)) => self.stack.push(*v),
                            other => self.stack.push(other),
                        }
                    }
                    4 | 5 => {
                        // Get tuple/list element: u8 index follows
                        let idx = chunk.read_u8(self.ip) as usize;
                        self.ip += 1;
                        let val = self.peek(line, col)?;
                        match val {
                            Value::Tuple(items) | Value::List(items) => {
                                self.stack
                                    .push(items.get(idx).cloned().unwrap_or(Value::Unit));
                            }
                            _ => self.stack.push(Value::Unit),
                        }
                    }
                    6 => {
                        let field_idx = chunk.read_u16(self.ip) as usize;
                        self.ip += 2;
                        let field = chunk
                            .constants
                            .get(field_idx)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    ion_str!("field constant index out of bounds"),
                                    line,
                                    col,
                                )
                            })
                            .and_then(|value| self.const_as_str(value, line, col))?;
                        let val = self.peek(line, col)?;
                        match val {
                            Value::HostStruct { fields, .. } => {
                                let fh = crate::hash::h(&field);
                                let field_value = fields.get(&fh).cloned();
                                self.stack.push(Value::Option(field_value.map(Box::new)));
                            }
                            _ => self.stack.push(Value::Option(None)),
                        }
                    }
                    7 => {
                        let idx = chunk.read_u8(self.ip) as usize;
                        self.ip += 1;
                        let val = self.peek(line, col)?;
                        match val {
                            Value::HostEnum { data, .. } => {
                                self.stack
                                    .push(data.get(idx).cloned().unwrap_or(Value::Unit));
                            }
                            _ => self.stack.push(Value::Unit),
                        }
                    }
                    _ => {}
                }
            }
            Op::MatchEnd => {
                return Err(IonError::runtime(
                    ion_str!("non-exhaustive match").to_string(),
                    line,
                    col,
                ));
            }

            // --- Range ---
            Op::BuildRange => {
                let inclusive = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let end = self.pop(line, col)?;
                let start = self.pop(line, col)?;
                let s = start.as_int().ok_or_else(|| {
                    IonError::type_err(ion_str!("range start must be int"), line, col)
                })?;
                let e = end.as_int().ok_or_else(|| {
                    IonError::type_err(ion_str!("range end must be int"), line, col)
                })?;
                self.stack.push(Value::Range {
                    start: s,
                    end: e,
                    inclusive,
                });
            }

            // --- Host types ---
            Op::ConstructStruct => {
                let type_name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let raw_count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let type_name = match &chunk.constants[type_name_idx] {
                    Value::Str(s) => s.clone(),
                    _ => return Err(IonError::runtime(ion_str!("invalid type name"), line, col)),
                };
                let has_spread = raw_count & 0x8000 != 0;
                let field_count = raw_count & 0x7FFF;
                let mut fields: IndexMap<u64, Value> = IndexMap::new();
                if has_spread {
                    // Stack: [..., spread_struct, field_name, field_value, ...]
                    // Pop override fields first
                    let override_start = self.stack.len() - field_count * 2;
                    let overrides: Vec<Value> = self.stack.drain(override_start..).collect();
                    // Pop spread struct
                    let spread_val = self.pop(line, col)?;
                    match spread_val {
                        Value::HostStruct { fields: sf, .. } => {
                            for (k, v) in sf {
                                fields.insert(k, v);
                            }
                        }
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("spread in struct constructor requires a struct"),
                                line,
                                col,
                            ));
                        }
                    }
                    // Apply overrides — names from chunk constants (script source) hashed at boundary
                    for pair in overrides.chunks(2) {
                        let fname = match &pair[0] {
                            Value::Str(s) => s.as_str(),
                            _ => {
                                return Err(IonError::runtime(
                                    ion_str!("invalid field name"),
                                    line,
                                    col,
                                ));
                            }
                        };
                        fields.insert(crate::hash::h(fname), pair[1].clone());
                    }
                } else {
                    // No spread: fields are pushed as name, value pairs
                    let start = self.stack.len() - field_count * 2;
                    let items: Vec<Value> = self.stack.drain(start..).collect();
                    for pair in items.chunks(2) {
                        let fname = match &pair[0] {
                            Value::Str(s) => s.as_str(),
                            _ => {
                                return Err(IonError::runtime(
                                    ion_str!("invalid field name"),
                                    line,
                                    col,
                                ));
                            }
                        };
                        fields.insert(crate::hash::h(fname), pair[1].clone());
                    }
                }
                match self.types.construct_struct(&type_name, fields) {
                    Ok(val) => self.stack.push(val),
                    Err(msg) => return Err(IonError::runtime(msg, line, col)),
                }
            }
            Op::ConstructEnum => {
                let enum_name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let variant_name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                let enum_name = match &chunk.constants[enum_name_idx] {
                    Value::Str(s) => s.clone(),
                    _ => return Err(IonError::runtime(ion_str!("invalid enum name"), line, col)),
                };
                let variant_name = match &chunk.constants[variant_name_idx] {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(IonError::runtime(
                            ion_str!("invalid variant name"),
                            line,
                            col,
                        ));
                    }
                };
                let start = self.stack.len() - arg_count;
                let args: Vec<Value> = self.stack.drain(start..).collect();
                match self.types.construct_enum(&enum_name, &variant_name, args) {
                    Ok(val) => self.stack.push(val),
                    Err(msg) => return Err(IonError::runtime(msg, line, col)),
                }
            }

            // --- Comprehensions ---
            Op::IterInit => {
                let val = self.pop(line, col)?;
                let iter: Box<dyn Iterator<Item = Value>> = match val {
                    Value::List(items) => Box::new(items.into_iter()),
                    Value::Set(items) => Box::new(items.into_iter()),
                    Value::Tuple(items) => Box::new(items.into_iter()),
                    Value::Dict(map) => Box::new(
                        map.into_iter()
                            .map(|(k, v)| Value::Tuple(vec![Value::Str(k), v])),
                    ),
                    Value::Str(s) => {
                        let chars: Vec<Value> =
                            s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Box::new(chars.into_iter())
                    }
                    Value::Bytes(bytes) => {
                        let vals: Vec<Value> =
                            bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
                        Box::new(vals.into_iter())
                    }
                    Value::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        if inclusive {
                            Box::new((start..=end).map(Value::Int))
                        } else {
                            // Use RangeInclusive with adjusted end to avoid two different types
                            if end > start {
                                Box::new((start..=(end - 1)).map(Value::Int))
                            } else {
                                Box::new(std::iter::empty())
                            }
                        }
                    }
                    other => {
                        return Err(IonError::type_err(
                            format!("{}{}", ion_str!("cannot iterate over "), other.type_name()),
                            line,
                            col,
                        ));
                    }
                };
                self.iterators.push(iter);
                // Push a placeholder on stack so IterNext has something
                self.stack.push(Value::Unit);
            }
            Op::IterNext => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                // Pop the previous iteration value/placeholder
                self.pop(line, col)?;
                let iter = self
                    .iterators
                    .last_mut()
                    .ok_or_else(|| IonError::runtime(ion_str!("no active iterator"), line, col))?;
                match iter.next() {
                    Some(val) => {
                        self.stack.push(val);
                    }
                    None => {
                        self.iterators.pop();
                        self.stack.push(Value::Unit); // placeholder for the pop after loop
                        self.ip += offset;
                    }
                }
            }
            Op::IterDrop => {
                self.iterators.pop();
            }
            Op::ListAppend => {
                // Stack: [..., list, iter_placeholder, ..., item]
                // Pop item, find the list deeper in the stack, append to it
                let item = self.pop(line, col)?;
                // Find the list — it's below the iterator placeholder
                // The list is at position: stack.len() - 1 (after popping item) minus
                // however many scope vars are between. Actually, the list is always
                // 2 below the current top: [..., list, iter_placeholder, ...]
                // But with scopes, it's simpler to find the last List on the stack.
                // Actually: stack layout is [..., list, Unit(iter_placeholder), ...]
                // Let's find the list by scanning backwards
                let mut found = false;
                for i in (0..self.stack.len()).rev() {
                    if let Value::List(_) = &self.stack[i] {
                        if let Value::List(ref mut items) = self.stack[i] {
                            items.push(item.clone());
                        }
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(IonError::runtime(
                        ion_str!("ListAppend: no list on stack"),
                        line,
                        col,
                    ));
                }
            }
            Op::ListExtend => {
                // Stack: [..., target_list, source_list]
                let source = self.pop(line, col)?;
                match source {
                    Value::List(other) => {
                        let mut found = false;
                        for i in (0..self.stack.len()).rev() {
                            if let Value::List(ref mut items) = self.stack[i] {
                                items.extend(other);
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(IonError::runtime(
                                ion_str!("ListExtend: no list on stack"),
                                line,
                                col,
                            ));
                        }
                    }
                    other => {
                        return Err(IonError::type_err(
                            format!(
                                "{}{}",
                                ion_str!("spread requires a list, got "),
                                other.type_name()
                            ),
                            line,
                            col,
                        ));
                    }
                }
            }
            Op::DictInsert => {
                // Stack: [..., dict, iter_placeholder, ..., key, value]
                let value = self.pop(line, col)?;
                let key = self.pop(line, col)?;
                let key_str = match key {
                    Value::Str(s) => s,
                    other => other.to_string(),
                };
                let mut found = false;
                for i in (0..self.stack.len()).rev() {
                    if let Value::Dict(_) = &self.stack[i] {
                        if let Value::Dict(ref mut map) = self.stack[i] {
                            map.insert(key_str.clone(), value.clone());
                        }
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(IonError::runtime(
                        ion_str!("DictInsert: no dict on stack"),
                        line,
                        col,
                    ));
                }
            }
            Op::DictMerge => {
                // Stack: [..., target_dict, source_dict]
                let source = self.pop(line, col)?;
                match source {
                    Value::Dict(other) => {
                        // Find the target dict on stack
                        let mut found = false;
                        for i in (0..self.stack.len()).rev() {
                            if let Value::Dict(ref mut map) = self.stack[i] {
                                for (k, v) in other {
                                    map.insert(k, v);
                                }
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(IonError::runtime(
                                ion_str!("DictMerge: no dict on stack"),
                                line,
                                col,
                            ));
                        }
                    }
                    _ => {
                        return Err(IonError::type_err(
                            ion_str!("spread requires a dict").to_string(),
                            line,
                            col,
                        ));
                    }
                }
            }

            // --- Slice ---
            Op::CheckType => {
                let idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let type_name = match &chunk.constants[idx] {
                    Value::Str(s) => s.clone(),
                    _ => unreachable!(),
                };
                // Peek at TOS without popping
                let val = self.stack.last().ok_or_else(|| {
                    IonError::runtime(ion_str!("CheckType: empty stack"), line, col)
                })?;
                let ok = match crate::hash::h(type_name.as_str()) {
                    h if h == crate::h!("int") => matches!(val, Value::Int(_)),
                    h if h == crate::h!("float") => matches!(val, Value::Float(_)),
                    h if h == crate::h!("bool") => matches!(val, Value::Bool(_)),
                    h if h == crate::h!("string") => matches!(val, Value::Str(_)),
                    h if h == crate::h!("bytes") => matches!(val, Value::Bytes(_)),
                    h if h == crate::h!("list") => matches!(val, Value::List(_)),
                    h if h == crate::h!("dict") => matches!(val, Value::Dict(_)),
                    h if h == crate::h!("tuple") => matches!(val, Value::Tuple(_)),
                    h if h == crate::h!("set") => matches!(val, Value::Set(_)),
                    h if h == crate::h!("fn") => match val {
                        Value::Fn(_) | Value::BuiltinFn { .. } | Value::BuiltinClosure { .. } => {
                            true
                        }
                        #[cfg(feature = "async-runtime")]
                        Value::AsyncBuiltinClosure { .. } => true,
                        _ => false,
                    },
                    h if h == crate::h!("cell") => matches!(val, Value::Cell(_)),
                    h if h == crate::h!("any") => true,
                    _ if crate::hash::starts_with_option_type(&type_name) => {
                        matches!(val, Value::Option(_))
                    }
                    _ if crate::hash::starts_with_result_type(&type_name) => {
                        matches!(val, Value::Result(_))
                    }
                    _ if crate::hash::starts_with_list_generic_type(&type_name) => {
                        matches!(val, Value::List(_))
                    }
                    _ if crate::hash::starts_with_dict_generic_type(&type_name) => {
                        matches!(val, Value::Dict(_))
                    }
                    _ => true,
                };
                if !ok {
                    return Err(IonError::type_err(
                        format!(
                            "{}{}, {}{}",
                            ion_str!("type mismatch: expected "),
                            type_name,
                            ion_str!("got "),
                            val.type_name()
                        ),
                        line,
                        col,
                    ));
                }
            }
            Op::Slice => {
                let flags = chunk.read_u8(self.ip);
                self.ip += 1;
                let has_start = flags & 1 != 0;
                let has_end = flags & 2 != 0;
                let inclusive = flags & 4 != 0;
                let end_val = if has_end {
                    Some(self.pop(line, col)?)
                } else {
                    None
                };
                let start_val = if has_start {
                    Some(self.pop(line, col)?)
                } else {
                    None
                };
                let obj = self.pop(line, col)?;
                let result = self.slice_access(obj, start_val, end_val, inclusive, line, col)?;
                self.stack.push(result);
            }

            Op::SpawnCall => {
                let _arg_count = chunk.read_u8(self.ip);
                self.ip += 1;
                return Err(IonError::runtime(
                    ion_str!(
                        "spawn requires the pollable async runtime; it cannot run in the synchronous VM"
                    ),
                    line,
                    col,
                ));
            }
            Op::SpawnCallNamed => {
                let _arg_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1;
                let named_count = chunk.read_u8(self.ip) as usize;
                self.ip += 1 + named_count * 3;
                return Err(IonError::runtime(
                    ion_str!(
                        "spawn requires the pollable async runtime; it cannot run in the synchronous VM"
                    ),
                    line,
                    col,
                ));
            }
            Op::AwaitTask => {
                return Err(IonError::runtime(
                    ion_str!(
                        "await requires the pollable async runtime; it cannot run in the synchronous VM"
                    ),
                    line,
                    col,
                ));
            }
            Op::SelectTasks => {
                let _branch_count = chunk.read_u8(self.ip);
                self.ip += 1;
                return Err(IonError::runtime(
                    ion_str!(
                        "select requires the pollable async runtime; it cannot run in the synchronous VM"
                    ),
                    line,
                    col,
                ));
            }

            // --- Print ---
            Op::Print => {
                let newline = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let val = self.pop(line, col)?;
                if newline {
                    let text = format!("{}\n", val);
                    self.output
                        .write(OutputStream::Stdout, &text)
                        .map_err(|e| IonError::runtime(e, line, col))?;
                } else {
                    let text = val.to_string();
                    self.output
                        .write(OutputStream::Stdout, &text)
                        .map_err(|e| IonError::runtime(e, line, col))?;
                }
                self.stack.push(Value::Unit);
            }

            Op::TryBegin | Op::TryEnd => {
                // Handled in run_chunk before dispatch
                unreachable!()
            }
        }
        Ok(None)
    }

    // ---- Helpers ----

    fn decode_op(&self, byte: u8, line: usize, col: usize) -> Result<Op, IonError> {
        if byte > Op::Print as u8 {
            return Err(IonError::runtime(
                format!("{}{}", ion_str!("invalid opcode: "), byte),
                line,
                col,
            ));
        }
        // SAFETY: Op is repr(u8) and we checked the range
        Ok(unsafe { std::mem::transmute::<u8, crate::bytecode::Op>(byte) })
    }

    fn slice_access(
        &self,
        obj: Value,
        start: Option<Value>,
        end: Option<Value>,
        inclusive: bool,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        let get_idx = |v: Option<Value>, default: i64| -> Result<i64, IonError> {
            match v {
                Some(Value::Int(n)) => Ok(n),
                None => Ok(default),
                Some(other) => Err(IonError::type_err(
                    format!(
                        "{}{}",
                        ion_str!("slice index must be int, got "),
                        other.type_name()
                    ),
                    line,
                    col,
                )),
            }
        };
        match &obj {
            Value::List(items) => {
                let len = items.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive {
                    (e_raw + 1).max(0).min(len) as usize
                } else {
                    e_raw.max(0).min(len) as usize
                };
                Ok(Value::List(items[s..e].to_vec()))
            }
            Value::Str(string) => {
                let chars: Vec<char> = string.chars().collect();
                let len = chars.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive {
                    (e_raw + 1).max(0).min(len) as usize
                } else {
                    e_raw.max(0).min(len) as usize
                };
                Ok(Value::Str(chars[s..e].iter().collect()))
            }
            Value::Bytes(bytes) => {
                let len = bytes.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive {
                    (e_raw + 1).max(0).min(len) as usize
                } else {
                    e_raw.max(0).min(len) as usize
                };
                Ok(Value::Bytes(bytes[s..e].to_vec()))
            }
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot slice "), obj.type_name()),
                line,
                col,
            )),
        }
    }

    fn pop(&mut self, line: usize, col: usize) -> Result<Value, IonError> {
        self.stack
            .pop()
            .ok_or_else(|| IonError::runtime(ion_str!("stack underflow"), line, col))
    }

    fn peek(&self, line: usize, col: usize) -> Result<Value, IonError> {
        self.stack
            .last()
            .cloned()
            .ok_or_else(|| IonError::runtime(ion_str!("stack underflow (peek)"), line, col))
    }

    /// Check if the top of stack is truthy without cloning.
    fn is_top_truthy(&self, line: usize, col: usize) -> Result<bool, IonError> {
        self.stack
            .last()
            .map(|v| v.is_truthy())
            .ok_or_else(|| IonError::runtime(ion_str!("stack underflow (peek)"), line, col))
    }

    fn const_as_str(&self, val: &Value, line: usize, col: usize) -> Result<String, IonError> {
        match val {
            Value::Str(s) => Ok(s.clone()),
            _ => Err(IonError::runtime(
                ion_str!("expected string constant"),
                line,
                col,
            )),
        }
    }

    /// Resolve a constant pool string to a Symbol, interning it in the env's string pool.
    fn const_to_sym(
        &mut self,
        val: &Value,
        line: usize,
        col: usize,
    ) -> Result<crate::intern::Symbol, IonError> {
        match val {
            Value::Str(s) => Ok(self.env.intern(s)),
            _ => Err(IonError::runtime(
                ion_str!("expected string constant"),
                line,
                col,
            )),
        }
    }

    // ---- Arithmetic ----

    fn op_add(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x + y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x + y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 + y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x + *y as f64)),
            (Value::Str(x), Value::Str(y)) => {
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(x);
                s.push_str(y);
                Ok(Value::Str(s))
            }
            (Value::List(x), Value::List(y)) => {
                let mut r = x.clone();
                r.extend(y.clone());
                Ok(Value::List(r))
            }
            (Value::Bytes(x), Value::Bytes(y)) => {
                let mut r = x.clone();
                r.extend(y);
                Ok(Value::Bytes(r))
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{} and {}",
                    ion_str!("cannot add "),
                    a.type_name(),
                    b.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn op_sub(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x - y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x - y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 - y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x - *y as f64)),
            _ => Err(IonError::type_err(
                format!(
                    "{}{} from {}",
                    ion_str!("cannot subtract "),
                    b.type_name(),
                    a.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn op_mul(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x * y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x * y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 * y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x * *y as f64)),
            (Value::Str(s), Value::Int(n)) | (Value::Int(n), Value::Str(s)) => {
                Ok(Value::Str(s.repeat(*n as usize)))
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{} and {}",
                    ion_str!("cannot multiply "),
                    a.type_name(),
                    b.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn op_div(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => {
                Err(IonError::runtime(ion_str!("division by zero"), line, col))
            }
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x / y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x / y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 / y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x / *y as f64)),
            _ => Err(IonError::type_err(
                format!(
                    "{}{} by {}",
                    ion_str!("cannot divide "),
                    a.type_name(),
                    b.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn op_mod(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => {
                Err(IonError::runtime(ion_str!("modulo by zero"), line, col))
            }
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x % y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 % y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x % *y as f64)),
            _ => Err(IonError::type_err(
                format!(
                    "{}{} by {}",
                    ion_str!("cannot modulo "),
                    a.type_name(),
                    b.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn op_neg(&self, val: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match val {
            Value::Int(n) => Ok(Value::Int(-n)),
            Value::Float(n) => Ok(Value::Float(-n)),
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot negate "), val.type_name()),
                line,
                col,
            )),
        }
    }

    fn compare_lt(&self, a: &Value, b: &Value, line: usize, col: usize) -> Result<bool, IonError> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x < y),
            (Value::Float(x), Value::Float(y)) => Ok(x < y),
            (Value::Int(x), Value::Float(y)) => Ok((*x as f64) < *y),
            (Value::Float(x), Value::Int(y)) => Ok(*x < (*y as f64)),
            (Value::Str(x), Value::Str(y)) => Ok(x < y),
            _ => Err(IonError::type_err(
                format!(
                    "{}{} and {}",
                    ion_str!("cannot compare "),
                    a.type_name(),
                    b.type_name()
                ),
                line,
                col,
            )),
        }
    }

    // ---- Field/Index access ----

    fn get_field(
        &self,
        obj: Value,
        field: &str,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match &obj {
            Value::Dict(map) => Ok(match map.get(field) {
                Some(v) => v.clone(),
                None => Value::Option(None),
            }),
            Value::Module(table) => {
                let fh = crate::hash::h(field);
                table.items.get(&fh).cloned().ok_or_else(|| {
                    IonError::runtime(
                        format!(
                            "{}{}{}",
                            ion_str!("'"),
                            field,
                            ion_str!("' not found in module")
                        ),
                        line,
                        col,
                    )
                })
            }
            Value::HostStruct { fields, .. } => {
                let fh = crate::hash::h(field);
                fields.get(&fh).cloned().ok_or_else(|| {
                    IonError::runtime(
                        format!(
                            "{}{}{}",
                            ion_str!("field '"),
                            field,
                            ion_str!("' not found")
                        ),
                        line,
                        col,
                    )
                })
            }
            Value::List(items) => match crate::hash::h(field) {
                h if h == crate::h!("len") => Ok(Value::Int(items.len() as i64)),
                _ => Err(IonError::runtime(
                    format!(
                        "{}{}{}",
                        ion_str!("list has no field '"),
                        field,
                        ion_str!("'")
                    ),
                    line,
                    col,
                )),
            },
            Value::Str(s) => match crate::hash::h(field) {
                h if h == crate::h!("len") => Ok(Value::Int(s.len() as i64)),
                _ => Err(IonError::runtime(
                    format!(
                        "{}{}{}",
                        ion_str!("string has no field '"),
                        field,
                        ion_str!("'")
                    ),
                    line,
                    col,
                )),
            },
            Value::Tuple(items) => match crate::hash::h(field) {
                h if h == crate::h!("len") => Ok(Value::Int(items.len() as i64)),
                _ => Err(IonError::runtime(
                    format!(
                        "{}{}{}",
                        ion_str!("tuple has no field '"),
                        field,
                        ion_str!("'")
                    ),
                    line,
                    col,
                )),
            },
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("cannot access field '"),
                    field,
                    ion_str!("' on "),
                    obj.type_name()
                ),
                line,
                col,
            )),
        }
    }

    fn get_index(
        &self,
        obj: Value,
        index: Value,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match (&obj, &index) {
            (Value::List(items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(idx).cloned().ok_or_else(|| {
                    IonError::runtime(
                        format!("{}{}{}", ion_str!("index "), i, ion_str!(" out of range")),
                        line,
                        col,
                    )
                })
            }
            (Value::Tuple(items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(idx).cloned().ok_or_else(|| {
                    IonError::runtime(
                        format!("{}{}{}", ion_str!("index "), i, ion_str!(" out of range")),
                        line,
                        col,
                    )
                })
            }
            (Value::Dict(map), Value::Str(key)) => Ok(match map.get(key) {
                Some(v) => v.clone(),
                None => Value::Option(None),
            }),
            (Value::Str(s), Value::Int(i)) => {
                let char_count = s.chars().count() as i64;
                let idx = if *i < 0 { char_count + i } else { *i } as usize;
                s.chars()
                    .nth(idx)
                    .map(|c| Value::Str(c.to_string()))
                    .ok_or_else(|| {
                        IonError::runtime(
                            format!("{}{}{}", ion_str!("index "), i, ion_str!(" out of range")),
                            line,
                            col,
                        )
                    })
            }
            (Value::Bytes(bytes), Value::Int(i)) => {
                let idx = if *i < 0 { bytes.len() as i64 + i } else { *i } as usize;
                bytes
                    .get(idx)
                    .map(|&b| Value::Int(b as i64))
                    .ok_or_else(|| {
                        IonError::runtime(
                            format!("{}{}{}", ion_str!("index "), i, ion_str!(" out of range")),
                            line,
                            col,
                        )
                    })
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    ion_str!("cannot index "),
                    obj.type_name(),
                    ion_str!(" with "),
                    index.type_name()
                ),
                line,
                col,
            )),
        }
    }

    /// Set index on a container, returning the modified container.
    fn set_index(
        &self,
        obj: Value,
        index: Value,
        value: Value,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match (obj, &index) {
            (Value::List(mut items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                if idx >= items.len() {
                    return Err(IonError::runtime(
                        format!("{}{}{}", ion_str!("index "), i, ion_str!(" out of range")),
                        line,
                        col,
                    ));
                }
                items[idx] = value;
                Ok(Value::List(items))
            }
            (Value::Dict(mut map), Value::Str(key)) => {
                map.insert(key.clone(), value);
                Ok(Value::Dict(map))
            }
            (obj, _) => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot set index on "), obj.type_name()),
                line,
                col,
            )),
        }
    }

    /// Set field on an object, returning the modified object.
    fn set_field(
        &self,
        obj: Value,
        field: &str,
        value: Value,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match obj {
            Value::Dict(mut map) => {
                map.insert(field.to_string(), value);
                Ok(Value::Dict(map))
            }
            Value::HostStruct {
                type_hash,
                mut fields,
            } => {
                let fh = crate::hash::h(field);
                if fields.contains_key(&fh) {
                    fields.insert(fh, value);
                    Ok(Value::HostStruct { type_hash, fields })
                } else {
                    Err(IonError::runtime(
                        format!(
                            "{}{}{}",
                            ion_str!("field '"),
                            field,
                            ion_str!("' not found on host struct"),
                        ),
                        line,
                        col,
                    ))
                }
            }
            _ => Err(IonError::type_err(
                format!("{}{}", ion_str!("cannot set field on "), obj.type_name()),
                line,
                col,
            )),
        }
    }

    // ---- Method calls ----

    fn call_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        // Universal methods available on all types
        if crate::hash::is_to_string_name(method) {
            return Ok(Value::Str(format!("{}", receiver)));
        }
        // Handle closure-based methods that need &mut self for invoke_value
        match (&receiver, crate::hash::h(method)) {
            // List closure methods
            (Value::List(items), h) if h == crate::h!("map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("map requires a function argument"), line, col)
                })?;
                let mut result = Vec::new();
                for item in items {
                    result.push(self.invoke_value(func, std::slice::from_ref(item), line, col)?);
                }
                return Ok(Value::List(result));
            }
            (Value::List(items), h) if h == crate::h!("filter") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("filter requires a function argument"), line, col)
                })?;
                let mut result = Vec::new();
                for item in items {
                    let keep = self.invoke_value(func, std::slice::from_ref(item), line, col)?;
                    if keep.is_truthy() {
                        result.push(item.clone());
                    }
                }
                return Ok(Value::List(result));
            }
            (Value::List(items), h) if h == crate::h!("fold") => {
                let init = args.first().cloned().unwrap_or(Value::Unit);
                let func = args.get(1).ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("fold requires an initial value and a function"),
                        line,
                        col,
                    )
                })?;
                let mut acc = init;
                for item in items {
                    acc = self.invoke_value(func, &[acc, item.clone()], line, col)?;
                }
                return Ok(acc);
            }
            (Value::List(items), h) if h == crate::h!("reduce") => {
                if items.is_empty() {
                    return Err(IonError::runtime(
                        ion_str!("reduce on empty list"),
                        line,
                        col,
                    ));
                }
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("reduce requires a function argument"), line, col)
                })?;
                let mut acc = items[0].clone();
                for item in items.iter().skip(1) {
                    acc = self.invoke_value(func, &[acc, item.clone()], line, col)?;
                }
                return Ok(acc);
            }
            (Value::List(items), h) if h == crate::h!("flat_map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("flat_map requires a function argument"), line, col)
                })?;
                let mut result = Vec::new();
                for item in items {
                    let mapped = self.invoke_value(func, std::slice::from_ref(item), line, col)?;
                    match mapped {
                        Value::List(sub) => result.extend(sub),
                        other => result.push(other),
                    }
                }
                return Ok(Value::List(result));
            }
            (Value::List(items), h) if h == crate::h!("any") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("any requires a function argument"), line, col)
                })?;
                for item in items {
                    if self
                        .invoke_value(func, std::slice::from_ref(item), line, col)?
                        .is_truthy()
                    {
                        return Ok(Value::Bool(true));
                    }
                }
                return Ok(Value::Bool(false));
            }
            (Value::List(items), h) if h == crate::h!("all") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("all requires a function argument"), line, col)
                })?;
                for item in items {
                    if !self
                        .invoke_value(func, std::slice::from_ref(item), line, col)?
                        .is_truthy()
                    {
                        return Ok(Value::Bool(false));
                    }
                }
                return Ok(Value::Bool(true));
            }
            (Value::List(items), h) if h == crate::h!("sort_by") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("sort_by requires a function argument"), line, col)
                })?;
                let mut result = items.to_vec();
                let mut err: Option<IonError> = None;
                let func_clone = func.clone();
                result.sort_by(|a, b| {
                    if err.is_some() {
                        return std::cmp::Ordering::Equal;
                    }
                    match self.invoke_value(&func_clone, &[a.clone(), b.clone()], line, col) {
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
                            err = Some(IonError::type_err(
                                ion_str!("sort_by function must return int"),
                                line,
                                col,
                            ));
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
                return Ok(Value::List(result));
            }

            // Range closure methods — materialize then delegate to list logic
            (
                Value::Range {
                    start,
                    end,
                    inclusive,
                },
                h,
            ) if h == crate::h!("map")
                || h == crate::h!("filter")
                || h == crate::h!("fold")
                || h == crate::h!("reduce")
                || h == crate::h!("flat_map")
                || h == crate::h!("any")
                || h == crate::h!("all")
                || h == crate::h!("sort_by") =>
            {
                let items = Value::range_to_list(*start, *end, *inclusive);
                let list_receiver = Value::List(items);
                return self.call_method(list_receiver, method, args, line, col);
            }

            // Dict closure methods
            (Value::Dict(map), h) if h == crate::h!("map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("map requires a function argument"), line, col)
                })?;
                let mut result = indexmap::IndexMap::new();
                for (k, v) in map {
                    let mapped =
                        self.invoke_value(func, &[Value::Str(k.clone()), v.clone()], line, col)?;
                    result.insert(k.clone(), mapped);
                }
                return Ok(Value::Dict(result));
            }
            (Value::Dict(map), h) if h == crate::h!("filter") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("filter requires a function argument"), line, col)
                })?;
                let mut result = indexmap::IndexMap::new();
                for (k, v) in map {
                    let keep =
                        self.invoke_value(func, &[Value::Str(k.clone()), v.clone()], line, col)?;
                    if keep.is_truthy() {
                        result.insert(k.clone(), v.clone());
                    }
                }
                return Ok(Value::Dict(result));
            }

            // Option closure methods
            (Value::Option(opt), h) if h == crate::h!("map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("map requires a function argument"), line, col)
                })?;
                return match opt {
                    Some(v) => {
                        let result = self.invoke_value(func, &[*v.clone()], line, col)?;
                        Ok(Value::Option(Some(Box::new(result))))
                    }
                    None => Ok(Value::Option(None)),
                };
            }
            (Value::Option(opt), h) if h == crate::h!("and_then") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("and_then requires a function argument"), line, col)
                })?;
                return match opt {
                    Some(v) => self.invoke_value(func, &[*v.clone()], line, col),
                    None => Ok(Value::Option(None)),
                };
            }
            (Value::Option(opt), h) if h == crate::h!("or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("or_else requires a function argument"), line, col)
                })?;
                return match opt {
                    Some(v) => Ok(Value::Option(Some(v.clone()))),
                    None => self.invoke_value(func, &[], line, col),
                };
            }
            (Value::Option(opt), h) if h == crate::h!("unwrap_or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("unwrap_or_else requires a function argument"),
                        line,
                        col,
                    )
                })?;
                return match opt {
                    Some(v) => Ok(*v.clone()),
                    None => self.invoke_value(func, &[], line, col),
                };
            }

            // Result closure methods
            (Value::Result(res), h) if h == crate::h!("map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("map requires a function argument"), line, col)
                })?;
                return match res {
                    Ok(v) => {
                        let result = self.invoke_value(func, &[*v.clone()], line, col)?;
                        Ok(Value::Result(Ok(Box::new(result))))
                    }
                    Err(e) => Ok(Value::Result(Err(e.clone()))),
                };
            }
            (Value::Result(res), h) if h == crate::h!("map_err") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("map_err requires a function argument"), line, col)
                })?;
                return match res {
                    Ok(v) => Ok(Value::Result(Ok(v.clone()))),
                    Err(e) => {
                        let result = self.invoke_value(func, &[*e.clone()], line, col)?;
                        Ok(Value::Result(Err(Box::new(result))))
                    }
                };
            }
            (Value::Result(res), h) if h == crate::h!("and_then") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("and_then requires a function argument"), line, col)
                })?;
                return match res {
                    Ok(v) => self.invoke_value(func, &[*v.clone()], line, col),
                    Err(e) => Ok(Value::Result(Err(e.clone()))),
                };
            }
            (Value::Result(res), h) if h == crate::h!("or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(ion_str!("or_else requires a function argument"), line, col)
                })?;
                return match res {
                    Ok(v) => Ok(Value::Result(Ok(v.clone()))),
                    Err(e) => self.invoke_value(func, &[*e.clone()], line, col),
                };
            }
            (Value::Result(res), h) if h == crate::h!("unwrap_or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("unwrap_or_else requires a function argument"),
                        line,
                        col,
                    )
                })?;
                return match res {
                    Ok(v) => Ok(*v.clone()),
                    Err(e) => self.invoke_value(func, &[*e.clone()], line, col),
                };
            }

            // Cell closure methods
            (Value::Cell(cell), h) if h == crate::h!("update") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime(
                        ion_str!("cell.update() requires a function argument"),
                        line,
                        col,
                    )
                })?;
                let current = { cell.lock().unwrap().clone() };
                let new_val = self.invoke_value(func, &[current], line, col)?;
                let mut inner = cell.lock().unwrap();
                *inner = new_val.clone();
                return Ok(new_val);
            }

            _ => {}
        }

        // Non-closure methods
        match &receiver {
            Value::List(items) => self.list_method(items, method, args, line, col),
            Value::Tuple(items) => self.tuple_method(items, method, args, line, col),
            Value::Str(s) => self.str_method(s, method, args, line, col),
            Value::Dict(map) => self.dict_method(map, method, args, line, col),
            Value::Bytes(b) => self.bytes_method(b, method, args, line, col),
            Value::Set(items) => self.set_method(items, method, args, line, col),
            Value::Option(_) => self.option_method(&receiver, method, args, line, col),
            Value::Result(_) => self.result_method(&receiver, method, args, line, col),
            Value::Range {
                start,
                end,
                inclusive,
            } => match crate::hash::h(method) {
                h if h == crate::h!("len") => {
                    Ok(Value::Int(Value::range_len(*start, *end, *inclusive)))
                }
                h if h == crate::h!("contains") => {
                    let val = args[0].as_int().ok_or_else(|| {
                        IonError::type_err(ion_str!("range.contains requires int"), line, col)
                    })?;
                    let in_range = if *inclusive {
                        val >= *start && val <= *end
                    } else {
                        val >= *start && val < *end
                    };
                    Ok(Value::Bool(in_range))
                }
                h if h == crate::h!("to_list") => {
                    Ok(Value::List(Value::range_to_list(*start, *end, *inclusive)))
                }
                _ => {
                    let items = Value::range_to_list(*start, *end, *inclusive);
                    self.list_method(&items, method, args, line, col)
                }
            },
            Value::Cell(cell) => match crate::hash::h(method) {
                h if h == crate::h!("get") => Ok(cell.lock().unwrap().clone()),
                h if h == crate::h!("set") => {
                    if let Some(val) = args.first() {
                        let mut inner = cell.lock().unwrap();
                        *inner = val.clone();
                        Ok(Value::Unit)
                    } else {
                        Err(IonError::runtime(
                            ion_str!("cell.set() requires 1 argument"),
                            line,
                            col,
                        ))
                    }
                }
                _ => Err(IonError::type_err(
                    format!(
                        "{}{}{}",
                        ion_str!("no method '"),
                        method,
                        ion_str!("' on cell")
                    ),
                    line,
                    col,
                )),
            },
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}{}",
                    receiver.type_name(),
                    ion_str!(" has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn list_method(
        &self,
        items: &[Value],
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::hash::h(method) {
            h if h == crate::h!("len") => Ok(Value::Int(items.len() as i64)),
            h if h == crate::h!("push") => {
                let mut new = items.to_vec();
                for a in args {
                    new.push(a.clone());
                }
                Ok(Value::List(new))
            }
            h if h == crate::h!("pop") => {
                if items.is_empty() {
                    Ok(Value::Tuple(vec![Value::List(vec![]), Value::Option(None)]))
                } else {
                    let mut new = items.to_vec();
                    let popped = new.pop().unwrap();
                    Ok(Value::Tuple(vec![
                        Value::List(new),
                        Value::Option(Some(Box::new(popped))),
                    ]))
                }
            }
            h if h == crate::h!("contains") => Ok(Value::Bool(
                args.first().map(|a| items.contains(a)).unwrap_or(false),
            )),
            h if h == crate::h!("is_empty") => Ok(Value::Bool(items.is_empty())),
            h if h == crate::h!("reverse") => {
                let mut new = items.to_vec();
                new.reverse();
                Ok(Value::List(new))
            }
            h if h == crate::h!("join") => {
                let sep = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let s: String = items
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(sep);
                Ok(Value::Str(s))
            }
            h if h == crate::h!("enumerate") => {
                let pairs: Vec<Value> = items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                    .collect();
                Ok(Value::List(pairs))
            }
            h if h == crate::h!("first") => Ok(match items.first() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            h if h == crate::h!("last") => Ok(match items.last() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            h if h == crate::h!("sort") => {
                if !items.is_empty() {
                    let first_type = std::mem::discriminant(&items[0]);
                    for item in items.iter().skip(1) {
                        if std::mem::discriminant(item) != first_type {
                            return Err(IonError::type_err(
                                ion_str!("sort() requires all elements to be the same type"),
                                line,
                                col,
                            ));
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
            h if h == crate::h!("flatten") => {
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
            h if h == crate::h!("zip") => {
                if let Some(Value::List(other)) = args.first() {
                    let result: Vec<Value> = items
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| Value::Tuple(vec![a.clone(), b.clone()]))
                        .collect();
                    Ok(Value::List(result))
                } else {
                    Err(IonError::type_err(
                        ion_str!("zip requires a list argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("index") => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err(ion_str!("index requires an argument"), line, col)
                })?;
                Ok(match items.iter().position(|v| v == target) {
                    Some(i) => Value::Option(Some(Box::new(Value::Int(i as i64)))),
                    None => Value::Option(None),
                })
            }
            h if h == crate::h!("count") => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err(ion_str!("count requires an argument"), line, col)
                })?;
                Ok(Value::Int(
                    items.iter().filter(|v| *v == target).count() as i64
                ))
            }
            h if h == crate::h!("slice") => {
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
            h if h == crate::h!("dedup") => {
                let mut result: Vec<Value> = Vec::new();
                for item in items {
                    if result.last() != Some(item) {
                        result.push(item.clone());
                    }
                }
                Ok(Value::List(result))
            }
            h if h == crate::h!("unique") => {
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
            h if h == crate::h!("min") => {
                if items.is_empty() {
                    return Ok(Value::Option(None));
                }
                let mut min = &items[0];
                for item in items.iter().skip(1) {
                    match (min, item) {
                        (Value::Int(a), Value::Int(b)) if b < a => min = item,
                        (Value::Float(a), Value::Float(b)) if b < a => min = item,
                        (Value::Str(a), Value::Str(b)) if b < a => min = item,
                        (Value::Int(_), Value::Int(_))
                        | (Value::Float(_), Value::Float(_))
                        | (Value::Str(_), Value::Str(_)) => {}
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("min() requires homogeneous comparable elements"),
                                line,
                                col,
                            ));
                        }
                    }
                }
                Ok(Value::Option(Some(Box::new(min.clone()))))
            }
            h if h == crate::h!("max") => {
                if items.is_empty() {
                    return Ok(Value::Option(None));
                }
                let mut max = &items[0];
                for item in items.iter().skip(1) {
                    match (max, item) {
                        (Value::Int(a), Value::Int(b)) if b > a => max = item,
                        (Value::Float(a), Value::Float(b)) if b > a => max = item,
                        (Value::Str(a), Value::Str(b)) if b > a => max = item,
                        (Value::Int(_), Value::Int(_))
                        | (Value::Float(_), Value::Float(_))
                        | (Value::Str(_), Value::Str(_)) => {}
                        _ => {
                            return Err(IonError::type_err(
                                ion_str!("max() requires homogeneous comparable elements"),
                                line,
                                col,
                            ));
                        }
                    }
                }
                Ok(Value::Option(Some(Box::new(max.clone()))))
            }
            h if h == crate::h!("sum") => {
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
                                ion_str!("sum() requires numeric elements"),
                                line,
                                col,
                            ));
                        }
                    }
                }
                if has_float {
                    Ok(Value::Float(float_sum + int_sum as f64))
                } else {
                    Ok(Value::Int(int_sum))
                }
            }
            h if h == crate::h!("window") => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(ion_str!("window requires int argument"), line, col)
                })? as usize;
                if n == 0 {
                    return Err(IonError::runtime(
                        ion_str!("window size must be > 0"),
                        line,
                        col,
                    ));
                }
                let result: Vec<Value> =
                    items.windows(n).map(|w| Value::List(w.to_vec())).collect();
                Ok(Value::List(result))
            }
            h if h == crate::h!("chunk") => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(ion_str!("chunk requires int argument"), line, col)
                })? as usize;
                if n == 0 {
                    return Err(IonError::type_err(
                        ion_str!("chunk size must be > 0"),
                        line,
                        col,
                    ));
                }
                let result: Vec<Value> = items.chunks(n).map(|c| Value::List(c.to_vec())).collect();
                Ok(Value::List(result))
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("list has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn set_method(
        &self,
        items: &[Value],
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::hash::h(method) {
            h if h == crate::h!("len") => Ok(Value::Int(items.len() as i64)),
            h if h == crate::h!("contains") => Ok(Value::Bool(
                args.first().map(|a| items.contains(a)).unwrap_or(false),
            )),
            h if h == crate::h!("is_empty") => Ok(Value::Bool(items.is_empty())),
            h if h == crate::h!("add") => {
                let val = &args[0];
                let mut new = items.to_vec();
                if !new.iter().any(|v| v == val) {
                    new.push(val.clone());
                }
                Ok(Value::Set(new))
            }
            h if h == crate::h!("remove") => {
                let val = &args[0];
                let new: Vec<Value> = items.iter().filter(|v| *v != val).cloned().collect();
                Ok(Value::Set(new))
            }
            h if h == crate::h!("union") => {
                if let Some(Value::Set(other)) = args.first() {
                    let mut new = items.to_vec();
                    for v in other {
                        if !new.iter().any(|x| x == v) {
                            new.push(v.clone());
                        }
                    }
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("union requires a set argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("intersection") => {
                if let Some(Value::Set(other)) = args.first() {
                    let new: Vec<Value> = items
                        .iter()
                        .filter(|v| other.iter().any(|x| x == *v))
                        .cloned()
                        .collect();
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("intersection requires a set argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("difference") => {
                if let Some(Value::Set(other)) = args.first() {
                    let new: Vec<Value> = items
                        .iter()
                        .filter(|v| !other.iter().any(|x| x == *v))
                        .cloned()
                        .collect();
                    Ok(Value::Set(new))
                } else {
                    Err(IonError::type_err(
                        ion_str!("difference requires a set argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("to_list") => Ok(Value::List(items.to_vec())),
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("set has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn tuple_method(
        &self,
        items: &[Value],
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::hash::h(method) {
            h if h == crate::h!("len") => Ok(Value::Int(items.len() as i64)),
            h if h == crate::h!("contains") => Ok(Value::Bool(
                args.first().map(|a| items.contains(a)).unwrap_or(false),
            )),
            h if h == crate::h!("to_list") => Ok(Value::List(items.to_vec())),
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("tuple has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn str_method(
        &self,
        s: &str,
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::hash::h(method) {
            h if h == crate::h!("len") => Ok(Value::Int(s.len() as i64)),
            h if h == crate::h!("to_upper") => Ok(Value::Str(s.to_uppercase())),
            h if h == crate::h!("to_lower") => Ok(Value::Str(s.to_lowercase())),
            h if h == crate::h!("trim") => Ok(Value::Str(s.trim().to_string())),
            h if h == crate::h!("contains") => match args.first() {
                Some(Value::Str(sub)) => Ok(Value::Bool(s.contains(sub.as_str()))),
                Some(Value::Int(code)) => {
                    let ch = char::from_u32(*code as u32).ok_or_else(|| {
                        IonError::type_err(ion_str!("invalid char code"), line, col)
                    })?;
                    Ok(Value::Bool(s.contains(ch)))
                }
                _ => Err(IonError::type_err(
                    ion_str!("contains requires string or int argument"),
                    line,
                    col,
                )),
            },
            h if h == crate::h!("starts_with") => {
                let prefix = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            h if h == crate::h!("ends_with") => {
                let suffix = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            h if h == crate::h!("split") => {
                let sep = args.first().and_then(|a| a.as_str()).unwrap_or(" ");
                let parts: Vec<Value> = s.split(sep).map(|p| Value::Str(p.to_string())).collect();
                Ok(Value::List(parts))
            }
            h if h == crate::h!("replace") => {
                let from = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let to = args.get(1).and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Str(s.replace(from, to)))
            }
            h if h == crate::h!("chars") => {
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Ok(Value::List(chars))
            }
            h if h == crate::h!("char_len") => Ok(Value::Int(s.chars().count() as i64)),
            h if h == crate::h!("is_empty") => Ok(Value::Bool(s.is_empty())),
            h if h == crate::h!("trim_start") => Ok(Value::Str(s.trim_start().to_string())),
            h if h == crate::h!("trim_end") => Ok(Value::Str(s.trim_end().to_string())),
            h if h == crate::h!("repeat") => {
                let n = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(ion_str!("repeat requires int argument"), line, col)
                })?;
                Ok(Value::Str(s.repeat(n as usize)))
            }
            h if h == crate::h!("find") => {
                let sub = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(match s.find(sub) {
                    Some(byte_idx) => {
                        let char_idx = s[..byte_idx].chars().count();
                        Value::Option(Some(Box::new(Value::Int(char_idx as i64))))
                    }
                    None => Value::Option(None),
                })
            }
            h if h == crate::h!("to_int") => Ok(match s.trim().parse::<i64>() {
                std::result::Result::Ok(n) => Value::Result(Ok(Box::new(Value::Int(n)))),
                std::result::Result::Err(e) => {
                    Value::Result(Err(Box::new(Value::Str(e.to_string()))))
                }
            }),
            h if h == crate::h!("to_float") => Ok(match s.trim().parse::<f64>() {
                std::result::Result::Ok(f) => Value::Result(Ok(Box::new(Value::Float(f)))),
                std::result::Result::Err(e) => {
                    Value::Result(Err(Box::new(Value::Str(e.to_string()))))
                }
            }),
            h if h == crate::h!("bytes") => {
                let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
                Ok(Value::List(bytes))
            }
            h if h == crate::h!("strip_prefix") => {
                let pre = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Str(s.strip_prefix(pre).unwrap_or(s).to_string()))
            }
            h if h == crate::h!("strip_suffix") => {
                let suf = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Str(s.strip_suffix(suf).unwrap_or(s).to_string()))
            }
            h if h == crate::h!("pad_start") => {
                let width = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(ion_str!("pad_start requires int argument"), line, col)
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
            h if h == crate::h!("pad_end") => {
                let width = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err(ion_str!("pad_end requires int argument"), line, col)
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
            h if h == crate::h!("reverse") => Ok(Value::Str(s.chars().rev().collect())),
            h if h == crate::h!("slice") => {
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
                    ion_str!("string has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn bytes_method(
        &self,
        bytes: &[u8],
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::stdlib::bytes_method_value(bytes, crate::hash::h(method), args) {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("bytes has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
            Err(message) => Err(IonError::type_err(message, line, col)),
        }
    }

    fn dict_method(
        &self,
        map: &IndexMap<String, Value>,
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        match crate::hash::h(method) {
            h if h == crate::h!("len") => Ok(Value::Int(map.len() as i64)),
            h if h == crate::h!("keys") => {
                let keys: Vec<Value> = map.keys().map(|k| Value::Str(k.clone())).collect();
                Ok(Value::List(keys))
            }
            h if h == crate::h!("values") => {
                let vals: Vec<Value> = map.values().cloned().collect();
                Ok(Value::List(vals))
            }
            h if h == crate::h!("contains_key") => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(map.contains_key(key)))
            }
            h if h == crate::h!("get") => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(match map.get(key) {
                    Some(v) => Value::Option(Some(Box::new(v.clone()))),
                    None => Value::Option(None),
                })
            }
            h if h == crate::h!("is_empty") => Ok(Value::Bool(map.is_empty())),
            h if h == crate::h!("entries") => Ok(Value::List(
                map.iter()
                    .map(|(k, v)| Value::Tuple(vec![Value::Str(k.clone()), v.clone()]))
                    .collect(),
            )),
            h if h == crate::h!("insert") => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let val = args.get(1).cloned().unwrap_or(Value::Unit);
                let mut new_map = map.clone();
                new_map.insert(key.to_string(), val);
                Ok(Value::Dict(new_map))
            }
            h if h == crate::h!("remove") => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let mut new_map = map.clone();
                new_map.shift_remove(key);
                Ok(Value::Dict(new_map))
            }
            h if h == crate::h!("merge") => {
                if let Some(Value::Dict(other)) = args.first() {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        ion_str!("merge requires a dict argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("update") => {
                if let Some(Value::Dict(other)) = args.first() {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        ion_str!("update requires a dict argument"),
                        line,
                        col,
                    ))
                }
            }
            h if h == crate::h!("keys_of") => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err(ion_str!("keys_of requires an argument"), line, col)
                })?;
                let keys: Vec<Value> = map
                    .iter()
                    .filter(|(_, v)| *v == target)
                    .map(|(k, _)| Value::Str(k.clone()))
                    .collect();
                Ok(Value::List(keys))
            }
            h if h == crate::h!("zip") => {
                if let Some(Value::Dict(other)) = args.first() {
                    let mut result = indexmap::IndexMap::new();
                    for (k, v) in map {
                        if let Some(ov) = other.get(k) {
                            result.insert(k.clone(), Value::Tuple(vec![v.clone(), ov.clone()]));
                        }
                    }
                    Ok(Value::Dict(result))
                } else {
                    Err(IonError::type_err(
                        ion_str!("zip requires a dict argument"),
                        line,
                        col,
                    ))
                }
            }
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("dict has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn option_method(
        &self,
        val: &Value,
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        let opt = match val {
            Value::Option(o) => o,
            _ => return Err(IonError::type_err(ion_str!("expected Option"), line, col)),
        };
        match crate::hash::h(method) {
            h if h == crate::h!("is_some") => Ok(Value::Bool(opt.is_some())),
            h if h == crate::h!("is_none") => Ok(Value::Bool(opt.is_none())),
            h if h == crate::h!("unwrap") => match opt {
                Some(v) => Ok(*v.clone()),
                None => Err(IonError::runtime(
                    ion_str!("called unwrap on None"),
                    line,
                    col,
                )),
            },
            h if h == crate::h!("unwrap_or") => Ok(opt
                .as_ref()
                .map(|v| *v.clone())
                .unwrap_or_else(|| args.first().cloned().unwrap_or(Value::Unit))),
            h if h == crate::h!("expect") => match opt {
                Some(v) => Ok(*v.clone()),
                None => {
                    #[cfg(debug_assertions)]
                    let msg = args
                        .first()
                        .and_then(|a| a.as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| ion_str!("called expect on None"));
                    #[cfg(not(debug_assertions))]
                    let msg = ion_str!("called expect on None");
                    Err(IonError::runtime(msg, line, col))
                }
            },
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("Option has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    fn result_method(
        &self,
        val: &Value,
        method: &str,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        let res = match val {
            Value::Result(r) => r,
            _ => return Err(IonError::type_err(ion_str!("expected Result"), line, col)),
        };
        match crate::hash::h(method) {
            h if h == crate::h!("is_ok") => Ok(Value::Bool(res.is_ok())),
            h if h == crate::h!("is_err") => Ok(Value::Bool(res.is_err())),
            h if h == crate::h!("unwrap") => match res {
                Ok(v) => Ok(*v.clone()),
                Err(e) => Err(IonError::runtime(
                    format!("{}{}", ion_str!("called unwrap on Err: "), e),
                    line,
                    col,
                )),
            },
            h if h == crate::h!("unwrap_or") => Ok(match res {
                Ok(v) => *v.clone(),
                Err(_) => args.first().cloned().unwrap_or(Value::Unit),
            }),
            h if h == crate::h!("expect") => match res {
                Ok(v) => Ok(*v.clone()),
                Err(e) => {
                    #[cfg(debug_assertions)]
                    let msg = args
                        .first()
                        .and_then(|a| a.as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| ion_str!("called expect on Err"));
                    #[cfg(debug_assertions)]
                    {
                        Err(IonError::runtime(format!("{}: {}", msg, e), line, col))
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        let _ = (args, e);
                        Err(IonError::runtime(
                            ion_str!("called expect on Err"),
                            line,
                            col,
                        ))
                    }
                }
            },
            _ => Err(IonError::type_err(
                format!(
                    "{}{}{}",
                    ion_str!("Result has no method '"),
                    method,
                    ion_str!("'")
                ),
                line,
                col,
            )),
        }
    }

    // ---- Function calls ----

    /// Invoke a function value with arguments directly (not from the stack).
    fn invoke_value(
        &mut self,
        func: &Value,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        // Push func and args onto stack, then call
        self.stack.push(func.clone());
        for arg in args {
            self.stack.push(arg.clone());
        }
        self.call_function(args.len(), line, col)?;
        self.pop(line, col)
    }

    fn eval_default_arg(
        &self,
        param_name: &str,
        default: &crate::ast::Expr,
        line: usize,
        col: usize,
    ) -> Result<Value, IonError> {
        let mut interp = crate::interpreter::Interpreter::with_env(self.env.clone());
        interp.types = self.types.clone();
        interp.eval_single_expr(default).map_err(|e| {
            IonError::runtime(
                format!(
                    "{}'{}': {}",
                    ion_str!("error evaluating default for "),
                    param_name,
                    e.message
                ),
                line,
                col,
            )
        })
    }

    fn prepare_positional_function_args(
        &mut self,
        ion_fn: &crate::value::IonFn,
        args: &[Value],
        line: usize,
        col: usize,
    ) -> Result<Vec<Value>, IonError> {
        let mut prepared = Vec::with_capacity(ion_fn.params.len());
        for (i, param) in ion_fn.params.iter().enumerate() {
            let val = if i < args.len() {
                args[i].clone()
            } else if let Some(default) = &param.default {
                self.eval_default_arg(&param.name, default, line, col)?
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
                    line,
                    col,
                ));
            };
            self.env.define(param.name.clone(), val.clone(), false);
            prepared.push(val);
        }
        Ok(prepared)
    }

    fn prepare_named_function_args(
        &mut self,
        ion_fn: &crate::value::IonFn,
        ordered: &[Option<Value>],
        line: usize,
        col: usize,
    ) -> Result<Vec<Value>, IonError> {
        let mut prepared = Vec::with_capacity(ion_fn.params.len());
        for (i, param) in ion_fn.params.iter().enumerate() {
            let val = if let Some(Some(val)) = ordered.get(i) {
                val.clone()
            } else if let Some(default) = &param.default {
                self.eval_default_arg(&param.name, default, line, col)?
            } else {
                return Err(IonError::runtime(
                    format!(
                        "{}{}{}",
                        ion_str!("missing argument '"),
                        param.name,
                        ion_str!("'"),
                    ),
                    line,
                    col,
                ));
            };
            self.env.define(param.name.clone(), val.clone(), false);
            prepared.push(val);
        }
        Ok(prepared)
    }

    fn call_function(&mut self, arg_count: usize, line: usize, col: usize) -> Result<(), IonError> {
        // Stack: [..., func, arg0, arg1, ..., argN-1]
        let args_start = self.stack.len() - arg_count;
        let func_idx = args_start - 1;
        let mut func = self.stack[func_idx].clone();
        let mut args: Vec<Value> = self.stack[args_start..].to_vec();
        self.stack.truncate(func_idx);

        // Trampoline loop: handles tail calls without growing the Rust stack
        loop {
            match func {
                #[cfg(all(
                    feature = "legacy-threaded-concurrency",
                    not(feature = "async-runtime")
                ))]
                Value::BuiltinFn { qualified_hash, .. }
                    if qualified_hash == crate::h!("timeout") =>
                {
                    let result = self.builtin_timeout(&args, line, col)?;
                    self.stack.push(result);
                    return Ok(());
                }
                Value::BuiltinFn { func, .. } => {
                    let result = func(&args).map_err(|e| IonError::runtime(e, line, col))?;
                    self.stack.push(result);
                    return Ok(());
                }
                Value::BuiltinClosure { func, .. } => {
                    let result = func
                        .call(&args)
                        .map_err(|e| IonError::runtime(e, line, col))?;
                    self.stack.push(result);
                    return Ok(());
                }
                Value::Module(ref table) => {
                    let Some(result) = crate::stdlib::call_stdlib_module(table, &args) else {
                        return Err(IonError::type_err(
                            format!("{}{}", ion_str!("cannot call "), func.type_name()),
                            line,
                            col,
                        ));
                    };
                    self.stack
                        .push(result.map_err(|e| IonError::runtime(e, line, col))?);
                    return Ok(());
                }
                #[cfg(feature = "async-runtime")]
                Value::AsyncBuiltinClosure { .. } => {
                    return Err(IonError::runtime(
                        ion_str!(
                            "async host function cannot be called by the synchronous evaluator; use eval_async"
                        ),
                        line,
                        col,
                    ));
                }
                Value::Fn(ion_fn) => {
                    self.env.push_scope();

                    for (name, val) in &ion_fn.captures {
                        self.env.define(name.clone(), val.clone(), false);
                    }

                    // Save locals state for this function call
                    let saved_locals_base = self.locals_base;
                    let saved_locals_len = self.locals.len();
                    let saved_frames_len = self.local_frames.len();
                    let prepared_args =
                        match self.prepare_positional_function_args(&ion_fn, &args, line, col) {
                            Ok(args) => args,
                            Err(e) => {
                                self.env.pop_scope();
                                return Err(e);
                            }
                        };

                    self.locals_base = self.locals.len(); // new base for function's locals

                    // Push params as slot-based locals (slots 0..N relative to base)
                    for val in prepared_args {
                        self.locals.push(LocalSlot {
                            value: val,
                            mutable: false,
                        });
                    }

                    let fn_id = ion_fn.fn_id;
                    let chunk_opt = if let Some(chunk) = self.fn_cache.get(&fn_id) {
                        Some(chunk.clone())
                    } else {
                        let compiler = crate::compiler::Compiler::new();
                        compiler
                            .compile_fn_body(&ion_fn.params, &ion_fn.body, line)
                            .ok()
                    };
                    if let Some(chunk) = chunk_opt {
                        self.fn_cache.entry(fn_id).or_insert_with(|| chunk.clone());
                        let saved_ip = self.ip;
                        let saved_iters = std::mem::take(&mut self.iterators);
                        self.ip = 0;
                        let result = self.run_chunk(&chunk);
                        self.ip = saved_ip;
                        self.iterators = saved_iters;
                        // Restore locals
                        self.locals.truncate(saved_locals_len);
                        self.local_frames.truncate(saved_frames_len);
                        self.locals_base = saved_locals_base;
                        self.env.pop_scope();

                        // Check for pending tail call (trampoline)
                        if let Some((tail_func, tail_args)) = self.pending_tail_call.take() {
                            func = tail_func;
                            args = tail_args;
                            continue; // loop back without growing Rust stack
                        }

                        match result {
                            Ok(val) => self.stack.push(val),
                            Err(e) if e.kind == crate::error::ErrorKind::PropagatedErr => {
                                self.stack.push(Value::Result(Err(Box::new(Value::Str(
                                    e.message.clone(),
                                )))));
                            }
                            Err(e) if e.kind == crate::error::ErrorKind::PropagatedNone => {
                                self.stack.push(Value::Option(None));
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        // Restore locals before tree-walk fallback
                        self.locals.truncate(saved_locals_len);
                        self.local_frames.truncate(saved_frames_len);
                        self.locals_base = saved_locals_base;
                        let mut interp =
                            crate::interpreter::Interpreter::with_env(self.env.clone());
                        let result = interp.eval_block(&ion_fn.body);
                        self.env = interp.take_env();
                        self.env.pop_scope();
                        match result {
                            Ok(val) => self.stack.push(val),
                            Err(e) if e.kind == crate::error::ErrorKind::PropagatedErr => {
                                self.stack.push(Value::Result(Err(Box::new(Value::Str(
                                    e.message.clone(),
                                )))));
                            }
                            Err(e) if e.kind == crate::error::ErrorKind::PropagatedNone => {
                                self.stack.push(Value::Option(None));
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    return Ok(());
                }
                _ => {
                    return Err(IonError::type_err(
                        format!("{}{}", ion_str!("cannot call "), func.type_name()),
                        line,
                        col,
                    ));
                }
            }
        }
    }

    fn call_function_named(
        &mut self,
        arg_count: usize,
        named_map: &[(usize, String)],
        line: usize,
        col: usize,
    ) -> Result<(), IonError> {
        // Stack: [..., func, arg0, arg1, ..., argN-1]
        let args_start = self.stack.len() - arg_count;
        let func_idx = args_start - 1;
        let func = self.stack[func_idx].clone();
        let raw_args: Vec<Value> = self.stack[args_start..].to_vec();
        self.stack.truncate(func_idx);

        match &func {
            Value::Fn(ion_fn) => {
                // Reorder args based on named_map
                let mut ordered = vec![None; ion_fn.params.len()];
                let mut pos_idx = 0;
                for (i, val) in raw_args.into_iter().enumerate() {
                    if let Some((_, ref name)) = named_map.iter().find(|(pos, _)| *pos == i) {
                        // Named arg: find param by name
                        let param_idx = ion_fn
                            .params
                            .iter()
                            .position(|p| &p.name == name)
                            .ok_or_else(|| {
                                IonError::runtime(
                                    format!(
                                        "{}'{}'{}'{}'",
                                        ion_str!("unknown parameter '"),
                                        name,
                                        ion_str!("' for function '"),
                                        ion_fn.name
                                    ),
                                    line,
                                    col,
                                )
                            })?;
                        ordered[param_idx] = Some(val);
                    } else {
                        // Positional arg: fill next available slot
                        while pos_idx < ordered.len() && ordered[pos_idx].is_some() {
                            pos_idx += 1;
                        }
                        if pos_idx < ordered.len() {
                            ordered[pos_idx] = Some(val);
                            pos_idx += 1;
                        }
                    }
                }
                // Fill defaults and push reordered args
                self.env.push_scope();
                for (name, val) in &ion_fn.captures {
                    self.env.define(name.clone(), val.clone(), false);
                }
                let reordered = match self.prepare_named_function_args(ion_fn, &ordered, line, col)
                {
                    Ok(args) => args,
                    Err(e) => {
                        self.env.pop_scope();
                        return Err(e);
                    }
                };
                self.env.pop_scope();
                // Push func + reordered args, then call normally
                self.stack.push(func.clone());
                for arg in &reordered {
                    self.stack.push(arg.clone());
                }
                self.call_function(reordered.len(), line, col)
            }
            Value::BuiltinFn { func, .. } => {
                // Builtins don't support named args, just pass positionally
                let result = func(&raw_args).map_err(|e| IonError::runtime(e, line, col))?;
                self.stack.push(result);
                Ok(())
            }
            Value::BuiltinClosure { func, .. } => {
                let result = func
                    .call(&raw_args)
                    .map_err(|e| IonError::runtime(e, line, col))?;
                self.stack.push(result);
                Ok(())
            }
            Value::Module(table) => {
                let Some(result) = crate::stdlib::call_stdlib_module(table, &raw_args) else {
                    return Err(IonError::type_err(
                        format!("cannot call {}", func.type_name()),
                        line,
                        col,
                    ));
                };
                self.stack
                    .push(result.map_err(|e| IonError::runtime(e, line, col))?);
                Ok(())
            }
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { .. } => Err(IonError::runtime(
                ion_str!(
                    "async host function cannot be called by the synchronous evaluator; use eval_async"
                ),
                line,
                col,
            )),
            _ => Err(IonError::type_err(
                format!("cannot call {}", func.type_name()),
                line,
                col,
            )),
        }
    }

    #[cfg(all(
        feature = "legacy-threaded-concurrency",
        not(feature = "async-runtime")
    ))]
    fn builtin_timeout(&self, args: &[Value], line: usize, col: usize) -> Result<Value, IonError> {
        if args.len() < 2 {
            return Err(IonError::runtime(
                ion_str!("timeout(ms, fn) requires 2 arguments"),
                line,
                col,
            ));
        }
        let ms = args[0].as_int().ok_or_else(|| {
            IonError::runtime(
                ion_str!("timeout: first argument must be int (ms)"),
                line,
                col,
            )
        })?;
        let func = args[1].clone();
        let captured_env = self.env.capture();
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = crate::async_rt::spawn_task_with_cancel(cancel_flag, move |flag| {
            let mut child = crate::interpreter::Interpreter::new();
            child.cancel_flag = Some(flag);
            for (name, val) in captured_env {
                child.env.define(name, val, false);
            }
            // Build a program that calls the function
            let program = crate::ast::Program {
                stmts: vec![crate::ast::Stmt {
                    kind: crate::ast::StmtKind::ExprStmt {
                        expr: crate::ast::Expr {
                            kind: crate::ast::ExprKind::Call {
                                func: Box::new(crate::ast::Expr {
                                    kind: crate::ast::ExprKind::Ident("__timeout_fn__".to_string()),
                                    span: crate::ast::Span { line: 0, col: 0 },
                                }),
                                args: vec![],
                            },
                            span: crate::ast::Span { line: 0, col: 0 },
                        },
                        has_semi: false,
                    },
                    span: crate::ast::Span { line: 0, col: 0 },
                }],
            };
            child.env.define("__timeout_fn__".to_string(), func, false);
            child.eval_program(&program)
        });
        match task.join_timeout(std::time::Duration::from_millis(ms as u64)) {
            Some(Ok(val)) => Ok(Value::Option(Some(Box::new(val)))),
            Some(Err(e)) => Err(e),
            None => {
                task.cancel();
                Ok(Value::Option(None))
            }
        }
    }
}
