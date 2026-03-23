//! Stack-based virtual machine for executing Ion bytecode.

use indexmap::IndexMap;

use crate::bytecode::{Chunk, Op};
use crate::env::Env;
use crate::error::IonError;
use crate::host_types::TypeRegistry;
use crate::value::Value;

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
        }
    }

    /// Create a VM with an existing environment (for engine integration).
    pub fn with_env(env: Env) -> Self {
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
        }
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
                                "'&' expects int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ))
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
                                "'|' expects int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ))
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
                                "'^' expects int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ))
                    }
                }
            }
            Op::Shl => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x << y)),
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "'<<' expects int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ))
                    }
                }
            }
            Op::Shr => {
                let b = self.pop(line, col)?;
                let a = self.pop(line, col)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x >> y)),
                    (a, b) => {
                        return Err(IonError::type_err(
                            format!(
                                "'>>' expects int, got {} and {}",
                                a.type_name(),
                                b.type_name()
                            ),
                            line,
                            col,
                        ))
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
                let top = self.peek(line, col)?;
                if !top.is_truthy() {
                    self.ip += offset; // short-circuit: keep falsy value
                }
                // If truthy, fall through — Pop will remove it, then eval right
            }
            Op::Or => {
                let offset = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let top = self.peek(line, col)?;
                if top.is_truthy() {
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
                    IonError::name(format!("undefined variable: {}", name), line, col)
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
                let val = self.env.get_sym(sym).cloned().ok_or_else(|| {
                    let name = self.env.resolve(sym);
                    IonError::name(format!("undefined variable: {}", name), line, col)
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
                        "cannot assign to immutable variable".to_string(),
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
                let val = self.peek(line, col)?;
                if !val.is_truthy() {
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
                                "? operator requires Option or Result, got {}",
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
                let s: String = parts.iter().map(|v| v.to_string()).collect();
                self.stack.push(Value::Str(s));
            }

            // --- Pipe ---
            Op::Pipe => {
                let _arg_count = chunk.read_u8(self.ip);
                self.ip += 1;
                // Pipe is handled by the compiler rewriting to Call
                return Err(IonError::runtime(
                    "pipe opcode should not be executed directly",
                    line,
                    col,
                ));
            }

            // --- Pattern matching ---
            Op::MatchBegin => {
                // u8: kind (1=Some, 2=Ok, 3=Err, 4=Tuple, 5=List)
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
                    _ => false,
                };
                // Push value back (needed for unwrap) and then bool
                self.stack.push(val);
                self.stack.push(Value::Bool(result));
            }
            Op::MatchArm => {
                // u8: kind (1=unwrap Some, 2=unwrap Ok, 3=unwrap Err, 4=get tuple element, 5=get list element)
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
                    _ => {}
                }
            }
            Op::MatchEnd => {
                // Currently unused — match uses Jump/JumpIfFalse directly
            }

            // --- Range ---
            Op::BuildRange => {
                let inclusive = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let end = self.pop(line, col)?;
                let start = self.pop(line, col)?;
                let s = start
                    .as_int()
                    .ok_or_else(|| IonError::type_err("range start must be int", line, col))?;
                let e = end
                    .as_int()
                    .ok_or_else(|| IonError::type_err("range end must be int", line, col))?;
                let items: Vec<Value> = if inclusive {
                    (s..=e).map(Value::Int).collect()
                } else {
                    (s..e).map(Value::Int).collect()
                };
                self.stack.push(Value::List(items));
            }

            // --- Host types ---
            Op::ConstructStruct => {
                let type_name_idx = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let raw_count = chunk.read_u16(self.ip) as usize;
                self.ip += 2;
                let type_name = match &chunk.constants[type_name_idx] {
                    Value::Str(s) => s.clone(),
                    _ => return Err(IonError::runtime("invalid type name", line, col)),
                };
                let has_spread = raw_count & 0x8000 != 0;
                let field_count = raw_count & 0x7FFF;
                let mut fields = IndexMap::new();
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
                                "spread in struct constructor requires a struct",
                                line,
                                col,
                            ))
                        }
                    }
                    // Apply overrides
                    for pair in overrides.chunks(2) {
                        let fname = match &pair[0] {
                            Value::Str(s) => s.clone(),
                            _ => return Err(IonError::runtime("invalid field name", line, col)),
                        };
                        fields.insert(fname, pair[1].clone());
                    }
                } else {
                    // No spread: fields are pushed as name, value pairs
                    let start = self.stack.len() - field_count * 2;
                    let items: Vec<Value> = self.stack.drain(start..).collect();
                    for pair in items.chunks(2) {
                        let fname = match &pair[0] {
                            Value::Str(s) => s.clone(),
                            _ => return Err(IonError::runtime("invalid field name", line, col)),
                        };
                        fields.insert(fname, pair[1].clone());
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
                    _ => return Err(IonError::runtime("invalid enum name", line, col)),
                };
                let variant_name = match &chunk.constants[variant_name_idx] {
                    Value::Str(s) => s.clone(),
                    _ => return Err(IonError::runtime("invalid variant name", line, col)),
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
                    other => {
                        return Err(IonError::type_err(
                            format!("cannot iterate over {}", other.type_name()),
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
                    .ok_or_else(|| IonError::runtime("no active iterator", line, col))?;
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
                    return Err(IonError::runtime("ListAppend: no list on stack", line, col));
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
                    return Err(IonError::runtime("DictInsert: no dict on stack", line, col));
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
                                "DictMerge: no dict on stack",
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
                        ))
                    }
                }
            }

            // --- Slice ---
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

            // --- Print ---
            Op::Print => {
                let newline = chunk.read_u8(self.ip) != 0;
                self.ip += 1;
                let val = self.pop(line, col)?;
                if newline {
                    println!("{}", val);
                } else {
                    print!("{}", val);
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
                format!("invalid opcode: {}", byte),
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
                    format!("slice index must be int, got {}", other.type_name()),
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
                format!("cannot slice {}", obj.type_name()),
                line,
                col,
            )),
        }
    }

    fn pop(&mut self, line: usize, col: usize) -> Result<Value, IonError> {
        self.stack
            .pop()
            .ok_or_else(|| IonError::runtime("stack underflow", line, col))
    }

    fn peek(&self, line: usize, col: usize) -> Result<Value, IonError> {
        self.stack
            .last()
            .cloned()
            .ok_or_else(|| IonError::runtime("stack underflow (peek)", line, col))
    }

    fn const_as_str(&self, val: &Value, line: usize, col: usize) -> Result<String, IonError> {
        match val {
            Value::Str(s) => Ok(s.clone()),
            _ => Err(IonError::runtime("expected string constant", line, col)),
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
            _ => Err(IonError::runtime("expected string constant", line, col)),
        }
    }

    // ---- Arithmetic ----

    fn op_add(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x + y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x + y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 + y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x + *y as f64)),
            (Value::Str(x), Value::Str(y)) => Ok(Value::Str(format!("{}{}", x, y))),
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
                format!("cannot add {} and {}", a.type_name(), b.type_name()),
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
                format!("cannot subtract {} from {}", b.type_name(), a.type_name()),
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
                format!("cannot multiply {} and {}", a.type_name(), b.type_name()),
                line,
                col,
            )),
        }
    }

    fn op_div(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("division by zero", line, col)),
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x / y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x / y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 / y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x / *y as f64)),
            _ => Err(IonError::type_err(
                format!("cannot divide {} by {}", a.type_name(), b.type_name()),
                line,
                col,
            )),
        }
    }

    fn op_mod(&self, a: Value, b: Value, line: usize, col: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("modulo by zero", line, col)),
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x % y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 % y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x % *y as f64)),
            _ => Err(IonError::type_err(
                format!("cannot modulo {} by {}", a.type_name(), b.type_name()),
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
                format!("cannot negate {}", val.type_name()),
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
                format!("cannot compare {} and {}", a.type_name(), b.type_name()),
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
            Value::HostStruct { fields, .. } => fields.get(field).cloned().ok_or_else(|| {
                IonError::runtime(format!("field '{}' not found", field), line, col)
            }),
            Value::List(items) => match field {
                "len" => Ok(Value::Int(items.len() as i64)),
                _ => Err(IonError::runtime(
                    format!("list has no field '{}'", field),
                    line,
                    col,
                )),
            },
            Value::Str(s) => match field {
                "len" => Ok(Value::Int(s.len() as i64)),
                _ => Err(IonError::runtime(
                    format!("string has no field '{}'", field),
                    line,
                    col,
                )),
            },
            Value::Tuple(items) => match field {
                "len" => Ok(Value::Int(items.len() as i64)),
                _ => Err(IonError::runtime(
                    format!("tuple has no field '{}'", field),
                    line,
                    col,
                )),
            },
            _ => Err(IonError::type_err(
                format!("cannot access field '{}' on {}", field, obj.type_name()),
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
                    IonError::runtime(format!("index {} out of range", i), line, col)
                })
            }
            (Value::Tuple(items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(idx).cloned().ok_or_else(|| {
                    IonError::runtime(format!("index {} out of range", i), line, col)
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
                        IonError::runtime(format!("index {} out of range", i), line, col)
                    })
            }
            (Value::Bytes(bytes), Value::Int(i)) => {
                let idx = if *i < 0 { bytes.len() as i64 + i } else { *i } as usize;
                bytes
                    .get(idx)
                    .map(|&b| Value::Int(b as i64))
                    .ok_or_else(|| {
                        IonError::runtime(format!("index {} out of range", i), line, col)
                    })
            }
            _ => Err(IonError::type_err(
                format!(
                    "cannot index {} with {}",
                    obj.type_name(),
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
                        format!("index {} out of range", i),
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
                format!("cannot set index on {}", obj.type_name()),
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
                type_name,
                mut fields,
            } => {
                if fields.contains_key(field) {
                    fields.insert(field.to_string(), value);
                    Ok(Value::HostStruct { type_name, fields })
                } else {
                    Err(IonError::runtime(
                        format!("field '{}' not found on {}", field, type_name),
                        line,
                        col,
                    ))
                }
            }
            _ => Err(IonError::type_err(
                format!("cannot set field on {}", obj.type_name()),
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
        if method == "to_string" {
            return Ok(Value::Str(format!("{}", receiver)));
        }
        // Handle closure-based methods that need &mut self for invoke_value
        match (&receiver, method) {
            // List closure methods
            (Value::List(items), "map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("map requires a function argument", line, col)
                })?;
                let mut result = Vec::new();
                for item in items {
                    result.push(self.invoke_value(func, std::slice::from_ref(item), line, col)?);
                }
                return Ok(Value::List(result));
            }
            (Value::List(items), "filter") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("filter requires a function argument", line, col)
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
            (Value::List(items), "fold") => {
                let init = args.first().cloned().unwrap_or(Value::Unit);
                let func = args.get(1).ok_or_else(|| {
                    IonError::runtime("fold requires an initial value and a function", line, col)
                })?;
                let mut acc = init;
                for item in items {
                    acc = self.invoke_value(func, &[acc, item.clone()], line, col)?;
                }
                return Ok(acc);
            }
            (Value::List(items), "flat_map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("flat_map requires a function argument", line, col)
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
            (Value::List(items), "any") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("any requires a function argument", line, col)
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
            (Value::List(items), "all") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("all requires a function argument", line, col)
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
            (Value::List(items), "sort_by") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("sort_by requires a function argument", line, col)
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
                                "sort_by function must return int",
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

            // Dict closure methods
            (Value::Dict(map), "map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("map requires a function argument", line, col)
                })?;
                let mut result = indexmap::IndexMap::new();
                for (k, v) in map {
                    let mapped =
                        self.invoke_value(func, &[Value::Str(k.clone()), v.clone()], line, col)?;
                    result.insert(k.clone(), mapped);
                }
                return Ok(Value::Dict(result));
            }
            (Value::Dict(map), "filter") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("filter requires a function argument", line, col)
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
            (Value::Option(opt), "map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("map requires a function argument", line, col)
                })?;
                return match opt {
                    Some(v) => {
                        let result = self.invoke_value(func, &[*v.clone()], line, col)?;
                        Ok(Value::Option(Some(Box::new(result))))
                    }
                    None => Ok(Value::Option(None)),
                };
            }
            (Value::Option(opt), "and_then") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("and_then requires a function argument", line, col)
                })?;
                return match opt {
                    Some(v) => self.invoke_value(func, &[*v.clone()], line, col),
                    None => Ok(Value::Option(None)),
                };
            }
            (Value::Option(opt), "or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("or_else requires a function argument", line, col)
                })?;
                return match opt {
                    Some(v) => Ok(Value::Option(Some(v.clone()))),
                    None => self.invoke_value(func, &[], line, col),
                };
            }
            (Value::Option(opt), "unwrap_or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("unwrap_or_else requires a function argument", line, col)
                })?;
                return match opt {
                    Some(v) => Ok(*v.clone()),
                    None => self.invoke_value(func, &[], line, col),
                };
            }

            // Result closure methods
            (Value::Result(res), "map") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("map requires a function argument", line, col)
                })?;
                return match res {
                    Ok(v) => {
                        let result = self.invoke_value(func, &[*v.clone()], line, col)?;
                        Ok(Value::Result(Ok(Box::new(result))))
                    }
                    Err(e) => Ok(Value::Result(Err(e.clone()))),
                };
            }
            (Value::Result(res), "map_err") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("map_err requires a function argument", line, col)
                })?;
                return match res {
                    Ok(v) => Ok(Value::Result(Ok(v.clone()))),
                    Err(e) => {
                        let result = self.invoke_value(func, &[*e.clone()], line, col)?;
                        Ok(Value::Result(Err(Box::new(result))))
                    }
                };
            }
            (Value::Result(res), "and_then") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("and_then requires a function argument", line, col)
                })?;
                return match res {
                    Ok(v) => self.invoke_value(func, &[*v.clone()], line, col),
                    Err(e) => Ok(Value::Result(Err(e.clone()))),
                };
            }
            (Value::Result(res), "or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("or_else requires a function argument", line, col)
                })?;
                return match res {
                    Ok(v) => Ok(Value::Result(Ok(v.clone()))),
                    Err(e) => self.invoke_value(func, &[*e.clone()], line, col),
                };
            }
            (Value::Result(res), "unwrap_or_else") => {
                let func = args.first().ok_or_else(|| {
                    IonError::runtime("unwrap_or_else requires a function argument", line, col)
                })?;
                return match res {
                    Ok(v) => Ok(*v.clone()),
                    Err(e) => self.invoke_value(func, &[*e.clone()], line, col),
                };
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
            Value::Option(_) => self.option_method(&receiver, method, args, line, col),
            Value::Result(_) => self.result_method(&receiver, method, args, line, col),
            _ => Err(IonError::type_err(
                format!("{} has no method '{}'", receiver.type_name(), method),
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
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "push" => {
                let mut new = items.to_vec();
                for a in args {
                    new.push(a.clone());
                }
                Ok(Value::List(new))
            }
            "pop" => {
                let mut new = items.to_vec();
                let val = new.pop().unwrap_or(Value::Unit);
                Ok(val)
            }
            "contains" => Ok(Value::Bool(
                args.first().map(|a| items.contains(a)).unwrap_or(false),
            )),
            "is_empty" => Ok(Value::Bool(items.is_empty())),
            "reverse" => {
                let mut new = items.to_vec();
                new.reverse();
                Ok(Value::List(new))
            }
            "join" => {
                let sep = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let s: String = items
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(sep);
                Ok(Value::Str(s))
            }
            "enumerate" => {
                let pairs: Vec<Value> = items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                    .collect();
                Ok(Value::List(pairs))
            }
            "first" => Ok(match items.first() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            "last" => Ok(match items.last() {
                Some(v) => Value::Option(Some(Box::new(v.clone()))),
                None => Value::Option(None),
            }),
            "sort" => {
                if !items.is_empty() {
                    let first_type = std::mem::discriminant(&items[0]);
                    for item in items.iter().skip(1) {
                        if std::mem::discriminant(item) != first_type {
                            return Err(IonError::type_err(
                                "sort() requires all elements to be the same type".to_string(),
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
                if let Some(Value::List(other)) = args.first() {
                    let result: Vec<Value> = items
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| Value::Tuple(vec![a.clone(), b.clone()]))
                        .collect();
                    Ok(Value::List(result))
                } else {
                    Err(IonError::type_err(
                        "zip requires a list argument".to_string(),
                        line,
                        col,
                    ))
                }
            }
            "index" => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err("index requires an argument".to_string(), line, col)
                })?;
                Ok(match items.iter().position(|v| v == target) {
                    Some(i) => Value::Option(Some(Box::new(Value::Int(i as i64)))),
                    None => Value::Option(None),
                })
            }
            "count" => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err("count requires an argument".to_string(), line, col)
                })?;
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
                        (Value::Int(a), Value::Int(b)) if b < a => min = item,
                        (Value::Float(a), Value::Float(b)) if b < a => min = item,
                        (Value::Str(a), Value::Str(b)) if b < a => min = item,
                        (Value::Int(_), Value::Int(_))
                        | (Value::Float(_), Value::Float(_))
                        | (Value::Str(_), Value::Str(_)) => {}
                        _ => {
                            return Err(IonError::type_err(
                                "min() requires homogeneous comparable elements".to_string(),
                                line,
                                col,
                            ))
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
                        (Value::Int(a), Value::Int(b)) if b > a => max = item,
                        (Value::Float(a), Value::Float(b)) if b > a => max = item,
                        (Value::Str(a), Value::Str(b)) if b > a => max = item,
                        (Value::Int(_), Value::Int(_))
                        | (Value::Float(_), Value::Float(_))
                        | (Value::Str(_), Value::Str(_)) => {}
                        _ => {
                            return Err(IonError::type_err(
                                "max() requires homogeneous comparable elements".to_string(),
                                line,
                                col,
                            ))
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
                                "sum() requires numeric elements".to_string(),
                                line,
                                col,
                            ))
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
                    IonError::type_err("window requires int argument".to_string(), line, col)
                })? as usize;
                let result: Vec<Value> =
                    items.windows(n).map(|w| Value::List(w.to_vec())).collect();
                Ok(Value::List(result))
            }
            _ => Err(IonError::type_err(
                format!("list has no method '{}'", method),
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
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "contains" => Ok(Value::Bool(
                args.first().map(|a| items.contains(a)).unwrap_or(false),
            )),
            "to_list" => Ok(Value::List(items.to_vec())),
            _ => Err(IonError::type_err(
                format!("tuple has no method '{}'", method),
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
        match method {
            "len" => Ok(Value::Int(s.len() as i64)),
            "to_upper" => Ok(Value::Str(s.to_uppercase())),
            "to_lower" => Ok(Value::Str(s.to_lowercase())),
            "trim" => Ok(Value::Str(s.trim().to_string())),
            "contains" => match args.first() {
                Some(Value::Str(sub)) => Ok(Value::Bool(s.contains(sub.as_str()))),
                Some(Value::Int(code)) => {
                    let ch = char::from_u32(*code as u32).ok_or_else(|| {
                        IonError::type_err("invalid char code".to_string(), line, col)
                    })?;
                    Ok(Value::Bool(s.contains(ch)))
                }
                _ => Err(IonError::type_err(
                    "contains requires string or int argument".to_string(),
                    line,
                    col,
                )),
            },
            "starts_with" => {
                let prefix = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            "ends_with" => {
                let suffix = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            "split" => {
                let sep = args.first().and_then(|a| a.as_str()).unwrap_or(" ");
                let parts: Vec<Value> = s.split(sep).map(|p| Value::Str(p.to_string())).collect();
                Ok(Value::List(parts))
            }
            "replace" => {
                let from = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let to = args.get(1).and_then(|a| a.as_str()).unwrap_or("");
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
                    IonError::type_err("repeat requires int argument".to_string(), line, col)
                })?;
                Ok(Value::Str(s.repeat(n as usize)))
            }
            "find" => {
                let sub = args.first().and_then(|a| a.as_str()).unwrap_or("");
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
                let pre = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Str(s.strip_prefix(pre).unwrap_or(s).to_string()))
            }
            "strip_suffix" => {
                let suf = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Str(s.strip_suffix(suf).unwrap_or(s).to_string()))
            }
            "pad_start" => {
                let width = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err("pad_start requires int argument".to_string(), line, col)
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
                    IonError::type_err("pad_end requires int argument".to_string(), line, col)
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
                format!("string has no method '{}'", method),
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
        match method {
            "len" => Ok(Value::Int(bytes.len() as i64)),
            "is_empty" => Ok(Value::Bool(bytes.is_empty())),
            "contains" => {
                let byte = args.first().and_then(|a| a.as_int()).ok_or_else(|| {
                    IonError::type_err("bytes.contains() requires an int".to_string(), line, col)
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
                    IonError::type_err("bytes.find() requires an int".to_string(), line, col)
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
                    IonError::type_err("bytes.push() requires an int".to_string(), line, col)
                })?;
                let mut new = bytes.to_vec();
                new.push(byte as u8);
                Ok(Value::Bytes(new))
            }
            _ => Err(IonError::type_err(
                format!("bytes has no method '{}'", method),
                line,
                col,
            )),
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
        match method {
            "len" => Ok(Value::Int(map.len() as i64)),
            "keys" => {
                let keys: Vec<Value> = map.keys().map(|k| Value::Str(k.clone())).collect();
                Ok(Value::List(keys))
            }
            "values" => {
                let vals: Vec<Value> = map.values().cloned().collect();
                Ok(Value::List(vals))
            }
            "contains_key" => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(map.contains_key(key)))
            }
            "get" => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(match map.get(key) {
                    Some(v) => Value::Option(Some(Box::new(v.clone()))),
                    None => Value::Option(None),
                })
            }
            "is_empty" => Ok(Value::Bool(map.is_empty())),
            "entries" => Ok(Value::List(
                map.iter()
                    .map(|(k, v)| Value::Tuple(vec![Value::Str(k.clone()), v.clone()]))
                    .collect(),
            )),
            "insert" => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let val = args.get(1).cloned().unwrap_or(Value::Unit);
                let mut new_map = map.clone();
                new_map.insert(key.to_string(), val);
                Ok(Value::Dict(new_map))
            }
            "remove" => {
                let key = args.first().and_then(|a| a.as_str()).unwrap_or("");
                let mut new_map = map.clone();
                new_map.shift_remove(key);
                Ok(Value::Dict(new_map))
            }
            "merge" => {
                if let Some(Value::Dict(other)) = args.first() {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        "merge requires a dict argument".to_string(),
                        line,
                        col,
                    ))
                }
            }
            "update" => {
                if let Some(Value::Dict(other)) = args.first() {
                    let mut new_map = map.clone();
                    for (k, v) in other {
                        new_map.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Dict(new_map))
                } else {
                    Err(IonError::type_err(
                        "update requires a dict argument".to_string(),
                        line,
                        col,
                    ))
                }
            }
            "keys_of" => {
                let target = args.first().ok_or_else(|| {
                    IonError::type_err("keys_of requires an argument".to_string(), line, col)
                })?;
                let keys: Vec<Value> = map
                    .iter()
                    .filter(|(_, v)| *v == target)
                    .map(|(k, _)| Value::Str(k.clone()))
                    .collect();
                Ok(Value::List(keys))
            }
            "zip" => {
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
                        "zip requires a dict argument".to_string(),
                        line,
                        col,
                    ))
                }
            }
            _ => Err(IonError::type_err(
                format!("dict has no method '{}'", method),
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
            _ => return Err(IonError::type_err("expected Option", line, col)),
        };
        match method {
            "is_some" => Ok(Value::Bool(opt.is_some())),
            "is_none" => Ok(Value::Bool(opt.is_none())),
            "unwrap" => match opt {
                Some(v) => Ok(*v.clone()),
                None => Err(IonError::runtime(
                    "called unwrap on None".to_string(),
                    line,
                    col,
                )),
            },
            "unwrap_or" => Ok(opt
                .as_ref()
                .map(|v| *v.clone())
                .unwrap_or_else(|| args.first().cloned().unwrap_or(Value::Unit))),
            "expect" => match opt {
                Some(v) => Ok(*v.clone()),
                None => {
                    let msg = args
                        .first()
                        .and_then(|a| a.as_str())
                        .unwrap_or("called expect on None");
                    Err(IonError::runtime(msg.to_string(), line, col))
                }
            },
            _ => Err(IonError::type_err(
                format!("Option has no method '{}'", method),
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
            _ => return Err(IonError::type_err("expected Result", line, col)),
        };
        match method {
            "is_ok" => Ok(Value::Bool(res.is_ok())),
            "is_err" => Ok(Value::Bool(res.is_err())),
            "unwrap" => match res {
                Ok(v) => Ok(*v.clone()),
                Err(e) => Err(IonError::runtime(
                    format!("called unwrap on Err: {}", e),
                    line,
                    col,
                )),
            },
            "unwrap_or" => Ok(match res {
                Ok(v) => *v.clone(),
                Err(_) => args.first().cloned().unwrap_or(Value::Unit),
            }),
            "expect" => match res {
                Ok(v) => Ok(*v.clone()),
                Err(e) => {
                    let msg = args
                        .first()
                        .and_then(|a| a.as_str())
                        .unwrap_or("called expect on Err");
                    Err(IonError::runtime(format!("{}: {}", msg, e), line, col))
                }
            },
            _ => Err(IonError::type_err(
                format!("Result has no method '{}'", method),
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
                Value::BuiltinFn(_name, f) => {
                    let result = f(&args).map_err(|e| IonError::runtime(e, line, col))?;
                    self.stack.push(result);
                    return Ok(());
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
                    self.locals_base = self.locals.len(); // new base for function's locals

                    // Push params as slot-based locals (slots 0..N relative to base)
                    for (i, param) in ion_fn.params.iter().enumerate() {
                        let val = if i < args.len() {
                            args[i].clone()
                        } else if let Some(default) = &param.default {
                            let mut interp = crate::interpreter::Interpreter::new();
                            interp.eval_single_expr(default).unwrap_or(Value::Unit)
                        } else {
                            Value::Unit
                        };
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
                        format!("cannot call {}", func.type_name()),
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
                                        "unknown parameter '{}' for function '{}'",
                                        name, ion_fn.name
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
                let reordered: Vec<Value> = ordered
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| {
                        v.unwrap_or_else(|| {
                            ion_fn
                                .params
                                .get(i)
                                .and_then(|p| p.default.as_ref())
                                .map(|d| {
                                    let mut interp = crate::interpreter::Interpreter::new();
                                    interp.eval_single_expr(d).unwrap_or(Value::Unit)
                                })
                                .unwrap_or(Value::Unit)
                        })
                    })
                    .collect();
                // Push func + reordered args, then call normally
                self.stack.push(func.clone());
                for arg in &reordered {
                    self.stack.push(arg.clone());
                }
                self.call_function(reordered.len(), line, col)
            }
            Value::BuiltinFn(_, f) => {
                // Builtins don't support named args, just pass positionally
                let result = f(&raw_args).map_err(|e| IonError::runtime(e, line, col))?;
                self.stack.push(result);
                Ok(())
            }
            _ => Err(IonError::type_err(
                format!("cannot call {}", func.type_name()),
                line,
                col,
            )),
        }
    }
}
