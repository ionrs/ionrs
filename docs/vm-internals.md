# Ion VM Internals

## Architecture
- Stack-based bytecode VM in `ion-core/src/vm.rs`
- Compiler in `ion-core/src/compiler.rs`, bytecode defs in `ion-core/src/bytecode.rs`
- ~75 opcodes, variable-width encoding for some
- Hybrid mode: `Engine::vm_eval()` tries compile, falls back to tree-walk on failure

## Key Opcodes
- `TryBegin` (3 bytes: u16 catch offset) — push exception handler
- `TryEnd` (3 bytes: u16 jump offset) — pop handler, jump over catch
- `CallNamed` (3 + named_count * 3 bytes) — u8 total_args, u8 named_count, then [u8 pos, u16 name_idx] per named arg
- `TailCall` — trampoline-style TCO
- `Closure` — captures env values
- `ConstructStruct` / `ConstructEnum` — host type construction
- `IterInit` / `IterNext` / `IterDrop` — for-loop protocol
- `CheckType` (2 bytes: u16 constant index) — peek TOS, check type matches string from constant pool

## Exception Handling
- `ExceptionHandler` struct: catch_ip, stack_depth, local_frames_depth, locals_depth
- `exception_handlers: Vec<ExceptionHandler>` on VM
- `run_chunk` calls `dispatch_instruction` which returns `Result<Option<Value>, IonError>`
  - `Some(val)` = Return/TailCall, `None` = continue
- On error: check handler stack, restore state, push error as string, jump to catch_ip
- PropagatedErr/PropagatedNone bypass exception handlers (propagate up)

## Named Args (VM)
- `CallNamed` opcode reads metadata, calls `call_function_named`
- `call_function_named` reorders args to match `IonFn` params by name

## Compilation
- `begin_scope()`/`end_scope()` track locals (NOT raw PushScope/PopScope)
- `emit_jump()`/`patch_jump()` for forward references
- `compile_program()` returns `(Chunk, FnChunkCache)`
- Fn bodies precompiled into `FnChunkCache`, loaded via `preload_fn_chunks()`

## Chunk Structure
- `code: Vec<u8>` — bytecode
- `constants: Vec<Value>` — constant pool
- `lines: Vec<usize>` + `cols: Vec<usize>` — source mapping
- `instruction_size()` — variable-width decoder for disassembly/optimization
