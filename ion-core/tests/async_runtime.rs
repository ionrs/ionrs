#![cfg(feature = "async-runtime")]

use indexmap::IndexMap;
use ion_core::ast::{BinOp, Expr, ExprKind, Param, Span};
use ion_core::async_runtime::{
    run_budgeted_steps, step_task, step_task_with_host_futures, AsyncChannel, ChannelRecv,
    ChannelSend, ChannelTable, ChunkArena, HostFutureTable, IonTask, NurseryState, NurseryTable,
    StepOutcome, TaskAwait, TaskId, TaskRunOutcome, TaskState, TaskTable, TimerTable,
    VmContinuation,
};
use ion_core::bytecode::{Chunk, Op};
use ion_core::compiler::Compiler;
use ion_core::engine::Engine;
use ion_core::error::{ErrorKind, IonError};
use ion_core::host_types::{HostEnumDef, HostStructDef, HostVariantDef};
use ion_core::lexer::Lexer;
use ion_core::module::Module;
use ion_core::parser::Parser;
use ion_core::stdlib::{OutputHandler, OutputStream};
use ion_core::value::{AsyncBuiltinClosureFn, BuiltinClosureFn, IonFn, Value};
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::Duration;

#[tokio::test]
async fn eval_async_runs_sync_script_for_scaffolding() {
    let mut engine = Engine::new();
    let value = engine.eval_async("1 + 2").await.unwrap();
    assert_eq!(value, Value::Int(3));
}

#[tokio::test]
async fn eval_async_preserves_full_sync_eval_for_scripts_without_async_hosts() {
    let mut engine = Engine::new();
    let value = engine
        .eval_async(
            r#"
            let mut total = 0;
            for x in [1, 2, 3] {
                total += x;
            }
            total
            "#,
        )
        .await
        .unwrap();
    assert_eq!(value, Value::Int(6));
}

// Removed: `sync_eval_rejects_async_host_function`,
// `async_module_function_rejects_sync_eval`, and
// `async_host_function_can_return_ion_error_type`. Each tested that a sync
// `Engine::eval` call rejected an async host function. As of the fs/path
// release, sync `Engine::eval` is removed at compile time under the
// `async-runtime` feature, so these scenarios cannot be expressed in source.
// The compile-time gate is a stronger guarantee than the previous runtime
// rejection, and the IonError-return path is exercised by every other test
// that registers an async builtin.

#[allow(dead_code)]
fn _placeholder_for_removed_sync_rejection_tests() {
    // Intentionally empty — keeps a hook in case we ever revive these
    // assertions through a different mechanism (e.g. a deny-list of fn names).
    let _ = true;
}

#[cfg(any())]
#[test]
fn sync_eval_rejects_async_host_function() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later"), |_args| async { Ok(Value::Int(7)) });

    let err = engine.eval("later()").unwrap_err();
    assert!(
        err.message.contains("async host function cannot be called"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn async_module_function_runs_under_eval_async() {
    let mut engine = Engine::new();

    let mut sensor = Module::new(ion_core::h!("sensor"));
    sensor.register_async_fn(ion_core::h!("call"), |args| async move {
        Ok(Value::Int(args.len() as i64))
    });
    engine.register_module(sensor);

    let value = engine
        .eval_async(
            r#"
            sensor::call("jobs.claim", #{})
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(2));
}

// See note above on the removed sync-rejection tests.
#[cfg(any())]
#[test]
fn async_module_function_rejects_sync_eval() {
    let mut engine = Engine::new();

    let mut sensor = Module::new(ion_core::h!("sensor"));
    sensor.register_async_fn(ion_core::h!("call"), |_args| async move { Ok(Value::Unit) });
    engine.register_module(sensor);

    let err = engine.eval("sensor::call()").unwrap_err();
    assert!(
        err.message.contains("async host function cannot be called")
            || err.message.contains("use eval_async"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn eval_async_calls_async_host_function_without_coloring_ion_code() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        Ok(Value::Int(args[0].as_int().unwrap() + 1))
    });

    let value = engine
        .eval_async(
            r#"
            fn load(x) {
                later(x) + 1
            }

            load(40)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn eval_async_uses_bytecode_runtime_for_async_host_inside_for_loop() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        Ok(Value::Int(args[0].as_int().unwrap()))
    });

    let value = engine
        .eval_async(
            r#"
            let mut total = 0;
            for x in [20, 22] {
                total += later(x);
            }
            total
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[test]
fn eval_async_parks_on_pending_host_future() {
    let mut engine = Engine::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
    let rx = Rc::new(RefCell::new(Some(rx)));
    let host_rx = Rc::clone(&rx);
    engine.register_async_fn(ion_core::h!("later"), move |_args| {
        let rx = host_rx.borrow_mut().take().unwrap();
        async move {
            rx.await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))
        }
    });

    let mut fut = Box::pin(engine.eval_async("later() + 1"));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    tx.send(Value::Int(41)).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
}

#[tokio::test]
async fn eval_async_sleep_uses_tokio_timer_without_async_host_registration() {
    let mut engine = Engine::new();
    let mut fut = Box::pin(engine.eval_async("sleep(1); 42"));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    tokio::time::sleep(Duration::from_millis(2)).await;

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
}

#[tokio::test]
async fn eval_async_sleep_overlay_does_not_replace_host_binding() {
    let mut engine = Engine::new();
    engine.register_fn(ion_core::h!("sleep"), |_args| Ok(Value::Int(99)));

    let value = engine.eval_async("sleep(1)").await.unwrap();

    assert_eq!(value, Value::Int(99));
}

#[tokio::test]
async fn eval_async_native_channel_sends_and_receives_without_threads() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            let (tx, rx) = channel(1);
            tx.send(41);
            rx.recv()
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Option(Some(Box::new(Value::Int(41)))));
}

#[tokio::test]
async fn eval_async_native_channel_recv_parks_until_spawned_sender_runs() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            fn send_later(tx) {
                sleep(1);
                tx.send(42)
            }

            async {
                let (tx, rx) = channel(1);
                spawn send_later(tx);
                rx.recv()
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Option(Some(Box::new(Value::Int(42)))));
}

#[tokio::test]
async fn eval_async_native_channel_try_recv_and_timeout_return_none_when_empty() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            let (tx, rx) = channel(1);
            [rx.try_recv(), rx.recv_timeout(1)]
            "#,
        )
        .await
        .unwrap();

    assert_eq!(
        value,
        Value::List(vec![Value::Option(None), Value::Option(None)])
    );
}

#[tokio::test]
async fn eval_async_native_channel_close_drains_buffer_then_returns_none() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            let (tx, rx) = channel(1);
            tx.send(7);
            tx.close();
            [rx.recv(), rx.recv()]
            "#,
        )
        .await
        .unwrap();

    assert_eq!(
        value,
        Value::List(vec![
            Value::Option(Some(Box::new(Value::Int(7)))),
            Value::Option(None),
        ])
    );
}

#[tokio::test]
async fn eval_async_timeout_returns_some_when_callback_finishes() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            timeout(50, || {
                sleep(1);
                42
            })
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Option(Some(Box::new(Value::Int(42)))));
}

#[tokio::test]
async fn eval_async_timeout_returns_none_when_callback_expires() {
    let mut engine = Engine::new();

    let value = engine
        .eval_async(
            r#"
            timeout(1, || {
                sleep(50);
                42
            })
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Option(None));
}

#[tokio::test]
async fn eval_async_timeout_propagates_callback_errors() {
    let mut engine = Engine::new();

    let err = engine
        .eval_async(
            r#"
            timeout(50, || {
                1 / 0
            })
            "#,
        )
        .await
        .unwrap_err();

    assert!(err.message.contains("division by zero"));
}

#[tokio::test]
async fn eval_async_try_catch_observes_async_host_error() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("fail_later"), |_args| async {
        Err(IonError::runtime("network failed", 3, 9))
    });

    let value = engine
        .eval_async(
            r#"
            try {
                fail_later()
            } catch err {
                "caught: " + err
            }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(value, Value::Str("caught: network failed".into()));
}

#[tokio::test]
async fn eval_async_try_operator_preserves_function_boundary_result_semantics() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("fallible_later"), |_args| async {
        Ok(Value::Result(Err(Box::new(Value::Str("bad".into())))))
    });

    let value = engine
        .eval_async(
            r#"
            fn load() {
                fallible_later()?
            }

            load()
            "#,
        )
        .await
        .unwrap();

    assert_eq!(
        value,
        Value::Result(Err(Box::new(Value::Str("bad".into()))))
    );
}

#[test]
fn eval_async_spawned_host_futures_overlap_without_os_threads() {
    let mut engine = Engine::new();
    let (tx_a, rx_a) = tokio::sync::oneshot::channel::<Value>();
    let (tx_b, rx_b) = tokio::sync::oneshot::channel::<Value>();
    let receivers = Rc::new(RefCell::new(vec![Some(rx_a), Some(rx_b)]));
    let active = Rc::new(Cell::new(0));
    let max_active = Rc::new(Cell::new(0));

    let host_receivers = Rc::clone(&receivers);
    let host_active = Rc::clone(&active);
    let host_max_active = Rc::clone(&max_active);
    engine.register_async_fn(ion_core::h!("later"), move |args| {
        let idx = args[0].as_int().unwrap() as usize;
        let rx = host_receivers.borrow_mut()[idx].take().unwrap();
        let active = Rc::clone(&host_active);
        let max_active = Rc::clone(&host_max_active);
        async move {
            let now_active = active.get() + 1;
            active.set(now_active);
            max_active.set(max_active.get().max(now_active));
            let value = rx
                .await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))?;
            active.set(active.get() - 1);
            Ok(value)
        }
    });

    let mut fut = Box::pin(engine.eval_async(
        r#"
        async {
            let a = spawn later(0);
            let b = spawn later(1);
            a.await + b.await
        }
        "#,
    ));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    assert_eq!(max_active.get(), 2);

    tx_a.send(Value::Int(20)).unwrap();
    tx_b.send(Value::Int(22)).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
    assert_eq!(active.get(), 0);
}

#[test]
fn eval_async_spawned_ion_functions_overlap_host_futures() {
    let mut engine = Engine::new();
    let (tx_a, rx_a) = tokio::sync::oneshot::channel::<Value>();
    let (tx_b, rx_b) = tokio::sync::oneshot::channel::<Value>();
    let receivers = Rc::new(RefCell::new(vec![Some(rx_a), Some(rx_b)]));
    let active = Rc::new(Cell::new(0));
    let max_active = Rc::new(Cell::new(0));

    let host_receivers = Rc::clone(&receivers);
    let host_active = Rc::clone(&active);
    let host_max_active = Rc::clone(&max_active);
    engine.register_async_fn(ion_core::h!("later"), move |args| {
        let idx = args[0].as_int().unwrap() as usize;
        let rx = host_receivers.borrow_mut()[idx].take().unwrap();
        let active = Rc::clone(&host_active);
        let max_active = Rc::clone(&host_max_active);
        async move {
            let now_active = active.get() + 1;
            active.set(now_active);
            max_active.set(max_active.get().max(now_active));
            let value = rx
                .await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))?;
            active.set(active.get() - 1);
            Ok(value)
        }
    });

    let mut fut = Box::pin(engine.eval_async(
        r#"
        fn load(idx) {
            later(idx) + 1
        }

        async {
            let a = spawn load(0);
            let b = spawn load(1);
            a.await + b.await
        }
        "#,
    ));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    assert_eq!(max_active.get(), 2);

    tx_a.send(Value::Int(19)).unwrap();
    tx_b.send(Value::Int(21)).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
    assert_eq!(active.get(), 0);
}

