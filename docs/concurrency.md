# Ion Concurrency Model

## Feature Flags
- `concurrency` — std::thread backend (zero external deps)
- `concurrency-tokio` — tokio backend (implies `concurrency`, adds `tokio` dep)

Both backends expose the same Ion script API. The tokio backend uses
`spawn_blocking` (bounded thread pool) instead of raw `std::thread::spawn`,
and `tokio::sync::mpsc` instead of `std::sync::mpsc`.

## Primitives
- `async {}` — scope boundary, waits for all spawned tasks
- `spawn expr` — launch child task, returns `Task<T>`
- `handle.await` / `handle.await?` — wait for result
- `handle.cancel()` — explicit cancellation (AtomicBool)
- `handle.is_finished()` / `handle.is_cancelled()` — status checks
- `select {}` — race multiple branches, losers cancelled
- `channel(n)` — bounded channel
- `tx.send()` / `rx.recv()` / `rx.try_recv()` — send/recv
- `tx.close()` — close channel
- `await_timeout(handle, ms)` / `recv_timeout(rx, ms)` — timeouts
- `sleep(ms)` — sleep for given milliseconds
- `timeout(ms, fn)` — run function with time limit, returns `Option` (Some or None on timeout)

## Architecture
- `async_rt.rs` — trait interface (`TaskHandle`, `ChannelSender`, `ChannelReceiver`, `Nursery`)
- `async_rt_std.rs` — std::thread implementation
- `async_rt_tokio.rs` — tokio implementation with lazy global runtime
- Backend dispatch via `async_rt::spawn_task()` and `async_rt::create_channel()`

## Semantics
- No top-level spawn — `async {}` required
- Structured: all tasks scoped to parent
- Fail-fast by default (nursery model)
- No Mutex/shared mutable state — channels-only
- 16 concurrency tests in test suite

## Future Unlocks (tokio backend)
- Async I/O builtins (HTTP fetch, file read) via tokio's IO
- True cooperative scheduling for lighter task workloads
