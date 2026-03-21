//! Bytecode instruction set and chunk representation for the Ion VM.

use crate::value::Value;

/// VM opcodes — each is a single byte.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Op {
    /// Push a constant from the constant pool onto the stack.
    Constant,       // u16 index
    /// Push common values without a constant pool lookup.
    True,
    False,
    Unit,
    None,

    // --- Arithmetic ---
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,

    // --- Bitwise ---
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,

    // --- Comparison ---
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // --- Logic ---
    Not,
    And,            // u16 jump offset (short-circuit)
    Or,             // u16 jump offset (short-circuit)

    // --- Variables ---
    /// Define a variable in the current scope.
    DefineLocal,    // u16 name constant index, u8 mutable flag
    /// Get a local variable by name.
    GetLocal,       // u16 name constant index
    /// Set a local variable by name.
    SetLocal,       // u16 name constant index
    /// Get a global/captured variable by name.
    GetGlobal,      // u16 name constant index
    /// Set a global variable.
    SetGlobal,      // u16 name constant index

    // --- Control flow ---
    /// Unconditional jump forward.
    Jump,           // u16 offset
    /// Jump forward if top of stack is falsy (pops condition).
    JumpIfFalse,    // u16 offset
    /// Jump backward (for loops).
    Loop,           // u16 offset back

    // --- Functions ---
    /// Call a function: pops func + args from stack.
    Call,           // u8 arg count
    /// Tail call: like Call but reuses the current frame (no stack growth).
    TailCall,       // u8 arg count
    /// Return from current function.
    Return,

    // --- Stack manipulation ---
    /// Pop and discard the top value.
    Pop,
    /// Duplicate the top value.
    Dup,

    // --- Composite types ---
    /// Build a list from N values on stack.
    BuildList,      // u16 count
    /// Build a tuple from N values on stack.
    BuildTuple,     // u16 count
    /// Build a dict from N key-value pairs on stack.
    BuildDict,      // u16 count (number of pairs)

    // --- Field/index access ---
    /// Get field: pop object, push object.field.
    GetField,       // u16 field name constant index
    /// Index access: pop index, pop object, push object[index].
    GetIndex,
    /// Set field: pop value, pop object, mutate, push value.
    SetField,       // u16 field name constant index
    /// Set index: pop value, pop index, pop object, mutate, push value.
    SetIndex,
    /// Method call: pop args + receiver, push result.
    MethodCall,     // u16 method name constant index, u8 arg count

    // --- Closures ---
    /// Create a closure from a function prototype.
    Closure,        // u16 function constant index

    // --- Option/Result ---
    WrapSome,
    WrapOk,
    WrapErr,
    /// Try operator (?): unwrap Ok/Some or propagate Err/None.
    Try,

    // --- Scope ---
    PushScope,
    PopScope,

    // --- String ---
    /// Build an f-string from N parts on stack.
    BuildFString,   // u16 part count

    // --- Pipe ---
    /// Pipe operator: rearranges stack for function call.
    Pipe,           // u8 arg count (always 1 extra)

    // --- Pattern matching ---
    /// Begin a match expression.
    MatchBegin,
    /// Test a pattern against the match subject.
    MatchArm,       // u16 jump offset if no match
    /// End match (cleanup).
    MatchEnd,

    // --- Range ---
    BuildRange,     // u8: 0 = exclusive, 1 = inclusive

    // --- Host types ---
    ConstructStruct,  // u16 type name index, u16 field count
    ConstructEnum,    // u16 enum name index, u16 variant name index, u8 arg count

    // --- Comprehensions ---
    /// List comprehension iteration setup
    IterInit,
    IterNext,       // u16 jump offset when exhausted
    ListAppend,
    DictInsert,
    /// Merge a dict into the dict below it on the stack (for spread).
    DictMerge,

    /// Slice access: pop end (or sentinel), pop start (or sentinel), pop object, push slice.
    Slice,          // u8: flags (bit 0 = has_start, bit 1 = has_end, bit 2 = inclusive)

    // --- Stack-slot locals (fast path) ---
    /// Define a local in the slot array.
    DefineLocalSlot,  // u8 mutable flag
    /// Get a local by slot index (relative to current frame base).
    GetLocalSlot,     // u16 slot index
    /// Set a local by slot index.
    SetLocalSlot,     // u16 slot index

    /// Print (for testing/debugging)
    Print,          // u8: 0 = print, 1 = println
}