#[test]
fn eval_async_spawned_ion_function_preserves_named_arguments() {
    let mut engine = Engine::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
    let rx = Rc::new(RefCell::new(Some(rx)));
    let host_rx = Rc::clone(&rx);
    engine.register_async_fn(ion_core::h!("later"), move |args| {
        let rx = host_rx.borrow_mut().take().unwrap();
        async move {
            let offset = rx
                .await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))?
                .as_int()
                .unwrap();
            Ok(Value::Int(args[0].as_int().unwrap() + offset))
        }
    });

    let mut fut = Box::pin(engine.eval_async(
        r#"
        fn load(x, y = 1) {
            later(x) + y
        }

        async {
            let task = spawn load(y: 20, x: 21);
            task.await
        }
        "#,
    ));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    tx.send(Value::Int(1)).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
}

#[tokio::test]
async fn eval_async_spawn_outside_async_block_is_rejected() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later"), |_args| async { Ok(Value::Int(1)) });

    let err = engine.eval_async("spawn later()").await.unwrap_err();
    assert!(err.message.contains("spawn is only allowed inside async"));
}

#[tokio::test]
async fn eval_async_bytecode_runtime_restores_engine_environment() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        Ok(Value::Int(args[0].as_int().unwrap() + 1))
    });

    let value = engine
        .eval_async(
            r#"
            let answer = later(41);
            answer
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
    assert_eq!(engine.get("answer"), Some(Value::Int(42)));
}

#[tokio::test]
async fn eval_async_async_host_programs_match_host_structs_in_bytecode() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_point"), |_args| async move {
        let mut fields = IndexMap::new();
        fields.insert(ion_core::h!("x"), Value::Int(42));
        Ok(Value::HostStruct {
            type_hash: ion_core::h!("Point"),
            fields,
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_point() {
                Point { x } => x,
                _ => 0,
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn eval_async_async_host_programs_match_host_struct_nested_fields() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_point"), |_args| async move {
        let mut fields = IndexMap::new();
        fields.insert(ion_core::h!("x"), Value::Int(42));
        Ok(Value::HostStruct {
            type_hash: ion_core::h!("Point"),
            fields,
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_point() {
                Point { x: 41 } => 1,
                Point { x: 42 } => 2,
                _ => 0,
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(2));
}

#[tokio::test]
async fn eval_async_async_host_programs_skip_struct_arm_when_field_is_missing() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_point"), |_args| async move {
        let mut fields = IndexMap::new();
        fields.insert(ion_core::h!("x"), Value::Int(42));
        Ok(Value::HostStruct {
            type_hash: ion_core::h!("Point"),
            fields,
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_point() {
                Point { y } => y,
                _ => 7,
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(7));
}

#[tokio::test]
async fn eval_async_async_host_programs_match_host_enum_payloads_in_bytecode() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_color"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Color"),
            variant_hash: ion_core::h!("Custom"),
            data: vec![Value::Int(255), Value::Int(128), Value::Int(0)],
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_color() {
                Color::Red => "red",
                Color::Custom(r, g, b) => f"rgb({r},{g},{b})",
                _ => "other",
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Str("rgb(255,128,0)".into()));
}

#[tokio::test]
async fn eval_async_async_host_programs_match_host_enum_unit_variants_in_bytecode() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_color"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Color"),
            variant_hash: ion_core::h!("Green"),
            data: vec![],
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_color() {
                Color::Red => "red",
                Color::Green => "green",
                _ => "other",
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Str("green".into()));
}

#[tokio::test]
async fn eval_async_async_host_programs_match_host_enum_nested_payload_patterns() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_status"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Status"),
            variant_hash: ion_core::h!("Success"),
            data: vec![Value::Tuple(vec![Value::Int(20), Value::Int(22)])],
        })
    });

    let value = engine
        .eval_async(
            r#"
            match later_status() {
                Status::Success((20, value)) => value,
                _ => 0,
            }
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(22));
}

#[tokio::test]
async fn eval_async_async_host_programs_let_destructure_host_structs() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_point"), |_args| async move {
        let mut fields = IndexMap::new();
        fields.insert(ion_core::h!("x"), Value::Int(20));
        fields.insert(ion_core::h!("y"), Value::Int(22));
        Ok(Value::HostStruct {
            type_hash: ion_core::h!("Point"),
            fields,
        })
    });

    let value = engine
        .eval_async(
            r#"
            let Point { x, y } = later_point();
            x + y
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn eval_async_async_host_programs_let_destructure_host_enums() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_color"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Color"),
            variant_hash: ion_core::h!("Custom"),
            data: vec![Value::Int(20), Value::Int(21), Value::Int(1)],
        })
    });

    let value = engine
        .eval_async(
            r#"
            let Color::Custom(r, g, b) = later_color();
            r + g + b
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn eval_async_async_host_programs_report_let_pattern_mismatch() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_color"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Color"),
            variant_hash: ion_core::h!("Red"),
            data: vec![],
        })
    });

    let err = engine
        .eval_async(
            r#"
            let Color::Custom(r, g, b) = later_color();
            r
            "#,
        )
        .await
        .unwrap_err();

    assert!(
        err.message.contains("non-exhaustive match"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn eval_async_select_branch_pattern_mismatch_is_reported() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("later_color"), |_args| async move {
        Ok(Value::HostEnum {
            enum_hash: ion_core::h!("Color"),
            variant_hash: ion_core::h!("Red"),
            data: vec![],
        })
    });

    let err = engine
        .eval_async(
            r#"
            async {
                select {
                    Color::Custom(r, g, b) = later_color() => r + g + b,
                }
            }
            "#,
        )
        .await
        .unwrap_err();

    assert!(
        err.message.contains("non-exhaustive match"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn eval_async_async_host_program_can_use_single_module_import() {
    let mut engine = Engine::new();
    let mut math = Module::new(ion_core::h!("math"));
    math.register_fn(ion_core::h!("add"), |args: &[Value]| {
        match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            _ => Err("expected ints".to_string()),
        }
    });
    engine.register_module(math);
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        Ok(Value::Int(args[0].as_int().unwrap()))
    });

    let value = engine
        .eval_async(
            r#"
            use math::add;
            later(add(20, 22))
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn eval_async_async_host_program_can_use_glob_module_import() {
    let mut engine = Engine::new();
    let mut math = Module::new(ion_core::h!("math"));
    math.register_fn(ion_core::h!("add"), |args: &[Value]| {
        match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            _ => Err("expected ints".to_string()),
        }
    });
    math.set(ion_core::h!("offset"), Value::Int(20));
    engine.register_module(math);
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        Ok(Value::Int(args[0].as_int().unwrap()))
    });

    let value = engine
        .eval_async(
            r#"
            use math::*;
            later(add(offset, 22))
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[test]
fn eval_async_async_block_joins_unawaited_spawned_task() {
    let mut engine = Engine::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
    let rx = Rc::new(RefCell::new(Some(rx)));
    let host_rx = Rc::clone(&rx);
    engine.register_async_fn(ion_core::h!("later"), move |_args| {
        let rx = host_rx.borrow_mut().take().unwrap();
        async move {
            rx.await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))
        }
    });

    let mut fut = Box::pin(engine.eval_async(
        r#"
        async {
            let _task = spawn later();
            7
        }
        "#,
    ));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    tx.send(Value::Int(99)).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Int(7))) => {}
        other => panic!("unexpected eval_async result: {:?}", other),
    }
}

#[test]
fn eval_async_select_returns_first_ready_branch() {
    let mut engine = Engine::new();
    let (tx_slow, rx_slow) = tokio::sync::oneshot::channel::<Value>();
    let (tx_fast, rx_fast) = tokio::sync::oneshot::channel::<Value>();
    let receivers = Rc::new(RefCell::new(vec![Some(rx_slow), Some(rx_fast)]));
    let host_receivers = Rc::clone(&receivers);

    engine.register_async_fn(ion_core::h!("later"), move |args| {
        let idx = args[0].as_int().unwrap() as usize;
        let rx = host_receivers.borrow_mut()[idx].take().unwrap();
        async move {
            rx.await
                .map_err(|_| IonError::runtime("host future cancelled", 0, 0))
        }
    });

    let mut fut = Box::pin(engine.eval_async(
        r#"
        async {
            select {
                value = later(0) => "slow " + value,
                value = later(1) => "fast " + value,
            }
        }
        "#,
    ));
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    tx_fast.send(Value::Str("done".into())).unwrap();

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(Value::Str(value))) => assert_eq!(value, "fast done"),
        other => panic!("unexpected eval_async result: {:?}", other),
    }

    let _ = tx_slow.send(Value::Str("late".into()));
}

#[tokio::test]
async fn eval_async_await_rejects_non_task_value() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("marker"), |_args| async { Ok(Value::Unit) });

    let err = engine
        .eval_async(
            r#"
            let x = marker();
            x.await
            "#,
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("cannot await"));
}

// `async_host_function_can_return_ion_error_type` removed — see note above
// on the removed sync-rejection tests. The IonError-return path is exercised
// by every test that registers an async builtin and observes its error.

#[cfg(any())]
fn _async_host_function_can_return_ion_error_type_legacy() {
    let mut engine = Engine::new();
    engine.register_async_fn(ion_core::h!("fail_later"), |_args| async {
        Err(IonError::runtime("nope", 0, 0))
    });

    let err = engine.eval("fail_later()").unwrap_err();
    assert!(err.message.contains("async host function cannot be called"));
}

#[test]
fn engine_handle_enqueues_external_call_requests() {
    let engine = Engine::new();
    let handle = engine.handle();
    let mut call = handle.call_async("on_event", vec![Value::Int(5)]);

    let requests = engine.drain_external_requests();
    assert_eq!(requests.len(), 1);

    match requests.into_iter().next().unwrap() {
        ion_core::async_runtime::ExternalRequest::Call {
            fn_name,
            args,
            result_tx,
        } => {
            assert_eq!(fn_name, "on_event");
            assert_eq!(args, vec![Value::Int(5)]);
            result_tx.send(Ok(Value::Str("ok".into()))).unwrap();
        }
    }

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    match Pin::new(&mut call).poll(&mut cx) {
        Poll::Ready(Ok(Value::Str(value))) => assert_eq!(value, "ok"),
        other => panic!("unexpected call future result: {:?}", other),
    }
}

