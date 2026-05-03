//! Example: embedding Ion inside a Tokio application.
//!
//! Run with: cargo run --example tokio_host --features async-runtime
//!
//! This is the native async embedding path: host functions are real Tokio
//! futures registered with `register_async_fn`, and Ion is evaluated with
//! `eval_async`. Script calls remain synchronous-looking; the runtime parks
//! Ion continuations on Tokio futures instead of using `spawn_blocking` to
//! wait for I/O.

use std::time::Duration;

use ion_core::engine::Engine;
use ion_core::error::IonError;
use ion_core::value::Value;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), IonError> {
    let mut engine = Engine::new();

    engine.register_async_fn(ion_core::h!("tokio_sleep"), |args| async move {
        let ms = args
            .first()
            .and_then(Value::as_int)
            .ok_or_else(|| IonError::runtime("tokio_sleep(ms): ms must be int", 0, 0))?;
        tokio::time::sleep(Duration::from_millis(ms as u64)).await;
        Ok(Value::Int(ms))
    });

    engine.register_async_fn(ion_core::h!("fake_fetch"), |args| async move {
        let url = args
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| IonError::runtime("fake_fetch(url): url must be string", 0, 0))?
            .to_string();
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(Value::Str(format!("GET {url} -> 200 OK")))
    });

    let script_out = engine
        .eval_async(
            r#"
            fn produce(tx, url, ms) {
                tokio_sleep(ms);
                tx.send(fake_fetch(url));
            }

            async {
                let (tx, rx) = channel(3);

                spawn produce(tx, "https://a", 30);
                spawn produce(tx, "https://b", 10);
                spawn produce(tx, "https://c", 20);

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
        "#,
        )
        .await?;

    println!("Ion returned: {}", script_out);
    Ok(())
}
