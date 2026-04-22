# Ion Concurrency Model

## Feature Flags
- `concurrency` — enables the runtime (std::thread + crossbeam-channel)

## Primitives
- `async {}` — scope boundary, waits for all spawned tasks
- `spawn expr` — launch child task, returns `Task<T>`
- `handle.await` / `handle.await?` — wait for result
- `handle.cancel()` — cooperative cancellation; child interpreter polls
  at statement boundaries and exits with a runtime error
- `handle.is_finished()` / `handle.is_cancelled()` — status checks
- `select {}` — race multiple branches; losers are automatically
  `cancel()`ed after the winner is chosen
- `channel(n)` — bounded MPMC channel (crossbeam)
- `tx.send()` / `rx.recv()` / `rx.try_recv()` — send/recv
- `tx.close()` — close channel
- `await_timeout(handle, ms)` / `recv_timeout(rx, ms)` — timeouts
- `sleep(ms)` — sleep for given milliseconds
- `timeout(ms, fn)` — run function with time limit, returns `Option`
  (Some or None on timeout). A timed-out task is cancelled.

## Architecture
- `async_rt.rs` — trait interface (`TaskHandle`, `ChannelSender`,
  `ChannelReceiver`, `Nursery`, `Subscriber`), `wait_any` helper
- `async_rt_std.rs` — the implementation: one OS thread per task,
  crossbeam-channel for Value channels, condvar + subscriber list for
  completion notification

## Implementation notes
- `wait_any` uses the per-task `Subscriber` list — no extra OS threads
  spawned to watch pending tasks.
- Channel receivers are `crossbeam_channel::Receiver<Value>` (MPMC,
  Send+Sync without a Mutex wrapper).
- Cancellation is cooperative: `TaskHandle::cancel()` flips an atomic
  flag that the child `Interpreter` polls at every statement. No
  preemption, no kill.

## Semantics
- No top-level spawn — `async {}` required
- Structured: all tasks scoped to parent
- Fail-fast by default (nursery model)
- No Mutex/shared mutable state — channels-only

## Embedding inside a tokio host

Ion's interpreter is synchronous — `engine.eval(...)` blocks the
calling thread. That's fine inside a tokio app as long as you keep
Ion off the async worker pool. The conventional pattern (same one
used by `rhai`, `rlua`, etc.):

```rust,no_run
use ion_core::engine::Engine;
use ion_core::value::Value;

#[tokio::main]
async fn main() {
    let script = r#"async { spawn 1 + 2 }"#.to_string();

    let result = tokio::task::spawn_blocking(move || {
        let mut engine = Engine::new();
        engine.eval(&script)
    })
    .await
    .expect("blocking task panicked")
    .expect("ion eval failed");

    println!("{}", result);
}
```

Works identically on `flavor = "multi_thread"` and
`flavor = "current_thread"` — `spawn_blocking` moves Ion off the
async workers either way, so Ion's own `std::thread`-based `spawn`
never contends with tokio.

### Calling tokio-backed async code from Ion

Register a closure-backed builtin that captures a
`tokio::runtime::Handle` and uses `handle.block_on(fut)`:

```rust,no_run
# use ion_core::engine::Engine;
# use ion_core::value::Value;
# fn register_http(engine: &mut Engine, rt: tokio::runtime::Handle) {
engine.register_closure("fetch", move |args| {
    let url = args.first().and_then(|v| v.as_str())
        .ok_or("fetch(url): url must be string")?
        .to_string();
    let body = rt.block_on(async move {
        // reqwest::get(&url).await?.text().await
        Ok::<_, String>(format!("fetched: {}", url))
    })?;
    Ok(Value::Str(body))
});
# }
```

`block_on` is safe here because the closure is only ever called
from Ion's interpreter, which itself is running inside
`spawn_blocking` — not on an async worker. Ion's own `spawn` uses
`std::thread`, so builtin calls from spawned tasks are also off the
async pool.

See `ion-core/examples/tokio_host.rs` for a runnable end-to-end
example with three concurrent Ion tasks each calling a
tokio-backed builtin.

## Planned
The next major version moves to a cooperative single-thread scheduler
with stackful coroutines. That will eliminate the OS-thread-per-task
cost and remove the `Send` requirement on `Value`.