#[tokio::test]
async fn eval_async_host_future_can_call_back_into_ion_function() {
    let mut engine = Engine::new();
    let handle = engine.handle();
    engine.register_async_fn(ion_core::h!("trigger"), move |args| {
        let handle = handle.clone();
        async move { handle.call_async("on_event", args).await }
    });

    let value = engine
        .eval_async(
            r#"
            fn on_event(value) {
                value + 1
            }

            trigger(41)
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[tokio::test]
async fn external_ion_callback_can_park_on_nested_async_host_future() {
    let mut engine = Engine::new();
    let handle = engine.handle();
    engine.register_async_fn(ion_core::h!("trigger"), move |args| {
        let handle = handle.clone();
        async move { handle.call_async("on_event", args).await }
    });
    engine.register_async_fn(ion_core::h!("later"), |args| async move {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok(args.into_iter().next().unwrap_or(Value::Unit))
    });

    let value = engine
        .eval_async(
            r#"
            fn on_event(value) {
                later(value + 1)
            }

            trigger(41)
            "#,
        )
        .await
        .unwrap();

    assert_eq!(value, Value::Int(42));
}

#[test]
fn host_future_table_polls_ready_future() {
    let mut table = HostFutureTable::new();
    let id = table.insert(TaskId(42), Box::pin(async { Ok(Value::Int(99)) }));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = table.poll_ready(&mut cx);

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, id);
    assert_eq!(ready[0].waiter, TaskId(42));
    assert_eq!(ready[0].result.as_ref().unwrap(), &Value::Int(99));
    assert!(table.is_empty());
}

#[test]
fn host_future_table_cancels_and_drops_by_id() {
    let dropped = Rc::new(Cell::new(false));
    let mut table = HostFutureTable::new();
    let id = table.insert(
        TaskId(1),
        Box::pin(DropFlagFuture::new(Rc::clone(&dropped))),
    );

    assert!(table.contains(id));
    assert!(!dropped.get());
    assert!(table.cancel_and_drop(id));
    assert!(dropped.get());
    assert!(!table.contains(id));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(table.poll_ready(&mut cx).is_empty());
}

#[test]
fn host_future_table_cancels_only_target_future() {
    let dropped_a = Rc::new(Cell::new(false));
    let dropped_b = Rc::new(Cell::new(false));
    let mut table = HostFutureTable::new();
    let id_a = table.insert(
        TaskId(1),
        Box::pin(DropFlagFuture::new(Rc::clone(&dropped_a))),
    );
    let id_b = table.insert(
        TaskId(2),
        Box::pin(DropFlagFuture::new(Rc::clone(&dropped_b))),
    );

    assert!(table.cancel_and_drop(id_a));
    assert!(dropped_a.get());
    assert!(!dropped_b.get());
    assert!(!table.contains(id_a));
    assert!(table.contains(id_b));

    assert!(table.cancel_and_drop(id_b));
    assert!(dropped_b.get());
}

#[test]
fn host_future_table_rejects_stale_future_id_after_slot_reuse() {
    let dropped = Rc::new(Cell::new(false));
    let mut table = HostFutureTable::new();
    let old_id = table.insert(
        TaskId(1),
        Box::pin(DropFlagFuture::new(Rc::clone(&dropped))),
    );

    assert!(table.cancel_and_drop(old_id));
    let new_id = table.insert(TaskId(2), Box::pin(async { Ok(Value::Str("done".into())) }));

    assert_ne!(old_id, new_id);
    assert!(!table.contains(old_id));
    assert!(!table.cancel_and_drop(old_id));
    assert!(table.contains(new_id));
}

#[test]
fn host_future_table_keeps_pending_future_until_ready() {
    let state = Rc::new(RefCell::new(None));
    let mut table = HostFutureTable::new();
    let id = table.insert(
        TaskId(7),
        Box::pin(ControlledFuture::new(Rc::clone(&state))),
    );

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(table.poll_ready(&mut cx).is_empty());
    assert!(table.contains(id));

    *state.borrow_mut() = Some(Ok(Value::Str("ready".into())));

    let ready = table.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, id);
    assert_eq!(ready[0].waiter, TaskId(7));
    assert_eq!(
        ready[0].result.as_ref().unwrap(),
        &Value::Str("ready".into())
    );
    assert!(!table.contains(id));
}

#[test]
fn async_builtin_closure_can_be_called_directly_by_runtime_scaffold() {
    let func = AsyncBuiltinClosureFn::new(|args| async move {
        let value = args[0].as_int().unwrap();
        Ok(Value::Int(value + 1))
    });
    let mut future = func.call(vec![Value::Int(41)]);

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let result = future.as_mut().poll(&mut cx);

    match result {
        Poll::Ready(Ok(Value::Int(42))) => {}
        other => panic!("unexpected poll result: {:?}", other),
    }
}

#[test]
fn async_host_future_can_suspend_and_resume_runtime_task_scaffold() {
    let func = AsyncBuiltinClosureFn::new(|args| async move {
        let value = args[0].as_int().unwrap();
        Ok(Value::Int(value * 2))
    });
    let mut tasks = TaskTable::new();
    let mut futures = HostFutureTable::new();
    let task = tasks.spawn_ready();

    let future_id = futures.insert(task, func.call(vec![Value::Int(21)]));
    assert!(tasks.park_on_host_future(task, future_id));
    assert_eq!(
        tasks.get(task).unwrap().state,
        TaskState::WaitingHostFuture(future_id)
    );

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].waiter, task);

    assert!(tasks.resume_from_host_result(task, ready.into_iter().next().unwrap().result));
    assert_eq!(tasks.get(task).unwrap().state, TaskState::Ready);
    assert_eq!(tasks.take_resumed_value(task), Some(Value::Int(42)));
    assert!(tasks.take_pending_error(task).is_none());
}

#[test]
fn async_host_future_error_resumes_task_with_pending_error_scaffold() {
    let func =
        AsyncBuiltinClosureFn::new(
            |_args| async move { Err(IonError::runtime("host failed", 3, 9)) },
        );
    let mut tasks = TaskTable::new();
    let mut futures = HostFutureTable::new();
    let task = tasks.spawn_ready();

    let future_id = futures.insert(task, func.call(vec![]));
    assert!(tasks.park_on_host_future(task, future_id));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);

    assert!(tasks.resume_from_host_result(task, ready.into_iter().next().unwrap().result));
    assert_eq!(tasks.get(task).unwrap().state, TaskState::Ready);
    assert!(tasks.take_resumed_value(task).is_none());
    let err = tasks.take_pending_error(task).unwrap();
    assert_eq!(err.message, "host failed");
    assert_eq!(err.line, 3);
    assert_eq!(err.col, 9);
}

#[test]
fn step_task_async_host_call_suspends_and_resumes_bytecode_continuation() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(
            |args| async move { Ok(Value::Int(args[0].as_int().unwrap() * 2)) },
        );
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("double"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_constant(Value::Int(21), 1);
    chunk.emit_op_u8(Op::Call, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(10);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    let future_id = match step_task_with_host_futures(&arena, &mut cont, task, &mut futures) {
        StepOutcome::Suspended(TaskState::WaitingHostFuture(future_id)) => future_id,
        other => panic!("unexpected step outcome: {:?}", other),
    };
    assert!(futures.contains(future_id));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].waiter, task);

    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Int(value))) => assert_eq!(value, 42),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_async_host_error_resumes_through_try_catch() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(|_args| async { Err(IonError::runtime("async boom", 4, 2)) });
    let mut chunk = Chunk::new();
    let catch_jump = chunk.emit_jump(Op::TryBegin, 1);
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("fail"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_op_u8(Op::Call, 0, 1);
    let after_catch_jump = chunk.emit_jump(Op::TryEnd, 1);
    chunk.patch_jump(catch_jump);
    chunk.emit_op(Op::Return, 1);
    chunk.patch_jump(after_catch_jump);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(11);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task_with_host_futures(&arena, &mut cont, task, &mut futures),
        StepOutcome::Suspended(TaskState::WaitingHostFuture(_))
    ));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));

    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Str(value))) => assert_eq!(value, "async boom"),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_async_host_result_error_resumes_into_try_operator() {
    let mut arena = ChunkArena::new();
    let async_fn = AsyncBuiltinClosureFn::new(|_args| async {
        Ok(Value::Result(Err(Box::new(Value::Str("bad".into())))))
    });
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("fallible"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_op_u8(Op::Call, 0, 1);
    chunk.emit_op(Op::Try, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(12);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task_with_host_futures(&arena, &mut cont, task, &mut futures),
        StepOutcome::Suspended(TaskState::WaitingHostFuture(_))
    ));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));

    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.kind, ErrorKind::PropagatedErr);
            assert_eq!(err.message, "bad");
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_async_host_tail_call_resumes_as_frame_return() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(
            |args| async move { Ok(Value::Int(args[0].as_int().unwrap() * 2)) },
        );
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("double"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_constant(Value::Int(21), 1);
    chunk.emit_op_u8(Op::TailCall, 1, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(13);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task_with_host_futures(&arena, &mut cont, task, &mut futures),
        StepOutcome::Suspended(TaskState::WaitingHostFuture(_))
    ));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));

    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Int(value))) => assert_eq!(value, 42),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_spawn_call_await_task_suspends_and_resumes() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(
            |args| async move { Ok(Value::Int(args[0].as_int().unwrap() + 1)) },
        );
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("later"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_constant(Value::Int(41), 1);
    chunk.emit_op_u8(Op::SpawnCall, 1, 1);
    chunk.emit_op(Op::AwaitTask, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert_eq!(
        run_continuation_with_host_futures(&arena, &mut cont, TaskId(20)).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn step_task_await_polls_other_spawned_tasks_for_overlap() {
    let poll_counts = Rc::new(RefCell::new(vec![0u32, 0u32]));
    let states = Rc::new(RefCell::new(vec![None, None]));
    let host_poll_counts = Rc::clone(&poll_counts);
    let host_states = Rc::clone(&states);
    let async_fn = AsyncBuiltinClosureFn::new(move |args| {
        let idx = args[0].as_int().unwrap() as usize;
        TaskProbeFuture::new(idx, Rc::clone(&host_poll_counts), Rc::clone(&host_states))
    });

    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("later"),
            func: async_fn,
        },
        1,
    );
    chunk.emit_constant(Value::Int(0), 1);
    chunk.emit_op_u8(Op::SpawnCall, 1, 1);
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("later"),
            func: AsyncBuiltinClosureFn::new({
                let poll_counts = Rc::clone(&poll_counts);
                let states = Rc::clone(&states);
                move |args| {
                    let idx = args[0].as_int().unwrap() as usize;
                    TaskProbeFuture::new(idx, Rc::clone(&poll_counts), Rc::clone(&states))
                }
            }),
        },
        1,
    );
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op_u8(Op::SpawnCall, 1, 1);
    chunk.emit_op(Op::Pop, 1);
    chunk.emit_op(Op::AwaitTask, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(21);

    let mut suspended = false;
    for _ in 0..10 {
        match step_task_with_host_futures(&arena, &mut cont, task, &mut futures) {
            StepOutcome::Continue => {}
            StepOutcome::Suspended(TaskState::WaitingHostFuture(_)) => {
                suspended = true;
                break;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    assert!(suspended);

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(futures.poll_ready(&mut cx).is_empty());
    assert_eq!(*poll_counts.borrow(), vec![1, 1]);

    states.borrow_mut()[0] = Some(Ok(Value::Int(40)));
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Int(value))) => assert_eq!(value, 40),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn compiler_emits_async_runtime_spawn_and_await_opcodes() {
    let source = r#"
        async {
            let task = spawn later(41);
            task.await
        }
    "#;
    let program = parse_program(source);
    let (chunk, _fn_chunks) = Compiler::new().compile_program(&program).unwrap();

    assert!(chunk.code.contains(&(Op::SpawnCall as u8)));
    assert!(chunk.code.contains(&(Op::AwaitTask as u8)));
}

#[test]
fn compiler_emits_async_runtime_named_spawn_opcode() {
    let source = r#"
        async {
            let task = spawn load(y: 20, x: 21);
            task.await
        }
    "#;
    let program = parse_program(source);
    let (chunk, _fn_chunks) = Compiler::new().compile_program(&program).unwrap();

    assert!(chunk.code.contains(&(Op::SpawnCallNamed as u8)));
    assert!(chunk.code.contains(&(Op::AwaitTask as u8)));
}

#[test]
fn compiled_async_spawn_await_runs_on_continuation_runtime() {
    let source = r#"
        async {
            let task = spawn later(41);
            task.await + 1
        }
    "#;
    let program = parse_program(source);
    let (chunk, fn_chunks) = Compiler::new().compile_program(&program).unwrap();
    let mut arena = ChunkArena::new();
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    for (fn_id, chunk) in fn_chunks {
        let chunk_id = arena.insert(chunk);
        cont.register_fn_chunk(fn_id, chunk_id);
    }
    cont.define_global(
        "later",
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("later"),
            func: AsyncBuiltinClosureFn::new(|args| async move {
                Ok(Value::Int(args[0].as_int().unwrap()))
            }),
        },
        false,
    );

    assert_eq!(
        run_continuation_with_host_futures(&arena, &mut cont, TaskId(22)).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn compiled_async_select_races_branch_tasks_on_continuation_runtime() {
    let source = r#"
        async {
            select {
                value = later(0) => "slow " + value,
                value = later(1) => "fast " + value,
            }
        }
    "#;
    let program = parse_program(source);
    let (chunk, fn_chunks) = Compiler::new().compile_program(&program).unwrap();
    assert!(chunk.code.contains(&(Op::SelectTasks as u8)));

    let poll_counts = Rc::new(RefCell::new(vec![0u32, 0u32]));
    let states = Rc::new(RefCell::new(vec![
        None,
        Some(Ok(Value::Str("done".into()))),
    ]));
    let mut arena = ChunkArena::new();
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    for (fn_id, chunk) in fn_chunks {
        let chunk_id = arena.insert(chunk);
        cont.register_fn_chunk(fn_id, chunk_id);
    }
    cont.define_global(
        "later",
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("later"),
            func: AsyncBuiltinClosureFn::new({
                let poll_counts = Rc::clone(&poll_counts);
                let states = Rc::clone(&states);
                move |args| {
                    let idx = args[0].as_int().unwrap() as usize;
                    TaskProbeFuture::new(idx, Rc::clone(&poll_counts), Rc::clone(&states))
                }
            }),
        },
        false,
    );

    let mut futures = HostFutureTable::new();
    let task = TaskId(23);
    let mut suspended = false;
    for _ in 0..32 {
        match step_task_with_host_futures(&arena, &mut cont, task, &mut futures) {
            StepOutcome::Continue => {}
            StepOutcome::Suspended(TaskState::WaitingHostFuture(_)) => {
                suspended = true;
                break;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    assert!(suspended);

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = futures.poll_ready(&mut cx);
    assert_eq!(ready.len(), 1);
    assert_eq!(*poll_counts.borrow(), vec![1, 1]);
    assert!(matches!(
        cont.resume_host_result(ready.into_iter().next().unwrap().result),
        StepOutcome::Continue
    ));

    assert_eq!(
        run_existing_continuation_to_value(&arena, &mut cont).unwrap(),
        Value::Str("fast done".into())
    );
}

#[test]
fn chunk_arena_returns_stable_chunk_ids() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op(Op::Unit, 1);

    let id = arena.insert(chunk);
    assert_eq!(arena.get(id).unwrap().code, vec![Op::Unit as u8]);

    arena.get_mut(id).unwrap().emit_op(Op::Return, 1);
    assert_eq!(
        arena.get(id).unwrap().code,
        vec![Op::Unit as u8, Op::Return as u8]
    );
}

#[test]
fn chunk_arena_rejects_stale_ids_after_reuse() {
    let mut arena = ChunkArena::new();
    let old_id = arena.insert(Chunk::new());
    assert!(arena.remove(old_id).is_some());

    let mut replacement = Chunk::new();
    replacement.emit_op(Op::True, 1);
    let new_id = arena.insert(replacement);

    assert_ne!(old_id, new_id);
    assert!(arena.get(old_id).is_none());
    assert_eq!(arena.get(new_id).unwrap().code, vec![Op::True as u8]);
}

#[test]
fn step_task_runs_constant_and_return_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Str("hello".into()), 4);
    chunk.emit_op(Op::Return, 4);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert_eq!(cont.stack, vec![Value::Str("hello".into())]);
    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Str(value))) => assert_eq!(value, "hello"),
        other => panic!("unexpected step outcome: {:?}", other),
    }
    assert!(cont.frames.is_empty());
}

