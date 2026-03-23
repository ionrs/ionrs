//! Bytecode instruction set and chunk representation for the Ion VM.

use crate::value::Value;

/// VM opcodes — each is a single byte.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Op {
    /// Push a constant from the constant pool onto the stack.
    Constant, // u16 index
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
    And, // u16 jump offset (short-circuit)
    Or,  // u16 jump offset (short-circuit)

    // --- Variables ---
    /// Define a variable in the current scope.
    DefineLocal, // u16 name constant index, u8 mutable flag
    /// Get a local variable by name.
    GetLocal, // u16 name constant index
    /// Set a local variable by name.
    SetLocal, // u16 name constant index
    /// Get a global/captured variable by name.
    GetGlobal, // u16 name constant index
    /// Set a global variable.
    SetGlobal, // u16 name constant index

    // --- Control flow ---
    /// Unconditional jump forward.
    Jump, // u16 offset
    /// Jump forward if top of stack is falsy (pops condition).
    JumpIfFalse, // u16 offset
    /// Jump backward (for loops).
    Loop, // u16 offset back

    // --- Functions ---
    /// Call a function: pops func + args from stack.
    Call, // u8 arg count
    /// Tail call: like Call but reuses the current frame (no stack growth).
    TailCall, // u8 arg count
    /// Return from current function.
    Return,

    // --- Stack manipulation ---
    /// Pop and discard the top value.
    Pop,
    /// Duplicate the top value.
    Dup,

    // --- Composite types ---
    /// Build a list from N values on stack.
    BuildList, // u16 count
    /// Build a tuple from N values on stack.
    BuildTuple, // u16 count
    /// Build a dict from N key-value pairs on stack.
    BuildDict, // u16 count (number of pairs)

    // --- Field/index access ---
    /// Get field: pop object, push object.field.
    GetField, // u16 field name constant index
    /// Index access: pop index, pop object, push object[index].
    GetIndex,
    /// Set field: pop value, pop object, mutate, push value.
    SetField, // u16 field name constant index
    /// Set index: pop value, pop index, pop object, mutate, push value.
    SetIndex,
    /// Method call: pop args + receiver, push result.
    MethodCall, // u16 method name constant index, u8 arg count

    // --- Closures ---
    /// Create a closure from a function prototype.
    Closure, // u16 function constant index

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
    BuildFString, // u16 part count

    // --- Pipe ---
    /// Pipe operator: rearranges stack for function call.
    Pipe, // u8 arg count (always 1 extra)

    // --- Pattern matching ---
    /// Begin a match expression.
    MatchBegin,
    /// Test a pattern against the match subject.
    MatchArm, // u8 kind (1=Some,2=Ok,3=Err,4=tuple,5=list), +u8 index for 4/5
    /// End match (cleanup).
    MatchEnd,

    // --- Range ---
    BuildRange, // u8: 0 = exclusive, 1 = inclusive

    // --- Host types ---
    ConstructStruct, // u16 type name index, u16 field count
    ConstructEnum,   // u16 enum name index, u16 variant name index, u8 arg count

    // --- Comprehensions ---
    /// List comprehension iteration setup
    IterInit,
    IterNext, // u16 jump offset when exhausted
    ListAppend,
    /// Pop a list from TOS, extend the list below TOS with its items (for spread).
    ListExtend,
    DictInsert,
    /// Merge a dict into the dict below it on the stack (for spread).
    DictMerge,
    /// Drop the current iterator (for break in for-loops).
    IterDrop,
    /// Runtime type check: peek TOS, compare type against constant at u16 index.
    CheckType, // u16: constant index (string type name)

    /// Slice access: pop end (or sentinel), pop start (or sentinel), pop object, push slice.
    Slice, // u8: flags (bit 0 = has_start, bit 1 = has_end, bit 2 = inclusive)

    // --- Stack-slot locals (fast path) ---
    /// Define a local in the slot array.
    DefineLocalSlot, // u8 mutable flag
    /// Get a local by slot index (relative to current frame base).
    GetLocalSlot, // u16 slot index
    /// Set a local by slot index.
    SetLocalSlot, // u16 slot index

    /// Call with named arguments: u8 arg count, then u8 count of named pairs, each is u16 (arg position) + u16 (name constant)
    CallNamed, // u8 total_args, u8 named_count, then [u8 position, u16 name_idx] * named_count

    /// Begin a try block: push exception handler.
    TryBegin, // u16 catch handler offset
    /// End a try block (no error): pop handler, jump over catch.
    TryEnd, // u16 jump offset past catch block

    /// Print (for testing/debugging)
    Print, // u8: 0 = print, 1 = println
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

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
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

    /// Returns true if the chunk contains no bytecode.
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
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

    /// Return the total size (opcode + operands) of the instruction at `offset`.
    pub fn instruction_size(code: &[u8], offset: usize) -> usize {
        if offset >= code.len() {
            return 1;
        }
        match code[offset] {
            // 1-byte (no operands)
            x if x == Op::True as u8
                || x == Op::False as u8
                || x == Op::Unit as u8
                || x == Op::None as u8
                || x == Op::Add as u8
                || x == Op::Sub as u8
                || x == Op::Mul as u8
                || x == Op::Div as u8
                || x == Op::Mod as u8
                || x == Op::Neg as u8
                || x == Op::BitAnd as u8
                || x == Op::BitOr as u8
                || x == Op::BitXor as u8
                || x == Op::Shl as u8
                || x == Op::Shr as u8
                || x == Op::Eq as u8
                || x == Op::NotEq as u8
                || x == Op::Lt as u8
                || x == Op::Gt as u8
                || x == Op::LtEq as u8
                || x == Op::GtEq as u8
                || x == Op::Not as u8
                || x == Op::Pop as u8
                || x == Op::Dup as u8
                || x == Op::GetIndex as u8
                || x == Op::SetIndex as u8
                || x == Op::WrapSome as u8
                || x == Op::WrapOk as u8
                || x == Op::WrapErr as u8
                || x == Op::Try as u8
                || x == Op::PushScope as u8
                || x == Op::PopScope as u8
                || x == Op::MatchEnd as u8
                || x == Op::Return as u8
                || x == Op::IterInit as u8
                || x == Op::ListAppend as u8
                || x == Op::ListExtend as u8
                || x == Op::DictInsert as u8
                || x == Op::DictMerge as u8
                || x == Op::IterDrop as u8 =>
            {
                1
            }
            // 2-byte (u8 operand)
            x if x == Op::Call as u8
                || x == Op::TailCall as u8
                || x == Op::Pipe as u8
                || x == Op::BuildRange as u8
                || x == Op::Slice as u8
                || x == Op::DefineLocalSlot as u8
                || x == Op::Print as u8 =>
            {
                2
            }
            // 3-byte (u16 operand)
            x if x == Op::Constant as u8
                || x == Op::And as u8
                || x == Op::Or as u8
                || x == Op::GetLocal as u8
                || x == Op::SetLocal as u8
                || x == Op::GetGlobal as u8
                || x == Op::SetGlobal as u8
                || x == Op::Jump as u8
                || x == Op::JumpIfFalse as u8
                || x == Op::Loop as u8
                || x == Op::BuildList as u8
                || x == Op::BuildTuple as u8
                || x == Op::BuildDict as u8
                || x == Op::GetField as u8
                || x == Op::SetField as u8
                || x == Op::Closure as u8
                || x == Op::BuildFString as u8
                || x == Op::IterNext as u8
                || x == Op::GetLocalSlot as u8
                || x == Op::SetLocalSlot as u8
                || x == Op::TryBegin as u8
                || x == Op::TryEnd as u8
                || x == Op::CheckType as u8 =>
            {
                3
            }
            // 4-byte (u16 + u8)
            x if x == Op::DefineLocal as u8 || x == Op::MethodCall as u8 => 4,
            // 5-byte (u16 + u16)
            x if x == Op::ConstructStruct as u8 => 5,
            // 6-byte (u16 + u16 + u8)
            x if x == Op::ConstructEnum as u8 => 6,
            // Variable-width: CallNamed: 1(op) + 1(total_args) + 1(named_count) + named_count * 3
            x if x == Op::CallNamed as u8 => {
                if offset + 2 < code.len() {
                    let named_count = code[offset + 2] as usize;
                    3 + named_count * 3 // op + total_args + named_count + (position u8 + name u16) * count
                } else {
                    3
                }
            }
            // Variable-width: MatchBegin (u8 kind + extra operands depending on kind)
            x if x == Op::MatchBegin as u8 => {
                if offset + 1 < code.len() {
                    match code[offset + 1] {
                        4 => 3, // Tuple: kind + u8 length
                        5 => 4, // List: kind + u8 length + u8 has_rest
                        _ => 2, // Some/Ok/Err: just kind
                    }
                } else {
                    2
                }
            }
            // Variable-width: MatchArm (u8 kind, then u8 index for kinds 4/5)
            x if x == Op::MatchArm as u8 => {
                if offset + 1 < code.len() {
                    let kind = code[offset + 1];
                    if kind == 4 || kind == 5 {
                        3
                    } else {
                        2
                    }
                } else {
                    2
                }
            }
            // Unknown — treat as 1 to avoid infinite loops
            _ => 1,
        }
    }

    /// Peephole optimization pass: removes dead instruction sequences and
    /// adjusts jump targets accordingly.
    /// - `Not; Not` → remove both (double negation)
    /// - `Neg; Neg` → remove both (double arithmetic negation)
    /// - `Jump 0` → remove (nop jump to next instruction)
    /// - Pure push + `Pop` → remove both (dead value)
    #[cfg(feature = "optimize")]
    pub fn peephole_optimize(&mut self) {
        let old_len = self.code.len();
        if old_len == 0 {
            return;
        }
        let mut dead = vec![false; old_len];
        let mut changed = true;
        while changed {
            changed = false;
            let mut i = 0;
            while i < old_len {
                if dead[i] {
                    i += 1;
                    continue;
                }
                let size = Self::instruction_size(&self.code, i);
                // Find next live instruction
                let mut next = i + size;
                while next < old_len && dead[next] {
                    next += 1;
                }
                if next >= old_len {
                    break;
                }
                let next_size = Self::instruction_size(&self.code, next);

                // Pattern: Not; Not → remove both
                if self.code[i] == Op::Not as u8 && self.code[next] == Op::Not as u8 {
                    dead[i] = true;
                    dead[next] = true;
                    changed = true;
                    i = next + next_size;
                    continue;
                }

                // Pattern: Neg; Neg → remove both
                if self.code[i] == Op::Neg as u8 && self.code[next] == Op::Neg as u8 {
                    dead[i] = true;
                    dead[next] = true;
                    changed = true;
                    i = next + next_size;
                    continue;
                }

                // Pattern: Jump 0 → remove (jump to next instruction)
                if self.code[i] == Op::Jump as u8 && size == 3 {
                    let target = self.read_u16(i + 1);
                    if target == 0 {
                        for item in dead.iter_mut().skip(i).take(3) {
                            *item = true;
                        }
                        changed = true;
                        i = next;
                        continue;
                    }
                }

                // Pattern: pure push followed by Pop → remove both
                if self.code[next] == Op::Pop as u8 {
                    let b = self.code[i];
                    let is_pure = b == Op::True as u8
                        || b == Op::False as u8
                        || b == Op::Unit as u8
                        || b == Op::None as u8
                        || b == Op::Constant as u8
                        || b == Op::Dup as u8
                        || b == Op::GetLocalSlot as u8;
                    if is_pure {
                        for item in dead.iter_mut().skip(i).take(size) {
                            *item = true;
                        }
                        dead[next] = true;
                        changed = true;
                        i = next + 1;
                        continue;
                    }
                }

                i = next;
            }
        }

        self.compact_dead(&dead);
    }

    /// Remove dead bytes and adjust jump targets.
    #[cfg(feature = "optimize")]
    fn compact_dead(&mut self, dead: &[bool]) {
        let old_len = self.code.len();

        // Build offset map: old position → new position
        let mut offset_map = vec![0usize; old_len + 1];
        let mut new_pos = 0;
        for old_pos in 0..old_len {
            offset_map[old_pos] = new_pos;
            if !dead[old_pos] {
                new_pos += 1;
            }
        }
        offset_map[old_len] = new_pos;

        if new_pos == old_len {
            return;
        } // nothing to compact

        // Adjust jump targets (walk instruction boundaries on live code)
        let mut i = 0;
        while i < old_len {
            if dead[i] {
                i += 1;
                continue;
            }
            let op = self.code[i];
            let size = Self::instruction_size(&self.code, i);

            // Forward jumps: offset is relative to the byte AFTER the instruction
            if (op == Op::Jump as u8
                || op == Op::JumpIfFalse as u8
                || op == Op::And as u8
                || op == Op::Or as u8
                || op == Op::IterNext as u8)
                && size == 3
            {
                let old_offset = self.read_u16(i + 1) as usize;
                let old_target = i + 3 + old_offset;
                let new_instr_end = offset_map[i] + 3; // new position of byte after this instr
                let new_target = if old_target <= old_len {
                    offset_map[old_target]
                } else {
                    offset_map[old_len]
                };
                let new_offset = new_target.saturating_sub(new_instr_end);
                self.code[i + 1] = (new_offset >> 8) as u8;
                self.code[i + 2] = (new_offset & 0xff) as u8;
            }

            // Backward jump: Loop
            if op == Op::Loop as u8 && size == 3 {
                let old_offset = self.read_u16(i + 1) as usize;
                let old_target = (i + 3).wrapping_sub(old_offset);
                let new_instr_end = offset_map[i] + 3;
                let new_target = if old_target <= old_len {
                    offset_map[old_target]
                } else {
                    0
                };
                let new_offset = new_instr_end.saturating_sub(new_target);
                self.code[i + 1] = (new_offset >> 8) as u8;
                self.code[i + 2] = (new_offset & 0xff) as u8;
            }

            i += size;
        }

        // Compact: remove dead bytes
        let mut new_code = Vec::with_capacity(new_pos);
        let mut new_lines = Vec::with_capacity(new_pos);
        let mut new_cols = Vec::with_capacity(new_pos);
        for (j, &is_dead) in dead.iter().enumerate().take(old_len) {
            if !is_dead {
                new_code.push(self.code[j]);
                new_lines.push(self.lines[j]);
                new_cols.push(self.cols[j]);
            }
        }
        self.code = new_code;
        self.lines = new_lines;
        self.cols = new_cols;
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
