//! std::thread backend for legacy-threaded-concurrency.
//!
//! Each `spawn` creates one OS thread. The thread wrapper signals a
//! completion condvar and notifies any `Subscriber`s when the body
//! returns, so `wait_any` and `join_timeout` don't need extra threads
//! or busy-polling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::async_rt::{ChannelEnd, ChannelReceiver, ChannelSender, Subscriber, TaskHandle};
use crate::error::IonError;
use crate::value::Value;

// ---- Task ----

/// Shared completion state between the worker thread and the handle.
#[derive(Debug)]
struct TaskSlot {
    /// Protected state: result once finished, and pending subscribers
    /// that want to be notified on completion.
    inner: Mutex<SlotInner>,
    /// Broadcast when the task finishes. Used by `join`/`join_timeout`.
    cv: Condvar,
}

#[derive(Debug)]
struct SlotInner {
    state: SlotState,
    /// Subscribers that get one-shot-notified when the task finishes.
    subs: Vec<Subscriber>,
}

#[derive(Debug)]
enum SlotState {
    Running,
    /// Result is `Some` until exactly one `join()` consumes it, then
    /// replaced with `None`. Subsequent `join()` calls return `None`
    /// from the state (we preserve the result by cloning before taking).
    Finished(Result<Value, IonError>),
}

#[derive(Debug)]
pub struct StdTaskHandle {
    slot: Arc<TaskSlot>,
    join_handle: Mutex<Option<thread::JoinHandle<()>>>,
    cancel_flag: Arc<AtomicBool>,
}

impl TaskHandle for StdTaskHandle {
    fn join(&self) -> Result<Value, IonError> {
        // Ensure the thread has exited before reading the result. We
        // take the JoinHandle under its own lock (it's not held across
        // the wait on the condvar to avoid contention with subscribers).
        let handle_opt = self.join_handle.lock().unwrap().take();
        if let Some(h) = handle_opt {
            // Wait on the condvar first (so panicking threads still
            // wake us via the panic path inside the wrapper).
            let mut inner = self.slot.inner.lock().unwrap();
            while matches!(inner.state, SlotState::Running) {
                inner = self.slot.cv.wait(inner).unwrap();
            }
            drop(inner);
            let _ = h.join();
        } else {
            // Another joiner already consumed the handle. Just wait
            // on the condvar until the state is Finished.
            let mut inner = self.slot.inner.lock().unwrap();
            while matches!(inner.state, SlotState::Running) {
                inner = self.slot.cv.wait(inner).unwrap();
            }
        }
        let inner = self.slot.inner.lock().unwrap();
        match &inner.state {
            SlotState::Finished(r) => r.clone(),
            SlotState::Running => Err(IonError::runtime(
                "task completion signalled but state still Running".to_string(),
                0,
                0,
            )),
        }
    }

    fn join_timeout(&self, timeout: Duration) -> Option<Result<Value, IonError>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut inner = self.slot.inner.lock().unwrap();
        while matches!(inner.state, SlotState::Running) {
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (g, res) = self.slot.cv.wait_timeout(inner, deadline - now).unwrap();
            inner = g;
            if res.timed_out() && matches!(inner.state, SlotState::Running) {
                return None;
            }
        }
        // Finished — reap the join handle opportunistically.
        drop(inner);
        if let Some(h) = self.join_handle.lock().unwrap().take() {
            let _ = h.join();
        }
        let inner = self.slot.inner.lock().unwrap();
        match &inner.state {
            SlotState::Finished(r) => Some(r.clone()),
            SlotState::Running => None,
        }
    }

    fn is_finished(&self) -> bool {
        let inner = self.slot.inner.lock().unwrap();
        matches!(inner.state, SlotState::Finished(_))
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    fn subscribe(&self, sub: Subscriber) {
        let mut inner = self.slot.inner.lock().unwrap();
        if matches!(inner.state, SlotState::Finished(_)) {
            drop(inner);
            notify_subscriber(sub);
        } else {
            inner.subs.push(sub);
        }
    }
}

fn notify_subscriber(sub: Subscriber) {
    let (mtx, cv) = &*sub.rendezvous;
    let mut guard = mtx.lock().unwrap();
    if guard.is_none() {
        *guard = Some(sub.my_index);
        cv.notify_one();
    }
}

/// Spawn a task on a new OS thread, threading the cancel flag into
/// the body closure so a child interpreter can poll it.
pub fn spawn_task_with_cancel<F>(cancel_flag: Arc<AtomicBool>, f: F) -> Arc<dyn TaskHandle>
where
    F: FnOnce(Arc<AtomicBool>) -> Result<Value, IonError> + Send + 'static,
{
    let slot = Arc::new(TaskSlot {
        inner: Mutex::new(SlotInner {
            state: SlotState::Running,
            subs: Vec::new(),
        }),
        cv: Condvar::new(),
    });

    let worker_slot = slot.clone();
    let worker_cancel = cancel_flag.clone();
    let join_handle = thread::spawn(move || {
        // Catch panics so they become an Ion runtime error rather than
        // poisoning the mutex. UnwindSafe on a captured closure is awkward;
        // use AssertUnwindSafe since the only shared state (slot) is
        // only read after we set Finished below.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(worker_cancel)));
        let result = match result {
            Ok(r) => r,
            Err(_) => Err(IonError::runtime("task panicked".to_string(), 0, 0)),
        };
        let subs_to_wake = {
            let mut inner = worker_slot.inner.lock().unwrap();
            inner.state = SlotState::Finished(result);
            std::mem::take(&mut inner.subs)
        };
        worker_slot.cv.notify_all();
        for sub in subs_to_wake {
            notify_subscriber(sub);
        }
    });

    Arc::new(StdTaskHandle {
        slot,
        join_handle: Mutex::new(Some(join_handle)),
        cancel_flag,
    })
}

// ---- Channels ----

use crossbeam_channel::{bounded, Receiver, Sender};

#[derive(Debug)]
pub struct StdChannelSender {
    inner: Mutex<Option<Sender<Value>>>,
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
    // crossbeam Receiver is Send+Sync, no mutex needed.
    inner: Receiver<Value>,
}

impl ChannelReceiver for StdChannelReceiver {
    fn recv(&self) -> Option<Value> {
        self.inner.recv().ok()
    }

    fn try_recv(&self) -> Option<Value> {
        self.inner.try_recv().ok()
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<Value> {
        self.inner.recv_timeout(timeout).ok()
    }
}

/// Create a bounded channel pair, returning (sender_value, receiver_value).
pub fn create_channel(buffer: usize) -> (Value, Value) {
    let (tx, rx) = bounded::<Value>(buffer.max(1));
    (
        Value::Channel(ChannelEnd::Sender(Arc::new(StdChannelSender {
            inner: Mutex::new(Some(tx)),
        }))),
        Value::Channel(ChannelEnd::Receiver(Arc::new(StdChannelReceiver {
            inner: rx,
        }))),
    )
}
