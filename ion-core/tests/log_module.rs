//! Behavioural tests for the `log::` stdlib module.
//!
//! These tests cover:
//! - runtime dispatch through a custom `LogHandler`
//! - the runtime threshold (`log::set_level` / `log::level`)
//! - structured field passing (the optional dict argument)
//! - argument-shape validation
//! - that compile-time elision drops calls above [`COMPILE_LOG_CAP`]
//!   (the `cap_strip_above_threshold` test exercises whichever cap the
//!   surrounding build picked — toggle features to verify variants).
//!
//! Sync-build only — async builds drive `log::*` through `eval_async` and
//! the `Mutex<Vec<Value>>` recording handler used here can't be `Send` once
//! `Value` carries an `AsyncBuiltinClosureFn` (Rc-backed under async).

#![cfg(not(feature = "async-runtime"))]

use std::sync::{Arc, Mutex};

use ion_core::engine::Engine;
use ion_core::log::{LogHandler, LogLevel, COMPILE_LOG_CAP};
use ion_core::value::Value;

#[derive(Default)]
struct RecordingHandler {
    records: Mutex<Vec<(LogLevel, String, Vec<(String, Value)>)>>,
    enabled_for: Mutex<Option<LogLevel>>,
}

impl RecordingHandler {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn drain(&self) -> Vec<(LogLevel, String, Vec<(String, Value)>)> {
        std::mem::take(&mut *self.records.lock().unwrap())
    }

    fn allow_only(&self, lvl: LogLevel) {
        *self.enabled_for.lock().unwrap() = Some(lvl);
    }
}

impl LogHandler for RecordingHandler {
    fn enabled(&self, level: LogLevel) -> bool {
        match *self.enabled_for.lock().unwrap() {
            Some(allow) => level == allow,
            None => true,
        }
    }

    fn log(&self, level: LogLevel, message: &str, fields: &[(String, Value)]) {
        self.records
            .lock()
            .unwrap()
            .push((level, message.to_string(), fields.to_vec()));
    }
}

fn engine_with(handler: Arc<RecordingHandler>) -> Engine {
    // No public stdout handler is needed here — tests don't print.
    let mut engine = Engine::new();
    engine.set_log_handler_arc(handler as Arc<dyn LogHandler>);
    engine
}

#[test]
fn dispatch_each_level() {
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    engine
        .eval(
            r#"
        log::trace("t");
        log::debug("d");
        log::info("i");
        log::warn("w");
        log::error("e");
    "#,
        )
        .unwrap();
    let records = h.drain();
    // Only levels at or below COMPILE_LOG_CAP survived elision.
    let want: Vec<(LogLevel, &'static str)> = [
        (LogLevel::Trace, "t"),
        (LogLevel::Debug, "d"),
        (LogLevel::Info, "i"),
        (LogLevel::Warn, "w"),
        (LogLevel::Error, "e"),
    ]
    .into_iter()
    .filter(|(lvl, _)| lvl.allowed_under(COMPILE_LOG_CAP))
    .collect();
    assert_eq!(records.len(), want.len(), "got: {:?}", records);
    for ((got_lvl, got_msg, _), (want_lvl, want_msg)) in records.iter().zip(want.iter()) {
        assert_eq!(got_lvl, want_lvl);
        assert_eq!(got_msg, want_msg);
    }
}

