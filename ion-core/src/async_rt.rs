//! Concurrency runtime for Ion's structured concurrency model.
//!
//! Uses `std::thread` for spawn (runtime-agnostic) and `futures::channel::mpsc`
//! for bounded channels. The `async {}` block enforces structured concurrency
//! by waiting for all child tasks before completing.

use std::sync::{Arc, Mutex};
use std::thread;

use crate::error::IonError;
use crate::value::Value;

/// Handle to a spawned task.
#[derive(Debug)]
pub struct TaskHandle {
    inner: Mutex<TaskState>,
}

#[derive(Debug)]
enum TaskState {
    Running(Option<thread::JoinHandle<Result<Value, IonError>>>),
    Finished(Result<Value, IonError>),
}

impl TaskHandle {
    pub fn new(handle: thread::JoinHandle<Result<Value, IonError>>) -> Self {
        Self { inner: Mutex::new(TaskState::Running(Some(handle))) }
    }

    /// Block until the task completes and return its result.
    pub fn join(&self) -> Result<Value, IonError> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            TaskState::Running(handle) => {
                let h = handle.take().unwrap();
                let result = h.join().unwrap_or_else(|_| {
                    Err(IonError::runtime("task panicked".to_string(), 0, 0))
                });
                let ret = result.clone();
                *state = TaskState::Finished(result);
                ret
            }
            TaskState::Finished(result) => result.clone(),
        }
    }

    /// Check if the task has completed without blocking.
    pub fn is_finished(&self) -> bool {
        let state = self.inner.lock().unwrap();
        matches!(&*state, TaskState::Finished(_))
    }
}

impl Clone for TaskHandle {
    fn clone(&self) -> Self {
        // TaskHandle is behind Arc, so Clone should not be called directly.
        // This is needed for Value::Clone but tasks are always wrapped in Arc.
        panic!("TaskHandle should not be cloned directly; use Arc<TaskHandle>");
    }
}

/// A channel endpoint (sender or receiver).
#[derive(Debug, Clone)]
pub enum ChannelEnd {
    Sender(Arc<Mutex<futures::channel::mpsc::Sender<Value>>>),
    Receiver(Arc<Mutex<futures::channel::mpsc::Receiver<Value>>>),
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    let (tx, rx) = futures::channel::mpsc::channel(buffer);
    (
        Value::Channel(ChannelEnd::Sender(Arc::new(Mutex::new(tx)))),
        Value::Channel(ChannelEnd::Receiver(Arc::new(Mutex::new(rx)))),
    )
}

/// Structured concurrency scope: tracks all spawned tasks within an async block.
#[derive(Debug)]
pub struct Nursery {
    tasks: Vec<Arc<TaskHandle>>,
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