#[test]
fn step_task_runs_unit_return_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op(Op::Unit, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Unit)) => {}
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_print_writes_to_configured_output_handler() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Str("hello".into()), 7);
    chunk.emit_op_u8_span(Op::Print, 1, 7, 3);
    chunk.emit_op(Op::Return, 7);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let captured = Arc::new(Mutex::new(Vec::new()));
    cont.set_output_handler(Arc::new(CaptureOutput {
        writes: Arc::clone(&captured),
    }));

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert_eq!(
        captured.lock().unwrap().as_slice(),
        &[(OutputStream::Stdout, "hello\n".to_string())]
    );
    match step_task(&arena, &mut cont) {
        StepOutcome::Done(Ok(Value::Unit)) => {}
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_print_without_output_handler_reports_config_error() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Str("hello".into()), 7);
    chunk.emit_op_u8_span(Op::Print, 0, 7, 3);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert!(err.message.contains("output handler is not configured"));
            assert_eq!(err.line, 7);
            assert_eq!(err.col, 3);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
    assert_eq!(cont.frames[0].ip, 3);
}

#[test]
fn step_task_pipe_opcode_reports_direct_execution_error() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op_u8(Op::Pipe, 1, 3);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.message, "pipe opcode should not be executed directly");
            assert_eq!(err.line, 3);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_reports_missing_chunk() {
    let mut arena = ChunkArena::new();
    let root = arena.insert(Chunk::new());
    assert!(arena.remove(root).is_some());
    let mut cont = VmContinuation::new(root);

    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => assert_eq!(err.message, "missing chunk"),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_reports_truncated_constant_without_advancing_ip() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op_span(Op::Constant, 9, 2);
    chunk.emit_span(0, 9, 2);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.message, "truncated constant operand");
            assert_eq!(err.line, 9);
            assert_eq!(err.col, 2);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
    assert_eq!(cont.frames[0].ip, 0);
}

#[test]
fn step_task_runs_arithmetic_and_comparison_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_constant(Value::Int(3), 1);
    chunk.emit_op(Op::Add, 1);
    chunk.emit_constant(Value::Int(4), 1);
    chunk.emit_op(Op::Mul, 1);
    chunk.emit_constant(Value::Int(20), 1);
    chunk.emit_op(Op::Eq, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn step_task_runs_bitwise_and_shift_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(6), 1);
    chunk.emit_constant(Value::Int(3), 1);
    chunk.emit_op(Op::BitAnd, 1);
    chunk.emit_constant(Value::Int(8), 1);
    chunk.emit_op(Op::BitOr, 1);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_op(Op::Shl, 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op(Op::Shr, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(20)
    );
}

#[test]
fn step_task_reports_shift_count_out_of_range() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(1), 2);
    chunk.emit_constant(Value::Int(64), 2);
    chunk.emit_op(Op::Shl, 2);
    let root = arena.insert(chunk);

    match run_continuation_to_value(&arena, root) {
        Err(err) => assert_eq!(err.message, "shift count 64 is out of range 0..64"),
        other => panic!("unexpected continuation result: {:?}", other),
    }
}

#[test]
fn step_task_runs_boolean_short_circuit_scaffold() {
    let mut arena = ChunkArena::new();
    let mut and_chunk = Chunk::new();
    and_chunk.emit_op(Op::False, 1);
    let and_jump = and_chunk.emit_jump(Op::And, 1);
    and_chunk.emit_constant(Value::Int(99), 1);
    and_chunk.patch_jump(and_jump);
    and_chunk.emit_op(Op::Return, 1);
    let and_root = arena.insert(and_chunk);

    let mut or_chunk = Chunk::new();
    or_chunk.emit_op(Op::True, 1);
    let or_jump = or_chunk.emit_jump(Op::Or, 1);
    or_chunk.emit_constant(Value::Int(99), 1);
    or_chunk.patch_jump(or_jump);
    or_chunk.emit_op(Op::Return, 1);
    let or_root = arena.insert(or_chunk);

    assert_eq!(
        run_continuation_to_value(&arena, and_root).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        run_continuation_to_value(&arena, or_root).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn step_task_runs_stack_wrappers_and_try_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(42), 1);
    chunk.emit_op(Op::WrapOk, 1);
    chunk.emit_op(Op::Try, 1);
    chunk.emit_op(Op::WrapSome, 1);
    chunk.emit_op(Op::Try, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn step_task_try_err_reports_propagated_error() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Str("bad".into()), 5);
    chunk.emit_op(Op::WrapErr, 5);
    chunk.emit_op(Op::Try, 5);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.kind, ErrorKind::PropagatedErr);
            assert_eq!(err.message, "bad");
            assert_eq!(err.line, 5);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_try_begin_catches_instruction_error_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let catch_jump = chunk.emit_jump(Op::TryBegin, 8);
    chunk.emit_constant(Value::Int(1), 8);
    chunk.emit_constant(Value::Int(0), 8);
    chunk.emit_op(Op::Div, 8);
    let after_catch_jump = chunk.emit_jump(Op::TryEnd, 8);
    chunk.patch_jump(catch_jump);
    chunk.emit_op(Op::Return, 8);
    chunk.patch_jump(after_catch_jump);
    chunk.emit_op(Op::Return, 8);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Str("division by zero".into())
    );
}

#[test]
fn step_task_try_end_skips_catch_when_body_succeeds_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let catch_jump = chunk.emit_jump(Op::TryBegin, 8);
    chunk.emit_constant(Value::Int(7), 8);
    let after_catch_jump = chunk.emit_jump(Op::TryEnd, 8);
    chunk.patch_jump(catch_jump);
    chunk.emit_constant(Value::Int(999), 8);
    chunk.emit_op(Op::Return, 8);
    chunk.patch_jump(after_catch_jump);
    chunk.emit_op(Op::Return, 8);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn step_task_try_begin_does_not_catch_question_mark_propagation() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let catch_jump = chunk.emit_jump(Op::TryBegin, 8);
    chunk.emit_constant(Value::Str("bad".into()), 8);
    chunk.emit_op(Op::WrapErr, 8);
    chunk.emit_op(Op::Try, 8);
    let after_catch_jump = chunk.emit_jump(Op::TryEnd, 8);
    chunk.patch_jump(catch_jump);
    chunk.emit_constant(Value::Str("caught".into()), 8);
    chunk.emit_op(Op::Return, 8);
    chunk.patch_jump(after_catch_jump);
    chunk.emit_op(Op::Return, 8);
    let root = arena.insert(chunk);

    match run_continuation_to_value(&arena, root) {
        Err(err) => {
            assert_eq!(err.kind, ErrorKind::PropagatedErr);
            assert_eq!(err.message, "bad");
        }
        other => panic!("unexpected continuation result: {:?}", other),
    }
}

#[test]
fn step_task_builds_list_and_tuple_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_op_u16(Op::BuildList, 2, 1);
    chunk.emit_constant(Value::Str("x".into()), 1);
    chunk.emit_op_u16(Op::BuildTuple, 2, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::Str("x".into())
        ])
    );
}

#[test]
fn step_task_builds_dict_f_string_and_range_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Str("answer".into()), 1);
    chunk.emit_constant(Value::Int(42), 1);
    chunk.emit_op_u16(Op::BuildDict, 1, 1);
    chunk.emit_constant(Value::Str("count=".into()), 1);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_op_u16(Op::BuildFString, 2, 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_constant(Value::Int(3), 1);
    chunk.emit_op_u8(Op::BuildRange, 1, 1);
    chunk.emit_op_u16(Op::BuildTuple, 3, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    let mut expected_dict = indexmap::IndexMap::new();
    expected_dict.insert("answer".into(), Value::Int(42));
    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::Dict(expected_dict),
            Value::Str("count=2".into()),
            Value::Range {
                start: 1,
                end: 3,
                inclusive: true,
            },
        ])
    );
}

