# Native Async Host Function Design for Ion

## Goal

Ion hosts should be able to register native Tokio async functions such as HTTP, database, timers, and other I/O. Ion scripts should call those functions normally, without async coloring:

```ion
fn load_title(url) {
    let body = http_get(url);
    parse_title(body)
}

load_title("https://example.com")
```

When `http_get` waits on network I/O, Ion must not block an OS thread and must not create an OS thread per Ion task. The async Ion runtime should park by returning `Poll::Pending` to Tokio and resume when Tokio wakes the underlying future.

## Architecture

Use a **pollable VM runtime**.

`Engine::eval_async()` returns a Rust future. That future owns or borrows an `IonRuntime` and drives Ion tasks until:

- the root script completes;
- the root script errors;
- all runnable Ion tasks are parked on host futures, timers, channels, or other Ion tasks;
- the per-poll instruction budget is exhausted.

If no work can proceed, `IonEvalFuture::poll` returns `Poll::Pending`. Tokio wakes it through registered wakers from host futures, timers, and external runtime handles.

This is not a recursive Rust `async fn` interpreter. Ion suspension is represented as VM state: instruction pointer, stack, call frames, exception handlers, locals, iterators, and wait state.

## Host Function Model

Host functions can complete synchronously or return a future.

```rust
use std::future::Future;
use std::pin::Pin;

pub type BoxIonFuture =
    Pin<Box<dyn Future<Output = Result<Value, IonError>> + 'static>>;

pub enum HostCallResult {
    Ready(Result<Value, IonError>),
    Pending(BoxIonFuture),
}

pub type HostFn = dyn Fn(Vec<Value>) -> HostCallResult + 'static;
```

Arguments are owned `Vec<Value>` because a host future may hold them across `.await`.

API sketch:

```rust
impl Engine {
    pub fn eval(&mut self, source: &str) -> Result<Value, IonError>;

    pub fn eval_async<'a>(&'a mut self, source: &'a str) -> IonEvalFuture<'a>;

    pub fn register_fn<F>(&mut self, name: &str, f: F)
    where
        F: Fn(Vec<Value>) -> Result<Value, IonError> + 'static;

    pub fn register_async_fn<F, Fut>(&mut self, name: &str, f: F)
    where
        F: Fn(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Value, IonError>> + 'static;
}
```

Example:

```rust
engine.register_async_fn("http_get", |args| async move {
    let url = args[0]
        .as_str()
        .ok_or_else(|| IonError::runtime("url must be string", 0, 0))?
        .to_string();

    let body = reqwest::get(url)
        .await
        .map_err(|e| IonError::runtime(e.to_string(), 0, 0))?
        .text()
        .await
        .map_err(|e| IonError::runtime(e.to_string(), 0, 0))?;

    Ok(Value::Str(body))
});
```

Calling an async host function from synchronous `eval()` should produce a clear runtime error: `async host function called from sync eval; use eval_async`.

## VM Continuations

The current VM has useful pieces (`stack`, `ip`, locals, local frames, exception handlers), but it is **not currently resumable across nested Ion calls**. Function calls recurse through `run_chunk` and save `ip`, locals, and iterators on the Rust stack. Native async suspension inside a nested Ion function therefore requires a real continuation refactor.

Required task state:

```rust
pub struct IonTask {
    frames: Vec<CallFrame>,
    stack: Vec<Value>,
    locals: Vec<LocalSlot>,
    local_frames: Vec<usize>,
    iterators: Vec<IteratorState>,
    exception_handlers: Vec<ExceptionHandler>,
    state: TaskState,
    nursery: Option<NurseryId>,
    waiters: Vec<TaskId>,
    cancel_requested: bool,
}

pub struct CallFrame {
    chunk: ChunkId,
    ip: usize,
    locals_base: usize,
    env_scope_depth: usize,
}
```

The async VM cannot rely on Rust stack recursion for Ion function calls. `Op::Call`, `Op::Return`, and tail calls need to manipulate `CallFrame` entries explicitly.

The runtime owns compiled chunks in an arena:

```rust
pub struct IonRuntime {
    chunks: SlotMap<ChunkId, Chunk>,
    tasks: SlotMap<TaskId, IonTask>,
    ready_queue: VecDeque<TaskId>,
    root_task: TaskId,
    host_futures: HostFutureTable,
    timers: TimerTable,
    channels: SlotMap<ChannelId, AsyncChannel>,
    nurseries: SlotMap<NurseryId, Nursery>,
    instr_budget_per_poll: u32,
    per_task_step_cap: u32,
}
```

