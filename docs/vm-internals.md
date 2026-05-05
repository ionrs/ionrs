# Ion VM Internals

## Architecture
- Stack-based bytecode VM in `ion-core/src/vm.rs`
- Compiler in `ion-core/src/compiler.rs`, bytecode defs in `ion-core/src/bytecode.rs`
- ~75 opcodes, variable-width encoding for some
- Hybrid mode: `Engine::vm_eval()` tries compile, falls back to tree-walk on failure
- Native async mode: `ion-core/src/async_runtime.rs` drives the same bytecode
  shape with explicit continuations so `Engine::eval_async()` can park on
  Tokio host futures without storing Ion calls on the Rust stack

## Key Opcodes
- `TryBegin` (3 bytes: u16 catch offset) — push exception handler
- `TryEnd` (3 bytes: u16 jump offset) — pop handler, jump over catch
- `CallNamed` (3 + named_count * 3 bytes) — u8 total_args, u8 named_count, then [u8 pos, u16 name_idx] per named arg
- `CallResolved` — pop function, positional-list, and keyword-pair-list; run the shared argument resolver
- `TailCall` — trampoline-style TCO
- `TailCallNamed` / `TailCallResolved` — tail-call variants for named and spread call sites
- `MethodCallResolved` — method call variant for `*expr` spreads; rejects keyword arguments
- `SpawnCall` / `SpawnCallNamed` — create child Ion tasks for async-runtime
  execution
- `SpawnCallResolved` — async-runtime spawn variant for calls containing `*` or `**` spreads
- `KwInsert` / `KwMerge` — build keyword-pair lists for resolved calls
- `AwaitTask` — park the current continuation until a child task completes
- `SelectTasks` — race child task handles and resume with the winning branch
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
- `call_function_named` preserves keyword metadata and dispatches through the shared resolver
- Calls containing `*expr` or `**expr` are lowered by the compiler to `CallResolved` or `TailCallResolved`
- Resolved calls use a positional `Value::List` plus a keyword-pair `Value::List`
- Keyword pair entries are `Value::Tuple([Value::Str(name), value])`
- Resolved Ion calls preserve bytecode tail-call behavior
- Method calls with `*expr` use `MethodCallResolved`; method keyword arguments are rejected before dispatch

## Compilation
- `begin_scope()`/`end_scope()` track locals (NOT raw PushScope/PopScope)
- `emit_jump()`/`patch_jump()` for forward references
- `compile_program()` returns `(Chunk, FnChunkCache)`
- Fn bodies precompiled into `FnChunkCache`, loaded via `preload_fn_chunks()`

## Async Continuations
- `VmContinuation` stores stack, instruction pointer, explicit call frames,
  exception handlers, locals, iterators, and task wait state.
- Async host functions registered with `Engine::register_async_fn` are stored
  in a host-future table and polled by `IonEvalFuture`.
- While one task is parked on a host future, timer, channel, or child task, the
  runtime can continue polling sibling Ion tasks.
- Sync `Engine::eval()` and `Engine::vm_eval()` keep their existing execution
  paths; async host functions require `Engine::eval_async()`.

## Chunk Structure
- `code: Vec<u8>` — bytecode
- `constants: Vec<Value>` — constant pool
- `lines: Vec<usize>` + `cols: Vec<usize>` — source mapping
- `instruction_size()` — variable-width decoder for disassembly/optimization
