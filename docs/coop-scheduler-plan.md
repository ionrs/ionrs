# Tier C: Cooperative Scheduler — Implementation Plan

Replaces 1-OS-thread-per-task with a single-thread cooperative scheduler
running Ion tasks as stackful coroutines. Target: the major version
following 0.2.x. This document is the before-you-code reference.

## Goals

- One OS thread, many tasks. `spawn` is microseconds, not milliseconds.
- Yield points: `.await`, channel send/recv on would-block, `sleep`,
  (optionally) periodic yield to prevent CPU-bound starvation.
- `Value` drops its `Send`/`Sync` bounds; `Arc<Mutex<>>` collapses to
  `Rc<RefCell<>>` where state was only ever shared within one interpreter.
- Channels become plain `Rc<RefCell<Channel>>` — no mutex, no condvar.
- Preserve the existing Ion script API: `async {}`, `spawn`, `.await`,
  `select {}`, `channel(n)`, `sleep(ms)`, `timeout(ms, fn)`.

## Non-goals

- Preemptive scheduling. Cooperative only.
- Parallelism. A future `spawn_thread` primitive with an explicit
  serialization boundary can add real threads later.
- Async I/O. Out of scope for this tier; revisit if/when tokio is
  re-introduced as a host-side integration.

## Coroutine library

Candidates:

| Library | Model | Status | Verdict |
|---|---|---|---|
| `corosensei` | Stackful, safe API, mmap stacks | Actively maintained (@Amanieu) | **Chosen** |
| `generator` | Stackful, older, some `Send` footguns | Maintained but stale | Fallback |
| `genawaiter` | Stackless | — | Rejected: would require rewriting every `eval_*` as `async fn` |
| `ucontext` / hand-rolled | POSIX | Deprecated on macOS | Rejected |

**Choice: `corosensei`.** Cross-platform (Linux/macOS/Windows on x86_64
and aarch64), safe, and the author also maintains `parking_lot` /
`hashbrown` — trust level is high.

Fallback plan: if a target platform isn't supported, keep an opt-in
`threads-backend` feature that restores the current thread-per-task
implementation as a portability escape hatch. Costs extra maintenance
but avoids losing users on exotic targets.

## Architecture

```
Scheduler  (thread-local, Rc-based, one per interpreter session)
  ├─ tasks:            SlotMap<TaskId, Task>
  ├─ run_queue:        VecDeque<TaskId>
  ├─ timers:           BinaryHeap<(Instant, TaskId)>
  ├─ blocked_on_recv:  HashMap<ChannelId, VecDeque<TaskId>>
  ├─ blocked_on_send:  HashMap<ChannelId, VecDeque<(TaskId, Value)>>
  └─ blocked_on_task:  HashMap<TaskId, Vec<TaskId>>  // subscribers

Task
  ├─ id:           TaskId
  ├─ coroutine:    corosensei::Coroutine<Resume, Yield, TaskExit>
  ├─ state:        Ready | Blocked(BlockReason) | Finished(Result<Value, IonError>)
  ├─ cancel_flag:  Rc<Cell<bool>>
  └─ nursery_id:   Option<NurseryId>   // who adopted us

Channel
  ├─ id:            ChannelId
  ├─ buffer:        VecDeque<Value>
  ├─ capacity:      usize
  ├─ closed:        bool
  └─ (waiters tracked in scheduler, not here)
```

All types use `Rc` / `RefCell`. The scheduler lives in a thread-local
so interpreter code can reach it via a free function.

## Yield / resume protocol

```rust
enum YieldKind {
    WaitTask(TaskId),             // .await
    WaitAny(Vec<TaskId>),         // select
    WaitChannelRecv(ChannelId),   // recv on empty
    WaitChannelSend(ChannelId, Value),  // send on full (value parked in scheduler)
    Sleep(Instant),               // sleep / timeout deadlines
    Yield,                        // voluntary fairness yield
}

enum Resume {
    Start,
    Value(Value),      // channel recv completes
    Done,              // channel send completes, sleep fires, task finishes
    Cancelled,         // cancel flag was set — raise IonError in task
    WinnerIndex(usize),// wait_any result
}
```

Interpreter calls `scheduler::suspend(yield_kind)` at each yield point
and receives back a `Resume`.

## Value changes (breaking)

| Field | From | To |
|---|---|---|
| `Value::Cell` | `Arc<Mutex<Value>>` | `Rc<RefCell<Value>>` |
| `Value::Task` | `Arc<dyn TaskHandle>` | `TaskId` (looked up in scheduler) |
| `Value::Channel` | `ChannelEnd` with `Arc<dyn ChannelSender/Receiver>` | `ChannelRef { id: ChannelId, role: Sender\|Receiver }` |
| `Interpreter.cancel_flag` | `Option<Arc<AtomicBool>>` | `Option<Rc<Cell<bool>>>` |

