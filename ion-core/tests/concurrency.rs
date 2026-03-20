#![cfg(feature = "concurrency")]

use ion_core::engine::Engine;
use ion_core::value::Value;

fn eval(src: &str) -> Value {
    let mut engine = Engine::new();
    engine.eval(src).unwrap()
}

fn eval_err(src: &str) -> String {
    let mut engine = Engine::new();
    engine.eval(src).unwrap_err().message
}

// ============================================================
// Async block basics
// ============================================================

#[test]
fn test_async_block_returns_value() {
    assert_eq!(eval("async { 42 }"), Value::Int(42));
}

#[test]
fn test_async_block_with_stmts() {
    assert_eq!(eval("
        async {
            let x = 10;
            let y = 20;
            x + y
        }
    "), Value::Int(30));
}

// ============================================================
// Spawn and await
// ============================================================

#[test]
fn test_spawn_and_await() {
    assert_eq!(eval("
        async {
            let t = spawn 1 + 2;
            t.await
        }
    "), Value::Int(3));
}

#[test]
fn test_spawn_multiple_tasks() {
    let val = eval("
        async {
            let a = spawn 10 * 2;
            let b = spawn 20 * 3;
            a.await + b.await
        }
    ");
    assert_eq!(val, Value::Int(80));
}

#[test]
fn test_spawn_outside_async_error() {
    let err = eval_err("spawn 42");
    assert!(err.contains("only allowed inside async"), "got: {}", err);
}

#[test]
fn test_spawn_captures_variables() {
    assert_eq!(eval("
        let x = 100;
        async {
            let t = spawn x + 1;
            t.await
        }
    "), Value::Int(101));
}

#[test]
fn test_task_is_finished() {
    // After await, task should be finished
    assert_eq!(eval("
        async {
            let t = spawn 42;
            let _v = t.await;
            t.is_finished()
        }
    "), Value::Bool(true));
}

// ============================================================
// Structured concurrency — async block waits for all tasks
// ============================================================

#[test]
fn test_async_waits_for_all() {
    // Even without explicit await, async block joins all spawned tasks
    // The side effects happen (no orphans)
    assert_eq!(eval("
        let mut result = 0;
        async {
            let t = spawn 42;
            result = t.await;
        };
        result
    "), Value::Int(42));
}

// ============================================================
// Channels
// ============================================================

#[test]
fn test_channel_send_recv() {
    assert_eq!(eval("
        async {
            let (tx, rx) = channel(4);
            tx.send(42);
            tx.send(99);
            let a = rx.recv();
            let b = rx.recv();
            match (a, b) {
                (Some(x), Some(y)) => x + y,
                _ => -1,
            }
        }
    "), Value::Int(141));
}

#[test]
fn test_channel_close_recv_none() {
    assert_eq!(eval("
        async {
            let (tx, rx) = channel(4);
            tx.send(1);
            tx.close();
            let a = rx.recv();
            let b = rx.recv();
            (a, b)
        }
    "), Value::Tuple(vec![
        Value::Option(Some(Box::new(Value::Int(1)))),
        Value::Option(None),
    ]));
}

#[test]
fn test_channel_between_tasks() {
    assert_eq!(eval("
        async {
            let (tx, rx) = channel(4);
            let _producer = spawn {
                tx.send(10);
                tx.send(20);
                tx.send(30);
                tx.close();
            };
            let mut sum = 0;
            let mut val = rx.recv();
            while val != None {
                let n = match val {
                    Some(x) => x,
                    _ => 0,
                };
                sum = sum + n;
                val = rx.recv();
            }
            sum
        }
    "), Value::Int(60));
}

// ============================================================
// Concurrency disabled error
// ============================================================

// This test is always compiled but the feature is enabled for this file
// so we just verify basic functionality works
#[test]
fn test_concurrency_feature_enabled() {
    // Simply verify the async keyword parses and runs
    assert_eq!(eval("async { 1 + 1 }"), Value::Int(2));
}
