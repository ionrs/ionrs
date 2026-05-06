# Ion VM audit

Status: audit fixes implemented and verified against the current tree on
2026-05-06.

Four independent reviewers covered correctness, soundness, performance, and
architecture across
`ion-core/src/{vm,bytecode,compiler,engine,value,call,env,intern,async_runtime}.rs`.
Findings are collapsed where they overlap and ordered by impact. This file now
includes the original audit notes plus implementation verification for the
high-impact correctness fixes completed on 2026-05-06.

## Verification summary

- `cargo fmt` passed.
- `cargo check -p ion-core` passed.
- `cargo test -p ion-core --test vm` passed: 174 tests.
- `cargo test -p ion-core --test integration` passed: 470 tests.
- `cargo test -p ion-core` passed.
- `cargo check -p ion-core --features async-runtime` passed.
- `cargo test -p ion-core --features async-runtime` passed.
- Targeted repros were run through the sync VM before fixing. The corresponding
  regression tests now cover exception-handler isolation, 256-argument calls,
  reverse slices, inclusive `i64::MAX` slice ends, integer overflow, and
  negative string repeat behavior.
- The original "locals are name-lookups, not slots" top issue is stale. Local
  reads/writes now compile to slot ops; the remaining env path is mostly
  closure/default-argument support and legacy opcode surface.

## Top issues (verified current status)

**1. Exception handlers leaked across function calls** - fixed in
`vm.rs:141-218, 3701-3742`

`exception_handlers` is a single VM-wide `Vec` with no save/restore around
Ion-fn calls. A `try` opened inside a callee can leave a stale
`ExceptionHandler` whose `catch_ip`, `stack_depth`, and `locals_depth` belong
to the callee's chunk. After return, the next `Err` in the caller can pop this
stale handler, set `self.ip = catch_ip` (an offset in a different chunk), and
truncate stack/locals to unrelated depths. The host-reentrancy path
(`invoke_value`) has the same shape.

Verified repro:

```ion
fn f() { try { return 1; } catch e { 2 } }
f();
1 / 0
```

Before the fix, the VM reported `cannot call string`; tree-walk reported the
expected `division by zero`. The VM now clears handlers for top-level execution
and runs each function chunk with a handler-floor equal to the caller's handler
depth. Errors can only catch handlers above that floor, and call return paths
truncate handlers back to the saved length. The repro now reports
`division by zero`.

**2. VM panic/misdispatch surfaces on normal source** - fixed for verified
crash classes in `compiler.rs`, `vm.rs`, `interpreter.rs`, and
`async_runtime.rs`

Confirmed failure classes:

- A 256-argument call truncated the emitted u8 count and misread the stack.
  Tree-walk returned `256`; VM reported `cannot call int`.
- Reverse slices panicked for lists, strings, and bytes:
  `[1, 2, 3][2..1]`, `"abc"[2..1]`, `b"abc"[2..1]`.
- Inclusive slice end overflowed on `i64::MAX`.
- Integer overflow panicked in debug for `+`, `/`, unary `-`, and related ops.
- Negative string repeat panicked through `String::repeat`; this affected both VM
  and tree-walk.

Implemented fixes:

- Calls and other u8-count bytecode operands now validate width before emission;
  large regular calls lower to the resolved-call path where possible.
- Sync VM, tree-walk, and async continuation scaffold integer arithmetic use
  checked integer ops and report `integer overflow`.
- Constant folding skips overflowing integer folds instead of wrapping or
  panicking.
- List/string/bytes slice syntax and list/string/bytes `.slice()` methods clamp
  reverse ranges to empty and use saturating inclusive ends.
- String repeat rejects negative counts in VM, tree-walk, and async scaffold.

Regression tests cover all verified classes.

**3. Heavy `Value` variants and chunk clones remain a large perf cost** -
`value.rs:22-112`, `vm.rs:1660`, `vm.rs:3685-3694`