Frames store `ChunkId`, not borrowed `&Chunk`, to avoid self-referential lifetimes.

## Step Outcomes

The VM executes in small steps.

```rust
pub enum StepOutcome {
    Continue,
    Yield,
    Suspended(TaskState),
    InstructionError(IonError),
    Done(Result<Value, IonError>),
}
```

`InstructionError` is separate from `Done(Err(_))`. It must pass through the same try/catch and `?` propagation machinery as ordinary VM instruction errors. A host future resolving to `Err(e)` is an instruction error at the original call site, not automatically task termination.

## Eval Future

`IonEvalFuture::poll` should be budgeted and wake-driven:

```rust
impl<'a> Future for IonEvalFuture<'a> {
    type Output = Result<Value, IonError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        let mut budget = this.rt.instr_budget_per_poll;

        this.rt.poll_ready_host_futures(cx);
        this.rt.poll_timers(cx);
        this.rt.drain_external_queue(cx);
        this.rt.move_ready_channel_waiters();

        while let Some(task_id) = this.rt.ready_queue.pop_front() {
            let consumed = this.rt.run_task_until_blocked(task_id, budget);
            budget = budget.saturating_sub(consumed.max(1));

            if let Some(result) = this.rt.root_result_if_finished(this.root) {
                return Poll::Ready(result);
            }

            if budget == 0 {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        }

        if let Some(result) = this.rt.root_result_if_finished(this.root) {
            return Poll::Ready(result);
        }

        Poll::Pending
    }
}
```

Important fairness rule: every scheduler transition charges at least one budget unit. A task that repeatedly yields or suspends before executing an instruction must not spin forever in one poll.

## Host Future Storage

Do not store host futures only in `FuturesUnordered` if cancellation needs removal by `FutureId`. `FuturesUnordered` is useful for wake-driven polling, but it does not support arbitrary removal by ID.

Use a cancellable table:

```rust
pub struct HostFutureTable {
    entries: SlotMap<FutureId, HostFutureEntry>,
    ready: VecDeque<FutureId>,
}

pub struct HostFutureEntry {
    waiter: TaskId,
    future: BoxIonFuture,
    waker_registered: bool,
}
```

The table can be implemented in one of two ways:

- custom polling table plus per-entry waker that pushes `FutureId` into `ready`;
- `futures_util::future::Abortable` plus an auxiliary map from `FutureId` to `AbortHandle`, while completed futures report `(FutureId, TaskId, Result<Value, IonError>)`.

The required operations are:

```rust
fn insert(&mut self, waiter: TaskId, fut: BoxIonFuture) -> FutureId;
fn poll_ready(&mut self, cx: &mut Context<'_>) -> Vec<(FutureId, TaskId, Result<Value, IonError>)>;
fn cancel_and_drop(&mut self, id: FutureId);
```

Cancellation must be able to drop one specific host future. Dropping the future is the cancellation mechanism for Tokio I/O futures.

## Host Function Dispatch

When a VM call instruction invokes a host function:

```rust
match host_fn(args) {
    HostCallResult::Ready(Ok(value)) => {
        task.stack.push(value);
        StepOutcome::Continue
    }

    HostCallResult::Ready(Err(err)) => {
        StepOutcome::InstructionError(err)
    }

    HostCallResult::Pending(future) => {
        let id = self.host_futures.insert(task_id, future);
        StepOutcome::Suspended(TaskState::WaitingHostFuture(id))
    }
}
```

When the host future resolves:

```rust
match result {
    Ok(value) => {
        self.tasks[task_id].stack.push(value);
        self.ready_queue.push_back(task_id);
    }
    Err(err) => {
        self.resume_task_with_instruction_error(task_id, err);
    }
}
```

`resume_task_with_instruction_error` must use the task's exception handler stack. If there is an active `try/catch`, jump to the catch handler. If the error is a propagated `?`, preserve the existing propagation behavior. Only unhandled errors finish the task with `Done(Err(_))`.

## Function Coloring

Ion functions remain uncolored. Any Ion function can call a host async function:

```ion
fn load_user(id) {
    let body = http_get(f"https://api.example.com/users/{id}");
    json::parse(body)
}
```

There is no `async` marker and no `await` at the call site. Suspension is a runtime behavior of the VM instruction that invokes the host function.

## Ion Tasks