/// A compiled bytecode chunk.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The bytecode instructions.
    pub code: Vec<u8>,
    /// Constant pool.
    pub constants: Vec<Value>,
    /// Line number for each instruction (for error reporting).
    pub lines: Vec<usize>,
    /// Column number for each instruction (for error reporting).
    pub cols: Vec<usize>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
            cols: Vec::new(),
        }
    }

    /// Emit a single byte with source location.
    pub fn emit(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
        self.cols.push(0);
    }

    /// Emit a single byte with full source span (line + col).
    pub fn emit_span(&mut self, byte: u8, line: usize, col: usize) {
        self.code.push(byte);
        self.lines.push(line);
        self.cols.push(col);
    }

    /// Emit an opcode.
    pub fn emit_op(&mut self, op: Op, line: usize) {
        self.emit(op as u8, line);
    }

    /// Emit an opcode with full span.
    pub fn emit_op_span(&mut self, op: Op, line: usize, col: usize) {
        self.emit_span(op as u8, line, col);
    }

    /// Emit an opcode followed by a u16 operand.
    pub fn emit_op_u16(&mut self, op: Op, operand: u16, line: usize) {
        self.emit(op as u8, line);
        self.emit((operand >> 8) as u8, line);
        self.emit((operand & 0xff) as u8, line);
    }

    /// Emit an opcode followed by a u16 operand with full span.
    pub fn emit_op_u16_span(&mut self, op: Op, operand: u16, line: usize, col: usize) {
        self.emit_span(op as u8, line, col);
        self.emit_span((operand >> 8) as u8, line, col);
        self.emit_span((operand & 0xff) as u8, line, col);
    }

    /// Emit an opcode followed by a u8 operand.
    pub fn emit_op_u8(&mut self, op: Op, operand: u8, line: usize) {
        self.emit(op as u8, line);
        self.emit(operand, line);
    }

    /// Emit an opcode followed by a u8 operand with full span.
    pub fn emit_op_u8_span(&mut self, op: Op, operand: u8, line: usize, col: usize) {
        self.emit_span(op as u8, line, col);
        self.emit_span(operand, line, col);
    }

    /// Add a constant to the pool, returning its index.
    /// Deduplicates string constants (used for variable names).
    pub fn add_constant(&mut self, value: Value) -> u16 {
        // Deduplicate string constants for variable name lookups
        if let Value::Str(ref s) = value {
            for (i, c) in self.constants.iter().enumerate() {
                if let Value::Str(ref cs) = c {
                    if cs == s {
                        return i as u16;
                    }
                }
            }
        }
        self.constants.push(value);
        (self.constants.len() - 1) as u16
    }

    /// Emit a constant load instruction.
    pub fn emit_constant(&mut self, value: Value, line: usize) {
        let idx = self.add_constant(value);
        self.emit_op_u16(Op::Constant, idx, line);
    }

    /// Current code offset.
    pub fn len(&self) -> usize {
        self.code.len()
    }

    /// Emit a jump instruction, returning the offset to patch later.
    pub fn emit_jump(&mut self, op: Op, line: usize) -> usize {
        self.emit_op_u16(op, 0xffff, line);
        self.code.len() - 2 // offset of the u16 placeholder
    }

    /// Patch a previously emitted jump to target the current position.
    pub fn patch_jump(&mut self, offset: usize) {
        let jump = self.code.len() - offset - 2;
        self.code[offset] = (jump >> 8) as u8;
        self.code[offset + 1] = (jump & 0xff) as u8;
    }

    /// Read a u16 operand at the given offset.
    pub fn read_u16(&self, offset: usize) -> u16 {
        ((self.code[offset] as u16) << 8) | (self.code[offset + 1] as u16)
    }

    /// Read a u8 operand at the given offset.
    pub fn read_u8(&self, offset: usize) -> u8 {
        self.code[offset]
    }

    /// Post-pass: replace `Call N; Return` with `TailCall N; Return`.
    #[allow(dead_code)]
    pub fn optimize_tail_calls(&mut self) {
        let call_byte = Op::Call as u8;
        let return_byte = Op::Return as u8;
        let tail_call_byte = Op::TailCall as u8;
        // Call is 2 bytes (opcode + u8 arg_count), Return is 1 byte
        let mut i = 0;
        while i + 2 < self.code.len() {
            if self.code[i] == call_byte && self.code[i + 2] == return_byte {
                self.code[i] = tail_call_byte;
            }
            i += 1;
        }
    }
}

/// A compiled function prototype (stored in the constant pool).
#[derive(Debug, Clone)]
pub struct FnProto {
    pub name: String,
    pub arity: usize,
    pub chunk: Chunk,
    pub param_names: Vec<String>,
    pub has_defaults: Vec<bool>,
}