Consequence: `Value: !Send + !Sync`. Embedders holding a `Value` across
threads must convert through `to_json` / serde. Document this clearly
in the release notes.

## Interpreter integration

The interpreter runs **on** a coroutine stack. A task's body is:

```rust
coroutine::new(move |_resume| {
    let mut interp = Interpreter::new();
    interp.cancel_flag = Some(cancel);
    for (name, val) in captured_env { interp.env.define(name, val, false); }
    let result = interp.eval_program(&program);
    TaskExit(result)
})
```

Every yield-point call-site becomes a thin wrapper around the
scheduler-provided `suspend`. The eval_* chain does not need the
`async fn` transformation — `suspend` is a normal function call from
the interpreter's perspective.

### Fairness

A CPU-bound task (`while true { ... }`) must not starve siblings.
Insert a `suspend(Yield)` every N statements in `eval_stmts`, tunable
via `Limits`. N=1000 is a reasonable default — negligible overhead,
still responsive.

## Sleep / timeout

- `sleep(ms)`: compute deadline, `suspend(Sleep(deadline))`. Scheduler's
  main loop checks the timer heap each tick and wakes expired tasks.
- `timeout(ms, fn)`: spawn the call as a child task, schedule a
  "fire-and-forget" timer that sets the child's cancel flag at deadline.
  Parent suspends with `WaitTask(child)` — either finishes normally or
  returns `IonError("task cancelled")`, which the parent converts to
  `None`.
- Scheduler idle path: if run queue is empty, `park_until(earliest
  timer)` — one `thread::sleep` call bounded by the next deadline.

## Structured concurrency (nursery)

`async {}` opens a nursery scope:

1. Parent enters `eval_async_block`, allocates a `NurseryId`.
2. Children spawned inside inherit `nursery_id = Some(this)`.
3. Block body runs on the parent task.
4. On exit, parent suspends with `WaitAll(children)` — scheduler
   wakes parent when every child is `Finished`.
5. Fail-fast: if any child errors, parent cancels the remaining
   children before propagating.

The parent is itself a task, so "block exit" is just a suspend.

## Select (`wait_any`)

Parent task emits `YieldKind::WaitAny(ids)`. Scheduler records the
parent under each `id` in `blocked_on_task`. First child to finish
wakes the parent with `Resume::WinnerIndex(idx)` and removes the
parent's registration from the other entries.

After the parent resumes, it cancels the losing branches (same behavior
as current 0.2.0).

## Channels (Rc-based)

```rust
fn send(ch: &mut Channel, v: Value) -> SendDecision {
    if ch.closed { return SendDecision::Closed; }
    if let Some(rx_id) = scheduler.pop_recv_waiter(ch.id) {
        scheduler.resume_with_value(rx_id, v);
        return SendDecision::Done;
    }
    if ch.buffer.len() < ch.capacity {
        ch.buffer.push_back(v);
        return SendDecision::Done;
    }
    SendDecision::Block(v)   // caller yields WaitChannelSend(ch.id, v)
}
```

No mutex, no condvar, no cross-thread visibility — just ordered
single-thread state. Rust's `RefCell` enforces that the interpreter
doesn't hold a borrow across a `suspend` call.

## Migration sequence

Branch `coop-scheduler`. Iterate until green. Rebase to main at end.

**Phase 1 — Foundation (0.5 day)**
- Add `corosensei` dep (optional, gated `coop-scheduler` feature
  initially — so the branch is bisectable).
- `src/scheduler.rs`: `Scheduler`, `Task`, `TaskId`, run-loop skeleton.
- Thread-local accessor: `SCHEDULER: RefCell<Option<Scheduler>>`.
- Unit test: spawn a coroutine that yields 3 times, resume, see it finish.

**Phase 2 — Value changes (1 day, single commit)**
- `Value::Cell`: `Arc<Mutex<>>` → `Rc<RefCell<>>`.
- `Value::Task`: `Arc<dyn TaskHandle>` → `TaskId`.
- `Value::Channel`: `ChannelEnd` → `ChannelRef { id, role }`.
- `Interpreter.cancel_flag`: `Arc<AtomicBool>` → `Rc<Cell<bool>>`.
- Remove `Send + Sync` bounds on traits that no longer need them.
- Run `cargo test --workspace --all-features`; fix compile errors in
  `env.rs`, `host_types.rs`, `stdlib.rs`.

**Phase 3 — Spawn / await (1 day)**
- `eval_async_block` drives the scheduler pump.
- `eval_spawn` pushes a coroutine onto the scheduler, returns
  `Value::Task(id)`.