#[test]
fn structured_fields_pass_through() {
    if !LogLevel::Info.allowed_under(COMPILE_LOG_CAP) {
        return; // skip when info is stripped
    }
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    engine
        .eval(r#"log::info("started", #{ port: 8080, host: "localhost" });"#)
        .unwrap();
    let records = h.drain();
    assert_eq!(records.len(), 1);
    let (_lvl, msg, fields) = &records[0];
    assert_eq!(msg, "started");
    assert_eq!(fields.len(), 2);
    let port = fields.iter().find(|(k, _)| k == "port").unwrap();
    assert_eq!(port.1, Value::Int(8080));
}

#[test]
fn runtime_threshold_blocks_via_enabled() {
    if !LogLevel::Warn.allowed_under(COMPILE_LOG_CAP) {
        return;
    }
    let h = RecordingHandler::new();
    h.allow_only(LogLevel::Warn);
    let mut engine = engine_with(Arc::clone(&h));
    engine
        .eval(
            r#"
        log::info("blocked");
        log::warn("through");
        log::error("blocked too");
    "#,
        )
        .unwrap();
    let records = h.drain();
    let levels: Vec<LogLevel> = records.iter().map(|(l, _, _)| *l).collect();
    assert_eq!(levels, vec![LogLevel::Warn]);
}

#[test]
fn set_level_and_level_round_trip() {
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    let level = engine.eval(r#"log::level()"#).unwrap();
    assert!(matches!(level, Value::Str(_)));
    engine.eval(r#"log::set_level("warn");"#).unwrap();
    let v = engine.eval(r#"log::level()"#).unwrap();
    assert_eq!(v, Value::Str("warn".to_string()));
}

#[test]
fn set_level_rejects_unknown() {
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    let err = engine.eval(r#"log::set_level("verbose");"#).unwrap_err();
    assert!(
        err.message.contains("unknown level"),
        "got: {}",
        err.message
    );
}

#[test]
fn arity_validation() {
    // Use `error` because it survives every cap above `Off`. For stripped
    // levels the call is a no-op and arity is never checked, so the test
    // would be a tautology.
    if !LogLevel::Error.allowed_under(COMPILE_LOG_CAP) {
        return;
    }
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    // Zero args
    assert!(engine.eval(r#"log::error();"#).is_err());
    // Three args (max is two: message, fields)
    assert!(engine.eval(r#"log::error("a", #{}, "extra");"#).is_err());
    // Wrong fields type
    assert!(engine.eval(r#"log::error("a", 42);"#).is_err());
}

#[test]
fn cap_strip_above_threshold() {
    // For each level above the compile cap, calling it should be a no-op:
    // the handler must not see it AND the args (an `expensive()` call that
    // would mutate observable state) must not be evaluated.
    let h = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h));
    engine
        .eval(
            r#"
        let counter = cell(0);
        fn bump() { counter.set(counter.get() + 1); counter.get() }
        log::trace(f"t={bump()}");
        log::debug(f"d={bump()}");
        log::info(f"i={bump()}");
        log::warn(f"w={bump()}");
        log::error(f"e={bump()}");
    "#,
        )
        .unwrap();
    let final_count = engine.eval("counter.get()").unwrap();
    let expected_calls: i64 = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ]
    .iter()
    .filter(|l| l.allowed_under(COMPILE_LOG_CAP))
    .count() as i64;
    assert_eq!(
        final_count,
        Value::Int(expected_calls),
        "expected {} bump() calls; cap is {:?}",
        expected_calls,
        COMPILE_LOG_CAP
    );
    let records = h.drain();
    assert_eq!(records.len(), expected_calls as usize);
}

#[test]
fn engine_set_log_handler_replaces_module() {
    if !LogLevel::Error.allowed_under(COMPILE_LOG_CAP) {
        return; // every level is stripped — handler is never observed
    }
    let h1 = RecordingHandler::new();
    let mut engine = engine_with(Arc::clone(&h1));
    engine.eval(r#"log::error("first");"#).unwrap();
    assert_eq!(h1.drain().len(), 1);

    // Swap in a second handler.
    let h2 = RecordingHandler::new();
    engine.set_log_handler_arc(Arc::clone(&h2) as Arc<dyn LogHandler>);
    engine.eval(r#"log::error("second");"#).unwrap();

    assert!(h1.drain().is_empty(), "old handler should be detached");
    let r2 = h2.drain();
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0].1, "second");
}