The exact `Value` variant count is feature-dependent, so avoid hard-coding a
number. The core point is verified: `Str(String)`, `Bytes(Vec<u8>)`,
`List(Vec<Value>)`, `Dict(IndexMap<String, Value>)`, `Tuple/Set`,
`HostStruct`, and `HostEnum` all live inline. Constant loads, `peek`, dup,
push/pop helpers, captures, and many method calls clone heavy values. The
function cache also clones whole `Chunk`s on every call. `Arc` indirection for
heavy variants and `Arc<Chunk>` for cached chunks are still high-leverage.

**4. Stale finding: locals are not primarily name-lookups anymore** -
`compiler.rs:421-453`, `vm.rs:459-550`

The VM still contains `GetLocal`/`SetLocal`/`DefineLocal` slow opcodes that do
symbol/env lookup. However, current compiler paths emit `GetLocalSlot` and
`SetLocalSlot` for local reads/writes. When closures exist, `DefineLocal` is
emitted in addition to `DefineLocalSlot` so values are mirrored into `Env` for
capture/default evaluation. Do not treat "make slot locals the only path" as a
top correctness/performance fix without first re-auditing closure capture and
default-argument semantics.

## Correctness bugs

- **Fixed: exception handlers leaked across calls** - covered by
  `test_vm_exception_handlers_do_not_leak_from_returning_callee`.
- **Fixed: `break value` from `for`/`while` left an orphan on the stack** -
  statement `for`, `while`, `while let`, and `loop` break paths now discard
  break values. Loop expressions keep break values.
- **`compile_pattern_test` produces stack-imbalanced branches** -
  `compiler.rs:2245-2342`. True/false paths for `Some`/`Ok`/`Err`/`Tuple`/`List`
  patterns converge at different stack heights. Current tests do not expose all
  cases.
- **Fixed: `emit(... as u8)` truncation in argument/element counts** - audited
  u8-count and u8-index bytecode operands now go through checked emission or
  lower to resolved-call bytecode for large calls.
- **Fixed: reverse slices panic for list/string/bytes** - syntax and `.slice()`
  method paths now clamp reverse ranges to empty.
- **Fixed: `Str * negative Int` panicked in `String::repeat`** - negative string
  repeat now reports `repeat count must be non-negative`.
- **Fixed: slice-end `(e_raw + 1)` overflowed on `i64::MAX`** - inclusive slice
  ends use saturating addition.
- **Fixed: integer arithmetic used unchecked ops** - sync VM, tree-walk, async
  continuation scaffold, and constant folding now avoid unchecked integer
  overflow.
- **`JumpIfFalse` documented to pop, actually peeks** -
  `bytecode.rs:60-61` vs `vm.rs:559-565`. The compiler compensates with
  explicit `Pop`s today; the opcode contract is still misleading.

## Soundness

- **One `unsafe` block at `vm.rs:1584`** (`transmute<u8, Op>`) - verified clean.
  `Op` is `#[repr(u8)]`, variants are sequential through `KwMerge`, and the
  bound check `byte > KwMerge as u8` matches the current enum.
- **Correction:** the sync VM/core path has no raw pointers or lifetime escapes,
  but `async_runtime.rs` does use `Rc<RefCell<...>>` for runtime queues/tasks.
  That is not UB by itself, but the original "no `RefCell` anywhere" claim was
  false.
- **Ref-cycle leak surface**: `Value::Cell(Arc<Mutex<Value>>)` plus
  `IonFn.captures` allows cycles. Not UB; it can leak. No `Weak` mitigation was
  found.
- **Pervasive unchecked indexing** remains: `chunk.constants[i]`,
  `self.locals[slot]`, and `self.stack[args_start..]` trust compiler output.
  `args_start = stack.len() - arg_count` can underflow when bytecode stack shape
  is wrong.

## Performance (in priority order)

