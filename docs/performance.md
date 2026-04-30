# Ion Performance Strategy

## Current Implementation
- Value type: enum-based (not yet NaN-boxed)
- String interning: `StringPool` with `Symbol(u32)` keys
- Env: `Vec<Binding>` stack with Symbol keys (shared by interpreter + VM)
- Bytecode VM: stack-based, ~75 opcodes, hybrid mode (falls back to tree-walk)
- Async bytecode runtime: pollable continuation VM behind `async-runtime`;
  host futures, timers, and channels park on Tokio instead of blocking an OS
  thread
- Stack-slot locals: compiler resolves locals to slot indices, VM uses O(1) indexed access
- VM fn compilation caching: `IonFn` has `fn_id: u64`, VM has `HashMap<u64, Chunk>` cache
- Precompiled fn chunks: `compile_program()` returns `(Chunk, FnChunkCache)`

## Optimizations (feature-gated: `optimize`, requires `vm`)
- Peephole optimizer: dead instruction removal with jump fixup
- Constant folding: literal BinOp/UnaryOp at compile time (int, float, string, bool)
- Dead code elimination: skip unreachable statements after return/break/continue
- Tail call optimization: compile-time `in_tail_position` detection + TailCall opcode

## Future Plans (not yet implemented)
- NaN-boxing: 8 bytes per value, fits in register
- Arena allocation for AST nodes
- Small-dict optimization (<=8 entries: flat array)
- Frame pool for function calls

## Benchmarks
- ion-core/benches/ion_bench.rs with criterion
- Targets: fib, loop, map/filter, string, match, comprehension, closures
