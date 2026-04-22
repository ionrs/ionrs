//! Example: embedding Ion inside a tokio application.
//!
//! Run with: cargo run --example tokio_host --features concurrency
//!
//! Demonstrates the two patterns you'll want when wiring Ion into a
//! tokio-based host:
//!
//! 1. **Ion eval from async code** — Ion's interpreter is synchronous
//!    and blocks. Wrap `engine.eval(...)` in
//!    `tokio::task::spawn_blocking` so it runs on the blocking pool
//!    rather than pinning an async worker.
//!
//! 2. **Tokio-backed builtins** — register a closure that captures a
//!    `tokio::runtime::Handle` and uses `handle.block_on(fut)` to
//!    drive async host work synchronously. Because Ion runs inside
//!    `spawn_blocking`, this is always called from a blocking-pool
//!    thread, never from an async worker, so `block_on` is safe.

use std::time::Duration;

use ion_core::engine::Engine;
use ion_core::value::Value;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // Capture the current tokio Handle so we can drive tokio futures
    // from inside an Ion builtin.
    let rt = tokio::runtime::Handle::current();

    // Ion's synchronous interpreter must run off the async workers.
    // Everything — Engine construction, builtin registration, script
    // evaluation — happens inside `spawn_blocking`.
    let script_out = tokio::task::spawn_blocking(move || {
        let mut engine = Engine::new();

        // Register a tokio-backed "sleep_ms" builtin. It captures the
        // tokio Handle from the outer async context.
        {
            let rt = rt.clone();
            engine.register_closure("tokio_sleep", move |args| {
                let ms = args
                    .first()
                    .and_then(|v| v.as_int())
                    .ok_or_else(|| "tokio_sleep(ms): ms must be int".to_string())?;
                rt.block_on(async move {
                    tokio::time::sleep(Duration::from_millis(ms as u64)).await;
                });
                Ok(Value::Int(ms))
            });
        }

        // Register a tokio-backed "fetch" stand-in. In a real host this
        // would be reqwest::get; here we just simulate latency.
        {
            let rt = rt.clone();
            engine.register_closure("fake_fetch", move |args| {
                let url = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "fake_fetch(url): url must be string".to_string())?
                    .to_string();
                let body = rt.block_on(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    format!("GET {} -> 200 OK", url)
                });
                Ok(Value::Str(body))
            });
        }

        // An Ion script that spawns three concurrent tasks, each of
        // which calls a tokio-backed builtin. The `async {}` nursery
        // waits for all three to finish; results are collected via
        // channels.
        engine.eval(r#"
            async {
                let (tx, rx) = channel(3);

                let _a = spawn {
                    tokio_sleep(30);
                    tx.send(fake_fetch("https://a"));
                };
                let _b = spawn {
                    tokio_sleep(10);
                    tx.send(fake_fetch("https://b"));
                };
                let _c = spawn {
                    tokio_sleep(20);
                    tx.send(fake_fetch("https://c"));
                };

                let mut results = [];
                let mut i = 0;
                while i < 3 {
                    if let Some(s) = rx.recv() {
                        results = results.push(s);
                    }
                    i = i + 1;
                }
                results
            }
        "#)
    })
    .await
    .expect("spawn_blocking panicked")
    .expect("ion eval failed");

    println!("Ion returned: {}", script_out);
}