Ion `spawn` creates another `IonTask`, not a Tokio task and not an OS thread:

```ion
async {
    let a = spawn http_get(url_a);
    let b = spawn http_get(url_b);
    [a.await, b.await]
}
```

`.await` on an Ion task sets the current task to `WaitingTask(child_id)` and records the caller in `child.waiters`. When the child finishes, waiters are resumed with cloned results.

Multiple waiters may be allowed, but this is a semantic choice. If multiple waits are allowed, task results must be retained or cloneable until all waiters resume. If only one waiter is allowed, attempting a second await should be a runtime error.

## Timers

Use Tokio timers for async evaluation.

Recommended structure:

```rust
pub struct TimerTable {
    timers: BinaryHeap<TimerEntry>,
    earliest: Option<Pin<Box<tokio::time::Sleep>>>,
}
```

Only one `tokio::time::Sleep` is needed for the earliest deadline. When it fires, drain all expired entries and ready their tasks, then reset the earliest sleep.

`timeout(ms, fn)` should be represented as a race between a child Ion task and a timer. If the timer wins, cancel the child and push `None`; if the child wins, cancel the timer and push `Some(value)`.

## Channels

Channels should be runtime-managed, not crossbeam-backed in async mode:

```rust
pub struct AsyncChannel {
    buffer: VecDeque<Value>,
    capacity: usize,
    closed: bool,
    recv_waiters: VecDeque<TaskId>,
    send_waiters: VecDeque<(TaskId, Value)>,
}
```

`tx.send(value)`:

- if closed, raise `send on closed channel`;
- if a receiver is waiting, deliver directly and resume it;
- if buffer has capacity, push the value and continue;
- otherwise suspend the sender with `WaitingChannelSend(channel, value)`.

`rx.recv()`:

- if buffer has a value, return `Some(value)`;
- if closed, return `None`;
- otherwise suspend the receiver with `WaitingChannelRecv(channel)`.

`tx.close()` should wake receivers with `None` and blocked senders with an error.

## Select

`select {}` is a runtime primitive over Ion wait states.

The current compiler rejects concurrency expressions for bytecode. Async runtime support requires new bytecode/compiler support for:

- `async {}`;
- `spawn expr`;
- `.await`;
- `select {}`;
- channel send/recv methods if they should suspend;
- `sleep` and `timeout` if they should suspend.

For `select`, create branch tasks or register branch waits with the runtime. First completion wins. The parent resumes with the winner branch index and value. Losing branch tasks/futures are cancelled and dropped.

## Structured Concurrency

`async {}` should create a nursery:

```rust
pub struct Nursery {
    parent: TaskId,
    children: Vec<TaskId>,
    state: NurseryState,
}
```

Rules:

- `spawn` inside the block registers children with the current nursery.
- The block cannot return until children finish or are cancelled.
- If the block body errors, cancel children, drop their host futures, drain cleanup, and propagate the body error.
- If a child errors before being awaited, cancel siblings and propagate the child error.
- No fire-and-forget child escapes the nursery.

## Cancellation

Cancellation is cooperative.

Each task has `cancel_requested`. The VM checks it:

- at instruction boundaries;
- before entering a host call;
- when resuming from host future, channel, timer, or task wait;
- when a fairness budget slice ends.

If a task is waiting on a host future and is cancelled:

1. Remove the specific `FutureId` from `HostFutureTable`.
2. Drop the `BoxIonFuture`.
3. Mark the task as cancelled.
4. Wake task waiters with the cancellation error.
5. Cascade cancellation through nursery children.

This requires host future storage with by-ID removal. A pure `FuturesUnordered` queue is insufficient.

Dropping `IonEvalFuture` should cancel the root task, cascade through nurseries, and drop all pending host futures. It should not await them.

## Re-entry From Host Futures

A host future may want to schedule Ion work, but it must not re-enter an in-flight `IonEvalFuture` directly.

Do not design `EngineHandle` as direct `Rc<RefCell<IonRuntime>>` access while `IonEvalFuture` holds `&mut IonRuntime`. That creates borrow conflicts.

Use an external queue handle:

```rust
pub struct EngineHandle {
    queue: Rc<RefCell<VecDeque<ExternalRequest>>>,
    waker: Rc<RefCell<Option<Waker>>>,
}

pub enum ExternalRequest {
    Call {
        fn_name: String,
        args: Vec<Value>,
        result_tx: oneshot::Sender<Result<Value, IonError>>,
    },
}
```

