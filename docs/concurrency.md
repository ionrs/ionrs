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

## Planned
The next major version moves to a cooperative single-thread scheduler
with stackful coroutines. That will eliminate the OS-thread-per-task
cost and remove the `Send` requirement on `Value`.
