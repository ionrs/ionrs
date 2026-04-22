//! Concurrency runtime for Ion's structured concurrency model.
//!
//! Single backend (std::thread + crossbeam-channel). Provides:
//! - `TaskHandle`: joinable, cancellable, subscribable task handles
//! - `Nursery`: structured concurrency scope (async {} in script)
//! - `ChannelSender` / `ChannelReceiver`: MPMC channels for script values
//! - `wait_any`: race N tasks without spawning extra watcher threads
//!
//! The trait surface is preserved so the cooperative scheduler in the
//! next major version can slot in without changing callers.

use std::fmt::Debug;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::error::IonError;
use crate::value::Value;

// ---- Task Handle Trait ----

/// Backend-agnostic handle to a spawned task.
pub trait TaskHandle: Send + Sync + Debug {
    /// Block until the task completes and return its result.
    fn join(&self) -> Result<Value, IonError>;

    /// Block until the task completes or the timeout expires.
    fn join_timeout(&self, timeout: Duration) -> Option<Result<Value, IonError>>;

    /// Check if the task has completed without blocking.
    fn is_finished(&self) -> bool;

    /// Signal cooperative cancellation to the task. The task polls the
    /// flag at statement boundaries and terminates with a runtime error.
    fn cancel(&self);

    /// Check if cancellation was requested.
    fn is_cancelled(&self) -> bool;

    /// Install a one-shot subscriber that is notified when the task
    /// completes. If the task is already done, the rendezvous is
    /// updated immediately. Used by `wait_any` to race tasks without
    /// spawning extra threads.
    fn subscribe(&self, sub: Subscriber);
}

/// A one-shot subscription: when the task finishes, the task runtime
/// sets `rendezvous.0` to `Some(my_index)` (if still `None`) and
/// notifies `rendezvous.1`.
#[derive(Debug)]
pub struct Subscriber {
    pub rendezvous: Arc<(Mutex<Option<usize>>, Condvar)>,
    pub my_index: usize,
}

// ---- Channel Traits ----

/// Sending end of a channel.
pub trait ChannelSender: Send + Sync + Debug {
    fn send(&self, val: Value) -> Result<(), IonError>;
    fn close(&self);
}

/// Receiving end of a channel.
pub trait ChannelReceiver: Send + Sync + Debug {
    fn recv(&self) -> Option<Value>;
    fn try_recv(&self) -> Option<Value>;
    fn recv_timeout(&self, timeout: Duration) -> Option<Value>;
}

/// Type-erased channel endpoint stored in `Value::Channel`.
#[derive(Debug, Clone)]
pub enum ChannelEnd {
    Sender(Arc<dyn ChannelSender>),
    Receiver(Arc<dyn ChannelReceiver>),
}

// ---- Nursery ----

/// Structured concurrency scope: tracks all spawned tasks within an async block.
#[derive(Debug)]
pub struct Nursery {
    tasks: Vec<Arc<dyn TaskHandle>>,
}

impl Default for Nursery {
    fn default() -> Self {
        Self::new()
    }
}

impl Nursery {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn spawn(&mut self, handle: Arc<dyn TaskHandle>) {
        self.tasks.push(handle);
    }

    /// Wait for all tasks, returning an error if any task failed.
    pub fn join_all(&self) -> Result<Vec<Value>, IonError> {
        let mut results = Vec::new();
        for task in &self.tasks {
            results.push(task.join()?);
        }
        Ok(results)
    }
}

// ---- Backend-facing helpers ----

use std::sync::atomic::AtomicBool;

/// Spawn a task. The caller's cancel flag is threaded into the closure
/// via `spawn_task_with_cancel` — this is the plain variant when the
/// caller doesn't need to share the flag with the task body.
pub fn spawn_task<F>(f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce() -> Result<Value, IonError> + Send + 'static,
{
    let flag = Arc::new(AtomicBool::new(false));
    spawn_task_with_cancel(flag, move |_| f())
}

/// Spawn a task with a pre-allocated cancel flag. The flag is shared
/// with the returned handle AND handed to `f` so it can be forwarded
/// to a child interpreter for cooperative cancellation.
pub fn spawn_task_with_cancel<F>(cancel: Arc<AtomicBool>, f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce(Arc<AtomicBool>) -> Result<Value, IonError> + Send + 'static,
{
    crate::async_rt_std::spawn_task_with_cancel(cancel, f)
}

/// Sleep for the given duration, blocking the current thread/task.
pub fn sleep(duration: Duration) {
    std::thread::sleep(duration);
}

/// Wait for any of the given tasks to complete. Returns
/// `(index, result)` of the first to finish. Uses the subscriber
/// mechanism — no extra OS threads are spawned.
pub fn wait_any(tasks: &[Arc<dyn TaskHandle>]) -> (usize, Result<Value, IonError>) {
    let rendezvous: Arc<(Mutex<Option<usize>>, Condvar)> =
        Arc::new((Mutex::new(None), Condvar::new()));

    for (i, task) in tasks.iter().enumerate() {
        task.subscribe(Subscriber {
            rendezvous: rendezvous.clone(),
            my_index: i,
        });
    }

    let (mtx, cv) = &*rendezvous;
    let mut guard = mtx.lock().unwrap();
    while guard.is_none() {
        guard = cv.wait(guard).unwrap();
    }
    let winner = guard.expect("rendezvous set but reads as None");
    drop(guard);

    (winner, tasks[winner].join())
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    crate::async_rt_std::create_channel(buffer)
}