| Tier | Issue | Location |
|---|---|---|
| HIGH | Heavy inline `Value` variants, no `Arc` indirection | `value.rs:22-112` |
| HIGH | `Op::Call` allocates/clones via positional vectors, slot resolution, and `env.define(name.clone(), val.clone())` | `vm.rs:3520-3527, 3482-3518`, `call.rs:65-121` |
| HIGH | `fn_cache` clones whole `Chunk` per call instead of storing `Arc<Chunk>` | `vm.rs:3685-3694` |
| HIGH | Dispatch loop reads `lines[ip]`/`cols[ip]` per instruction, then splits execution across `run_chunk` and `dispatch_instruction` | `vm.rs:155-223` |
| MED | Closure capture/default support mirrors values into `Env` and rebuilds captures | `compiler.rs:433-443`, `vm.rs:765`, `env.rs:203-211` |
| MED | `iterators: Vec<Box<dyn Iterator>>` means virtual calls per `IterNext`; string/bytes iteration materializes values | `vm.rs:49, 1156-1221` |
| MED | `BuildList`/`BuildTuple`/`BuildDict`/`MethodCall` use `drain().collect()` per literal/call | `vm.rs:673, 680, 689, 737, 1070, 1146` |
| MED | `resolve_ion_slots` does O(params x named) string compares; host path already does hash compare | `call.rs:80-115` |
| MED | `BuildDict` formats non-string keys; `Value::Dict` is keyed by full `String` | `vm.rs:690-695`, `value.rs:29` |
| LOW | `eval_default_arg` spins up a fresh `Interpreter` with an env clone per defaulted-arg call | `vm.rs:3447-3468` |
| LOW | Legacy `GetLocal`/`SetLocal` name-lookup opcodes remain, but current compiler does not emit them for normal local reads/writes | `vm.rs:459-506` |

## Architecture

- **`vm.rs` is 3871 lines but roughly 1200 are container method dispatch**
  (list/str/dict/set/tuple/option/result/bytes methods, `vm.rs:2104-3323`).
  This duplicates large parts of `interpreter.rs`. Extract a shared `methods/`
  layer that can call through a small invoker trait.
- **Two complete bytecode interpreters coexist**: sync `vm.rs` and
  `async_runtime.rs`, which reimplements the stepper with continuations. Method
  dispatch is duplicated again inside the async path. `async_rt.rs` and
  `async_rt_std.rs` are legacy. Long-term: one stepper with a host-call resolver
  returning `Ready | Pending(future)`; `HostCallResult` already exists in
  `value.rs`.
- **`Value` mixes several jobs**: primitives/containers, callable variants,
  cfg-gated async handles, host-injected types, and runtime plumbing (`Cell`).
  Collapse callables to a single variant and consider moving async handles
  behind a resource abstraction.
- **Error handling is dual-mode**: `Result<_, IonError>` propagation plus manual
  unwind via `exception_handlers`, with `ErrorKind::PropagatedErr` and
  `PropagatedNone` used as control-flow tokens for `?`. Every throw site has to
  remember to filter on `e.kind` before matching handlers (`vm.rs:206-217`).
  Make `?` a real opcode-level early return.
- **Compiler/VM contract is implicit; no disassembler**. Adding an opcode means
  updating `Op`, `instruction_size`, peephole allowlists, dispatcher, and emitter
  with no mechanical check they agree. Add `Chunk::disassemble` driven from a
  single opcode metadata table.
- **Dead/duplicated bookkeeping**: `Op::Pipe` is emitted nowhere
  (`vm.rs:837-846`); `add_constant` only deduplicates `Str`/`Int`/`Float`/`Bool`
  (`bytecode.rs:284-298`).

## What's well-designed

- Hash-keyed host-name dispatch (`Module`, `globals_h`) keeps source identifiers
  out of `.rodata` and dispatches with integer comparison.
- Local reads/writes now use slot opcodes on current compiler paths.
- Tail calls use a real `pending_tail_call` trampoline instead of Rust stack
  recursion.
- `Chunk::instruction_size` is consistent enough that the peephole pass can
  mechanically walk instruction boundaries.
- The single sync-VM `unsafe` block is narrowly bounded and currently correct.

---

The first two priority findings are now fixed and covered by regression tests:
**(1)** handler-frame isolation for `exception_handlers`, and **(2)** verified
VM panic/misdispatch surfaces from operand truncation, unchecked arithmetic,
unsafe slice bounds, and negative string repeat. The remaining high-leverage
follow-up is **(3)** reducing heavy clone pressure with `Arc` indirection for
large `Value` payloads and cached chunks. Slot-local cleanup is a follow-up, not
the current top blocker.