`IonEvalFuture::poll` drains this queue before running tasks. The handle only enqueues work and wakes the runtime. It does not borrow the runtime.

Synchronous host functions should not call back into Ion during the same VM step. If re-entry is needed, use `register_async_fn` plus `EngineHandle`.

## Send vs Local Runtime

Initial mode should be local and non-`Send`:

```rust
pub type BoxIonFuture =
    Pin<Box<dyn Future<Output = Result<Value, IonError>> + 'static>>;
```

Run under a Tokio `LocalSet` or current-thread runtime. This is enough for native Tokio network I/O: sockets are reactor-driven and do not block an OS thread while pending.

An optional later multi-thread mode can require:

```rust
pub type BoxIonFutureSend =
    Pin<Box<dyn Future<Output = Result<Value, IonError>> + Send + 'static>>;
```

That mode requires `Value` and captured host state crossing `.await` to be `Send`, so it should be a separate feature or API.

## Bytecode And Compiler Work

This design should target the bytecode VM, but the current bytecode VM does not support concurrency syntax. The compiler currently rejects `async`, `spawn`, `.await`, and `select` for VM compilation.

The current implementation includes a deliberately narrow async tree-walk bridge behind `Engine::eval_async`. That bridge is useful because it already lets host-provided Tokio futures park and resume without blocking an OS thread, and Ion functions remain uncolored at the source level. It is not the final runtime shape: the bridge should not grow into a second full interpreter. The final design is still the pollable bytecode VM with explicit continuations described above.

Required implementation work:

1. Add bytecode opcodes for async runtime operations or lower them into existing host-call-like opcodes.
2. Replace Rust-stack recursive Ion calls with explicit `CallFrame` push/pop.
3. Make `step_task` execute one instruction or one bounded compound operation.
4. Preserve exception handler semantics across suspension.
5. Preserve `?` propagation semantics across suspension.
6. Add chunk arena ownership so suspended frames can safely reference bytecode.
7. Audit compound operations for stack consistency before suspension.

Suspension should occur only at well-defined instruction boundaries or after a compound operation has left the stack in a resumable state.

## Compatibility

- Existing `Engine::eval()` remains synchronous.
- Existing sync host functions can work under both sync and async evaluation, but blocking sync functions will block the async runtime thread.
- Provide `register_blocking_fn` later if hosts want automatic `tokio::task::spawn_blocking` wrapping.
- Async host functions are only callable under `Engine::eval_async()`.
- Existing Ion script syntax can remain unchanged.
- Do not claim broad WASM portability unless the chosen Tokio/runtime/HTTP stack is verified for the target.

## Final Concurrency Shape

Ion source stays mostly uncolored. A script calls host async functions as ordinary functions; the VM decides whether that call returns immediately or parks the current continuation on a Tokio-polled host future.

The core user-facing model is:

- Plain call: `let body = http_get(url)` parks only if the host future is pending.
- Explicit overlap: `spawn f(x)` starts a child task and returns a task handle.
- Join: `task.await` parks until the task completes.
- Race: `select { ... }` resumes the first completed branch and drops the losing branch tasks.
- Timer: under `eval_async`, `sleep(ms)` is a native Tokio timer future.
- Timeout: under `eval_async`, `timeout(ms, fn)` runs the callback as a pollable child future and returns `Some(value)` if it wins or `None` if the Tokio timer wins.
- Host callback: Rust async host code can use `EngineHandle::call_async("fn_name", args)` to enqueue Ion work back onto the same local runtime.
- Channel: under `eval_async`, `channel(capacity)` returns native async sender/receiver endpoints. `send`, `recv`, and `recv_timeout` park on Tokio futures; `try_recv` is immediate; `close` closes the endpoint without using the old thread/blocking backend.

Real-world scripts should look like this:

```ion
fn fetch_json(url) {
    let body = http_get(url)
    json::decode(body)
}

fetch_json("https://api.example.test/users/42")
```

Parallel I/O is explicit where the program needs overlap:

```ion
async {
    let profile = spawn http_get("/profile")
    let orders = spawn http_get("/orders")

    {
        profile: json::decode(profile.await),
        orders: json::decode(orders.await),
    }
}
```

Timeouts can be expressed as a race between real I/O and the Tokio-backed timer:

```ion
async {
    select {
        body = spawn http_get("/slow") => Some(body),
        _ = spawn sleep(250) => None,
    }
}
```