- `eval_await` suspends with `WaitTask(id)`.
- Delete `async_rt.rs`, `async_rt_std.rs`.
- Remove `concurrency` feature flag (it becomes always-on) or rename
  to `coop-scheduler` and remove the gate.
- Concurrency test suite green.

**Phase 4 — Channels (0.5 day)**
- Replace crossbeam `Channel` with `Rc<RefCell<Channel>>`.
- `send`/`recv` yield on would-block.
- `close` wakes all waiters with `None`.

**Phase 5 — Sleep / timeout (0.5 day)**
- `sleep(ms)` uses `Sleep(deadline)` yield.
- `timeout(ms, fn)` schedules deferred cancel.
- Scheduler idle path: `thread::park_timeout(earliest_timer)`.

**Phase 6 — Select (0.5 day)**
- Replace `wait_any` with `WaitAny(ids)` yield.

**Phase 7 — Fairness (0.5 day)**
- Insert periodic `Yield` in `eval_stmts` every `Limits.yield_every`
  statements.
- Add a test: two CPU-bound tasks both make forward progress.

**Phase 8 — Release (0.5 day)**
- Bump to 0.3.0 (or 1.0.0 — see open question below).
- Update `DESIGN.md`, `docs/concurrency.md`, `CHANGELOG`.
- Rebase branch to main; tag release.

Total: ~6–8 days of focused work. Spread over 3–5 sessions with a
green test suite at every phase boundary.

## Risks

1. **Stack overflow on small coroutine stacks.** Default 64KB may
   underflow for deep interpreter recursion (e.g. a recursive Ion
   function with big stack frames). Mitigations: bump default to 256KB;
   tighten `Limits.max_call_depth` to match; evaluate corosensei's
   growable-stack support.

2. **Embedder `Value` !Send breakage.** Hosts that send `Value` between
   threads will no longer compile. Mitigations: document clearly; offer
   `value.to_json()` / serde-based marshalling; plan a future
   `OwnedValue` type for cross-thread transfers if demand arises.

3. **Debugger / stacktrace quality.** Stackful coroutines confuse GDB,
   LLDB, and most profilers — stacktraces jump across coroutines.
   Mitigations: annotate each task with a diagnostic label; consider an
   opt-in `--sync-tasks` debug mode that runs tasks inline on the
   parent's stack without switching (loses fairness, gains clarity).

4. **Portability.** corosensei covers x86_64 + aarch64 on the big
   three OSes. Targets outside that set (WASM, some BSDs, riscv) fall
   off. Mitigation: keep a `threads-backend` feature flag that restores
   the 0.2 thread-per-task implementation. Extra maintenance, but
   preserves reach.

5. **Host type method re-entry.** If a host-registered method calls
   back into Ion (e.g. a callback in the host's async runtime), the
   callback must enter a scheduler context. Mitigation: document that
   host callbacks must use a provided `call_ion_fn` helper that pumps
   the scheduler.

## Testing strategy

- All existing concurrency tests pass unchanged (17 tests + channel,
  select, timeout cases).
- New tests:
  - **Fairness**: two CPU-bound tasks interleave; disabling periodic
    yield reproduces starvation.
  - **Stack depth**: recursive task runs without overflow on default
    stack (bound below `Limits.max_call_depth`).
  - **Mass spawn**: 10k tasks spawned, all complete, run queue drains
    cleanly (memory leak check via `track_allocations`).
  - **Nursery fail-fast**: child errors; siblings see cancellation;
    parent re-raises.
  - **Drop cancels**: dropped task handle sets cancel flag.
- Benchmark: spawn-and-await of 10k tasks, compare 0.2.0 vs coop.
  Expected: 100–1000× improvement on spawn latency.

## Open questions (answer before coding starts)

1. **Portability fallback**: keep the OS-thread backend behind
   `threads-backend`, or commit fully to coroutines and drop exotic
   targets?
2. **Version number**: 0.2.x → 0.3.0, or → 1.0.0? The `!Send` change
   on `Value` is a real API break; 1.0 might be the honest signal.
3. **Coroutine stack size default**: 64 KB (fast, risks overflow),
   256 KB (balanced), or growable (heavier runtime)?
4. **Fairness yield default**: 1000 stmts between yields, or tunable
   via `Limits` with a sensible default?
5. **Host callback re-entry**: acceptable to require host-side tokio
   to call through a `call_ion_fn` helper, or should the scheduler
   tolerate arbitrary re-entry?

## References

- `corosensei`: https://github.com/Amanieu/corosensei
- Previous audit findings: see `DESIGN.md` §concurrency, `docs/concurrency.md`
- Current implementation: `ion-core/src/async_rt.rs`, `async_rt_std.rs`
