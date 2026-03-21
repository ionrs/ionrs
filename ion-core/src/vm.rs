//! Stack-based virtual machine for executing Ion bytecode.

use indexmap::IndexMap;

use crate::bytecode::{Chunk, Op};
use crate::env::Env;
use crate::error::IonError;
use crate::value::Value;

/// The Ion virtual machine.
pub struct Vm {
    /// Value stack.
    stack: Vec<Value>,
    /// Environment for variable bindings (reuses existing Env).
    env: Env,
    /// Instruction pointer.
    ip: usize,
    /// Iterator stack for for-loops.
    iterators: Vec<Box<dyn Iterator<Item = Value>>>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(256),
            env: Env::new(),
            ip: 0,
            iterators: Vec::new(),
        }
    }

    /// Create a VM with an existing environment (for engine integration).
    pub fn with_env(env: Env) -> Self {
        Self {
            stack: Vec::with_capacity(256),
            env,
            ip: 0,
            iterators: Vec::new(),
        }
    }

    /// Get a reference to the environment.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Get a mutable reference to the environment.
    pub fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    /// Execute a compiled chunk, returning the final value.
    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, IonError> {
        self.ip = 0;
        self.stack.clear();
        self.run_chunk(chunk)
    }

    /// Run a chunk without resetting state (used for recursive function calls).
    fn run_chunk(&mut self, chunk: &Chunk) -> Result<Value, IonError> {
        while self.ip < chunk.code.len() {
            let op_byte = chunk.code[self.ip];
            let line = chunk.lines[self.ip];
            self.ip += 1;

            let op = self.decode_op(op_byte, line)?;

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
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(self.op_add(a, b, line)?);
                }
                Op::Sub => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(self.op_sub(a, b, line)?);
                }
                Op::Mul => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(self.op_mul(a, b, line)?);
                }
                Op::Div => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(self.op_div(a, b, line)?);
                }
                Op::Mod => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(self.op_mod(a, b, line)?);
                }
                Op::Neg => {
                    let val = self.pop(line)?;
                    self.stack.push(self.op_neg(val, line)?);
                }

                // --- Bitwise ---
                Op::BitAnd => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    match (a, b) {
                        (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x & y)),
                        (a, b) => return Err(IonError::type_err(
                            format!("'&' expects int, got {} and {}", a.type_name(), b.type_name()),
                            line, 0,
                        )),
                    }
                }
                Op::BitOr => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    match (a, b) {
                        (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x | y)),
                        (a, b) => return Err(IonError::type_err(
                            format!("'|' expects int, got {} and {}", a.type_name(), b.type_name()),
                            line, 0,
                        )),
                    }
                }
                Op::BitXor => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    match (a, b) {
                        (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x ^ y)),
                        (a, b) => return Err(IonError::type_err(
                            format!("'^' expects int, got {} and {}", a.type_name(), b.type_name()),
                            line, 0,
                        )),
                    }
                }
                Op::Shl => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    match (a, b) {
                        (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x << y)),
                        (a, b) => return Err(IonError::type_err(
                            format!("'<<' expects int, got {} and {}", a.type_name(), b.type_name()),
                            line, 0,
                        )),
                    }
                }
                Op::Shr => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    match (a, b) {
                        (Value::Int(x), Value::Int(y)) => self.stack.push(Value::Int(x >> y)),
                        (a, b) => return Err(IonError::type_err(
                            format!("'>>' expects int, got {} and {}", a.type_name(), b.type_name()),
                            line, 0,
                        )),
                    }
                }

                // --- Comparison ---
                Op::Eq => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(a == b));
                }
                Op::NotEq => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(a != b));
                }
                Op::Lt => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(self.compare_lt(&a, &b, line)?));
                }
                Op::Gt => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(self.compare_lt(&b, &a, line)?));
                }
                Op::LtEq => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(!self.compare_lt(&b, &a, line)?));
                }
                Op::GtEq => {
                    let b = self.pop(line)?;
                    let a = self.pop(line)?;
                    self.stack.push(Value::Bool(!self.compare_lt(&a, &b, line)?));
                }

                // --- Logic ---
                Op::Not => {
                    let val = self.pop(line)?;
                    self.stack.push(Value::Bool(!val.is_truthy()));
                }
                Op::And => {
                    let offset = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let top = self.peek(line)?;
                    if !top.is_truthy() {
                        self.ip += offset; // short-circuit: keep falsy value
                    }
                    // If truthy, fall through — Pop will remove it, then eval right
                }
                Op::Or => {
                    let offset = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let top = self.peek(line)?;
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
                    let name = self.const_as_str(&chunk.constants[name_idx], line)?;
                    let val = self.pop(line)?;
                    self.env.define(name, val, mutable);
                }
                Op::GetLocal => {
                    let name_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let name = self.const_as_str(&chunk.constants[name_idx], line)?;
                    let val = self.env.get(&name)
                        .cloned()
                        .ok_or_else(|| IonError::name(format!("undefined variable: {}", name), line, 0))?;
                    self.stack.push(val);
                }
                Op::SetLocal => {
                    let name_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let name = self.const_as_str(&chunk.constants[name_idx], line)?;
                    let val = self.pop(line)?;
                    self.env.set(&name, val.clone())
                        .map_err(|e| IonError::runtime(e, line, 0))?;
                    self.stack.push(val); // assignment is an expression
                }
                Op::GetGlobal => {
                    let name_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let name = self.const_as_str(&chunk.constants[name_idx], line)?;
                    let val = self.env.get(&name)
                        .cloned()
                        .ok_or_else(|| IonError::name(format!("undefined variable: {}", name), line, 0))?;
                    self.stack.push(val);
                }
                Op::SetGlobal => {
                    let name_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let name = self.const_as_str(&chunk.constants[name_idx], line)?;
                    let val = self.pop(line)?;
                    self.env.set(&name, val.clone())
                        .map_err(|e| IonError::runtime(e, line, 0))?;
                    self.stack.push(val);
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
                    let val = self.peek(line)?;
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
                    self.call_function(arg_count, line)?;
                }
                Op::Return => {
                    // Return the top of stack value
                    let val = if self.stack.is_empty() {
                        Value::Unit
                    } else {
                        self.pop(line)?
                    };
                    return Ok(val);
                }

                // --- Stack ---
                Op::Pop => {
                    self.pop(line)?;
                }
                Op::Dup => {
                    let val = self.peek(line)?;
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
                    let field = self.const_as_str(&chunk.constants[field_idx], line)?;
                    let obj = self.pop(line)?;
                    self.stack.push(self.get_field(obj, &field, line)?);
                }
                Op::GetIndex => {
                    let index = self.pop(line)?;
                    let obj = self.pop(line)?;
                    self.stack.push(self.get_index(obj, index, line)?);
                }
                Op::SetField => {
                    let field_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let field = self.const_as_str(&chunk.constants[field_idx], line)?;
                    let value = self.pop(line)?;
                    let obj = self.pop(line)?;
                    let result = self.set_field(obj, &field, value, line)?;
                    self.stack.push(result);
                }
                Op::SetIndex => {
                    let value = self.pop(line)?;
                    let index = self.pop(line)?;
                    let obj = self.pop(line)?;
                    let result = self.set_index(obj, index, value, line)?;
                    self.stack.push(result);
                }
                Op::MethodCall => {
                    let method_idx = chunk.read_u16(self.ip) as usize;
                    self.ip += 2;
                    let arg_count = chunk.read_u8(self.ip) as usize;
                    self.ip += 1;
                    let method = self.const_as_str(&chunk.constants[method_idx], line)?;
                    // Stack: [..., receiver, arg0, arg1, ...]
                    let start = self.stack.len() - arg_count;
                    let args: Vec<Value> = self.stack.drain(start..).collect();
                    let receiver = self.pop(line)?;
                    let result = self.call_method(receiver, &method, &args, line)?;
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
                    let val = self.pop(line)?;
                    self.stack.push(Value::Option(Some(Box::new(val))));
                }
                Op::WrapOk => {
                    let val = self.pop(line)?;
                    self.stack.push(Value::Result(Ok(Box::new(val))));
                }
                Op::WrapErr => {
                    let val = self.pop(line)?;
                    self.stack.push(Value::Result(Err(Box::new(val))));
                }
                Op::Try => {
                    let val = self.pop(line)?;
                    match val {
                        Value::Option(Some(v)) => self.stack.push(*v),
                        Value::Option(None) => {
                            return Err(IonError::propagated_none(line, 0));
                        }
                        Value::Result(Ok(v)) => self.stack.push(*v),
                        Value::Result(Err(e)) => {
                            return Err(IonError::propagated_err(e.to_string(), line, 0));
                        }
                        other => {
                            return Err(IonError::type_err(
                                format!("? operator requires Option or Result, got {}", other.type_name()),
                                line, 0,
                            ));
                        }
                    }
                }

                // --- Scope ---
                Op::PushScope => {
                    self.env.push_scope();
                }
                Op::PopScope => {
                    self.env.pop_scope();
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
                    return Err(IonError::runtime("pipe opcode should not be executed directly", line, 0));
                }

                // --- Pattern matching ---
                Op::MatchBegin => {
                    // u8: kind (1=Some, 2=Ok, 3=Err, 4=Tuple)
                    let kind = chunk.read_u8(self.ip);
                    self.ip += 1;
                    let val = self.pop(line)?;
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
                        _ => false,
                    };
                    // Push value back (needed for unwrap) and then bool
                    self.stack.push(val);
                    self.stack.push(Value::Bool(result));
                }
                Op::MatchArm => {
                    // u8: kind (1=unwrap Some, 2=unwrap Ok, 3=unwrap Err, 4=get tuple element)
                    let kind = chunk.read_u8(self.ip);
                    self.ip += 1;
                    match kind {
                        1 => {
                            // Unwrap Some: pop Option(Some(v)), push v
                            let val = self.pop(line)?;
                            match val {
                                Value::Option(Some(v)) => self.stack.push(*v),
                                other => self.stack.push(other), // shouldn't happen
                            }
                        }
                        2 => {
                            let val = self.pop(line)?;
                            match val {
                                Value::Result(Ok(v)) => self.stack.push(*v),
                                other => self.stack.push(other),
                            }
                        }
                        3 => {
                            let val = self.pop(line)?;
                            match val {
                                Value::Result(Err(v)) => self.stack.push(*v),
                                other => self.stack.push(other),
                            }
                        }
                        4 => {
                            // Get tuple element: u8 index follows
                            let idx = chunk.read_u8(self.ip) as usize;
                            self.ip += 1;
                            // Peek at the value on top (don't pop — may need more elements)
                            let val = self.peek(line)?;
                            match val {
                                Value::Tuple(items) => {
                                    self.stack.push(items.get(idx).cloned().unwrap_or(Value::Unit));
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
                    let end = self.pop(line)?;
                    let start = self.pop(line)?;
                    let s = start.as_int().ok_or_else(|| IonError::type_err("range start must be int", line, 0))?;
                    let e = end.as_int().ok_or_else(|| IonError::type_err("range end must be int", line, 0))?;
                    let items: Vec<Value> = if inclusive {
                        (s..=e).map(Value::Int).collect()
                    } else {
                        (s..e).map(Value::Int).collect()
                    };
                    self.stack.push(Value::List(items));
                }

                // --- Host types ---
                Op::ConstructStruct | Op::ConstructEnum => {
                    return Err(IonError::runtime("host types not yet supported in bytecode VM", line, 0));
                }

                // --- Comprehensions ---
                Op::IterInit => {
                    let val = self.pop(line)?;
                    let iter: Box<dyn Iterator<Item = Value>> = match val {
                        Value::List(items) => Box::new(items.into_iter()),
                        Value::Tuple(items) => Box::new(items.into_iter()),
                        Value::Dict(map) => Box::new(
                            map.into_iter()
                                .map(|(k, v)| Value::Tuple(vec![Value::Str(k), v]))
                        ),
                        Value::Str(s) => {
                            let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                            Box::new(chars.into_iter())
                        }
                        Value::Bytes(bytes) => {
                            let vals: Vec<Value> = bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
                            Box::new(vals.into_iter())
                        }
                        other => {
                            return Err(IonError::type_err(
                                format!("cannot iterate over {}", other.type_name()),
                                line, 0,
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
                    self.pop(line)?;
                    let iter = self.iterators.last_mut()
                        .ok_or_else(|| IonError::runtime("no active iterator", line, 0))?;
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
                Op::ListAppend => {
                    // Stack: [..., list, iter_placeholder, ..., item]
                    // Pop item, find the list deeper in the stack, append to it
                    let item = self.pop(line)?;
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
                        return Err(IonError::runtime("ListAppend: no list on stack", line, 0));
                    }
                }
                Op::DictInsert => {
                    // Stack: [..., dict, iter_placeholder, ..., key, value]
                    let value = self.pop(line)?;
                    let key = self.pop(line)?;
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
                        return Err(IonError::runtime("DictInsert: no dict on stack", line, 0));
                    }
                }

                // --- Slice ---
                Op::Slice => {
                    let flags = chunk.read_u8(self.ip);
                    self.ip += 1;
                    let has_start = flags & 1 != 0;
                    let has_end = flags & 2 != 0;
                    let inclusive = flags & 4 != 0;
                    let end_val = if has_end { Some(self.pop(line)?) } else { None };
                    let start_val = if has_start { Some(self.pop(line)?) } else { None };
                    let obj = self.pop(line)?;
                    let result = self.slice_access(obj, start_val, end_val, inclusive, line)?;
                    self.stack.push(result);
                }

                // --- Print ---
                Op::Print => {
                    let newline = chunk.read_u8(self.ip) != 0;
                    self.ip += 1;
                    let val = self.pop(line)?;
                    if newline {
                        println!("{}", val);
                    } else {
                        print!("{}", val);
                    }
                    self.stack.push(Value::Unit);
                }
            }
        }

        // If we reach the end without Return, return the top of stack or Unit
        Ok(self.stack.pop().unwrap_or(Value::Unit))
    }

    // ---- Helpers ----

    fn decode_op(&self, byte: u8, line: usize) -> Result<Op, IonError> {
        if byte > Op::Print as u8 {
            return Err(IonError::runtime(format!("invalid opcode: {}", byte), line, 0));
        }
        // SAFETY: Op is repr(u8) and we checked the range
        Ok(unsafe { std::mem::transmute(byte) })
    }

    fn slice_access(&self, obj: Value, start: Option<Value>, end: Option<Value>, inclusive: bool, line: usize) -> Result<Value, IonError> {
        let get_idx = |v: Option<Value>, default: i64| -> Result<i64, IonError> {
            match v {
                Some(Value::Int(n)) => Ok(n),
                None => Ok(default),
                Some(other) => Err(IonError::type_err(
                    format!("slice index must be int, got {}", other.type_name()), line, 0,
                )),
            }
        };
        match &obj {
            Value::List(items) => {
                let len = items.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive { (e_raw + 1).max(0).min(len) as usize } else { e_raw.max(0).min(len) as usize };
                Ok(Value::List(items[s..e].to_vec()))
            }
            Value::Str(string) => {
                let chars: Vec<char> = string.chars().collect();
                let len = chars.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive { (e_raw + 1).max(0).min(len) as usize } else { e_raw.max(0).min(len) as usize };
                Ok(Value::Str(chars[s..e].iter().collect()))
            }
            Value::Bytes(bytes) => {
                let len = bytes.len() as i64;
                let s = get_idx(start, 0)?.max(0).min(len) as usize;
                let e_raw = get_idx(end, len)?;
                let e = if inclusive { (e_raw + 1).max(0).min(len) as usize } else { e_raw.max(0).min(len) as usize };
                Ok(Value::Bytes(bytes[s..e].to_vec()))
            }
            _ => Err(IonError::type_err(
                format!("cannot slice {}", obj.type_name()), line, 0,
            )),
        }
    }

    fn pop(&mut self, line: usize) -> Result<Value, IonError> {
        self.stack.pop()
            .ok_or_else(|| IonError::runtime("stack underflow", line, 0))
    }

    fn peek(&self, line: usize) -> Result<Value, IonError> {
        self.stack.last()
            .cloned()
            .ok_or_else(|| IonError::runtime("stack underflow (peek)", line, 0))
    }

    fn const_as_str(&self, val: &Value, line: usize) -> Result<String, IonError> {
        match val {
            Value::Str(s) => Ok(s.clone()),
            _ => Err(IonError::runtime("expected string constant", line, 0)),
        }
    }

    // ---- Arithmetic ----

    fn op_add(&self, a: Value, b: Value, line: usize) -> Result<Value, IonError> {
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
                line, 0,
            )),
        }
    }

    fn op_sub(&self, a: Value, b: Value, line: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x - y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x - y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 - y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x - *y as f64)),
            _ => Err(IonError::type_err(
                format!("cannot subtract {} from {}", b.type_name(), a.type_name()),
                line, 0,
            )),
        }
    }

    fn op_mul(&self, a: Value, b: Value, line: usize) -> Result<Value, IonError> {
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
                line, 0,
            )),
        }
    }

    fn op_div(&self, a: Value, b: Value, line: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("division by zero", line, 0)),
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x / y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x / y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 / y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x / *y as f64)),
            _ => Err(IonError::type_err(
                format!("cannot divide {} by {}", a.type_name(), b.type_name()),
                line, 0,
            )),
        }
    }

    fn op_mod(&self, a: Value, b: Value, line: usize) -> Result<Value, IonError> {
        match (&a, &b) {
            (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("modulo by zero", line, 0)),
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x % y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 % y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x % *y as f64)),
            _ => Err(IonError::type_err(
                format!("cannot modulo {} by {}", a.type_name(), b.type_name()),
                line, 0,
            )),
        }
    }

    fn op_neg(&self, val: Value, line: usize) -> Result<Value, IonError> {
        match val {
            Value::Int(n) => Ok(Value::Int(-n)),
            Value::Float(n) => Ok(Value::Float(-n)),
            _ => Err(IonError::type_err(
                format!("cannot negate {}", val.type_name()),
                line, 0,
            )),
        }
    }

    fn compare_lt(&self, a: &Value, b: &Value, line: usize) -> Result<bool, IonError> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x < y),
            (Value::Float(x), Value::Float(y)) => Ok(x < y),
            (Value::Int(x), Value::Float(y)) => Ok((*x as f64) < *y),
            (Value::Float(x), Value::Int(y)) => Ok(*x < (*y as f64)),
            (Value::Str(x), Value::Str(y)) => Ok(x < y),
            _ => Err(IonError::type_err(
                format!("cannot compare {} and {}", a.type_name(), b.type_name()),
                line, 0,
            )),
        }
    }

    // ---- Field/Index access ----

    fn get_field(&self, obj: Value, field: &str, line: usize) -> Result<Value, IonError> {
        match &obj {
            Value::Dict(map) => {
                map.get(field).cloned()
                    .ok_or_else(|| IonError::runtime(format!("key '{}' not found in dict", field), line, 0))
            }
            Value::HostStruct { fields, .. } => {
                fields.get(field).cloned()
                    .ok_or_else(|| IonError::runtime(format!("field '{}' not found", field), line, 0))
            }
            Value::List(items) => {
                match field {
                    "len" => Ok(Value::Int(items.len() as i64)),
                    _ => Err(IonError::runtime(format!("list has no field '{}'", field), line, 0)),
                }
            }
            Value::Str(s) => {
                match field {
                    "len" => Ok(Value::Int(s.len() as i64)),
                    _ => Err(IonError::runtime(format!("string has no field '{}'", field), line, 0)),
                }
            }
            Value::Tuple(items) => {
                match field {
                    "len" => Ok(Value::Int(items.len() as i64)),
                    _ => Err(IonError::runtime(format!("tuple has no field '{}'", field), line, 0)),
                }
            }
            _ => Err(IonError::type_err(
                format!("cannot access field '{}' on {}", field, obj.type_name()),
                line, 0,
            )),
        }
    }

    fn get_index(&self, obj: Value, index: Value, line: usize) -> Result<Value, IonError> {
        match (&obj, &index) {
            (Value::List(items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(idx).cloned()
                    .ok_or_else(|| IonError::runtime(format!("index {} out of range", i), line, 0))
            }
            (Value::Tuple(items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                items.get(idx).cloned()
                    .ok_or_else(|| IonError::runtime(format!("index {} out of range", i), line, 0))
            }
            (Value::Dict(map), Value::Str(key)) => {
                map.get(key).cloned()
                    .ok_or_else(|| IonError::runtime(format!("key '{}' not found", key), line, 0))
            }
            (Value::Str(s), Value::Int(i)) => {
                let idx = if *i < 0 { s.len() as i64 + i } else { *i } as usize;
                s.chars().nth(idx)
                    .map(|c| Value::Str(c.to_string()))
                    .ok_or_else(|| IonError::runtime(format!("index {} out of range", i), line, 0))
            }
            (Value::Bytes(bytes), Value::Int(i)) => {
                let idx = if *i < 0 { bytes.len() as i64 + i } else { *i } as usize;
                bytes.get(idx)
                    .map(|&b| Value::Int(b as i64))
                    .ok_or_else(|| IonError::runtime(format!("index {} out of range", i), line, 0))
            }
            _ => Err(IonError::type_err(
                format!("cannot index {} with {}", obj.type_name(), index.type_name()),
                line, 0,
            )),
        }
    }

    /// Set index on a container, returning the modified container.
    fn set_index(&self, obj: Value, index: Value, value: Value, line: usize) -> Result<Value, IonError> {
        match (obj, &index) {
            (Value::List(mut items), Value::Int(i)) => {
                let idx = if *i < 0 { items.len() as i64 + i } else { *i } as usize;
                if idx >= items.len() {
                    return Err(IonError::runtime(format!("index {} out of range", i), line, 0));
                }
                items[idx] = value;
                Ok(Value::List(items))
            }
            (Value::Dict(mut map), Value::Str(key)) => {
                map.insert(key.clone(), value);
                Ok(Value::Dict(map))
            }
            (obj, _) => Err(IonError::type_err(
                format!("cannot set index on {}", obj.type_name()), line, 0,
            )),
        }
    }

    /// Set field on an object, returning the modified object.
    fn set_field(&self, obj: Value, field: &str, value: Value, line: usize) -> Result<Value, IonError> {
        match obj {
            Value::Dict(mut map) => {
                map.insert(field.to_string(), value);
                Ok(Value::Dict(map))
            }
            Value::HostStruct { type_name, mut fields } => {
                if fields.contains_key(field) {
                    fields.insert(field.to_string(), value);
                    Ok(Value::HostStruct { type_name, fields })
                } else {
                    Err(IonError::runtime(format!("field '{}' not found on {}", field, type_name), line, 0))
                }
            }
            _ => Err(IonError::type_err(
                format!("cannot set field on {}", obj.type_name()), line, 0,
            )),
        }
    }

    // ---- Method calls ----

    fn call_method(&mut self, receiver: Value, method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        match &receiver {
            Value::List(items) => self.list_method(items, method, args, line),
            Value::Str(s) => self.str_method(s, method, args, line),
            Value::Dict(map) => self.dict_method(map, method, args, line),
            Value::Bytes(b) => self.bytes_method(b, method, args, line),
            Value::Option(_) => self.option_method(&receiver, method, args, line),
            Value::Result(_) => self.result_method(&receiver, method, args, line),
            _ => Err(IonError::type_err(
                format!("{} has no method '{}'", receiver.type_name(), method),
                line, 0,
            )),
        }
    }

    fn list_method(&self, items: &[Value], method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        match method {
            "len" => Ok(Value::Int(items.len() as i64)),
            "push" => {
                let mut new = items.to_vec();
                for a in args { new.push(a.clone()); }
                Ok(Value::List(new))
            }
            "pop" => {
                let mut new = items.to_vec();
                let val = new.pop().unwrap_or(Value::Unit);
                Ok(val)
            }
            "contains" => {
                Ok(Value::Bool(args.first().map(|a| items.contains(a)).unwrap_or(false)))
            }
            "is_empty" => Ok(Value::Bool(items.is_empty())),
            "reverse" => {
                let mut new = items.to_vec();
                new.reverse();
                Ok(Value::List(new))
            }
            "join" => {
                let sep = args.first()
                    .and_then(|a| a.as_str())
                    .unwrap_or("");
                let s: String = items.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(sep);
                Ok(Value::Str(s))
            }
            "enumerate" => {
                let pairs: Vec<Value> = items.iter().enumerate()
                    .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                    .collect();
                Ok(Value::List(pairs))
            }
            _ => Err(IonError::type_err(format!("list has no method '{}'", method), line, 0)),
        }
    }

    fn str_method(&self, s: &str, method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        match method {
            "len" => Ok(Value::Int(s.len() as i64)),
            "to_upper" => Ok(Value::Str(s.to_uppercase())),
            "to_lower" => Ok(Value::Str(s.to_lowercase())),
            "trim" => Ok(Value::Str(s.trim().to_string())),
            "contains" => {
                let sub = args.first().and_then(|a| a.as_str()).unwrap_or("");
                Ok(Value::Bool(s.contains(sub)))
            }
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
            "is_empty" => Ok(Value::Bool(s.is_empty())),
            _ => Err(IonError::type_err(format!("string has no method '{}'", method), line, 0)),
        }
    }

    fn bytes_method(&self, bytes: &[u8], method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        match method {
            "len" => Ok(Value::Int(bytes.len() as i64)),
            "is_empty" => Ok(Value::Bool(bytes.is_empty())),
            "contains" => {
                let byte = args.first().and_then(|a| a.as_int())
                    .ok_or_else(|| IonError::type_err("bytes.contains() requires an int".to_string(), line, 0))?;
                Ok(Value::Bool(bytes.contains(&(byte as u8))))
            }
            "slice" => {
                let start = args.first().and_then(|a| a.as_int()).unwrap_or(0) as usize;
                let end = args.get(1).and_then(|a| a.as_int()).map(|n| n as usize).unwrap_or(bytes.len());
                let start = start.min(bytes.len());
                let end = end.min(bytes.len());
                Ok(Value::Bytes(bytes[start..end].to_vec()))
            }
            "to_list" => Ok(Value::List(bytes.iter().map(|&b| Value::Int(b as i64)).collect())),
            "to_str" => {
                match std::str::from_utf8(bytes) {
                    std::result::Result::Ok(s) => Ok(Value::Result(Ok(Box::new(Value::Str(s.to_string()))))),
                    std::result::Result::Err(e) => Ok(Value::Result(Err(Box::new(Value::Str(format!("{}", e)))))),
                }
            }
            "to_hex" => {
                let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                Ok(Value::Str(hex))
            }
            "find" => {
                let needle = args.first().and_then(|a| a.as_int())
                    .ok_or_else(|| IonError::type_err("bytes.find() requires an int".to_string(), line, 0))?;
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
                let byte = args.first().and_then(|a| a.as_int())
                    .ok_or_else(|| IonError::type_err("bytes.push() requires an int".to_string(), line, 0))?;
                let mut new = bytes.to_vec();
                new.push(byte as u8);
                Ok(Value::Bytes(new))
            }
            _ => Err(IonError::type_err(format!("bytes has no method '{}'", method), line, 0)),
        }
    }

    fn dict_method(&self, map: &IndexMap<String, Value>, method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
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
                let default = args.get(1).cloned().unwrap_or(Value::Option(None));
                Ok(map.get(key).cloned().unwrap_or(default))
            }
            "is_empty" => Ok(Value::Bool(map.is_empty())),
            _ => Err(IonError::type_err(format!("dict has no method '{}'", method), line, 0)),
        }
    }

    fn option_method(&self, val: &Value, method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        let opt = match val {
            Value::Option(o) => o,
            _ => return Err(IonError::type_err("expected Option", line, 0)),
        };
        match method {
            "is_some" => Ok(Value::Bool(opt.is_some())),
            "is_none" => Ok(Value::Bool(opt.is_none())),
            "unwrap_or" => {
                Ok(opt.as_ref().map(|v| *v.clone()).unwrap_or_else(|| {
                    args.first().cloned().unwrap_or(Value::Unit)
                }))
            }
            "expect" => {
                match opt {
                    Some(v) => Ok(*v.clone()),
                    None => {
                        let msg = args.first().and_then(|a| a.as_str()).unwrap_or("called expect on None");
                        Err(IonError::runtime(msg.to_string(), line, 0))
                    }
                }
            }
            // Closure-based methods require tree-walk fallback; the hybrid engine handles this.
            "map" | "and_then" | "or_else" | "unwrap_or_else" => {
                Err(IonError::runtime(
                    format!("Option.{} requires tree-walk fallback", method),
                    line, 0,
                ))
            }
            _ => Err(IonError::type_err(format!("Option has no method '{}'", method), line, 0)),
        }
    }

    fn result_method(&self, val: &Value, method: &str, args: &[Value], line: usize) -> Result<Value, IonError> {
        let res = match val {
            Value::Result(r) => r,
            _ => return Err(IonError::type_err("expected Result", line, 0)),
        };
        match method {
            "is_ok" => Ok(Value::Bool(res.is_ok())),
            "is_err" => Ok(Value::Bool(res.is_err())),
            "unwrap_or" => {
                Ok(match res {
                    Ok(v) => *v.clone(),
                    Err(_) => args.first().cloned().unwrap_or(Value::Unit),
                })
            }
            "expect" => {
                match res {
                    Ok(v) => Ok(*v.clone()),
                    Err(e) => {
                        let msg = args.first().and_then(|a| a.as_str())
                            .unwrap_or("called expect on Err");
                        Err(IonError::runtime(format!("{}: {}", msg, e), line, 0))
                    }
                }
            }
            // Closure-based methods require tree-walk fallback; the hybrid engine handles this.
            "map" | "map_err" | "and_then" | "or_else" | "unwrap_or_else" => {
                Err(IonError::runtime(
                    format!("Result.{} requires tree-walk fallback", method),
                    line, 0,
                ))
            }
            _ => Err(IonError::type_err(format!("Result has no method '{}'", method), line, 0)),
        }
    }

    // ---- Function calls ----

    fn call_function(&mut self, arg_count: usize, line: usize) -> Result<(), IonError> {
        // Stack: [..., func, arg0, arg1, ..., argN-1]
        // But we pushed func first, then args
        let args_start = self.stack.len() - arg_count;
        let func_idx = args_start - 1;
        let func = self.stack[func_idx].clone();
        let args: Vec<Value> = self.stack[args_start..].to_vec();
        // Remove func + args from stack
        self.stack.truncate(func_idx);

        match func {
            Value::BuiltinFn(_name, f) => {
                let result = f(&args).map_err(|e| IonError::runtime(e, line, 0))?;
                self.stack.push(result);
            }
            Value::Fn(ion_fn) => {
                self.env.push_scope();

                // Bind captures
                for (name, val) in &ion_fn.captures {
                    self.env.define(name.clone(), val.clone(), false);
                }

                // Bind parameters
                for (i, param) in ion_fn.params.iter().enumerate() {
                    let val = if i < args.len() {
                        args[i].clone()
                    } else if let Some(default) = &param.default {
                        let mut interp = crate::interpreter::Interpreter::new();
                        interp.eval_single_expr(default).unwrap_or(Value::Unit)
                    } else {
                        Value::Unit
                    };
                    self.env.define(param.name.clone(), val, false);
                }

                // Try to compile and execute function body as bytecode
                let compiler = crate::compiler::Compiler::new();
                match compiler.compile_fn_body(&ion_fn.body, line) {
                    Ok(chunk) => {
                        let saved_ip = self.ip;
                        let saved_iters = std::mem::take(&mut self.iterators);
                        self.ip = 0;
                        let result = self.run_chunk(&chunk);
                        self.ip = saved_ip;
                        self.iterators = saved_iters;
                        self.env.pop_scope();
                        match result {
                            Ok(val) => self.stack.push(val),
                            Err(e) => return Err(e),
                        }
                    }
                    Err(_) => {
                        // Fallback to tree-walk for complex function bodies
                        let mut interp = crate::interpreter::Interpreter::with_env(self.env.clone());
                        let result = interp.eval_block(&ion_fn.body);
                        self.env = interp.take_env();
                        self.env.pop_scope();
                        match result {
                            Ok(val) => self.stack.push(val),
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            _ => {
                return Err(IonError::type_err(
                    format!("cannot call {}", func.type_name()),
                    line, 0,
                ));
            }
        }
        Ok(())
    }
}