The host can call back into Ion without re-entering the VM on the Rust stack:

```ion
fn on_event(event) {
    audit_log(event.id)
    event.id
}

wait_for_host_event()
```

In Rust, `wait_for_host_event` can await Tokio I/O and then call `handle.call_async("on_event", vec![event])`. The runtime drains that request, runs `on_event` as a local future, and resolves the host future when the callback returns.

Native channels are useful for fan-in/fan-out workflows:

```ion
async {
    let (tx, rx) = channel(32)

    spawn produce_pages(tx, "/crawl/start")
    spawn produce_pages(tx, "/crawl/archive")

    let mut count = 0
    while let Some(page) = rx.recv() {
        index_page(page)
        count += 1
    }

    count
}
```

That channel example uses the native async runtime shape. It does not use an OS-thread blocking `recv`; `rx.recv()` parks the Ion task on a Tokio-polled receive future and the runtime keeps sibling Ion tasks and Tokio futures moving.

## Migration Plan

Progress checklist:

- [x] Add `HostCallResult` and async host registration API.
- [x] Make sync evaluation reject async host functions with a clear error.
- [x] Add `async-runtime` feature scaffolding.
- [x] Add `IonEvalFuture` public scaffold.
- [x] Add a narrow `eval_async` bridge that can await native async host functions without blocking an OS thread.
- [x] Prove Ion source remains uncolored for async host calls through an Ion function.
- [x] Prove `eval_async` returns `Poll::Pending` while a host future is pending and resumes when the host future wakes.
- [x] Preserve full existing sync evaluation for `eval_async` programs that do not reference async host functions.
- [x] Add bridge-level `spawn` / `.await` task handles for overlapping async host futures.
- [x] Add bridge-level `async {}` nursery enforcement; reject `spawn` outside an async block.
- [x] Add bridge-level `select {}` over direct async branch futures.
- [x] Prove two spawned host futures overlap under one Tokio runtime without `spawn_blocking` or OS-thread parking.
- [x] Add explicit continuation `CallFrame` push/pop for registered Ion function chunks.
- [x] Add continuation `TailCall` frame reuse, including tail-position async host suspension/resume.
- [x] Add continuation calls for sync builtins/closures and fully supplied named Ion function calls.
- [x] Add continuation call-frame scopes for captures and positional/named default arguments.
- [x] Add named-argument `spawn` bytecode so spawned Ion wrapper functions preserve normal Ion call semantics.
- [x] Add async-aware continuation method calls for list/range `map`, `filter`, `any`, `all`, `flat_map`, `fold`, `reduce`, dict `map`/`filter`, Option/Result closure methods, and `cell.update`.
- [x] Preserve function-boundary `?` semantics in continuation calls by converting propagated `Err`/`None` back into `Result`/`Option` at caller frames.
- [x] Finish VM calls into explicit `CallFrame` continuations while preserving sync VM behavior, including spawned Ion function calls that suspend on async host I/O.
- [x] Add chunk arena ownership for resumable frames.
- [x] Add minimal continuation-shaped `step_task` scaffold for simple non-suspending opcodes.
- [x] Expand `step_task` over arithmetic, bitwise, comparison, boolean short-circuit, env/global locals, stack-slot locals, stack, wrapper, sequence/dict, field/index, method-call, slice, f-string, range, host type construction, match/type-check, iterator, pipe-error, print, jump, and loop opcodes.
- [x] Add `step_task` support for stack-slot locals and lexical scope cleanup.
- [x] Add bytecode `use` support for single, named, and glob module imports so async-host programs can import stdlib/host module members without the bridge.
- [x] Add bytecode host-struct pattern support for `match` and destructuring binds so async host results can be matched as normal Ion values.
- [x] Add bytecode host-enum pattern support for unit and positional payload variants so async host enum results match sync interpreter semantics.
- [x] Add checked destructuring for nontrivial `let`, `for`, comprehension, and `select` branch patterns so mismatches fail cleanly before binding.
- [x] Add continuation-level exception handler stack for VM `try/catch` instruction errors.
- [x] Remove bytecode dispatch gaps from the continuation scaffold; every current `Op` has a `step_task` dispatch arm.
- [x] Add continuation output-handler support for bytecode `Print`.
- [x] Add continuation method-call support for non-closure built-in methods and closure-driven list/range/dict/Option/Result/cell methods, including async-aware `sort_by` comparators.
- [x] Convert the async-runtime VM dispatch into `step_task` with `StepOutcome` for every current bytecode opcode, while keeping the synchronous VM as the compatibility execution path for `eval()`.
- [x] Add `IonRuntime`/`IonEvalFuture` scaffold; make pure CPU scripts run under `eval_async`.
- [x] Add host future table with by-ID cancellation.
- [x] Make one async host function future suspend and resume through runtime tables.
- [x] Add bridge-level `try/catch` and `?` tests around async host errors/results.
- [x] Add VM-level `try/catch` and `?` tests around continuation instruction errors.
- [x] Add VM-level `try/catch` and `?` tests around async host errors once async host calls are wired into bytecode dispatch.
- [x] Add fairness budget tests, including zero-instruction yield cases.
- [x] Add cancellation tests proving cancelled host futures are dropped.
- [x] Add Tokio timer table scaffold for `sleep` and `timeout`.
- [x] Make `eval_async` route `sleep(ms)` through a native Tokio timer future so normal-looking script sleep parks instead of blocking an OS thread.
- [x] Add runtime-managed Ion task table scaffold for `spawn` / `.await`.
- [x] Add async channel data-structure scaffold.
- [x] Expose native async `channel(capacity)` under `eval_async` with `send`, `recv`, `try_recv`, `recv_timeout`, and `close` methods backed by Tokio channel futures.
- [x] Drive spawned sibling Ion tasks while the root continuation is parked on a host future, so channel fan-in can make progress without requiring `.await` or `select`.
- [x] Expose `timeout(ms, fn)` as a native async runtime helper that races a pollable callback future against a Tokio timer and drops the callback future on timeout.
- [x] Add bytecode opcodes for `SpawnCall` and `AwaitTask`.
- [x] Add continuation runtime support for `spawn async_host(...)` and `.await` that parks on the host-future table.
- [x] Add bytecode `SelectTasks` support that races spawned branch tasks and resumes with the winning `(index, value)`.
- [x] Add compiler support for `async {}`, `spawn async_host(...)`, `.await`, and `select` under `async-runtime`.
- [x] Prove bytecode-spawned async host tasks can overlap while the awaiting continuation is parked.
- [x] Prove compiled bytecode `select` can pick the first ready async branch and run the winning branch body.
- [x] Route `eval_async` programs that reference async host functions through `IonRuntime` and `VmContinuation`; unsupported bytecode forms now return the compiler error instead of falling back to the bridge.
- [x] Prove `eval_async` can run an async host call inside a bytecode `for` loop, which the bridge cannot execute.
- [x] Add final structured join for unawaited spawned tasks at runtime completion while excluding internal `select` branch tasks.
- [x] Add structured nursery data-structure scaffold.
- [x] Add external queue based `EngineHandle::call_async` scaffold.
- [x] Wire `EngineHandle::call_async` into `IonRuntime` so host futures can schedule Ion callbacks that run as local Tokio-polled futures.
- [x] Prove an externally scheduled Ion callback can itself park on a nested async host future without blocking an OS thread.

