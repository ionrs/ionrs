# Ion Concurrency Model

## Feature Flags

- `async-runtime` — preferred native async runtime. Enables
  `Engine::eval_async`, `Engine::register_async_fn`, `EngineHandle`,
  Tokio timers, Tokio-backed channels, and bytecode continuations that park
  without blocking an OS thread.
- `legacy-threaded-concurrency` — legacy synchronous evaluation backend. It
  keeps the older `std::thread`/crossbeam implementation available for
  compatibility, but it is not the Tokio-native runtime.

If both features are enabled, `async-runtime` wins and the legacy thread
backend is not compiled. Test the legacy backend with `--no-default-features
--features legacy-threaded-concurrency`.

## Core Rule

Ion source stays mostly uncolored. A script calls a host async function like a
normal function:

```ion
fn load_user(id) {
    json::decode(http_get(f"/users/{id}"))
}
```

If `http_get` is registered with `Engine::register_async_fn` and the host runs
the script through `Engine::eval_async`, the runtime saves the current bytecode
continuation, returns `Poll::Pending` to Tokio, and resumes the script when the
host future wakes. It does not spawn a blocking OS thread to wait for the HTTP
request.

## Rust Host API

```rust,no_run
use ion_core::{Engine, Value};
use ion_core::error::IonError;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), IonError> {
    let mut engine = Engine::new();

    engine.register_async_fn("http_get", |args| async move {
        let url = args[0].as_str().unwrap_or("").to_string();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(Value::Str(format!("GET {url} -> 200 OK")))
    });

    let result = engine.eval_async(r#"
        async {
            let a = spawn http_get("/profile");
            let b = spawn http_get("/orders");
            [a.await, b.await]
        }
    "#).await?;

    println!("{result}");
    Ok(())
}
```

Synchronous `engine.eval(...)` rejects async host functions with a clear error.
Use `eval_async` when any script path may call `register_async_fn` functions.

## Primitives

- `async {}` — structured scope boundary. Unawaited child tasks are joined
  before the block completes.
- `spawn function_call(...)` — launch a function call as a child Ion task.
  Valid only inside an `async {}` scope.
- `handle.await` / `handle.await?` — wait for a child task result.
- `select {}` — race branch tasks. The first branch to finish wins; losing
  branch tasks are cancelled and dropped.
- `sleep(ms)` — native Tokio timer under `eval_async`.
- `timeout(ms, fn)` — run a callback as a pollable child future and return
  `Some(value)` if it wins, or `None` if the Tokio timer wins.
- `channel(n)` — bounded Tokio-backed channel returning `(tx, rx)`.
- `tx.send(value)` — park if the bounded channel is full.
- `rx.recv()` — park until a value is available or return `None` after close.
- `rx.try_recv()` — immediate receive attempt, returns `Option`.
- `rx.recv_timeout(ms)` — park until a value arrives or the timer expires.
- `tx.close()` — close the sender endpoint.

## Runtime Architecture

- `ion-core/src/async_runtime.rs` owns the native async runtime.
- `Engine::eval_async` returns `IonEvalFuture`, a Rust future that drives Ion
  tasks until completion or until all work is parked.
- Ion calls use explicit VM continuations: instruction pointer, stack, locals,
  call frames, exception handlers, iterators, and wait state are stored in heap
  runtime state instead of on the Rust stack.
- Host async calls are stored in a host future table and polled by the same
  `IonEvalFuture`.
- `spawn` creates an Ion task in the runtime task table. It does not create a
  Tokio task and does not create an OS thread.
- Timers use `tokio::time`; native channels use `tokio::sync::mpsc`.
- While the root task is parked, the runtime continues polling runnable sibling
  Ion tasks and pending host futures, so fan-in workflows can make progress.

## Structured Semantics

- No top-level `spawn`; an `async {}` nursery is required.
- Children are scoped to their parent nursery.
- If a child errors before it is awaited, the nursery cancels siblings and
  propagates the error.
- Dropping `IonEvalFuture` drops pending host futures and cancels runtime tasks.
- Ion `try/catch` and `?` semantics are preserved across async host suspension.
- Host structs/enums returned from async host functions can be matched and
  destructured like synchronous host values.

## Channels

```ion
fn produce(tx) {
    tx.send(http_get("/a"));
    tx.send(http_get("/b"));
    tx.close();
}

async {
    let (tx, rx) = channel(32);

    spawn produce(tx);

    let mut out = [];
    let mut next = rx.recv();
    while next != None {
        out = out.push(next.unwrap());
        next = rx.recv();
    }
    out
}
```

`rx.recv()` and `tx.send()` park Ion tasks on Tokio-backed operations. They do
not block the runtime thread.

## Select and Timeout

```ion
async {
    select {
        body = spawn http_get("/slow") => Some(body),
        _ = spawn sleep(250) => None,
    }
}
```

For callback-shaped timeouts:

```ion
let maybe_body = timeout(250, || http_get("/slow"));
```

`timeout` cancels and drops the callback future if the timer wins. If the
callback raises an Ion error, the error propagates instead of being converted
to `None`.

## Host Callbacks Into Ion

Hosts can call back into Ion from async host code without re-entering the VM
directly:

```rust,no_run
# use ion_core::{Engine, Value};
# use ion_core::error::IonError;
# fn install(engine: &mut Engine) {
let handle = engine.handle();
engine.register_async_fn("wait_for_event", move |_args| {
    let handle = handle.clone();
    async move {
        let event = Value::Str("ready".into());
        handle.call_async("on_event", vec![event]).await
    }
});
# }
```

`EngineHandle::call_async` enqueues the callback into the local Ion runtime.
The callback is evaluated as another pollable future and may itself call nested
async host functions.

## Embedding Inside a Tokio Host

Use a Tokio runtime or `LocalSet`, construct an `Engine`, register async host
functions, and await `eval_async`. A current-thread Tokio runtime is enough for
native network I/O because sockets and timers are reactor-driven while pending.

Do not wrap native async host calls in `Handle::block_on` or move Ion to
`spawn_blocking` just to wait for I/O. Blocking host functions can still be
registered with `register_fn` or `register_closure`, but those functions block
the runtime thread while they run. Put truly blocking work behind an explicit
host-side blocking pool.

## Legacy Backend

The older OS-thread backend is now named `legacy-threaded-concurrency` for
synchronous `eval()` programs. It uses `std::thread` for spawned tasks and
crossbeam channels. New Tokio embedding should use `async-runtime`; the legacy
backend is compatibility surface, not the target design for native async I/O.
