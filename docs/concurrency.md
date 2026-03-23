# Ion Concurrency Model

Feature-gated: `concurrency` (std-only, no external deps like tokio)

## Primitives
- `async {}` — scope boundary, waits for all spawned tasks
- `spawn expr` — launch child task, returns `Task<T>`
- `handle.await` / `handle.await?` — wait for result
- `handle.cancel()` — explicit cancellation (AtomicBool)
- `handle.is_finished()` / `handle.is_cancelled()` — status checks
- `select {}` — race multiple branches, losers cancelled
- `channel(n)` — bounded, backed by `std::sync::mpsc`
- `tx.send()` / `rx.recv()` / `rx.try_recv()` — send/recv
- `tx.close()` — close channel
- `await_timeout(handle, ms)` / `recv_timeout(rx, ms)` — timeouts

## Semantics
- No top-level spawn — `async {}` required
- Structured: all tasks scoped to parent
- Fail-fast by default (nursery model)
- No Mutex/shared mutable state — channels-only
- 12 concurrency tests in test suite
