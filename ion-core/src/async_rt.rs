//! Concurrency runtime for Ion's structured concurrency model.
//!
//! Uses `std::thread` for spawn and `std::sync::mpsc` for channels.
//! The `async {}` block enforces structured concurrency by waiting for
//! all child tasks before completing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::error::IonError;
use crate::value::Value;

/// Handle to a spawned task.
#[derive(Debug)]
pub struct TaskHandle {
    inner: Mutex<TaskState>,
    cancel_flag: Arc<AtomicBool>,
}

#[derive(Debug)]
enum TaskState {
    Running(Option<thread::JoinHandle<Result<Value, IonError>>>),
    Finished(Result<Value, IonError>),
}

impl TaskHandle {
    pub fn new(
        handle: thread::JoinHandle<Result<Value, IonError>>,
        cancel_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner: Mutex::new(TaskState::Running(Some(handle))),
            cancel_flag,
        }
    }

    /// Block until the task completes and return its result.
    pub fn join(&self) -> Result<Value, IonError> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            TaskState::Running(handle) => {
                let h = handle.take().unwrap();
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err(IonError::runtime("task panicked".to_string(), 0, 0)));
                let ret = result.clone();
                *state = TaskState::Finished(result);
                ret
            }
            TaskState::Finished(result) => result.clone(),
        }
    }

    /// Block until the task completes or the timeout expires.
    /// Returns `Some(result)` if finished, `None` if timed out.
    pub fn join_timeout(&self, timeout: Duration) -> Option<Result<Value, IonError>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let state = self.inner.lock().unwrap();
                if matches!(&*state, TaskState::Finished(_)) {
                    drop(state);
                    return Some(self.join());
                }
                // Check if the thread handle is finished
                if let TaskState::Running(Some(h)) = &*state {
                    if h.is_finished() {
                        drop(state);
                        return Some(self.join());
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Check if the task has completed without blocking.
    pub fn is_finished(&self) -> bool {
        let state = self.inner.lock().unwrap();
        match &*state {
            TaskState::Finished(_) => true,
            TaskState::Running(Some(h)) => h.is_finished(),
            TaskState::Running(None) => true,
        }
    }

    /// Signal cancellation to the task.
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// Check if cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }
}

impl Clone for TaskHandle {
    fn clone(&self) -> Self {
        panic!("TaskHandle should not be cloned directly; use Arc<TaskHandle>");
    }
}

/// A channel endpoint (sender or receiver).
#[derive(Debug, Clone)]
pub enum ChannelEnd {
    Sender(Arc<Mutex<Option<std::sync::mpsc::SyncSender<Value>>>>),
    Receiver(Arc<Mutex<std::sync::mpsc::Receiver<Value>>>),
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    let (tx, rx) = std::sync::mpsc::sync_channel(buffer);
    (
        Value::Channel(ChannelEnd::Sender(Arc::new(Mutex::new(Some(tx))))),
        Value::Channel(ChannelEnd::Receiver(Arc::new(Mutex::new(rx)))),
    )
}

/// Structured concurrency scope: tracks all spawned tasks within an async block.
#[derive(Debug)]
pub struct Nursery {
    tasks: Vec<Arc<TaskHandle>>,
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

    pub fn spawn(&mut self, handle: Arc<TaskHandle>) {
        self.tasks.push(handle);
    }

    /// Wait for all tasks, returning an error if any task failed.
    /// Implements fail-fast: returns the first error encountered.
    pub fn join_all(&self) -> Result<Vec<Value>, IonError> {
        let mut results = Vec::new();
        for task in &self.tasks {
            results.push(task.join()?);
        }
        Ok(results)
    }
}
