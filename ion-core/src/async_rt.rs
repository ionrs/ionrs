//! Concurrency runtime abstraction for Ion's structured concurrency model.
//!
//! Provides a backend-agnostic interface. Two implementations:
//! - `async_rt_std` — uses `std::thread` + `std::sync::mpsc` (no external deps)
//! - `async_rt_tokio` — uses `tokio::task::spawn_blocking` + `tokio::sync::mpsc`
//!
//! The Ion script API is identical regardless of backend:
//! `async {}`, `spawn`, `.await`, `channel()`, `select`.

use std::fmt::Debug;
use std::sync::Arc;
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

    /// Signal cancellation to the task.
    fn cancel(&self);

    /// Check if cancellation was requested.
    fn is_cancelled(&self) -> bool;
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

// ---- Backend dispatch ----

/// Spawn a task using the active backend.
/// The closure runs a child interpreter on the spawned task's expression.
pub fn spawn_task<F>(f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce() -> Result<Value, IonError> + Send + 'static,
{
    #[cfg(feature = "concurrency-tokio")]
    {
        crate::async_rt_tokio::spawn_task(f)
    }
    #[cfg(not(feature = "concurrency-tokio"))]
    {
        crate::async_rt_std::spawn_task(f)
    }
}

/// Sleep for the given duration, blocking the current thread/task.
pub fn sleep(duration: Duration) {
    #[cfg(feature = "concurrency-tokio")]
    {
        crate::async_rt_tokio::sleep(duration);
    }
    #[cfg(not(feature = "concurrency-tokio"))]
    {
        std::thread::sleep(duration);
    }
}

/// Wait for any of the given tasks to complete.
/// Returns (index, result) of the first task that finishes.
/// Uses a channel internally — no polling/busy-wait.
pub fn wait_any(tasks: &[Arc<dyn TaskHandle>]) -> (usize, Result<Value, IonError>) {
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, task) in tasks.iter().enumerate() {
        let tx = tx.clone();
        let task = task.clone();
        std::thread::spawn(move || {
            let result = task.join();
            let _ = tx.send((i, result));
        });
    }
    rx.recv().unwrap()
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    #[cfg(feature = "concurrency-tokio")]
    {
        crate::async_rt_tokio::create_channel(buffer)
    }
    #[cfg(not(feature = "concurrency-tokio"))]
    {
        crate::async_rt_std::create_channel(buffer)
    }
}
