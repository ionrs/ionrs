//! std::thread backend for Ion concurrency.
//!
//! Uses `std::thread::spawn` for tasks and `std::sync::mpsc` for channels.
//! Zero external dependencies. Active when `concurrency` is enabled
//! but `concurrency-tokio` is not.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::async_rt::{ChannelEnd, ChannelReceiver, ChannelSender, TaskHandle};
use crate::error::IonError;
use crate::value::Value;

// ---- Task ----

#[derive(Debug)]
pub struct StdTaskHandle {
    inner: Mutex<StdTaskState>,
    cancel_flag: Arc<AtomicBool>,
}

#[derive(Debug)]
enum StdTaskState {
    Running(Option<thread::JoinHandle<Result<Value, IonError>>>),
    Finished(Result<Value, IonError>),
}

impl TaskHandle for StdTaskHandle {
    fn join(&self) -> Result<Value, IonError> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            StdTaskState::Running(handle) => {
                let h = handle.take().unwrap();
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err(IonError::runtime("task panicked".to_string(), 0, 0)));
                let ret = result.clone();
                *state = StdTaskState::Finished(result);
                ret
            }
            StdTaskState::Finished(result) => result.clone(),
        }
    }

    fn join_timeout(&self, timeout: Duration) -> Option<Result<Value, IonError>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let state = self.inner.lock().unwrap();
                if matches!(&*state, StdTaskState::Finished(_)) {
                    drop(state);
                    return Some(self.join());
                }
                if let StdTaskState::Running(Some(h)) = &*state {
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

    fn is_finished(&self) -> bool {
        let state = self.inner.lock().unwrap();
        match &*state {
            StdTaskState::Finished(_) => true,
            StdTaskState::Running(Some(h)) => h.is_finished(),
            StdTaskState::Running(None) => true,
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }
}

/// Spawn a task on a new OS thread.
pub fn spawn_task<F>(f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce() -> Result<Value, IonError> + Send + 'static,
{
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let handle = thread::spawn(f);
    Arc::new(StdTaskHandle {
        inner: Mutex::new(StdTaskState::Running(Some(handle))),
        cancel_flag,
    })
}

/// Get the cancel flag for cooperative cancellation checks.
/// Called before spawn_task so the flag can be shared with the child interpreter.
pub fn new_cancel_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

// ---- Channels ----

#[derive(Debug)]
pub struct StdChannelSender {
    inner: Mutex<Option<std::sync::mpsc::SyncSender<Value>>>,
}

impl ChannelSender for StdChannelSender {
    fn send(&self, val: Value) -> Result<(), IonError> {
        let guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(sender) => sender
                .send(val)
                .map_err(|e| IonError::runtime(format!("channel send failed: {}", e), 0, 0)),
            None => Err(IonError::runtime("channel is closed".to_string(), 0, 0)),
        }
    }

    fn close(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = None;
    }
}

#[derive(Debug)]
pub struct StdChannelReceiver {
    inner: Mutex<std::sync::mpsc::Receiver<Value>>,
}

impl ChannelReceiver for StdChannelReceiver {
    fn recv(&self) -> Option<Value> {
        let receiver = self.inner.lock().unwrap();
        receiver.recv().ok()
    }

    fn try_recv(&self) -> Option<Value> {
        let receiver = self.inner.lock().unwrap();
        receiver.try_recv().ok()
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<Value> {
        let receiver = self.inner.lock().unwrap();
        receiver.recv_timeout(timeout).ok()
    }
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    let (tx, rx) = std::sync::mpsc::sync_channel(buffer);
    (
        Value::Channel(ChannelEnd::Sender(Arc::new(StdChannelSender {
            inner: Mutex::new(Some(tx)),
        }))),
        Value::Channel(ChannelEnd::Receiver(Arc::new(StdChannelReceiver {
            inner: Mutex::new(rx),
        }))),
    )
}