The bridge-level load-bearing milestone is covered: `eval_async` has a test where two spawned host futures overlap on one Tokio runtime without `spawn_blocking`, and async host errors obey Ion `try/catch`. The bytecode runtime milestone is also covered: `SpawnCall`/`SpawnCallNamed` create pollable async tasks for host calls and Ion wrapper functions, `AwaitTask` parks the continuation on the host-future table, awaiting one task polls sibling spawned tasks so native network futures can make progress without an OS-thread handoff, and `SelectTasks` races branch task handles before normal bytecode dispatch binds the winner and evaluates the branch body. `IonRuntime` now drives async-host programs through the continuation VM, restores the engine environment on completion, joins unawaited spawned tasks before returning, supports bytecode imports plus host-struct and host-enum pattern matching/destructuring for async host results, runs `EngineHandle::call_async` callbacks as local futures, routes `sleep(ms)`, `timeout(ms, fn)`, and `channel(capacity)` to Tokio-backed native async operations under `eval_async`, and has opcode coverage for every current bytecode opcode. The old tree-walk bridge is no longer the production fallback for async-host programs; remaining unsupported language forms should be lowered to bytecode rather than implemented in a second runtime.

## Core Principle

Ion code stays synchronous-looking. Rust host async functions stay real Rust futures. The VM is the boundary: it stores continuations, polls host futures without blocking an OS thread, preserves Ion error semantics, and resumes scripts when Tokio wakes the relevant operation.