#[test]
fn step_task_runs_field_index_set_and_slice_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();

    let mut dict = indexmap::IndexMap::new();
    dict.insert("answer".into(), Value::Int(42));
    let field_idx = chunk.add_constant(Value::Str("answer".into()));
    chunk.emit_constant(Value::Dict(dict), 1);
    chunk.emit_op_u16(Op::GetField, field_idx, 1);

    chunk.emit_constant(
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        1,
    );
    chunk.emit_constant(Value::Int(-1), 1);
    chunk.emit_op(Op::GetIndex, 1);

    chunk.emit_constant(Value::List(vec![Value::Int(10), Value::Int(20)]), 1);
    chunk.emit_constant(Value::Int(0), 1);
    chunk.emit_constant(Value::Int(99), 1);
    chunk.emit_op(Op::SetIndex, 1);

    chunk.emit_constant(Value::Str("abcd".into()), 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_op_u8(Op::Slice, 0b111, 1);

    chunk.emit_op_u16(Op::BuildTuple, 4, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::Int(42),
            Value::Int(3),
            Value::List(vec![Value::Int(99), Value::Int(20)]),
            Value::Str("bc".into()),
        ])
    );
}

#[test]
fn step_task_runs_non_closure_method_calls_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let list_len = chunk.add_constant(Value::Str("len".into()));
    let str_upper = chunk.add_constant(Value::Str("to_upper".into()));
    let dict_keys = chunk.add_constant(Value::Str("keys".into()));
    let unwrap_or = chunk.add_constant(Value::Str("unwrap_or".into()));
    let range_contains = chunk.add_constant(Value::Str("contains".into()));

    chunk.emit_constant(Value::List(vec![Value::Int(1), Value::Int(2)]), 1);
    emit_method_call(&mut chunk, list_len, 0, 1);

    chunk.emit_constant(Value::Str("ion".into()), 1);
    emit_method_call(&mut chunk, str_upper, 0, 1);

    let mut dict = indexmap::IndexMap::new();
    dict.insert("a".into(), Value::Int(1));
    dict.insert("b".into(), Value::Int(2));
    chunk.emit_constant(Value::Dict(dict), 1);
    emit_method_call(&mut chunk, dict_keys, 0, 1);

    chunk.emit_op(Op::None, 1);
    chunk.emit_constant(Value::Int(9), 1);
    emit_method_call(&mut chunk, unwrap_or, 1, 1);

    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_constant(Value::Int(4), 1);
    chunk.emit_op_u8(Op::BuildRange, 0, 1);
    chunk.emit_constant(Value::Int(3), 1);
    emit_method_call(&mut chunk, range_contains, 1, 1);

    chunk.emit_op_u16(Op::BuildTuple, 5, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::Int(2),
            Value::Str("ION".into()),
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]),
            Value::Int(9),
            Value::Bool(true),
        ])
    );
}

#[test]
fn step_task_method_call_maps_with_registered_function_continuation() {
    let mut arena = ChunkArena::new();
    let mut mapper = Chunk::new();
    mapper.emit_op_u16(Op::GetLocalSlot, 0, 1);
    mapper.emit_constant(Value::Int(2), 1);
    mapper.emit_op(Op::Mul, 1);
    mapper.emit_op(Op::Return, 1);
    let mapper_id = arena.insert(mapper);

    let function = IonFn::new(
        "double".into(),
        vec![Param {
            name: "x".into(),
            default: None,
        }],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;

    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("map".into()));
    chunk.emit_constant(
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        1,
    );
    chunk.emit_constant(Value::Fn(function), 1);
    emit_method_call(&mut chunk, method, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    cont.register_fn_chunk(fn_id, mapper_id);

    assert_eq!(
        run_existing_continuation_to_value(&arena, &mut cont).unwrap(),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
    );
}

#[test]
fn step_task_method_call_filters_with_builtin_closure_continuation() {
    let mut arena = ChunkArena::new();
    let keep_even =
        BuiltinClosureFn::new(|args| Ok(Value::Bool(args[0].as_int().unwrap() % 2 == 0)));
    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("filter".into()));
    chunk.emit_constant(
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]),
        1,
    );
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("keep_even"),
            func: keep_even,
        },
        1,
    );
    emit_method_call(&mut chunk, method, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::List(vec![Value::Int(2), Value::Int(4)])
    );
}

#[test]
fn step_task_method_call_map_suspends_on_async_host_callback() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(
            |args| async move { Ok(Value::Int(args[0].as_int().unwrap() * 2)) },
        );
    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("map".into()));
    chunk.emit_constant(Value::List(vec![Value::Int(20), Value::Int(21)]), 1);
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("double_later"),
            func: async_fn,
        },
        1,
    );
    emit_method_call(&mut chunk, method, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(55);

    let mut saw_suspend = false;
    for _ in 0..32 {
        match step_task_with_host_futures(&arena, &mut cont, task, &mut futures) {
            StepOutcome::Continue => {}
            StepOutcome::Suspended(TaskState::WaitingHostFuture(_)) => {
                saw_suspend = true;
                let waker = noop_waker();
                let mut cx = Context::from_waker(&waker);
                let mut ready = futures.poll_ready(&mut cx);
                assert_eq!(ready.len(), 1);
                assert!(matches!(
                    cont.resume_host_result(ready.remove(0).result),
                    StepOutcome::Continue
                ));
            }
            StepOutcome::Done(Ok(value)) => {
                assert!(saw_suspend);
                assert_eq!(value, Value::List(vec![Value::Int(40), Value::Int(42)]));
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("async host callback method call did not finish");
}

#[test]
fn step_task_option_and_cell_closure_methods_use_continuations() {
    let mut arena = ChunkArena::new();
    let mut mapper = Chunk::new();
    mapper.emit_op_u16(Op::GetLocalSlot, 0, 1);
    mapper.emit_constant(Value::Int(1), 1);
    mapper.emit_op(Op::Add, 1);
    mapper.emit_op(Op::Return, 1);
    let mapper_id = arena.insert(mapper);

    let function = IonFn::new(
        "inc".into(),
        vec![Param {
            name: "x".into(),
            default: None,
        }],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;
    let cell = Arc::new(Mutex::new(Value::Int(10)));

    let mut chunk = Chunk::new();
    let option_map = chunk.add_constant(Value::Str("map".into()));
    let cell_update = chunk.add_constant(Value::Str("update".into()));
    chunk.emit_constant(Value::Option(Some(Box::new(Value::Int(41)))), 1);
    chunk.emit_constant(Value::Fn(function.clone()), 1);
    emit_method_call(&mut chunk, option_map, 1, 1);
    chunk.emit_constant(Value::Cell(Arc::clone(&cell)), 1);
    chunk.emit_constant(Value::Fn(function), 1);
    emit_method_call(&mut chunk, cell_update, 1, 1);
    chunk.emit_op_u16(Op::BuildTuple, 2, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    cont.register_fn_chunk(fn_id, mapper_id);

    assert_eq!(
        run_existing_continuation_to_value(&arena, &mut cont).unwrap(),
        Value::Tuple(vec![
            Value::Option(Some(Box::new(Value::Int(42)))),
            Value::Int(11),
        ])
    );
    assert_eq!(*cell.lock().unwrap(), Value::Int(11));
}

#[test]
fn step_task_result_closure_method_suspends_on_async_host_callback() {
    let mut arena = ChunkArena::new();
    let async_fn =
        AsyncBuiltinClosureFn::new(
            |args| async move { Ok(Value::Str(format!("handled {}", args[0]))) },
        );
    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("map_err".into()));
    chunk.emit_constant(Value::Result(Err(Box::new(Value::Str("boom".into())))), 1);
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("handle_later"),
            func: async_fn,
        },
        1,
    );
    emit_method_call(&mut chunk, method, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    let mut futures = HostFutureTable::new();
    let task = TaskId(56);

    let mut saw_suspend = false;
    for _ in 0..16 {
        match step_task_with_host_futures(&arena, &mut cont, task, &mut futures) {
            StepOutcome::Continue => {}
            StepOutcome::Suspended(TaskState::WaitingHostFuture(_)) => {
                saw_suspend = true;
                let waker = noop_waker();
                let mut cx = Context::from_waker(&waker);
                let mut ready = futures.poll_ready(&mut cx);
                assert_eq!(ready.len(), 1);
                assert!(matches!(
                    cont.resume_host_result(ready.remove(0).result),
                    StepOutcome::Continue
                ));
            }
            StepOutcome::Done(Ok(value)) => {
                assert!(saw_suspend);
                assert_eq!(
                    value,
                    Value::Result(Err(Box::new(Value::Str("handled boom".into()))))
                );
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("async result map_err method call did not finish");
}

#[test]
fn step_task_dict_closure_methods_use_two_arg_continuations() {
    let mut arena = ChunkArena::new();
    let map_value = BuiltinClosureFn::new(|args| {
        Ok(Value::Str(format!(
            "{}={}",
            args[0].as_str().unwrap(),
            args[1]
        )))
    });
    let keep_even =
        BuiltinClosureFn::new(|args| Ok(Value::Bool(args[1].as_int().unwrap() % 2 == 0)));
    let mut input = indexmap::IndexMap::new();
    input.insert("a".into(), Value::Int(1));
    input.insert("b".into(), Value::Int(2));

    let mut chunk = Chunk::new();
    let map_method = chunk.add_constant(Value::Str("map".into()));
    let filter_method = chunk.add_constant(Value::Str("filter".into()));
    chunk.emit_constant(Value::Dict(input.clone()), 1);
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("map_value"),
            func: map_value,
        },
        1,
    );
    emit_method_call(&mut chunk, map_method, 1, 1);
    chunk.emit_constant(Value::Dict(input), 1);
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("keep_even"),
            func: keep_even,
        },
        1,
    );
    emit_method_call(&mut chunk, filter_method, 1, 1);
    chunk.emit_op_u16(Op::BuildTuple, 2, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    let mut mapped = indexmap::IndexMap::new();
    mapped.insert("a".into(), Value::Str("a=1".into()));
    mapped.insert("b".into(), Value::Str("b=2".into()));
    let mut filtered = indexmap::IndexMap::new();
    filtered.insert("b".into(), Value::Int(2));
    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![Value::Dict(mapped), Value::Dict(filtered)])
    );
}

#[test]
fn step_task_list_fold_reduce_and_flat_map_use_continuations() {
    fn add(args: &[Value]) -> Result<Value, String> {
        Ok(Value::Int(
            args[0].as_int().unwrap() + args[1].as_int().unwrap(),
        ))
    }

    let mut arena = ChunkArena::new();
    let duplicate = BuiltinClosureFn::new(|args| {
        let value = args[0].as_int().unwrap();
        Ok(Value::List(vec![Value::Int(value), Value::Int(value)]))
    });
    let mut chunk = Chunk::new();
    let fold = chunk.add_constant(Value::Str("fold".into()));
    let reduce = chunk.add_constant(Value::Str("reduce".into()));
    let flat_map = chunk.add_constant(Value::Str("flat_map".into()));

    chunk.emit_constant(
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        1,
    );
    chunk.emit_constant(Value::Int(10), 1);
    chunk.emit_constant(
        Value::BuiltinFn {
            qualified_hash: ion_core::h!("add"),
            func: add,
        },
        1,
    );
    emit_method_call(&mut chunk, fold, 2, 1);

    chunk.emit_constant(
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        1,
    );
    chunk.emit_constant(
        Value::BuiltinFn {
            qualified_hash: ion_core::h!("add"),
            func: add,
        },
        1,
    );
    emit_method_call(&mut chunk, reduce, 1, 1);

    chunk.emit_constant(Value::List(vec![Value::Int(4), Value::Int(5)]), 1);
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("duplicate"),
            func: duplicate,
        },
        1,
    );
    emit_method_call(&mut chunk, flat_map, 1, 1);

    chunk.emit_op_u16(Op::BuildTuple, 3, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::Int(16),
            Value::Int(6),
            Value::List(vec![
                Value::Int(4),
                Value::Int(4),
                Value::Int(5),
                Value::Int(5),
            ]),
        ])
    );
}

