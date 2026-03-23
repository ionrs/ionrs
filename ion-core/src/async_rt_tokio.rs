//! Tokio backend for Ion concurrency.
//!
//! Uses `tokio::task::spawn_blocking` for tasks (since the Ion interpreter
//! is synchronous/CPU-bound) and `tokio::sync::mpsc` for channels.
//! Active when `concurrency-tokio` feature is enabled.
//!
//! ## Future Unlocks (with tokio in place)
//! - Async I/O builtins (HTTP fetch, file read) via tokio's IO
//! - True cooperative scheduling for lighter task workloads
//! - `select` using `tokio::select!` for efficient branch racing

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::async_rt::{ChannelEnd, ChannelReceiver, ChannelSender, TaskHandle};
use crate::error::IonError;
use crate::value::Value;

/// Lazily-initialized tokio runtime for the Ion concurrency backend.
/// If the host already has a tokio runtime running (e.g. `#[tokio::main]`),
/// we use that. Otherwise we create a multi-threaded runtime on first use.
fn get_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

/// Get a runtime handle — either from the current context or from the lazy global.
fn runtime_handle() -> tokio::runtime::Handle {
    tokio::runtime::Handle::try_current().unwrap_or_else(|_| get_runtime().handle().clone())
}

// ---- Task ----

#[derive(Debug)]
pub struct TokioTaskHandle {
    inner: Mutex<TokioTaskState>,
    cancel_flag: Arc<AtomicBool>,
}

enum TokioTaskState {
    Running(Option<tokio::task::JoinHandle<Result<Value, IonError>>>),
    Finished(Result<Value, IonError>),
}

impl std::fmt::Debug for TokioTaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokioTaskState::Running(_) => write!(f, "Running"),
            TokioTaskState::Finished(_) => write!(f, "Finished"),
        }
    }
}

impl TaskHandle for TokioTaskHandle {
    fn join(&self) -> Result<Value, IonError> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            TokioTaskState::Running(handle) => {
                let h = handle.take().unwrap();
                let rt = runtime_handle();
                let result = rt.block_on(h).unwrap_or_else(|e| {
                    Err(IonError::runtime(format!("task panicked: {}", e), 0, 0))
                });
                let ret = result.clone();
                *state = TokioTaskState::Finished(result);
                ret
            }
            TokioTaskState::Finished(result) => result.clone(),
        }
    }

    fn join_timeout(&self, timeout: Duration) -> Option<Result<Value, IonError>> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            TokioTaskState::Finished(_) => {
                drop(state);
                Some(self.join())
            }
            TokioTaskState::Running(handle) => {
                let h = handle.take().unwrap();
                let rt = runtime_handle();
                let timed = rt.block_on(async { tokio::time::timeout(timeout, h).await });
                match timed {
                    Ok(join_result) => {
                        let result = join_result.unwrap_or_else(|e| {
                            Err(IonError::runtime(format!("task panicked: {}", e), 0, 0))
                        });
                        let ret = result.clone();
                        *state = TokioTaskState::Finished(result);
                        Some(ret)
                    }
                    Err(_) => None,
                }
            }
        }
    }

    fn is_finished(&self) -> bool {
        let state = self.inner.lock().unwrap();
        match &*state {
            TokioTaskState::Finished(_) => true,
            TokioTaskState::Running(Some(h)) => h.is_finished(),
            TokioTaskState::Running(None) => true,
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }
}

/// Spawn a task on tokio's blocking thread pool.
pub fn spawn_task<F>(f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce() -> Result<Value, IonError> + Send + 'static,
{
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let rt = runtime_handle();
    let handle = rt.spawn_blocking(f);
    Arc::new(TokioTaskHandle {
        inner: Mutex::new(TokioTaskState::Running(Some(handle))),
        cancel_flag,
    })
}

// ---- Sleep ----

/// Sleep for the given duration.
/// Uses std::thread::sleep since the Ion interpreter is synchronous.
pub fn sleep(duration: Duration) {
    std::thread::sleep(duration);
}

// ---- Channels ----

#[derive(Debug)]
pub struct TokioChannelSender {
    inner: Mutex<Option<mpsc::Sender<Value>>>,
}

impl ChannelSender for TokioChannelSender {
    fn send(&self, val: Value) -> Result<(), IonError> {
        let guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(sender) => {
                // Use blocking_send since we're in a sync context
                sender
                    .blocking_send(val)
                    .map_err(|e| IonError::runtime(format!("channel send failed: {}", e), 0, 0))
            }
            None => Err(IonError::runtime("channel is closed".to_string(), 0, 0)),
        }
    }

    fn close(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = None;
    }
}

#[derive(Debug)]
pub struct TokioChannelReceiver {
    inner: Mutex<mpsc::Receiver<Value>>,
}

impl ChannelReceiver for TokioChannelReceiver {
    fn recv(&self) -> Option<Value> {
        let mut receiver = self.inner.lock().unwrap();
        // Use blocking_recv since we're in a sync context
        receiver.blocking_recv()
    }

    fn try_recv(&self) -> Option<Value> {
        let mut receiver = self.inner.lock().unwrap();
        receiver.try_recv().ok()
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<Value> {
        let mut receiver = self.inner.lock().unwrap();
        let rt = runtime_handle();
        rt.block_on(async {
            tokio::time::timeout(timeout, receiver.recv())
                .await
                .unwrap_or_default()
        })
    }
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    let (tx, rx) = mpsc::channel(buffer);
    (
        Value::Channel(ChannelEnd::Sender(Arc::new(TokioChannelSender {
            inner: Mutex::new(Some(tx)),
        }))),
        Value::Channel(ChannelEnd::Receiver(Arc::new(TokioChannelReceiver {
            inner: Mutex::new(rx),
        }))),
    )
}