#[test]
fn step_task_method_call_sort_by_uses_continuation_comparator() {
    let mut arena = ChunkArena::new();
    let compare = BuiltinClosureFn::new(|args| {
        Ok(Value::Int(
            args[0].as_int().unwrap() - args[1].as_int().unwrap(),
        ))
    });
    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("sort_by".into()));
    chunk.emit_constant(
        Value::List(vec![Value::Int(3), Value::Int(1), Value::Int(2)]),
        4,
    );
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("compare"),
            func: compare,
        },
        4,
    );
    emit_method_call(&mut chunk, method, 1, 4);
    chunk.emit_op(Op::Return, 4);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn step_task_method_call_sort_by_suspends_on_async_host_comparator() {
    let mut arena = ChunkArena::new();
    let async_fn = AsyncBuiltinClosureFn::new(|args| async move {
        Ok(Value::Int(
            args[0].as_int().unwrap() - args[1].as_int().unwrap(),
        ))
    });
    let mut chunk = Chunk::new();
    let method = chunk.add_constant(Value::Str("sort_by".into()));
    chunk.emit_constant(Value::List(vec![Value::Int(2), Value::Int(1)]), 4);
    chunk.emit_constant(
        Value::AsyncBuiltinClosure {
            qualified_hash: ion_core::h!("compare"),
            func: async_fn,
        },
        4,
    );
    emit_method_call(&mut chunk, method, 1, 4);
    chunk.emit_op(Op::Return, 4);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert_eq!(
        run_continuation_with_host_futures(&arena, &mut cont, TaskId(57)).unwrap(),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn step_task_runs_type_check_and_match_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let type_idx = chunk.add_constant(Value::Str("int".into()));
    chunk.emit_constant(Value::Int(7), 1);
    chunk.emit_op_u16(Op::CheckType, type_idx, 1);
    chunk.emit_op(Op::WrapSome, 1);
    chunk.emit_op_u8(Op::MatchBegin, 1, 1);
    chunk.emit_op(Op::Pop, 1);
    chunk.emit_op_u8(Op::MatchArm, 1, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn step_task_constructs_host_struct_and_enum_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let point_type = chunk.add_constant(Value::Str("Point".into()));
    chunk.emit_constant(Value::Str("x".into()), 1);
    chunk.emit_constant(Value::Int(3), 1);
    chunk.emit_constant(Value::Str("y".into()), 1);
    chunk.emit_constant(Value::Int(4), 1);
    chunk.emit_op(Op::ConstructStruct, 1);
    chunk.emit((point_type >> 8) as u8, 1);
    chunk.emit((point_type & 0xff) as u8, 1);
    chunk.emit(0, 1);
    chunk.emit(2, 1);

    let status_type = chunk.add_constant(Value::Str("Status".into()));
    let ok_variant = chunk.add_constant(Value::Str("Ok".into()));
    chunk.emit_constant(Value::Str("done".into()), 1);
    chunk.emit_op(Op::ConstructEnum, 1);
    chunk.emit((status_type >> 8) as u8, 1);
    chunk.emit((status_type & 0xff) as u8, 1);
    chunk.emit((ok_variant >> 8) as u8, 1);
    chunk.emit((ok_variant & 0xff) as u8, 1);
    chunk.emit(1, 1);

    chunk.emit_op_u16(Op::BuildTuple, 2, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);
    cont.types_mut().register_struct(HostStructDef {
        name_hash: ion_core::h!("Point"),
        fields: vec![ion_core::h!("x"), ion_core::h!("y")],
    });
    cont.types_mut().register_enum(HostEnumDef {
        name_hash: ion_core::h!("Status"),
        variants: vec![HostVariantDef {
            name_hash: ion_core::h!("Ok"),
            arity: 1,
        }],
    });

    let mut fields = indexmap::IndexMap::new();
    fields.insert(ion_core::h!("x"), Value::Int(3));
    fields.insert(ion_core::h!("y"), Value::Int(4));
    for _ in 0..16 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(
                    value,
                    Value::Tuple(vec![
                        Value::HostStruct {
                            type_hash: ion_core::h!("Point"),
                            fields,
                        },
                        Value::HostEnum {
                            enum_hash: ion_core::h!("Status"),
                            variant_hash: ion_core::h!("Ok"),
                            data: vec![Value::Str("done".into())],
                        },
                    ])
                );
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("host construction did not finish");
}

#[test]
fn step_task_type_check_reports_mismatch() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let type_idx = chunk.add_constant(Value::Str("string".into()));
    chunk.emit_constant(Value::Int(7), 4);
    chunk.emit_op_u16(Op::CheckType, type_idx, 4);
    let root = arena.insert(chunk);

    match run_continuation_to_value(&arena, root) {
        Err(err) => assert_eq!(err.message, "type mismatch: expected string, got int"),
        other => panic!("unexpected continuation result: {:?}", other),
    }
}

#[test]
fn step_task_runs_list_and_dict_compound_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::List(vec![]), 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op(Op::ListAppend, 1);
    chunk.emit_constant(Value::List(vec![Value::Int(2), Value::Int(3)]), 1);
    chunk.emit_op(Op::ListExtend, 1);

    chunk.emit_constant(Value::Dict(indexmap::IndexMap::new()), 1);
    chunk.emit_constant(Value::Str("x".into()), 1);
    chunk.emit_constant(Value::Int(4), 1);
    chunk.emit_op(Op::DictInsert, 1);
    let mut merge = indexmap::IndexMap::new();
    merge.insert("y".into(), Value::Int(5));
    chunk.emit_constant(Value::Dict(merge), 1);
    chunk.emit_op(Op::DictMerge, 1);

    chunk.emit_op_u16(Op::BuildTuple, 2, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    let mut expected = indexmap::IndexMap::new();
    expected.insert("x".into(), Value::Int(4));
    expected.insert("y".into(), Value::Int(5));
    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Tuple(vec![
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            Value::Dict(expected),
        ])
    );
}

#[test]
fn step_task_runs_iterator_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::List(vec![Value::Int(1), Value::Int(2)]), 1);
    chunk.emit_op(Op::IterInit, 1);
    chunk.emit_op_u16(Op::IterNext, 0, 1);
    chunk.emit_op_u16(Op::IterNext, 0, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn step_task_iterator_exhaustion_jumps_and_pushes_unit() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::List(vec![Value::Int(1)]), 1);
    chunk.emit_op(Op::IterInit, 1);
    chunk.emit_op_u16(Op::IterNext, 0, 1);
    let done_jump = chunk.emit_jump(Op::IterNext, 1);
    chunk.emit_constant(Value::Int(99), 1);
    chunk.patch_jump(done_jump);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Unit
    );
}

#[test]
fn step_task_iter_drop_removes_active_iterator() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::List(vec![Value::Int(1)]), 1);
    chunk.emit_op(Op::IterInit, 1);
    chunk.emit_op(Op::IterDrop, 1);
    chunk.emit_op_u16(Op::IterNext, 0, 1);
    let root = arena.insert(chunk);

    match run_continuation_to_value(&arena, root) {
        Err(err) => assert_eq!(err.message, "no active iterator"),
        other => panic!("unexpected continuation result: {:?}", other),
    }
}

#[test]
fn step_task_runs_jump_if_false_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op(Op::False, 1);
    let jump = chunk.emit_jump(Op::JumpIfFalse, 1);
    chunk.emit_constant(Value::Int(1), 1);
    let end_jump = chunk.emit_jump(Op::Jump, 1);
    chunk.patch_jump(jump);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.patch_jump(end_jump);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn step_task_runs_stack_slot_locals_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op(Op::PushScope, 1);
    chunk.emit_constant(Value::Int(10), 1);
    chunk.emit_op_u8(Op::DefineLocalSlot, 1, 1);
    chunk.emit_constant(Value::Int(32), 1);
    chunk.emit_op_u16(Op::SetLocalSlot, 0, 1);
    chunk.emit_op(Op::Pop, 1);
    chunk.emit_op_u16(Op::GetLocalSlot, 0, 1);
    chunk.emit_op(Op::PopScope, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(32)
    );
}

#[test]
fn step_task_runs_env_local_and_global_scaffold() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let name = chunk.add_constant(Value::Str("x".into()));
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op_u16(Op::DefineLocal, name, 1);
    chunk.emit(1, 1);
    chunk.emit_op_u16(Op::GetLocal, name, 1);
    chunk.emit_constant(Value::Int(2), 1);
    chunk.emit_op(Op::Add, 1);
    chunk.emit_op_u16(Op::SetLocal, name, 1);
    chunk.emit_op(Op::Pop, 1);
    chunk.emit_op_u16(Op::GetGlobal, name, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(3)
    );
}

#[test]
fn step_task_env_scope_pop_hides_local() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    let name = chunk.add_constant(Value::Str("hidden".into()));
    chunk.emit_op(Op::PushScope, 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op_u16(Op::DefineLocal, name, 1);
    chunk.emit(0, 1);
    chunk.emit_op(Op::PopScope, 1);
    chunk.emit_op_u16(Op::GetLocal, name, 1);
    let root = arena.insert(chunk);

    match run_continuation_to_value(&arena, root) {
        Err(err) => assert_eq!(err.message, "undefined variable: hidden"),
        other => panic!("unexpected continuation result: {:?}", other),
    }
}

#[test]
fn step_task_pop_scope_truncates_stack_slot_locals() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op(Op::PushScope, 1);
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op_u8(Op::DefineLocalSlot, 0, 1);
    chunk.emit_op(Op::PopScope, 1);
    chunk.emit_op_u16(Op::GetLocalSlot, 0, 1);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => assert_eq!(err.message, "local slot out of bounds"),
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_rejects_assignment_to_immutable_stack_slot() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_constant(Value::Int(1), 1);
    chunk.emit_op_u8(Op::DefineLocalSlot, 0, 1);
    chunk.emit_constant(Value::Int(2), 2);
    chunk.emit_op_u16_span(Op::SetLocalSlot, 0, 2, 5);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.message, "cannot assign to immutable variable");
            assert_eq!(err.line, 2);
            assert_eq!(err.col, 5);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_calls_registered_function_chunk_with_explicit_frame() {
    let mut arena = ChunkArena::new();
    let mut callee = Chunk::new();
    callee.emit_op_u16(Op::GetLocalSlot, 0, 1);
    callee.emit_constant(Value::Int(1), 1);
    callee.emit_op(Op::Add, 1);
    callee.emit_op(Op::Return, 1);
    let callee_id = arena.insert(callee);

    let function = IonFn::new(
        "add_one".into(),
        vec![Param {
            name: "x".into(),
            default: None,
        }],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;

    let mut root = Chunk::new();
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(41), 1);
    root.emit_op_u8(Op::Call, 1, 1);
    root.emit_op(Op::Return, 1);
    let root_id = arena.insert(root);

    let mut cont = VmContinuation::new(root_id);
    cont.register_fn_chunk(fn_id, callee_id);
    for _ in 0..16 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(value, Value::Int(42));
                assert_eq!(cont.frames.len(), 0);
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("function call did not finish");
}

#[test]
fn step_task_registered_function_chunk_uses_captures_and_default_args() {
    let mut arena = ChunkArena::new();
    let mut callee = Chunk::new();
    let captured_name = callee.add_constant(Value::Str("offset".into()));
    callee.emit_op_u16(Op::GetLocalSlot, 0, 1);
    callee.emit_op_u16(Op::GetLocalSlot, 1, 1);
    callee.emit_op(Op::Add, 1);
    callee.emit_op_u16(Op::GetGlobal, captured_name, 1);
    callee.emit_op(Op::Add, 1);
    callee.emit_op(Op::Return, 1);
    let callee_id = arena.insert(callee);

    let mut captures = HashMap::new();
    captures.insert("offset".into(), Value::Int(1));
    let function = IonFn::new(
        "with_default".into(),
        vec![
            Param {
                name: "a".into(),
                default: None,
            },
            Param {
                name: "b".into(),
                default: Some(default_expr_add_ident_int("a", 20)),
            },
        ],
        vec![],
        captures,
    );
    let fn_id = function.fn_id;

    let mut root = Chunk::new();
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(20), 1);
    root.emit_op_u8(Op::Call, 1, 1);
    root.emit_op(Op::Return, 1);
    let root_id = arena.insert(root);

    let mut cont = VmContinuation::new(root_id);
    cont.register_fn_chunk(fn_id, callee_id);
    for _ in 0..24 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(value, Value::Int(61));
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("function call with defaults did not finish");
}

#[test]
fn step_task_calls_sync_builtin_and_closure_inline() {
    fn inc(args: &[Value]) -> Result<Value, String> {
        Ok(Value::Int(args[0].as_int().unwrap() + 1))
    }

    let mut arena = ChunkArena::new();
    let closure = BuiltinClosureFn::new(|args| Ok(Value::Int(args[0].as_int().unwrap() + 2)));
    let mut chunk = Chunk::new();
    chunk.emit_constant(
        Value::BuiltinFn {
            qualified_hash: ion_core::h!("inc"),
            func: inc,
        },
        1,
    );
    chunk.emit_constant(Value::Int(40), 1);
    chunk.emit_op_u8(Op::Call, 1, 1);
    chunk.emit_constant(
        Value::BuiltinClosure {
            qualified_hash: ion_core::h!("plus_two"),
            func: closure,
        },
        1,
    );
    chunk.emit_constant(Value::Int(40), 1);
    chunk.emit_op_u8(Op::Call, 1, 1);
    chunk.emit_op(Op::Add, 1);
    chunk.emit_op(Op::Return, 1);
    let root = arena.insert(chunk);

    assert_eq!(
        run_continuation_to_value(&arena, root).unwrap(),
        Value::Int(83)
    );
}

#[test]
fn step_task_call_named_reorders_registered_function_chunk_args() {
    let mut arena = ChunkArena::new();
    let mut callee = Chunk::new();
    callee.emit_op_u16(Op::GetLocalSlot, 0, 1);
    callee.emit_op_u16(Op::GetLocalSlot, 1, 1);
    callee.emit_op(Op::Sub, 1);
    callee.emit_op(Op::Return, 1);
    let callee_id = arena.insert(callee);

    let function = IonFn::new(
        "subtract".into(),
        vec![
            Param {
                name: "a".into(),
                default: None,
            },
            Param {
                name: "b".into(),
                default: None,
            },
        ],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;

    let mut root = Chunk::new();
    let b_name = root.add_constant(Value::Str("b".into()));
    let a_name = root.add_constant(Value::Str("a".into()));
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(2), 1);
    root.emit_constant(Value::Int(10), 1);
    root.emit_op(Op::CallNamed, 1);
    root.emit(2, 1);
    root.emit(2, 1);
    root.emit(0, 1);
    root.emit((b_name >> 8) as u8, 1);
    root.emit((b_name & 0xff) as u8, 1);
    root.emit(1, 1);
    root.emit((a_name >> 8) as u8, 1);
    root.emit((a_name & 0xff) as u8, 1);
    root.emit_op(Op::Return, 1);
    let root_id = arena.insert(root);

    let mut cont = VmContinuation::new(root_id);
    cont.register_fn_chunk(fn_id, callee_id);
    for _ in 0..16 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(value, Value::Int(8));
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("named call did not finish");
}

#[test]
fn step_task_call_named_fills_registered_function_defaults() {
    let mut arena = ChunkArena::new();
    let mut callee = Chunk::new();
    callee.emit_op_u16(Op::GetLocalSlot, 0, 1);
    callee.emit_op_u16(Op::GetLocalSlot, 1, 1);
    callee.emit_op(Op::Sub, 1);
    callee.emit_op(Op::Return, 1);
    let callee_id = arena.insert(callee);

    let function = IonFn::new(
        "subtract".into(),
        vec![
            Param {
                name: "a".into(),
                default: None,
            },
            Param {
                name: "b".into(),
                default: Some(expr_int(2)),
            },
        ],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;

    let mut root = Chunk::new();
    let a_name = root.add_constant(Value::Str("a".into()));
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(10), 1);
    root.emit_op(Op::CallNamed, 1);
    root.emit(1, 1);
    root.emit(1, 1);
    root.emit(0, 1);
    root.emit((a_name >> 8) as u8, 1);
    root.emit((a_name & 0xff) as u8, 1);
    root.emit_op(Op::Return, 1);
    let root_id = arena.insert(root);

    let mut cont = VmContinuation::new(root_id);
    cont.register_fn_chunk(fn_id, callee_id);
    for _ in 0..16 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(value, Value::Int(8));
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("named call with default did not finish");
}

#[test]
fn step_task_tail_calls_registered_function_chunk_by_reusing_frame() {
    let mut arena = ChunkArena::new();
    let mut callee = Chunk::new();
    callee.emit_op_u16(Op::GetLocalSlot, 0, 1);
    callee.emit_constant(Value::Int(1), 1);
    callee.emit_op(Op::Add, 1);
    callee.emit_op(Op::Return, 1);
    let callee_id = arena.insert(callee);

    let function = IonFn::new(
        "add_one".into(),
        vec![Param {
            name: "x".into(),
            default: None,
        }],
        vec![],
        HashMap::new(),
    );
    let fn_id = function.fn_id;

    let mut root = Chunk::new();
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(41), 1);
    root.emit_op_u8(Op::TailCall, 1, 1);
    let root_id = arena.insert(root);

    let mut cont = VmContinuation::new(root_id);
    cont.register_fn_chunk(fn_id, callee_id);

    for _ in 0..16 {
        match step_task(&arena, &mut cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(Ok(value)) => {
                assert_eq!(value, Value::Int(42));
                assert_eq!(cont.frames.len(), 0);
                return;
            }
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("tail call did not finish");
}

#[test]
fn step_task_reports_missing_registered_function_chunk() {
    let mut arena = ChunkArena::new();
    let function = IonFn::new(
        "missing".into(),
        vec![Param {
            name: "x".into(),
            default: None,
        }],
        vec![],
        HashMap::new(),
    );
    let mut root = Chunk::new();
    root.emit_constant(Value::Fn(function), 1);
    root.emit_constant(Value::Int(1), 1);
    root.emit_op_u8(Op::Call, 1, 1);
    let root_id = arena.insert(root);
    let mut cont = VmContinuation::new(root_id);

    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    assert!(matches!(
        step_task(&arena, &mut cont),
        StepOutcome::Continue
    ));
    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.message, "function chunk not registered");
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn step_task_reports_stack_underflow_for_binary_op() {
    let mut arena = ChunkArena::new();
    let mut chunk = Chunk::new();
    chunk.emit_op_span(Op::Add, 11, 4);
    let root = arena.insert(chunk);
    let mut cont = VmContinuation::new(root);

    match step_task(&arena, &mut cont) {
        StepOutcome::InstructionError(err) => {
            assert_eq!(err.message, "stack underflow");
            assert_eq!(err.line, 11);
            assert_eq!(err.col, 4);
        }
        other => panic!("unexpected step outcome: {:?}", other),
    }
}

#[test]
fn budgeted_runner_stops_when_budget_exhausts() {
    let mut task = IonTask::default();
    let mut steps = 0;
    let run = run_budgeted_steps(&mut task, 3, |_task| {
        steps += 1;
        StepOutcome::Continue
    });

    assert_eq!(steps, 3);
    assert_eq!(run.consumed, 3);
    assert!(matches!(run.outcome, TaskRunOutcome::BudgetExhausted));
    assert_eq!(task.state, TaskState::Ready);
}

#[test]
fn budgeted_runner_charges_zero_instruction_yield() {
    let mut task = IonTask::default();
    let run = run_budgeted_steps(&mut task, 100, |_task| StepOutcome::Yield);

    assert_eq!(run.consumed, 1);
    assert!(matches!(run.outcome, TaskRunOutcome::Yielded));
    assert_eq!(task.state, TaskState::Ready);
}

#[test]
fn budgeted_runner_charges_zero_instruction_suspend() {
    let mut task = IonTask::default();
    let mut table = HostFutureTable::new();
    let future_id = table.insert(TaskId(99), Box::pin(async { Ok(Value::Unit) }));
    let state = TaskState::WaitingHostFuture(future_id);

    let run = run_budgeted_steps(&mut task, 100, |_task| {
        StepOutcome::Suspended(state.clone())
    });

    assert_eq!(run.consumed, 1);
    assert!(matches!(
        run.outcome,
        TaskRunOutcome::Suspended(TaskState::WaitingHostFuture(id)) if id == future_id
    ));
    assert_eq!(task.state, TaskState::WaitingHostFuture(future_id));
}

#[test]
fn budgeted_runner_observes_cancellation_before_stepping() {
    let mut task = IonTask::default();
    task.cancel_requested = true;
    let mut stepped = false;

    let run = run_budgeted_steps(&mut task, 100, |_task| {
        stepped = true;
        StepOutcome::Continue
    });

    assert!(!stepped);
    assert_eq!(run.consumed, 1);
    assert!(matches!(run.outcome, TaskRunOutcome::Cancelled));
    assert_eq!(task.state, TaskState::Done);
}

#[test]
fn task_table_awaits_and_resumes_waiter_on_finish() {
    let mut tasks = TaskTable::new();
    let waiter = tasks.spawn_ready();
    let child = tasks.spawn_ready();

    assert!(matches!(
        tasks.await_task(waiter, child),
        TaskAwait::Waiting
    ));
    assert_eq!(
        tasks.get(waiter).unwrap().state,
        TaskState::WaitingTask(child)
    );

    let resumes = tasks.finish(child, Ok(Value::Int(123)));
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0].waiter, waiter);
    assert_eq!(resumes[0].result.as_ref().unwrap(), &Value::Int(123));
    assert_eq!(tasks.get(waiter).unwrap().state, TaskState::Ready);
}

#[test]
fn task_table_await_finished_task_returns_immediately() {
    let mut tasks = TaskTable::new();
    let waiter = tasks.spawn_ready();
    let child = tasks.spawn_ready();
    assert!(tasks
        .finish(child, Ok(Value::Str("done".into())))
        .is_empty());

    match tasks.await_task(waiter, child) {
        TaskAwait::Ready(Ok(Value::Str(value))) => assert_eq!(value, "done"),
        other => panic!("unexpected await result: {:?}", other),
    }
    assert_eq!(tasks.get(waiter).unwrap().state, TaskState::Ready);
}

#[test]
fn task_table_supports_multiple_waiters() {
    let mut tasks = TaskTable::new();
    let waiter_a = tasks.spawn_ready();
    let waiter_b = tasks.spawn_ready();
    let child = tasks.spawn_ready();

    assert!(matches!(
        tasks.await_task(waiter_a, child),
        TaskAwait::Waiting
    ));
    assert!(matches!(
        tasks.await_task(waiter_b, child),
        TaskAwait::Waiting
    ));

    let resumes = tasks.finish(child, Ok(Value::Bool(true)));
    assert_eq!(resumes.len(), 2);
    assert_eq!(resumes[0].waiter, waiter_a);
    assert_eq!(resumes[1].waiter, waiter_b);
    assert_eq!(tasks.get(waiter_a).unwrap().state, TaskState::Ready);
    assert_eq!(tasks.get(waiter_b).unwrap().state, TaskState::Ready);
}

#[test]
fn task_table_marks_task_cancel_requested() {
    let mut tasks = TaskTable::new();
    let task = tasks.spawn_ready();
    assert!(tasks.cancel(task));
    assert!(tasks.get(task).unwrap().cancel_requested);
    assert!(!tasks.cancel(TaskId(999)));
}

#[test]
fn nursery_table_registers_and_drains_children() {
    let mut nurseries = NurseryTable::new();
    let nursery = nurseries.open(TaskId(1));

    assert!(nurseries.add_child(nursery, TaskId(10)));
    assert!(nurseries.add_child(nursery, TaskId(11)));
    assert_eq!(
        nurseries.get(nursery).unwrap().children,
        vec![TaskId(10), TaskId(11)]
    );

    assert!(!nurseries.child_finished(nursery, TaskId(10)));
    assert_eq!(nurseries.get(nursery).unwrap().children, vec![TaskId(11)]);
    assert!(nurseries.child_finished(nursery, TaskId(11)));

    let drained = nurseries.drain(nursery).unwrap();
    assert_eq!(drained.parent, TaskId(1));
    assert!(drained.children.is_empty());
    assert!(nurseries.get(nursery).is_none());
}

#[test]
fn nursery_table_fail_fast_returns_children_to_cancel() {
    let mut nurseries = NurseryTable::new();
    let nursery = nurseries.open(TaskId(1));
    assert!(nurseries.add_child(nursery, TaskId(10)));
    assert!(nurseries.add_child(nursery, TaskId(11)));

    let to_cancel = nurseries.fail_fast(nursery, IonError::runtime("boom", 0, 0));
    assert_eq!(to_cancel, vec![TaskId(10), TaskId(11)]);

    match &nurseries.get(nursery).unwrap().state {
        NurseryState::Failing(err) => assert_eq!(err.message, "boom"),
        other => panic!("unexpected nursery state: {:?}", other),
    }
}

#[test]
fn nursery_table_rejects_stale_id_after_drain_and_reuse() {
    let mut nurseries = NurseryTable::new();
    let old = nurseries.open(TaskId(1));
    assert!(nurseries.drain(old).is_some());

    let new = nurseries.open(TaskId(2));
    assert_ne!(old, new);
    assert!(nurseries.get(old).is_none());
    assert_eq!(nurseries.get(new).unwrap().parent, TaskId(2));
}

#[tokio::test]
async fn timer_table_polls_expired_timer() {
    let mut table = TimerTable::new();
    let id = table.insert_sleep(TaskId(5), Duration::from_millis(1));

    tokio::time::sleep(Duration::from_millis(2)).await;

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let ready = table.poll_ready(&mut cx);

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, id);
    assert_eq!(ready[0].waiter, TaskId(5));
    assert!(!table.contains(id));
}

#[tokio::test]
async fn timer_table_keeps_pending_timer_until_deadline() {
    let mut table = TimerTable::new();
    let id = table.insert_sleep(TaskId(6), Duration::from_secs(60));

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(table.poll_ready(&mut cx).is_empty());
    assert!(table.contains(id));
    assert!(table.cancel(id));
}

#[tokio::test]
async fn timer_table_cancels_only_target_timer() {
    let mut table = TimerTable::new();
    let id_a = table.insert_sleep(TaskId(1), Duration::from_secs(60));
    let id_b = table.insert_sleep(TaskId(2), Duration::from_secs(60));

    assert!(table.cancel(id_a));
    assert!(!table.contains(id_a));
    assert!(table.contains(id_b));
    assert!(table.cancel(id_b));
}

#[test]
fn async_channel_buffers_values_until_received() {
    let mut channel = AsyncChannel::new(2);
    assert_eq!(channel.send(TaskId(1), Value::Int(10)), ChannelSend::Sent);
    assert_eq!(channel.send(TaskId(1), Value::Int(20)), ChannelSend::Sent);
    assert_eq!(channel.buffered_len(), 2);

    assert_eq!(
        channel.recv(TaskId(2)),
        ChannelRecv::Received {
            value: Value::Int(10),
            unblocked_sender: None,
        }
    );
    assert_eq!(
        channel.recv(TaskId(2)),
        ChannelRecv::Received {
            value: Value::Int(20),
            unblocked_sender: None,
        }
    );
}

#[test]
fn async_channel_delivers_directly_to_waiting_receiver() {
    let mut channel = AsyncChannel::new(1);
    assert_eq!(channel.recv(TaskId(2)), ChannelRecv::Blocked);
    assert_eq!(
        channel.send(TaskId(1), Value::Str("msg".into())),
        ChannelSend::Delivered {
            receiver: TaskId(2),
            value: Value::Str("msg".into()),
        }
    );
    assert_eq!(channel.buffered_len(), 0);
}

#[test]
fn async_channel_blocks_sender_when_full_and_unblocks_after_recv() {
    let mut channel = AsyncChannel::new(1);
    assert_eq!(channel.send(TaskId(1), Value::Int(1)), ChannelSend::Sent);
    assert_eq!(channel.send(TaskId(2), Value::Int(2)), ChannelSend::Blocked);

    assert_eq!(
        channel.recv(TaskId(3)),
        ChannelRecv::Received {
            value: Value::Int(1),
            unblocked_sender: Some(TaskId(2)),
        }
    );
    assert_eq!(channel.buffered_len(), 1);
    assert_eq!(
        channel.recv(TaskId(3)),
        ChannelRecv::Received {
            value: Value::Int(2),
            unblocked_sender: None,
        }
    );
}

#[test]
fn async_channel_close_wakes_waiters_and_rejects_sends() {
    let mut channel = AsyncChannel::new(1);
    assert_eq!(channel.recv(TaskId(10)), ChannelRecv::Blocked);

    let (receivers, senders) = channel.close();
    assert_eq!(receivers, vec![TaskId(10)]);
    assert!(senders.is_empty());
    assert!(channel.is_closed());
    assert_eq!(
        channel.send(TaskId(22), Value::Int(3)),
        ChannelSend::Closed(Value::Int(3))
    );

    let mut full_channel = AsyncChannel::new(1);
    assert_eq!(
        full_channel.send(TaskId(20), Value::Int(1)),
        ChannelSend::Sent
    );
    assert_eq!(
        full_channel.send(TaskId(21), Value::Int(2)),
        ChannelSend::Blocked
    );
    let (receivers, senders) = full_channel.close();
    assert!(receivers.is_empty());
    assert_eq!(senders, vec![TaskId(21)]);
}

#[test]
fn channel_table_returns_channels_by_stable_id() {
    let mut table = ChannelTable::new();
    let id = table.insert(AsyncChannel::new(1));
    assert!(table.get(id).is_some());

    assert_eq!(
        table.get_mut(id).unwrap().send(TaskId(1), Value::Int(9)),
        ChannelSend::Sent
    );
    assert_eq!(table.get(id).unwrap().buffered_len(), 1);
}

#[derive(Debug)]
struct CaptureOutput {
    writes: Arc<Mutex<Vec<(OutputStream, String)>>>,
}

impl OutputHandler for CaptureOutput {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push((stream, text.to_string()));
        Ok(())
    }
}

struct DropFlagFuture {
    dropped: Rc<Cell<bool>>,
}

impl DropFlagFuture {
    fn new(dropped: Rc<Cell<bool>>) -> Self {
        Self { dropped }
    }
}

impl Future for DropFlagFuture {
    type Output = Result<Value, IonError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for DropFlagFuture {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

struct ControlledFuture {
    state: Rc<RefCell<Option<Result<Value, IonError>>>>,
}

impl ControlledFuture {
    fn new(state: Rc<RefCell<Option<Result<Value, IonError>>>>) -> Self {
        Self { state }
    }
}

impl Future for ControlledFuture {
    type Output = Result<Value, IonError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state.borrow_mut().take() {
            Some(result) => Poll::Ready(result),
            None => Poll::Pending,
        }
    }
}

struct TaskProbeFuture {
    idx: usize,
    poll_counts: Rc<RefCell<Vec<u32>>>,
    states: Rc<RefCell<Vec<Option<Result<Value, IonError>>>>>,
}

impl TaskProbeFuture {
    fn new(
        idx: usize,
        poll_counts: Rc<RefCell<Vec<u32>>>,
        states: Rc<RefCell<Vec<Option<Result<Value, IonError>>>>>,
    ) -> Self {
        Self {
            idx,
            poll_counts,
            states,
        }
    }
}

impl Future for TaskProbeFuture {
    type Output = Result<Value, IonError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_counts.borrow_mut()[self.idx] += 1;
        match self.states.borrow_mut()[self.idx].take() {
            Some(result) => Poll::Ready(result),
            None => Poll::Pending,
        }
    }
}

fn run_continuation_to_value(
    arena: &ChunkArena,
    root: ion_core::async_runtime::ChunkId,
) -> Result<Value, IonError> {
    let mut cont = VmContinuation::new(root);
    run_existing_continuation_to_value(arena, &mut cont)
}

fn run_existing_continuation_to_value(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
) -> Result<Value, IonError> {
    for _ in 0..128 {
        match step_task(arena, cont) {
            StepOutcome::Continue => {}
            StepOutcome::Done(result) => return result,
            StepOutcome::InstructionError(err) => return Err(err),
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("continuation did not finish within test step limit");
}

fn run_continuation_with_host_futures(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    task: TaskId,
) -> Result<Value, IonError> {
    let mut futures = HostFutureTable::new();
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    for _ in 0..128 {
        match step_task_with_host_futures(arena, cont, task, &mut futures) {
            StepOutcome::Continue => {}
            StepOutcome::Suspended(TaskState::WaitingHostFuture(_)) => {
                let mut ready = futures.poll_ready(&mut cx);
                if ready.is_empty() {
                    continue;
                }
                assert!(matches!(
                    cont.resume_host_result(ready.remove(0).result),
                    StepOutcome::Continue
                ));
            }
            StepOutcome::Done(result) => return result,
            StepOutcome::InstructionError(err) => return Err(err),
            other => panic!("unexpected step outcome: {:?}", other),
        }
    }
    panic!("continuation did not finish within test step limit");
}

fn parse_program(source: &str) -> ion_core::ast::Program {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().unwrap();
    let mut parser = Parser::new(tokens);
    parser.parse_program().unwrap()
}

fn emit_method_call(chunk: &mut Chunk, method_idx: u16, arg_count: u8, line: usize) {
    chunk.emit_op(Op::MethodCall, line);
    chunk.emit((method_idx >> 8) as u8, line);
    chunk.emit((method_idx & 0xff) as u8, line);
    chunk.emit(arg_count, line);
}

fn expr_int(value: i64) -> Expr {
    Expr {
        kind: ExprKind::Int(value),
        span: Span { line: 1, col: 1 },
    }
}

fn expr_ident(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Ident(name.into()),
        span: Span { line: 1, col: 1 },
    }
}

fn default_expr_add_ident_int(name: &str, value: i64) -> Expr {
    Expr {
        kind: ExprKind::BinOp {
            left: Box::new(expr_ident(name)),
            op: BinOp::Add,
            right: Box::new(expr_int(value)),
        },
        span: Span { line: 1, col: 1 },
    }
}

fn noop_waker() -> Waker {
    unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    unsafe fn wake(_: *const ()) {}
    unsafe fn wake_by_ref(_: *const ()) {}
    unsafe fn drop(_: *const ()) {}

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}
