// This is the sync-build integration suite. Async builds (`async-runtime`
// feature) drop sync `Engine::eval` and use `Engine::eval_async` instead;
// the async-side stdlib coverage lives in `integration_async.rs`.
#![cfg(not(feature = "async-runtime"))]
#![allow(clippy::approx_constant)]

use ion_core::engine::Engine;
use ion_core::h;
use ion_core::host_types::{HostEnumDef, HostStructDef, HostVariantDef, IonType};
use ion_core::interpreter::Limits;
use ion_core::module::Module;
use ion_core::stdlib::{OutputHandler, OutputStream};
use ion_core::value::Value;
use ion_core::IonType;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct CaptureOutput {
    stdout: Mutex<String>,
    stderr: Mutex<String>,
}

impl OutputHandler for CaptureOutput {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String> {
        let buffer = match stream {
            OutputStream::Stdout => &self.stdout,
            OutputStream::Stderr => &self.stderr,
        };
        buffer.lock().unwrap().push_str(text);
        Ok(())
    }
}

fn eval(src: &str) -> Value {
    let mut engine = Engine::new();
    engine.eval(src).unwrap()
}

fn eval_err(src: &str) -> String {
    let mut engine = Engine::new();
    engine.eval(src).unwrap_err().message
}

// ============================================================
// Section 1: Primitives & Literals
// ============================================================

#[test]
fn test_int_literal() {
    assert_eq!(eval("42"), Value::Int(42));
}

#[test]
fn test_float_literal() {
    assert_eq!(eval("3.14"), Value::Float(3.14));
}

#[test]
fn test_bool_literal() {
    assert_eq!(eval("true"), Value::Bool(true));
    assert_eq!(eval("false"), Value::Bool(false));
}

#[test]
fn test_string_literal() {
    assert_eq!(eval(r#""hello""#), Value::Str("hello".into()));
}

#[test]
fn test_unit() {
    assert_eq!(eval("()"), Value::Unit);
}

// ============================================================
// Section 2: Variables & Mutability
// ============================================================

#[test]
fn test_let_immutable() {
    assert_eq!(eval("let x = 10; x"), Value::Int(10));
}

#[test]
fn test_let_mut() {
    assert_eq!(eval("let mut x = 1; x = 2; x"), Value::Int(2));
}

#[test]
fn test_immutable_assign_error() {
    let msg = eval_err("let x = 1; x = 2;");
    assert!(
        msg.contains("immutable"),
        "expected immutable error, got: {}",
        msg
    );
}

#[test]
fn test_shadowing() {
    assert_eq!(eval("let x = 1; let x = 2; x"), Value::Int(2));
}

#[test]
fn test_shadowing_type_change() {
    assert_eq!(
        eval(r#"let x = 1; let x = "hello"; x"#),
        Value::Str("hello".into())
    );
}

#[test]
fn test_shadowing_freeze() {
    let msg = eval_err("let mut x = 1; let x = x; x = 3;");
    assert!(
        msg.contains("immutable"),
        "expected immutable error, got: {}",
        msg
    );
}

#[test]
fn test_shadowing_unfreeze() {
    assert_eq!(eval("let x = 1; let mut x = x; x = 10; x"), Value::Int(10));
}

// ============================================================
// Section 3: Scoping
// ============================================================

#[test]
fn test_block_scope() {
    assert_eq!(eval("let x = 1; { let x = 2; x }"), Value::Int(2));
}

#[test]
fn test_outer_scope_visible() {
    assert_eq!(eval("let x = 10; { x + 5 }"), Value::Int(15));
}

#[test]
fn test_inner_scope_not_visible_outside() {
    let msg = eval_err("{ let y = 1; } y;");
    assert!(
        msg.contains("undefined"),
        "expected undefined error, got: {}",
        msg
    );
}

// ============================================================
// Section 4: Arithmetic
// ============================================================

#[test]
fn test_arithmetic() {
    assert_eq!(eval("2 + 3"), Value::Int(5));
    assert_eq!(eval("10 - 4"), Value::Int(6));
    assert_eq!(eval("3 * 7"), Value::Int(21));
    assert_eq!(eval("20 / 4"), Value::Int(5));
    assert_eq!(eval("10 % 3"), Value::Int(1));
}

#[test]
fn test_float_arithmetic() {
    assert_eq!(eval("1.5 + 2.5"), Value::Float(4.0));
    assert_eq!(eval("3.0 * 2.0"), Value::Float(6.0));
}

#[test]
fn test_int_float_mixed() {
    assert_eq!(eval("1 + 2.5"), Value::Float(3.5));
    assert_eq!(eval("2.5 + 1"), Value::Float(3.5));
}

#[test]
fn test_string_concat() {
    assert_eq!(
        eval(r#""hello" + " " + "world""#),
        Value::Str("hello world".into())
    );
}

#[test]
fn test_type_error_add() {
    let msg = eval_err(r#"1 + "hello""#);
    assert!(
        msg.contains("cannot apply"),
        "expected type error, got: {}",
        msg
    );
}

#[test]
fn test_division_by_zero() {
    let msg = eval_err("1 / 0");
    assert!(
        msg.contains("division by zero"),
        "expected div zero, got: {}",
        msg
    );
}

#[test]
fn test_unary_neg() {
    assert_eq!(eval("-5"), Value::Int(-5));
    assert_eq!(eval("-3.14"), Value::Float(-3.14));
}

#[test]
fn test_unary_not() {
    assert_eq!(eval("!true"), Value::Bool(false));
    assert_eq!(eval("!false"), Value::Bool(true));
}

// ============================================================
// Section 5: Comparison & Logical
// ============================================================

#[test]
fn test_comparison() {
    assert_eq!(eval("1 == 1"), Value::Bool(true));
    assert_eq!(eval("1 != 2"), Value::Bool(true));
    assert_eq!(eval("1 < 2"), Value::Bool(true));
    assert_eq!(eval("2 > 1"), Value::Bool(true));
    assert_eq!(eval("1 <= 1"), Value::Bool(true));
    assert_eq!(eval("2 >= 1"), Value::Bool(true));
}

#[test]
fn test_logical_and_or() {
    assert_eq!(eval("true && true"), Value::Bool(true));
    assert_eq!(eval("true && false"), Value::Bool(false));
    assert_eq!(eval("false || true"), Value::Bool(true));
    assert_eq!(eval("false || false"), Value::Bool(false));
}

#[test]
fn test_short_circuit() {
    // Should not error because of short-circuit
    assert_eq!(eval("false && (1 / 0 == 0)"), Value::Bool(false));
    assert_eq!(eval("true || (1 / 0 == 0)"), Value::Bool(true));
}

// ============================================================
// Section 6: Functions
// ============================================================

#[test]
fn test_fn_decl_and_call() {
    assert_eq!(eval("fn add(a, b) { a + b } add(3, 4)"), Value::Int(7));
}

#[test]
fn test_fn_return_last_expr() {
    assert_eq!(eval("fn double(x) { x * 2 } double(5)"), Value::Int(10));
}

#[test]
fn test_fn_explicit_return() {
    assert_eq!(
        eval(
            "
        fn f(x) {
            if x > 0 { return x; }
            0 - x
        }
        f(5)
    "
        ),
        Value::Int(5)
    );
    assert_eq!(
        eval(
            "
        fn f(x) {
            if x > 0 { return x; }
            0 - x
        }
        f(-5)
    "
        ),
        Value::Int(5)
    );
}

#[test]
fn test_fn_default_args() {
    assert_eq!(
        eval("fn greet(name, greeting = \"hello\") { greeting + \" \" + name } greet(\"world\")"),
        Value::Str("hello world".into())
    );
}

#[test]
fn test_lambda() {
    assert_eq!(eval("let f = |x| x * 2; f(5)"), Value::Int(10));
}

#[test]
fn test_lambda_multi_arg() {
    assert_eq!(eval("let f = |x, y| x + y; f(3, 4)"), Value::Int(7));
}

#[test]
fn test_closure_capture() {
    assert_eq!(eval("let x = 10; let f = |y| x + y; f(5)"), Value::Int(15));
}

#[test]
fn test_closure_capture_by_value() {
    // Closure captures at time of creation, not affected by later shadowing
    assert_eq!(
        eval("let x = 1; let f = |y| x + y; let x = 100; f(0)"),
        Value::Int(1)
    );
}

#[test]
fn test_higher_order_fn() {
    assert_eq!(
        eval("fn apply(f, x) { f(x) } apply(|x| x * 3, 5)"),
        Value::Int(15)
    );
}

#[test]
fn test_recursion() {
    assert_eq!(
        eval(
            "
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(6)
    "
        ),
        Value::Int(8)
    );
}

// ============================================================
// Section 7: Control Flow
// ============================================================

#[test]
fn test_if_else_expr() {
    assert_eq!(eval("if true { 1 } else { 2 }"), Value::Int(1));
    assert_eq!(eval("if false { 1 } else { 2 }"), Value::Int(2));
}

#[test]
fn test_if_as_value() {
    assert_eq!(
        eval("let x = if 1 > 0 { 10 } else { 20 }; x"),
        Value::Int(10)
    );
}

#[test]
fn test_else_if() {
    assert_eq!(
        eval(
            "
        let x = 5;
        if x > 10 { \"big\" } else if x > 3 { \"medium\" } else { \"small\" }
    "
        ),
        Value::Str("medium".into())
    );
}

#[test]
fn test_block_expr_returns_last() {
    assert_eq!(
        eval("let x = { let a = 1; let b = 2; a + b }; x"),
        Value::Int(3)
    );
}

#[test]
fn test_block_trailing_semi_returns_unit() {
    assert_eq!(eval("{ 42; }"), Value::Unit);
}

// ============================================================
// Section 8: Loops
// ============================================================

#[test]
fn test_for_loop() {
    assert_eq!(
        eval(
            "
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            sum = sum + x;
        }
        sum
    "
        ),
        Value::Int(15)
    );
}

#[test]
fn test_for_range() {
    assert_eq!(
        eval(
            "
        let mut sum = 0;
        for i in 0..5 {
            sum = sum + i;
        }
        sum
    "
        ),
        Value::Int(10)
    );
}

#[test]
fn test_while_loop() {
    assert_eq!(
        eval(
            "
        let mut i = 0;
        while i < 5 {
            i = i + 1;
        }
        i
    "
        ),
        Value::Int(5)
    );
}

#[test]
fn test_loop_break() {
    assert_eq!(
        eval(
            "
        let mut i = 0;
        loop {
            if i >= 3 { break; }
            i = i + 1;
        }
        i
    "
        ),
        Value::Int(3)
    );
}

#[test]
fn test_loop_break_value() {
    assert_eq!(
        eval(
            "
        let result = loop {
            break 42;
        };
        result
    "
        ),
        Value::Int(42)
    );
}

#[test]
fn test_for_continue() {
    assert_eq!(
        eval(
            "
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            if x == 3 { continue; }
            sum = sum + x;
        }
        sum
    "
        ),
        Value::Int(12)
    );
}

#[test]
fn test_compound_assignment() {
    assert_eq!(eval("let mut x = 10; x += 5; x"), Value::Int(15));
    assert_eq!(eval("let mut x = 10; x -= 3; x"), Value::Int(7));
    assert_eq!(eval("let mut x = 10; x *= 2; x"), Value::Int(20));
    assert_eq!(eval("let mut x = 10; x /= 5; x"), Value::Int(2));
}

#[test]
fn test_compound_assign_immutable_error() {
    let msg = eval_err("let x = 10; x += 1;");
    assert!(
        msg.contains("immutable"),
        "expected immutable error, got: {}",
        msg
    );
}

// ============================================================
// Section 9: Match
// ============================================================

#[test]
fn test_match_int() {
    assert_eq!(
        eval(r#"match 2 { 1 => "one", 2 => "two", _ => "other" }"#),
        Value::Str("two".into())
    );
}

#[test]
fn test_match_wildcard() {
    assert_eq!(
        eval(r#"match 99 { 1 => "one", _ => "other" }"#),
        Value::Str("other".into())
    );
}

#[test]
fn test_match_with_guard() {
    assert_eq!(
        eval(
            r#"
        let score = 85;
        match score {
            s if s >= 90 => "A",
            s if s >= 80 => "B",
            _ => "C",
        }
    "#
        ),
        Value::Str("B".into())
    );
}

#[test]
fn test_match_option() {
    assert_eq!(
        eval(
            r#"
        let x = Some(42);
        match x {
            Some(v) => v,
            None => 0,
        }
    "#
        ),
        Value::Int(42)
    );
}

#[test]
fn test_match_result() {
    assert_eq!(
        eval(
            r#"
        let x = Ok(10);
        match x {
            Ok(v) => v * 2,
            Err(e) => 0,
        }
    "#
        ),
        Value::Int(20)
    );
}

#[test]
fn test_match_nested() {
    assert_eq!(
        eval(
            r#"
        let x = Ok(Some(5));
        match x {
            Ok(Some(v)) => v,
            Ok(None) => 0,
            Err(e) => -1,
        }
    "#
        ),
        Value::Int(5)
    );
}

#[test]
fn test_non_exhaustive_match_error() {
    let msg = eval_err("match 5 { 1 => 10 }");
    assert!(
        msg.contains("non-exhaustive"),
        "expected non-exhaustive, got: {}",
        msg
    );
}

// ============================================================
// Section 10: Option & Result
// ============================================================

#[test]
fn test_some_none() {
    assert_eq!(
        eval("Some(42)"),
        Value::Option(Some(Box::new(Value::Int(42))))
    );
    assert_eq!(eval("None"), Value::Option(None));
}

#[test]
fn test_ok_err() {
    assert_eq!(eval("Ok(1)"), Value::Result(Ok(Box::new(Value::Int(1)))));
}

#[test]
fn test_question_mark_ok() {
    assert_eq!(eval("fn f() { let x = Ok(42); x? } f()"), Value::Int(42));
}

#[test]
fn test_question_mark_err_propagation() {
    // ? on Err inside a function returns Result(Err) at function boundary
    let result = Engine::new()
        .eval(
            r#"
        fn inner() { let x = Err("fail"); x? }
        inner()
    "#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::Result(Err(Box::new(Value::Str("fail".to_string()))))
    );
}

#[test]
fn test_question_mark_some() {
    assert_eq!(eval("fn f() { let x = Some(10); x? } f()"), Value::Int(10));
}

#[test]
fn test_question_mark_none_propagation() {
    // ? on None inside a function returns Option(None) at function boundary
    let result = Engine::new()
        .eval("fn f() { let x = None; x? } f()")
        .unwrap();
    assert_eq!(result, Value::Option(None));
}

#[test]
fn test_question_mark_type_error() {
    let result = Engine::new().eval("fn f() { let x = 42; x? } f()");
    assert!(result.is_err());
}

#[test]
fn test_question_mark_top_level_err() {
    // ? at top-level returns Result(Err) as value instead of runtime error
    let result = Engine::new().eval(r#"let x = Err("oops"); x?"#).unwrap();
    assert_eq!(
        result,
        Value::Result(Err(Box::new(Value::Str("oops".to_string()))))
    );
}

#[test]
fn test_question_mark_top_level_none() {
    // ? at top-level returns Option(None) as value instead of runtime error
    let result = Engine::new().eval("let x = None; x?").unwrap();
    assert_eq!(result, Value::Option(None));
}

#[test]
fn test_question_mark_top_level_ok() {
    // ? at top-level on Ok unwraps successfully
    assert_eq!(eval("let x = Ok(42); x?"), Value::Int(42));
}

#[test]
fn test_question_mark_top_level_some() {
    // ? at top-level on Some unwraps successfully
    assert_eq!(eval("let x = Some(10); x?"), Value::Int(10));
}

#[test]
fn test_unwrap_or() {
    assert_eq!(eval("Some(5).unwrap_or(0)"), Value::Int(5));
    assert_eq!(eval("None.unwrap_or(0)"), Value::Int(0));
    assert_eq!(eval("Ok(5).unwrap_or(0)"), Value::Int(5));
    assert_eq!(eval(r#"Err("fail").unwrap_or(0)"#), Value::Int(0));
}

#[test]
fn test_expect_some() {
    assert_eq!(eval(r#"Some(5).expect("should exist")"#), Value::Int(5));
}

#[test]
fn test_expect_none_error() {
    let msg = eval_err(r#"None.expect("value missing")"#);
    assert!(
        msg.contains("value missing"),
        "expected expect msg, got: {}",
        msg
    );
}

#[test]
fn test_is_some_is_none() {
    assert_eq!(eval("Some(1).is_some()"), Value::Bool(true));
    assert_eq!(eval("Some(1).is_none()"), Value::Bool(false));
    assert_eq!(eval("None.is_some()"), Value::Bool(false));
    assert_eq!(eval("None.is_none()"), Value::Bool(true));
}

#[test]
fn test_is_ok_is_err() {
    assert_eq!(eval("Ok(1).is_ok()"), Value::Bool(true));
    assert_eq!(eval("Ok(1).is_err()"), Value::Bool(false));
    assert_eq!(eval(r#"Err("x").is_ok()"#), Value::Bool(false));
    assert_eq!(eval(r#"Err("x").is_err()"#), Value::Bool(true));
}

// ============================================================
// Section 11: If-let / While-let
// ============================================================

#[test]
fn test_if_let() {
    assert_eq!(
        eval(
            "
        let x = Some(42);
        if let Some(v) = x { v } else { 0 }
    "
        ),
        Value::Int(42)
    );
}

#[test]
fn test_if_let_no_match() {
    assert_eq!(
        eval(
            "
        let x = None;
        if let Some(v) = x { v } else { 0 }
    "
        ),
        Value::Int(0)
    );
}

#[test]
fn test_while_let() {
    // Simulates popping from a list
    assert_eq!(
        eval(
            "
        let mut items = [1, 2, 3];
        let mut sum = 0;
        while let [first, ...rest] = items {
            sum = sum + first;
            items = rest;
        }
        sum
    "
        ),
        Value::Int(6)
    );
}

// ============================================================
// Section 12: Lists
// ============================================================

#[test]
fn test_list_literal() {
    assert_eq!(
        eval("[1, 2, 3]"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_map() {
    assert_eq!(
        eval("[1, 2, 3].map(|x| x * 2)"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
    );
}

#[test]
fn test_list_filter() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5].filter(|x| x > 3)"),
        Value::List(vec![Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn test_list_fold() {
    assert_eq!(
        eval("[1, 2, 3, 4].fold(0, |acc, x| acc + x)"),
        Value::Int(10)
    );
}

#[test]
fn test_list_push_returns_new() {
    assert_eq!(
        eval(
            "
        let a = [1, 2];
        let b = a.push(3);
        a
    "
        ),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn test_list_push_result() {
    assert_eq!(
        eval("[1, 2].push(3)"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_len() {
    assert_eq!(eval("[1, 2, 3].len()"), Value::Int(3));
}

#[test]
fn test_list_first_last() {
    assert_eq!(
        eval("[10, 20, 30].first()"),
        Value::Option(Some(Box::new(Value::Int(10))))
    );
    assert_eq!(
        eval("[10, 20, 30].last()"),
        Value::Option(Some(Box::new(Value::Int(30))))
    );
    assert_eq!(eval("[].first()"), Value::Option(None));
}

#[test]
fn test_list_any_all() {
    assert_eq!(eval("[1, 2, 3].any(|x| x > 2)"), Value::Bool(true));
    assert_eq!(eval("[1, 2, 3].any(|x| x > 5)"), Value::Bool(false));
    assert_eq!(eval("[1, 2, 3].all(|x| x > 0)"), Value::Bool(true));
    assert_eq!(eval("[1, 2, 3].all(|x| x > 1)"), Value::Bool(false));
}

#[test]
fn test_list_reverse() {
    assert_eq!(
        eval("[1, 2, 3].reverse()"),
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn test_list_sort() {
    assert_eq!(
        eval("[3, 1, 2].sort()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_contains() {
    assert_eq!(eval("[1, 2, 3].contains(2)"), Value::Bool(true));
    assert_eq!(eval("[1, 2, 3].contains(5)"), Value::Bool(false));
}

#[test]
fn test_list_flatten() {
    assert_eq!(
        eval("[[1, 2], [3, 4]].flatten()"),
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4)
        ])
    );
}

#[test]
fn test_list_zip() {
    assert_eq!(
        eval("[1, 2].zip([3, 4])"),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(1), Value::Int(3)]),
            Value::Tuple(vec![Value::Int(2), Value::Int(4)]),
        ])
    );
}

// ============================================================
// Section 13: Dicts
// ============================================================

#[test]
fn test_dict_literal() {
    let val = eval(r#"#{ "a": 1, "b": 2 }"#);
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(2));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_bracket_access() {
    assert_eq!(eval(r#"let d = #{ "x": 42 }; d["x"]"#), Value::Int(42));
}

#[test]
fn test_dict_missing_key() {
    assert_eq!(eval(r#"let d = #{ "x": 1 }; d["y"]"#), Value::Option(None));
}

#[test]
fn test_dict_methods() {
    assert_eq!(eval(r#"#{ "a": 1, "b": 2 }.len()"#), Value::Int(2));
    assert_eq!(eval(r#"#{ "a": 1 }.contains_key("a")"#), Value::Bool(true));
    assert_eq!(eval(r#"#{ "a": 1 }.contains_key("b")"#), Value::Bool(false));
}

#[test]
fn test_dict_keys_values() {
    assert_eq!(
        eval(r#"#{ "a": 1, "b": 2 }.keys()"#),
        Value::List(vec![Value::Str("a".into()), Value::Str("b".into())])
    );
    assert_eq!(
        eval(r#"#{ "a": 1, "b": 2 }.values()"#),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn test_dict_insert_returns_new() {
    let val = eval(
        r#"
        let d = #{ "a": 1 };
        d.insert("b", 2)
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["b"], Value::Int(2));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_remove_returns_new() {
    let val = eval(
        r#"
        let d = #{ "a": 1, "b": 2 };
        d.remove("a")
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key("a"));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_merge() {
    let val = eval(
        r#"
        let a = #{ "x": 1 };
        let b = #{ "y": 2 };
        a.merge(b)
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["x"], Value::Int(1));
        assert_eq!(map["y"], Value::Int(2));
    } else {
        panic!("expected dict");
    }
}

// ============================================================
// Section 14: Tuples
// ============================================================

#[test]
fn test_tuple_literal() {
    assert_eq!(
        eval("(1, 2, 3)"),
        Value::Tuple(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_tuple_destructuring() {
    assert_eq!(eval("let (a, b) = (10, 20); a + b"), Value::Int(30));
}

// ============================================================
// Section 15: String Interpolation
// ============================================================

#[test]
fn test_fstring() {
    assert_eq!(
        eval(r#"let name = "world"; f"hello {name}""#),
        Value::Str("hello world".into())
    );
}

#[test]
fn test_fstring_expr() {
    assert_eq!(
        eval(r#"f"result = {1 + 2}""#),
        Value::Str("result = 3".into())
    );
}

#[test]
fn test_fstring_rejects_trailing_tokens_in_expr() {
    let msg = eval_err(r#"f"{1 2}""#);
    assert!(
        msg.contains("unexpected token in f-string expression"),
        "got: {}",
        msg
    );
}

#[test]
fn test_fstring_nested_quotes() {
    // Nested double quotes inside f-string interpolation
    assert_eq!(
        eval(r#"fn greet(name) { f"hi {name}" } f"says: {greet("world")}""#),
        Value::Str("says: hi world".into())
    );
    // Method call with string arg inside f-string
    assert_eq!(
        eval(r#"f"result: {"hello world".replace("world", "ion")}""#),
        Value::Str("result: hello ion".into())
    );
}

#[test]
fn test_regular_string_no_interp() {
    assert_eq!(eval(r#""hello {name}""#), Value::Str("hello {name}".into()));
}

// ============================================================
// Section 16: Ranges
// ============================================================

#[test]
fn test_range_exclusive() {
    assert_eq!(
        eval("0..3"),
        Value::Range {
            start: 0,
            end: 3,
            inclusive: false,
        }
    );
}

#[test]
fn test_range_inclusive() {
    assert_eq!(
        eval("0..=3"),
        Value::Range {
            start: 0,
            end: 3,
            inclusive: true,
        }
    );
}

// ============================================================
// Section 17: Pipe Operator
// ============================================================

#[test]
fn test_pipe_basic() {
    assert_eq!(
        eval(
            "
        fn double(x) { x * 2 }
        5 |> double()
    "
        ),
        Value::Int(10)
    );
}

#[test]
fn test_pipe_chain() {
    assert_eq!(
        eval(
            "
        fn add(x, y) { x + y }
        fn double(x) { x * 2 }
        5 |> add(3) |> double()
    "
        ),
        Value::Int(16)
    );
}

#[test]
fn test_pipe_bare_fn() {
    assert_eq!(
        eval(
            "
        fn double(x) { x * 2 }
        5 |> double
    "
        ),
        Value::Int(10)
    );
}

// ============================================================
// Section 18: Builtins
// ============================================================

#[test]
fn test_len() {
    assert_eq!(eval("len([1, 2, 3])"), Value::Int(3));
    assert_eq!(eval(r#"len("hello")"#), Value::Int(5));
}

#[test]
fn test_range_fn() {
    assert_eq!(
        eval("range(3)"),
        Value::Range {
            start: 0,
            end: 3,
            inclusive: false,
        }
    );
    assert_eq!(
        eval("range(2, 5)"),
        Value::Range {
            start: 2,
            end: 5,
            inclusive: false,
        }
    );
}

#[test]
fn test_type_of() {
    assert_eq!(eval("type_of(42)"), Value::Str("int".into()));
    assert_eq!(eval(r#"type_of("hello")"#), Value::Str("string".into()));
    assert_eq!(eval("type_of([1, 2])"), Value::Str("list".into()));
}

// ============================================================
// Section 19: String Methods
// ============================================================

#[test]
fn test_string_methods() {
    assert_eq!(eval(r#""hello".len()"#), Value::Int(5));
    assert_eq!(
        eval(r#""hello world".contains("world")"#),
        Value::Bool(true)
    );
    assert_eq!(eval(r#""hello".starts_with("hel")"#), Value::Bool(true));
    assert_eq!(eval(r#""hello".ends_with("llo")"#), Value::Bool(true));
    assert_eq!(eval(r#""  hello  ".trim()"#), Value::Str("hello".into()));
    assert_eq!(eval(r#""hello".to_upper()"#), Value::Str("HELLO".into()));
    assert_eq!(eval(r#""HELLO".to_lower()"#), Value::Str("hello".into()));
}

#[test]
fn test_string_split() {
    assert_eq!(
        eval(r#""a,b,c".split(",")"#),
        Value::List(vec![
            Value::Str("a".into()),
            Value::Str("b".into()),
            Value::Str("c".into())
        ])
    );
}

#[test]
fn test_string_replace() {
    assert_eq!(
        eval(r#""hello world".replace("world", "ion")"#),
        Value::Str("hello ion".into())
    );
}

#[test]
fn test_string_trim_variants() {
    assert_eq!(
        eval(r#""  hello  ".trim_start()"#),
        Value::Str("hello  ".into())
    );
    assert_eq!(
        eval(r#""  hello  ".trim_end()"#),
        Value::Str("  hello".into())
    );
}

#[test]
fn test_string_repeat() {
    assert_eq!(eval(r#""ab".repeat(3)"#), Value::Str("ababab".into()));
    assert_eq!(eval(r#""x".repeat(0)"#), Value::Str("".into()));
}

#[test]
fn test_string_find() {
    assert_eq!(
        eval(r#""hello world".find("world")"#),
        Value::Option(Some(Box::new(Value::Int(6))))
    );
    assert_eq!(eval(r#""hello".find("xyz")"#), Value::Option(None));
}

#[test]
fn test_string_to_int() {
    assert_eq!(
        eval(r#""42".to_int()"#),
        Value::Result(Ok(Box::new(Value::Int(42))))
    );
    assert_eq!(
        eval(r#"" -7 ".to_int()"#),
        Value::Result(Ok(Box::new(Value::Int(-7))))
    );
}

#[test]
fn test_string_to_float() {
    assert_eq!(
        eval(r#""3.14".to_float()"#),
        Value::Result(Ok(Box::new(Value::Float(3.14))))
    );
}

#[test]
fn test_string_reverse() {
    assert_eq!(eval(r#""hello".reverse()"#), Value::Str("olleh".into()));
}

// ============================================================
// Section 20: Engine API
// ============================================================

#[test]
fn test_engine_set_get() {
    let mut engine = Engine::new();
    engine.set("x", Value::Int(42));
    let result = engine.eval("x + 8").unwrap();
    assert_eq!(result, Value::Int(50));
}

#[test]
fn test_engine_get_variable() {
    let mut engine = Engine::new();
    engine.eval("let result = 1 + 2;").unwrap();
    assert_eq!(engine.get("result"), Some(Value::Int(3)));
}

#[test]
fn test_engine_last_expr_return() {
    let mut engine = Engine::new();
    let result = engine.eval("let x = 10; x * 2").unwrap();
    assert_eq!(result, Value::Int(20));
}

#[test]
fn test_engine_register_fn() {
    let mut engine = Engine::new();
    engine.register_fn(ion_core::h!("square"), |args: &[Value]| match &args[0] {
        Value::Int(n) => Ok(Value::Int(n * n)),
        _ => Err("expected int".to_string()),
    });
    assert_eq!(engine.eval("square(5)").unwrap(), Value::Int(25));
}

// ============================================================
// Section 21: Complex Programs
// ============================================================

#[test]
fn test_fibonacci_functional() {
    assert_eq!(
        eval(
            "
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        [0, 1, 2, 3, 4, 5, 6].map(|n| fib(n))
    "
        ),
        Value::List(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(5),
            Value::Int(8),
        ])
    );
}

#[test]
fn test_dict_pipeline() {
    let val = eval(
        r#"
        let data = #{ "name": "Alice", "age": 30 };
        let updated = data.insert("role", "admin");
        updated.get("role")
    "#,
    );
    assert_eq!(
        val,
        Value::Option(Some(Box::new(Value::Str("admin".into()))))
    );
}

#[test]
fn test_error_propagation_chain() {
    // Chain of ? operators
    let result = Engine::new()
        .eval(
            r#"
        fn parse(input) {
            if input == "bad" {
                Err("parse error")
            } else {
                Ok(42)
            }
        }
        fn process(input) {
            let val = parse(input)?;
            Ok(val * 2)
        }
        process("good")
    "#,
        )
        .unwrap();
    assert_eq!(result, Value::Result(Ok(Box::new(Value::Int(84)))));
}

#[test]
fn test_error_propagation_failure() {
    // ? propagation across nested function boundaries returns Result(Err) value
    let result = Engine::new()
        .eval(
            r#"
        fn parse(input) {
            if input == "bad" {
                Err("parse error")
            } else {
                Ok(42)
            }
        }
        fn process(input) {
            let val = parse(input)?;
            Ok(val * 2)
        }
        process("bad")
    "#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::Result(Err(Box::new(Value::Str("parse error".to_string()))))
    );
}

#[test]
fn test_question_mark_success_path() {
    // ? on Ok/Some inside a function unwraps and continues
    assert_eq!(
        eval(
            r#"
        fn process(input) {
            let val = Ok(input)?;
            val * 2
        }
        process(21)
    "#
        ),
        Value::Int(42)
    );
}

#[test]
fn test_question_mark_option_propagation_across_fns() {
    // ? on None propagates across function boundary as Option(None)
    let result = Engine::new()
        .eval(
            r#"
        fn get_first(items) {
            let v = items.first()?;
            Some(v * 10)
        }
        get_first([])
    "#,
        )
        .unwrap();
    assert_eq!(result, Value::Option(None));

    // ? on Some succeeds
    let result2 = Engine::new()
        .eval(
            r#"
        fn get_first(items) {
            let v = items.first()?;
            Some(v * 10)
        }
        get_first([5, 6, 7])
    "#,
        )
        .unwrap();
    assert_eq!(result2, Value::Option(Some(Box::new(Value::Int(50)))));
}

#[test]
fn test_nested_closures() {
    assert_eq!(
        eval(
            "
        fn make_adder(x) {
            |y| x + y
        }
        let add5 = make_adder(5);
        let add10 = make_adder(10);
        add5(3) + add10(3)
    "
        ),
        Value::Int(21)
    );
}

#[test]
fn test_for_dict_iteration() {
    assert_eq!(
        eval(
            r#"
        let mut sum = 0;
        for (key, val) in #{ "a": 1, "b": 2, "c": 3 } {
            sum = sum + val;
        }
        sum
    "#
        ),
        Value::Int(6)
    );
}

#[test]
fn test_list_of_dicts() {
    let val = eval(
        r#"
        let people = [
            #{ "name": "Alice", "age": 30 },
            #{ "name": "Bob", "age": 25 },
        ];
        people.map(|p| p["name"])
    "#,
    );
    assert_eq!(
        val,
        Value::List(vec![Value::Str("Alice".into()), Value::Str("Bob".into()),])
    );
}

// ============================================================
// Section 22: List Comprehensions
// ============================================================

#[test]
fn test_list_comp_basic() {
    assert_eq!(
        eval("[x * 2 for x in [1, 2, 3]]"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
    );
}

#[test]
fn test_list_comp_with_filter() {
    assert_eq!(
        eval("[x for x in [1, 2, 3, 4, 5] if x > 3]"),
        Value::List(vec![Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn test_list_comp_with_transform_and_filter() {
    assert_eq!(
        eval("[x * x for x in [1, 2, 3, 4, 5] if x % 2 == 0]"),
        Value::List(vec![Value::Int(4), Value::Int(16)])
    );
}

#[test]
fn test_list_comp_tuple_pattern() {
    assert_eq!(
        eval(
            r#"
        let pairs = [(1, "a"), (2, "b"), (3, "c")];
        [n for (n, _s) in pairs if n > 1]
    "#
        ),
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_comp_over_range() {
    assert_eq!(
        eval("[x * x for x in range(5)]"),
        Value::List(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(4),
            Value::Int(9),
            Value::Int(16)
        ])
    );
}

// ============================================================
// Section 23: Dict Comprehensions
// ============================================================

#[test]
fn test_dict_comp_basic() {
    let val = eval(r#"#{ f"key_{x}": x * 10 for x in [1, 2, 3] }"#);
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 3);
        assert_eq!(map["key_1"], Value::Int(10));
        assert_eq!(map["key_2"], Value::Int(20));
        assert_eq!(map["key_3"], Value::Int(30));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_comp_with_filter() {
    let val = eval(r#"#{ f"n_{x}": x for x in [1, 2, 3, 4] if x % 2 == 0 }"#);
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["n_2"], Value::Int(2));
        assert_eq!(map["n_4"], Value::Int(4));
    } else {
        panic!("expected dict");
    }
}

// ============================================================
// Section 24: Dict Spread
// ============================================================

#[test]
fn test_dict_spread_basic() {
    let val = eval(
        r#"
        let base = #{ "a": 1, "b": 2 };
        #{ ...base, "c": 3 }
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 3);
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(2));
        assert_eq!(map["c"], Value::Int(3));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_spread_override() {
    let val = eval(
        r#"
        let base = #{ "a": 1, "b": 2 };
        #{ ...base, "b": 99 }
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(99));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_spread_multiple() {
    let val = eval(
        r#"
        let a = #{ "x": 1 };
        let b = #{ "y": 2 };
        #{ ...a, ...b, "z": 3 }
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 3);
        assert_eq!(map["x"], Value::Int(1));
        assert_eq!(map["y"], Value::Int(2));
        assert_eq!(map["z"], Value::Int(3));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_spread_non_dict_error() {
    let err = eval_err(r#"#{ ...[1, 2, 3] }"#);
    assert!(err.contains("spread requires a dict"), "got: {}", err);
}

// ============================================================
// Section 25: JSON Encode / Decode
// ============================================================

#[test]
fn test_json_encode_int() {
    assert_eq!(eval("json::encode(42)"), Value::Str("42".into()));
}

#[test]
fn test_json_encode_dict() {
    let val = eval(r#"json::encode(#{ "name": "Ion", "version": 1 })"#);
    if let Value::Str(s) = val {
        assert!(s.contains("\"name\""));
        assert!(s.contains("\"Ion\""));
        assert!(s.contains("\"version\""));
    } else {
        panic!("expected string");
    }
}

#[test]
fn test_json_encode_list() {
    assert_eq!(
        eval("json::encode([1, 2, 3])"),
        Value::Str("[1,2,3]".into())
    );
}

#[test]
fn test_json_decode_object() {
    let val = eval(r#"json::decode("{\"a\": 1, \"b\": 2}")"#);
    if let Value::Dict(map) = val {
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(2));
    } else {
        panic!("expected dict, got: {:?}", val);
    }
}

#[test]
fn test_json_decode_array() {
    assert_eq!(
        eval(r#"json::decode("[1, 2, 3]")"#),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_json_roundtrip() {
    let val = eval(
        r#"
        let data = #{ "name": "test", "values": [1, 2, 3] };
        let encoded = json::encode(data);
        json::decode(encoded)
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map["name"], Value::Str("test".into()));
        assert_eq!(
            map["values"],
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_json_decode_invalid() {
    // Phase 7 cleanup: error literals no longer include the `mod::fn`
    // prefix; the runtime span carries the call-site identification.
    let err = eval_err(r#"json::decode("not json")"#);
    assert!(err.contains("error:"), "got: {}", err);
}

// ============================================================
// Section 26: Execution Limits (Sandboxing)
// ============================================================

#[test]
fn test_max_call_depth() {
    let mut engine = Engine::new();
    engine.set_limits(Limits {
        max_call_depth: 10,
        max_loop_iters: 1_000_000,
    });
    let err = engine
        .eval(
            "
        fn recurse(n) { recurse(n + 1) }
        recurse(0)
    ",
        )
        .unwrap_err();
    assert!(
        err.message.contains("maximum call depth"),
        "got: {}",
        err.message
    );
}

#[test]
fn test_max_loop_iters() {
    let mut engine = Engine::new();
    engine.set_limits(Limits {
        max_call_depth: 512,
        max_loop_iters: 100,
    });
    let err = engine
        .eval(
            "
        let mut i = 0;
        while true { i = i + 1; }
    ",
        )
        .unwrap_err();
    assert!(
        err.message.contains("maximum loop iterations"),
        "got: {}",
        err.message
    );
}

#[test]
fn test_loop_within_limit() {
    let mut engine = Engine::new();
    engine.set_limits(Limits {
        max_call_depth: 512,
        max_loop_iters: 100,
    });
    let result = engine
        .eval(
            "
        let mut sum = 0;
        let mut i = 0;
        while i < 50 { sum = sum + i; i = i + 1; }
        sum
    ",
        )
        .unwrap();
    assert_eq!(result, Value::Int(1225));
}

// ============================================================
// Section 27: Stdlib Builtins
// ============================================================

#[test]
fn test_abs() {
    assert_eq!(eval("math::abs(-5)"), Value::Int(5));
    assert_eq!(eval("math::abs(5)"), Value::Int(5));
    assert_eq!(eval("math::abs(-3.14)"), Value::Float(3.14));
}

#[test]
fn test_min_max() {
    assert_eq!(eval("math::min(3, 1, 2)"), Value::Int(1));
    assert_eq!(eval("math::max(3, 1, 2)"), Value::Int(3));
    assert_eq!(eval("math::min(1.5, 2.5)"), Value::Float(1.5));
    assert_eq!(eval("math::max(1, 2.5)"), Value::Float(2.5));
}

#[test]
fn test_str_conversion() {
    assert_eq!(eval("str(42)"), Value::Str("42".into()));
    assert_eq!(eval("str(true)"), Value::Str("true".into()));
    assert_eq!(eval("str([1, 2])"), Value::Str("[1, 2]".into()));
}

#[test]
fn test_int_conversion() {
    assert_eq!(eval("int(3.7)"), Value::Int(3));
    assert_eq!(eval(r#"int("42")"#), Value::Int(42));
    assert_eq!(eval("int(true)"), Value::Int(1));
    assert_eq!(eval("int(false)"), Value::Int(0));
}

#[test]
fn test_float_conversion() {
    assert_eq!(eval("float(42)"), Value::Float(42.0));
    assert_eq!(eval(r#"float("3.14")"#), Value::Float(3.14));
}

#[test]
fn test_int_parse_error() {
    let err = eval_err(r#"int("abc")"#);
    assert!(err.contains("cannot convert"), "got: {}", err);
}

// ============================================================
// Section 28: Host Types — Structs
// ============================================================

fn engine_with_types() -> Engine {
    let mut engine = Engine::new();
    engine.register_struct(HostStructDef {
        name_hash: h!("Config"),
        fields: vec![h!("host"), h!("port"), h!("debug")],
    });
    engine.register_enum(HostEnumDef {
        name_hash: h!("Color"),
        variants: vec![
            HostVariantDef {
                name_hash: h!("Red"),
                arity: 0,
            },
            HostVariantDef {
                name_hash: h!("Green"),
                arity: 0,
            },
            HostVariantDef {
                name_hash: h!("Blue"),
                arity: 0,
            },
            HostVariantDef {
                name_hash: h!("Custom"),
                arity: 3,
            },
        ],
    });
    engine
}

#[test]
fn test_host_struct_construct() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(r#"Config { host: "localhost", port: 8080, debug: true }"#)
        .unwrap();
    if let Value::HostStruct { type_hash, fields } = &val {
        assert_eq!(*type_hash, h!("Config"));
        assert_eq!(fields[&h!("host")], Value::Str("localhost".into()));
        assert_eq!(fields[&h!("port")], Value::Int(8080));
        assert_eq!(fields[&h!("debug")], Value::Bool(true));
    } else {
        panic!("expected HostStruct, got: {:?}", val);
    }
}

#[test]
fn test_host_struct_field_access() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let cfg = Config { host: "localhost", port: 8080, debug: false };
        cfg.host
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Str("localhost".into()));
}

#[test]
fn test_host_struct_missing_field_error() {
    let mut engine = engine_with_types();
    let err = engine.eval(r#"Config { host: "localhost" }"#).unwrap_err();
    assert!(
        err.message.contains("missing field"),
        "got: {}",
        err.message
    );
}

#[test]
fn test_host_struct_unknown_field_error() {
    let mut engine = engine_with_types();
    let err = engine
        .eval(r#"Config { host: "x", port: 80, debug: true, extra: 1 }"#)
        .unwrap_err();
    assert!(
        err.message.contains("unknown field"),
        "got: {}",
        err.message
    );
}

#[test]
fn test_host_struct_pattern_match() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let cfg = Config { host: "localhost", port: 8080, debug: true };
        match cfg {
            Config { host, port } => f"{host}:{port}",
        }
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Str("localhost:8080".into()));
}

#[test]
fn test_host_struct_spread() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let base = Config { host: "localhost", port: 8080, debug: false };
        let updated = Config { ...base, debug: true };
        updated.debug
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Bool(true));
}

// ============================================================
// Section 29: Host Types — Enums
// ============================================================

#[test]
fn test_host_enum_unit_variant() {
    let mut engine = engine_with_types();
    let val = engine.eval("Color::Red").unwrap();
    assert_eq!(
        val,
        Value::HostEnum {
            enum_hash: h!("Color"),
            variant_hash: h!("Red"),
            data: vec![],
        }
    );
}

#[test]
fn test_host_enum_data_variant() {
    let mut engine = engine_with_types();
    let val = engine.eval("Color::Custom(255, 128, 0)").unwrap();
    assert_eq!(
        val,
        Value::HostEnum {
            enum_hash: h!("Color"),
            variant_hash: h!("Custom"),
            data: vec![Value::Int(255), Value::Int(128), Value::Int(0)],
        }
    );
}

#[test]
fn test_host_enum_unknown_variant_error() {
    let mut engine = engine_with_types();
    let err = engine.eval("Color::Yellow").unwrap_err();
    assert!(
        err.message.contains("unknown variant"),
        "got: {}",
        err.message
    );
}

#[test]
fn test_host_enum_wrong_arity_error() {
    let mut engine = engine_with_types();
    let err = engine.eval("Color::Custom(255)").unwrap_err();
    assert!(err.message.contains("expects 3"), "got: {}", err.message);
}

#[test]
fn test_host_enum_pattern_match() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let c = Color::Custom(255, 128, 0);
        match c {
            Color::Red => "red",
            Color::Custom(r, g, b) => f"rgb({r},{g},{b})",
            _ => "other",
        }
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Str("rgb(255,128,0)".into()));
}

#[test]
fn test_host_enum_match_unit_variant() {
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let c = Color::Green;
        match c {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue => "blue",
            _ => "other",
        }
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Str("green".into()));
}

#[test]
fn test_host_struct_display() {
    // Phase 7: in debug builds the `h!()` sites auto-register names, so
    // Display renders readably. In release without a sidecar, the same
    // value would print `<struct#hhhh> { #fffff: ... }`. Tests run in
    // debug, so we assert the readable form.
    let mut engine = engine_with_types();
    let val = engine
        .eval(
            r#"
        let cfg = Config { host: "localhost", port: 8080, debug: true };
        f"{cfg}"
    "#,
        )
        .unwrap();
    if let Value::Str(s) = &val {
        assert!(s.contains("Config"), "got: {}", s);
        assert!(s.contains("localhost"), "got: {}", s);
    } else {
        panic!("expected string");
    }
}

#[test]
fn test_host_enum_display() {
    // Phase 7: debug-mode auto-registration restores `Color::Red`.
    let mut engine = engine_with_types();
    let val = engine.eval(r#"f"{Color::Red}""#).unwrap();
    assert_eq!(val, Value::Str("Color::Red".into()));
}

#[test]
fn test_unregistered_type_error() {
    let mut engine = Engine::new();
    let err = engine.eval(r#"Unknown { field: 1 }"#).unwrap_err();
    assert!(err.message.contains("unknown type"), "got: {}", err.message);
}

/// Regression test for an earlier audit finding: `TypeRegistry`'s collision
/// detection was a tautological `assert_eq!` of two values that were equal
/// by HashMap construction. Re-registering the same shape must succeed
/// silently (idempotent) but a colliding definition must panic.
#[test]
fn test_register_same_struct_twice_is_idempotent() {
    let mut engine = Engine::new();
    engine.register_struct(HostStructDef {
        name_hash: h!("Idem"),
        fields: vec![h!("a"), h!("b")],
    });
    // Same shape, different cloned definition — must not panic.
    engine.register_struct(HostStructDef {
        name_hash: h!("Idem"),
        fields: vec![h!("a"), h!("b")],
    });
    let val = engine.eval(r#"Idem { a: 1, b: 2 }"#).unwrap();
    assert!(matches!(val, Value::HostStruct { .. }));
}

#[test]
#[should_panic(expected = "registered shape differs")]
fn test_register_struct_collision_panics() {
    let mut engine = Engine::new();
    engine.register_struct(HostStructDef {
        name_hash: h!("Collide"),
        fields: vec![h!("x")],
    });
    // Same name_hash, different field set — collision must panic.
    engine.register_struct(HostStructDef {
        name_hash: h!("Collide"),
        fields: vec![h!("y"), h!("z")],
    });
}

/// Phase 7 audit fix: derived types must auto-register their identifier
/// names with `ion_core::names` in debug builds, even when the host
/// never calls `h!()` directly with the same literal. Without the fix,
/// Display'd derived values rendered the opaque hash form even under
/// `cargo test`.
#[test]
fn test_derive_auto_registers_names_in_debug_builds() {
    use ion_core::host_types::IonType;
    // First touch via to_ion — drives the derive's debug-only Once.
    let p = Point { x: 1.0, y: 2.0 };
    let _ = p.to_ion();
    // Names should now be in the registry (no h!("Point") in this test).
    assert_eq!(
        ion_core::names::lookup(ion_core::hash::h("Point")),
        Some("Point"),
    );
    assert_eq!(ion_core::names::lookup(ion_core::hash::h("x")), Some("x"),);

    // Same for an enum touched via to_ion.
    let _ = Shape::Circle(1.0).to_ion();
    assert_eq!(
        ion_core::names::lookup(ion_core::hash::h("Shape")),
        Some("Shape"),
    );
    assert_eq!(
        ion_core::names::lookup(ion_core::hash::h("Circle")),
        Some("Circle"),
    );
}

#[cfg(debug_assertions)]
#[test]
fn test_module_builtin_qualified_names_register_in_debug_builds() {
    let mut module = Module::new(h!("audit_mod"));
    module.register_fn(h!("audit_fn"), |_args| Ok(Value::Unit));

    let Value::Module(table) = module.into_value() else {
        panic!("expected module value");
    };
    let builtin = table.items.get(&h!("audit_fn")).unwrap();
    let qualified_hash = ion_core::hash::mix(h!("audit_mod"), h!("audit_fn"));

    assert_eq!(
        ion_core::names::lookup(qualified_hash),
        Some("audit_mod::audit_fn")
    );
    assert_eq!(format!("{}", builtin), "<builtin audit_mod::audit_fn>");
}

// ============================================================
// Section 30: Extended Stdlib
// ============================================================

#[test]
fn test_floor_ceil_round() {
    assert_eq!(eval("math::floor(3.7)"), Value::Float(3.0));
    assert_eq!(eval("math::ceil(3.2)"), Value::Float(4.0));
    assert_eq!(eval("math::round(3.5)"), Value::Float(4.0));
    assert_eq!(eval("math::round(3.4)"), Value::Float(3.0));
    assert_eq!(eval("math::floor(5)"), Value::Int(5));
}

#[test]
fn test_pow() {
    assert_eq!(eval("math::pow(2, 10)"), Value::Int(1024));
    assert_eq!(eval("math::pow(2.0, 0.5)"), Value::Float(2.0_f64.sqrt()));
}

#[test]
fn test_sqrt() {
    assert_eq!(eval("math::sqrt(16)"), Value::Float(4.0));
    assert_eq!(eval("math::sqrt(2.0)"), Value::Float(2.0_f64.sqrt()));
}

#[test]
fn test_list_join() {
    assert_eq!(
        eval(r#"["a", "b", "c"].join(", ")"#),
        Value::Str("a, b, c".into())
    );
    assert_eq!(eval(r#"[1, 2, 3].join("-")"#), Value::Str("1-2-3".into()));
}

#[test]
fn test_list_enumerate() {
    assert_eq!(
        eval(r#"["a", "b"].enumerate()"#),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Str("a".into())]),
            Value::Tuple(vec![Value::Int(1), Value::Str("b".into())]),
        ])
    );
}

#[test]
fn test_enumerate_builtin() {
    assert_eq!(
        eval(r#"enumerate(["x", "y"])"#),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Str("x".into())]),
            Value::Tuple(vec![Value::Int(1), Value::Str("y".into())]),
        ])
    );
}

#[test]
fn test_json_encode_pretty() {
    let val = eval(r#"json::pretty(#{ "a": 1 })"#);
    if let Value::Str(s) = val {
        assert!(s.contains('\n'), "expected newlines in pretty JSON: {}", s);
    } else {
        panic!("expected string");
    }
}

// ============================================================
// Section 31: #[derive(IonType)] Proc Macro
// ============================================================

#[derive(Debug, Clone, IonType)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, IonType)]
struct UserProfile {
    name: String,
    age: i64,
    active: bool,
}

#[derive(Debug, Clone, IonType)]
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Empty,
}

#[test]
fn test_derive_struct_to_ion() {
    let p = Point { x: 1.0, y: 2.0 };
    let val = p.to_ion();
    if let Value::HostStruct { type_hash, fields } = &val {
        assert_eq!(*type_hash, h!("Point"));
        assert_eq!(fields[&h!("x")], Value::Float(1.0));
        assert_eq!(fields[&h!("y")], Value::Float(2.0));
    } else {
        panic!("expected HostStruct");
    }
}

#[test]
fn test_derive_struct_from_ion() {
    let p = Point { x: 3.0, y: 4.0 };
    let val = p.to_ion();
    let p2 = Point::from_ion(&val).unwrap();
    assert_eq!(p2.x, 3.0);
    assert_eq!(p2.y, 4.0);
}

#[test]
fn test_derive_struct_in_script() {
    let mut engine = Engine::new();
    engine.register_type::<Point>();
    let val = engine
        .eval(
            "
        let p = Point { x: 10.0, y: 20.0 };
        p.x + p.y
    ",
        )
        .unwrap();
    assert_eq!(val, Value::Float(30.0));
}

#[test]
fn test_derive_set_typed_get_typed() {
    let mut engine = Engine::new();
    engine.register_type::<UserProfile>();
    let profile = UserProfile {
        name: "Alice".into(),
        age: 30,
        active: true,
    };
    engine.set_typed("user", &profile);
    let val = engine.eval(r#"f"{user.name} is {user.age}""#).unwrap();
    assert_eq!(val, Value::Str("Alice is 30".into()));

    engine
        .eval("let result = UserProfile { name: \"Bob\", age: 25, active: false };")
        .unwrap();
    let result: UserProfile = engine.get_typed("result").unwrap();
    assert_eq!(result.name, "Bob");
    assert_eq!(result.age, 25);
    assert!(!result.active);
}

#[test]
fn test_derive_enum_to_ion() {
    let s = Shape::Circle(5.0);
    let val = s.to_ion();
    assert_eq!(
        val,
        Value::HostEnum {
            enum_hash: h!("Shape"),
            variant_hash: h!("Circle"),
            data: vec![Value::Float(5.0)],
        }
    );
}

#[test]
fn test_derive_enum_from_ion() {
    let val = Value::HostEnum {
        enum_hash: h!("Shape"),
        variant_hash: h!("Rect"),
        data: vec![Value::Float(3.0), Value::Float(4.0)],
    };
    let s = Shape::from_ion(&val).unwrap();
    match s {
        Shape::Rect(w, h) => {
            assert_eq!(w, 3.0);
            assert_eq!(h, 4.0);
        }
        _ => panic!("expected Rect"),
    }
}

#[test]
fn test_derive_enum_in_script() {
    let mut engine = Engine::new();
    engine.register_type::<Shape>();
    let val = engine
        .eval(
            r#"
        let s = Shape::Circle(5.0);
        match s {
            Shape::Circle(r) => r * r * 3.14,
            Shape::Rect(w, h) => w * h,
            Shape::Empty => 0.0,
        }
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Float(78.5));
}

#[test]
fn test_derive_enum_unit_variant_in_script() {
    let mut engine = Engine::new();
    engine.register_type::<Shape>();
    let val = engine
        .eval(
            r#"
        let s = Shape::Empty;
        match s {
            Shape::Circle(r) => r,
            Shape::Empty => 0.0,
            _ => -1.0,
        }
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Float(0.0));
}

#[test]
fn test_derive_roundtrip_typed() {
    let mut engine = Engine::new();
    engine.register_type::<Point>();
    let original = Point { x: 42.0, y: 99.0 };
    engine.set_typed("p", &original);
    engine
        .eval("let p2 = Point { x: p.x * 2.0, y: p.y * 2.0 };")
        .unwrap();
    let result: Point = engine.get_typed("p2").unwrap();
    assert_eq!(result.x, 84.0);
    assert_eq!(result.y, 198.0);
}

// ============================================================
// Section 32: Bitwise Operators
// ============================================================

#[test]
fn test_bitwise_and() {
    assert_eq!(eval("12 & 10"), Value::Int(8));
    assert_eq!(eval("255 & 15"), Value::Int(15));
}

#[test]
fn test_bitwise_or() {
    assert_eq!(eval("12 | 10"), Value::Int(14));
    assert_eq!(eval("8 | 4"), Value::Int(12));
}

#[test]
fn test_bitwise_xor() {
    assert_eq!(eval("12 ^ 10"), Value::Int(6));
    assert_eq!(eval("5 ^ 3"), Value::Int(6));
}

#[test]
fn test_shift_left() {
    assert_eq!(eval("1 << 4"), Value::Int(16));
    assert_eq!(eval("3 << 2"), Value::Int(12));
}

#[test]
fn test_shift_right() {
    assert_eq!(eval("16 >> 2"), Value::Int(4));
    assert_eq!(eval("255 >> 4"), Value::Int(15));
}

#[test]
fn test_bitwise_precedence() {
    // & binds tighter than |
    assert_eq!(eval("1 | 2 & 3"), Value::Int(1 | (2 & 3)));
    // ^ is between & and |
    assert_eq!(eval("3 | 5 ^ 6 & 7"), Value::Int(3 | (5 ^ (6 & 7))));
}

#[test]
fn test_bitwise_type_error() {
    let err = eval_err("1.0 & 2");
    assert!(err.contains("int"), "got: {}", err);
}

// ============================================================
// Section 33: Option/Result Functional Methods
// ============================================================

#[test]
fn test_option_map() {
    assert_eq!(
        eval("Some(5).map(|x| x * 2)"),
        Value::Option(Some(Box::new(Value::Int(10))))
    );
    assert_eq!(eval("None.map(|x| x * 2)"), Value::Option(None));
}

#[test]
fn test_option_and_then() {
    assert_eq!(
        eval("Some(5).and_then(|x| Some(x + 1))"),
        Value::Option(Some(Box::new(Value::Int(6))))
    );
    assert_eq!(eval("None.and_then(|x| Some(x + 1))"), Value::Option(None));
}

#[test]
fn test_option_or_else() {
    assert_eq!(
        eval("Some(5).or_else(|| Some(0))"),
        Value::Option(Some(Box::new(Value::Int(5))))
    );
    assert_eq!(
        eval("None.or_else(|| Some(99))"),
        Value::Option(Some(Box::new(Value::Int(99))))
    );
}

#[test]
fn test_option_unwrap_or_else() {
    assert_eq!(eval("Some(5).unwrap_or_else(|| 0)"), Value::Int(5));
    assert_eq!(eval("None.unwrap_or_else(|| 42)"), Value::Int(42));
}

#[test]
fn test_result_map() {
    assert_eq!(
        eval("Ok(5).map(|x| x * 2)"),
        Value::Result(Ok(Box::new(Value::Int(10))))
    );
    assert_eq!(
        eval("Err(\"bad\").map(|x| x * 2)"),
        Value::Result(Err(Box::new(Value::Str("bad".to_string()))))
    );
}

#[test]
fn test_result_map_err() {
    assert_eq!(
        eval("Ok(5).map_err(|e| f\"wrapped: {e}\")"),
        Value::Result(Ok(Box::new(Value::Int(5))))
    );
    assert_eq!(
        eval("Err(\"bad\").map_err(|e| f\"wrapped: {e}\")"),
        Value::Result(Err(Box::new(Value::Str("wrapped: bad".to_string()))))
    );
}

#[test]
fn test_result_and_then() {
    assert_eq!(
        eval("Ok(5).and_then(|x| Ok(x + 1))"),
        Value::Result(Ok(Box::new(Value::Int(6))))
    );
    assert_eq!(
        eval("Err(\"bad\").and_then(|x| Ok(x + 1))"),
        Value::Result(Err(Box::new(Value::Str("bad".to_string()))))
    );
}

#[test]
fn test_result_or_else() {
    assert_eq!(
        eval("Ok(5).or_else(|e| Ok(0))"),
        Value::Result(Ok(Box::new(Value::Int(5))))
    );
    assert_eq!(
        eval("Err(\"bad\").or_else(|e| Ok(99))"),
        Value::Result(Ok(Box::new(Value::Int(99))))
    );
}

#[test]
fn test_result_unwrap_or_else() {
    assert_eq!(eval("Ok(5).unwrap_or_else(|e| 0)"), Value::Int(5));
    assert_eq!(eval("Err(\"bad\").unwrap_or_else(|e| 42)"), Value::Int(42));
}

// ============================================================
// Section 34: String/List Missing Methods
// ============================================================

#[test]
fn test_string_chars() {
    assert_eq!(
        eval("\"abc\".chars()"),
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
            Value::Str("c".to_string()),
        ])
    );
}

#[test]
fn test_string_is_empty() {
    assert_eq!(eval("\"\".is_empty()"), Value::Bool(true));
    assert_eq!(eval("\"hello\".is_empty()"), Value::Bool(false));
}

#[test]
fn test_list_is_empty() {
    assert_eq!(eval("[].is_empty()"), Value::Bool(true));
    assert_eq!(eval("[1].is_empty()"), Value::Bool(false));
}

// ============================================================
// 35. Bytes type
// ============================================================

#[test]
fn test_bytes_literal() {
    assert_eq!(eval(r#"b"hello""#), Value::Bytes(b"hello".to_vec()));
}

#[test]
fn test_bytes_escape_sequences() {
    assert_eq!(eval(r#"b"\x00\xff""#), Value::Bytes(vec![0x00, 0xff]));
    assert_eq!(
        eval(r#"b"\n\t\r""#),
        Value::Bytes(vec![b'\n', b'\t', b'\r'])
    );
    assert_eq!(eval(r#"b"\0""#), Value::Bytes(vec![0]));
}

#[test]
fn test_bytes_concat() {
    assert_eq!(
        eval(r#"b"hello" + b" world""#),
        Value::Bytes(b"hello world".to_vec())
    );
}

#[test]
fn test_bytes_index() {
    assert_eq!(eval(r#"b"abc"[0]"#), Value::Int(97));
    assert_eq!(eval(r#"b"abc"[-1]"#), Value::Int(99));
}

#[test]
fn test_bytes_methods() {
    assert_eq!(eval(r#"b"hello".len()"#), Value::Int(5));
    assert_eq!(eval(r#"b"".is_empty()"#), Value::Bool(true));
    assert_eq!(eval(r#"b"abc".contains(97)"#), Value::Bool(true));
    assert_eq!(eval(r#"b"abc".contains(0)"#), Value::Bool(false));
}

#[test]
fn test_bytes_slice() {
    assert_eq!(
        eval(r#"b"hello".slice(1, 3)"#),
        Value::Bytes(b"el".to_vec())
    );
    assert_eq!(eval(r#"b"hello".slice(2)"#), Value::Bytes(b"llo".to_vec()));
}

#[test]
fn test_bytes_to_list() {
    assert_eq!(
        eval(r#"b"abc".to_list()"#),
        Value::List(vec![Value::Int(97), Value::Int(98), Value::Int(99),])
    );
}

#[test]
fn test_bytes_to_str() {
    assert_eq!(
        eval(r#"b"hello".to_str()"#),
        Value::Result(Ok(Box::new(Value::Str("hello".to_string()))))
    );
}

#[test]
fn test_bytes_to_hex() {
    assert_eq!(
        eval(r#"b"\xde\xad".to_hex()"#),
        Value::Str("dead".to_string())
    );
}

#[test]
fn test_bytes_find() {
    assert_eq!(
        eval(r#"b"abc".find(98)"#),
        Value::Option(Some(Box::new(Value::Int(1))))
    );
    assert_eq!(eval(r#"b"abc".find(0)"#), Value::Option(None));
}

#[test]
fn test_bytes_reverse() {
    assert_eq!(eval(r#"b"abc".reverse()"#), Value::Bytes(vec![99, 98, 97]));
}

#[test]
fn test_bytes_push() {
    assert_eq!(eval(r#"b"ab".push(99)"#), Value::Bytes(b"abc".to_vec()));
}

#[test]
fn test_bytes_constructor() {
    assert_eq!(
        eval(r#"bytes([65, 66, 67])"#),
        Value::Bytes(b"ABC".to_vec())
    );
    assert_eq!(eval(r#"bytes("hello")"#), Value::Bytes(b"hello".to_vec()));
    assert_eq!(eval(r#"bytes(3)"#), Value::Bytes(vec![0, 0, 0]));
    assert_eq!(eval(r#"bytes()"#), Value::Bytes(Vec::new()));
}

#[test]
fn test_bytes_from_hex() {
    assert_eq!(
        eval(r#"bytes_from_hex("deadbeef")"#),
        Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
    );
}

#[test]
fn test_bytes_module_constructors() {
    assert_eq!(eval(r#"bytes::new()"#), Value::Bytes(Vec::new()));
    assert_eq!(eval(r#"bytes::zeroed(3)"#), Value::Bytes(vec![0, 0, 0]));
    assert_eq!(
        eval(r#"bytes::repeat(65, 3)"#),
        Value::Bytes(b"AAA".to_vec())
    );
    assert_eq!(
        eval(r#"bytes::from_list([65, 66, 67])"#),
        Value::Bytes(b"ABC".to_vec())
    );
    assert_eq!(
        eval(r#"bytes::from_str("hello")"#),
        Value::Bytes(b"hello".to_vec())
    );
    assert_eq!(
        eval(r#"bytes::from_hex("deadbeef").unwrap()"#),
        Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
    );
    assert_eq!(
        eval(r#"bytes::from_base64("aGVsbG8=").unwrap()"#),
        Value::Bytes(b"hello".to_vec())
    );
    assert_eq!(
        eval(r#"bytes::concat([b"ab", b"cd", b"ef"])"#),
        Value::Bytes(b"abcdef".to_vec())
    );
    assert_eq!(
        eval(r#"bytes::join([b"ab", b"cd", b"ef"], b",")"#),
        Value::Bytes(b"ab,cd,ef".to_vec())
    );
}

#[test]
fn test_bytes_sequence_methods() {
    assert_eq!(eval(r#"b"abcabc".contains(b"bc")"#), Value::Bool(true));
    assert_eq!(
        eval(r#"b"abcabc".find(b"ca")"#),
        Value::Option(Some(Box::new(Value::Int(2))))
    );
    assert_eq!(eval(r#"b"abcabc".count(b"ab")"#), Value::Int(2));
    assert_eq!(eval(r#"b"abc".starts_with(b"ab")"#), Value::Bool(true));
    assert_eq!(eval(r#"b"abc".ends_with(99)"#), Value::Bool(true));
    assert_eq!(
        eval(r#"b"a,b,c".split(b",")"#),
        Value::List(vec![
            Value::Bytes(b"a".to_vec()),
            Value::Bytes(b"b".to_vec()),
            Value::Bytes(b"c".to_vec()),
        ])
    );
    assert_eq!(
        eval(r#"b"abcabc".replace(b"ab", b"z")"#),
        Value::Bytes(b"zczc".to_vec())
    );
    assert_eq!(eval(r#"b"ab".repeat(3)"#), Value::Bytes(b"ababab".to_vec()));
    assert_eq!(
        eval(r#"b"ab".extend(b"cd")"#),
        Value::Bytes(b"abcd".to_vec())
    );
    assert_eq!(
        eval(r#"b"abc".set(-1, 100)"#),
        Value::Bytes(b"abd".to_vec())
    );
    assert_eq!(
        eval(r#"b"abc".pop()"#),
        Value::Tuple(vec![
            Value::Bytes(b"ab".to_vec()),
            Value::Option(Some(Box::new(Value::Int(99)))),
        ])
    );
    assert_eq!(
        eval(r#"b"hello".to_base64()"#),
        Value::Str("aGVsbG8=".into())
    );
    assert_eq!(eval(r#"b"abc".bytes()"#), Value::Bytes(b"abc".to_vec()));
}

#[test]
fn test_bytes_endian_helpers() {
    assert_eq!(eval("bytes::u16_le(4660)"), Value::Bytes(vec![0x34, 0x12]));
    assert_eq!(
        eval("bytes::u32_be(305419896)"),
        Value::Bytes(vec![0x12, 0x34, 0x56, 0x78])
    );
    assert_eq!(eval("bytes::i16_le(-2)"), Value::Bytes(vec![0xfe, 0xff]));
    assert_eq!(
        eval("bytes::u32_le(305419896).read_u32_le(0).unwrap()"),
        Value::Int(305419896)
    );
    assert_eq!(
        eval("bytes::u32_be(305419896).read_u16_be(0).unwrap()"),
        Value::Int(4660)
    );
    assert_eq!(
        eval(r#"b"\xff\xff".read_i16_be(0).unwrap()"#),
        Value::Int(-1)
    );
    assert_eq!(
        eval(r#"b"\x01".read_u16_be(0).is_err()"#),
        Value::Bool(true)
    );
}

#[test]
fn test_bytes_len_builtin() {
    assert_eq!(eval(r#"len(b"hello")"#), Value::Int(5));
}

#[test]
fn test_bytes_equality() {
    assert_eq!(eval(r#"b"abc" == b"abc""#), Value::Bool(true));
    assert_eq!(eval(r#"b"abc" != b"def""#), Value::Bool(true));
}

#[test]
fn test_bytes_truthy() {
    assert_eq!(
        eval(r#"if b"hello" { true } else { false }"#),
        Value::Bool(true)
    );
    assert_eq!(
        eval(r#"if b"" { true } else { false }"#),
        Value::Bool(false)
    );
}

#[test]
fn test_bytes_pattern_match() {
    assert_eq!(
        eval(r#"match b"abc" { b"abc" => 1, _ => 2 }"#),
        Value::Int(1)
    );
    assert_eq!(
        eval(r#"match b"xyz" { b"abc" => 1, _ => 2 }"#),
        Value::Int(2)
    );
}

#[test]
fn test_bytes_for_loop() {
    assert_eq!(
        eval(r#"let mut sum = 0; for b in b"abc".to_list() { sum += b; } sum"#),
        Value::Int(97 + 98 + 99)
    );
}

#[test]
fn test_bytes_display() {
    assert_eq!(
        eval("let x = b\"hello\"; f\"{x}\""),
        Value::Str("b\"hello\"".to_string())
    );
}

// ============================================================
// 36. Slice syntax
// ============================================================

#[test]
fn test_list_slice() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5][1..3]"),
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_slice_from_start() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5][..2]"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn test_list_slice_to_end() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5][3..]"),
        Value::List(vec![Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn test_list_slice_inclusive() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5][1..=3]"),
        Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
    );
}

#[test]
fn test_list_slice_full() {
    assert_eq!(
        eval("[1, 2, 3][..]"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_string_slice() {
    assert_eq!(eval(r#""hello"[1..3]"#), Value::Str("el".to_string()));
    assert_eq!(eval(r#""hello"[..2]"#), Value::Str("he".to_string()));
    assert_eq!(eval(r#""hello"[3..]"#), Value::Str("lo".to_string()));
}

#[test]
fn test_bytes_slice_syntax() {
    assert_eq!(eval(r#"b"hello"[1..3]"#), Value::Bytes(b"el".to_vec()));
    assert_eq!(eval(r#"b"hello"[..2]"#), Value::Bytes(b"he".to_vec()));
    assert_eq!(eval(r#"b"hello"[3..]"#), Value::Bytes(b"lo".to_vec()));
}

#[test]
fn test_slice_out_of_bounds_clamps() {
    assert_eq!(
        eval("[1, 2, 3][0..100]"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(eval("[1, 2, 3][5..10]"), Value::List(vec![]));
}

#[test]
fn test_slice_with_variables() {
    assert_eq!(
        eval("let start = 1; let end = 3; [10, 20, 30, 40][start..end]"),
        Value::List(vec![Value::Int(20), Value::Int(30)])
    );
}

// ============================================================
// 37. Iterator protocol
// ============================================================

#[test]
fn test_for_over_bytes() {
    assert_eq!(
        eval(r#"let mut sum = 0; for b in b"abc" { sum += b; } sum"#),
        Value::Int(97 + 98 + 99)
    );
}

#[test]
fn test_for_over_tuple() {
    assert_eq!(
        eval("let mut sum = 0; for x in (1, 2, 3) { sum += x; } sum"),
        Value::Int(6)
    );
}

#[test]
fn test_for_over_string() {
    assert_eq!(
        eval(r#"let mut s = ""; for c in "abc" { s = s + c + "-"; } s"#),
        Value::Str("a-b-c-".to_string())
    );
}

// ============================================================
// 38. VM function compilation
// ============================================================

#[cfg(feature = "vm")]
mod vm_integration {
    use super::*;

    #[test]
    fn test_vm_fn_simple() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.vm_eval("fn add(a, b) { a + b } add(3, 4)").unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn test_vm_fn_nested_calls() {
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .vm_eval("fn double(x) { x * 2 } fn quad(x) { double(double(x)) } quad(5)")
                .unwrap(),
            Value::Int(20)
        );
    }

    #[test]
    fn test_vm_fn_closure() {
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .vm_eval("let x = 10; fn add_x(y) { x + y } add_x(5)")
                .unwrap(),
            Value::Int(15)
        );
    }

    #[test]
    fn test_vm_fn_default_params() {
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .vm_eval("fn greet(name = \"world\") { name } greet()")
                .unwrap(),
            Value::Str("world".to_string())
        );
    }

    #[test]
    fn test_vm_fn_if_in_body() {
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .vm_eval("fn abs(x) { if x < 0 { -x } else { x } } abs(-5)")
                .unwrap(),
            Value::Int(5)
        );
    }

    #[test]
    fn test_vm_fn_loop_in_body() {
        let mut engine = Engine::new();
        assert_eq!(engine.vm_eval("fn sum(n) { let mut s = 0; let mut i = 0; while i < n { s += i; i += 1; } s } sum(5)").unwrap(), Value::Int(10));
    }

    #[test]
    fn test_vm_slice() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.vm_eval("[1, 2, 3, 4, 5][1..3]").unwrap(),
            Value::List(vec![Value::Int(2), Value::Int(3)])
        );
        assert_eq!(
            engine.vm_eval("[1, 2, 3][..2]").unwrap(),
            Value::List(vec![Value::Int(1), Value::Int(2)])
        );
        assert_eq!(
            engine.vm_eval("[1, 2, 3][1..]").unwrap(),
            Value::List(vec![Value::Int(2), Value::Int(3)])
        );
    }

    #[test]
    fn test_vm_for_bytes() {
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .vm_eval(r#"let mut sum = 0; for b in b"abc" { sum += b; } sum"#)
                .unwrap(),
            Value::Int(97 + 98 + 99)
        );
    }
} // mod vm_integration

// ============================================================
// Index/field assignment (tree-walk)
// ============================================================

#[test]
fn test_list_index_assign() {
    assert_eq!(
        eval("let mut a = [1, 2, 3]; a[0] = 10; a"),
        Value::List(vec![Value::Int(10), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval("let mut a = [10, 20, 30]; a[1] += 5; a"),
        Value::List(vec![Value::Int(10), Value::Int(25), Value::Int(30)])
    );
}

#[test]
fn test_dict_index_assign() {
    assert_eq!(
        eval("let mut d = #{\"x\": 1}; d[\"x\"] = 42; d.x"),
        Value::Int(42)
    );
}

#[test]
fn test_dict_field_assign() {
    assert_eq!(
        eval("let mut d = #{\"x\": 1, \"y\": 2}; d.x = 99; d.x"),
        Value::Int(99)
    );
    assert_eq!(
        eval("let mut d = #{\"count\": 0}; d.count += 1; d.count += 1; d.count"),
        Value::Int(2)
    );
}

#[test]
fn test_index_assign_negative() {
    assert_eq!(
        eval("let mut a = [1, 2, 3]; a[-1] = 99; a"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(99)])
    );
}

// --- Multi-error reporting ---

#[test]
fn test_multi_error_reports_all() {
    let mut engine = Engine::new();
    let err = engine.eval("let x = ; let y = ; 42").unwrap_err();
    // Should have the first error plus at least one additional
    assert!(
        !err.additional.is_empty(),
        "expected multiple errors, got just one: {}",
        err.message
    );
}

#[test]
fn test_multi_error_single_error_no_additional() {
    let mut engine = Engine::new();
    let err = engine.eval("let x = ;").unwrap_err();
    // Single error should have no additional errors
    assert!(
        err.additional.is_empty(),
        "expected single error, got additional: {:?}",
        err.additional
    );
}

#[test]
fn test_multi_error_format_with_source() {
    let mut engine = Engine::new();
    let src = "let x = ;\nlet y = ;";
    let err = engine.eval(src).unwrap_err();
    let formatted = err.format_with_source(src);
    // Should contain error text for both lines
    assert!(
        formatted.contains("error[parse]"),
        "formatted: {}",
        formatted
    );
}

// ============================================================
// Section: flat_map, triple strings, string indexing, named args, tuple methods
// ============================================================

#[test]
fn test_flat_map() {
    assert_eq!(
        eval("[1, 2, 3].flat_map(|x| [x, x * 10])"),
        Value::List(vec![
            Value::Int(1),
            Value::Int(10),
            Value::Int(2),
            Value::Int(20),
            Value::Int(3),
            Value::Int(30),
        ])
    );
}

#[test]
fn test_triple_quoted_string() {
    assert_eq!(
        eval(
            r#""""hello
world""""#
        ),
        Value::Str("hello\nworld".to_string())
    );
}

#[test]
fn test_triple_quoted_fstring() {
    assert_eq!(
        eval(r#"let x = 42; f"""value: {x}""""#),
        Value::Str("value: 42".to_string())
    );
}

#[test]
fn test_string_index() {
    assert_eq!(eval(r#""hello"[1]"#), Value::Str("e".to_string()));
}

#[test]
fn test_string_index_negative() {
    assert_eq!(eval(r#""hello"[-1]"#), Value::Str("o".to_string()));
}

#[test]
fn test_string_slice_char_based() {
    assert_eq!(eval(r#""hello"[1..3]"#), Value::Str("el".to_string()));
    assert_eq!(eval(r#""hello"[..2]"#), Value::Str("he".to_string()));
    assert_eq!(eval(r#""hello"[3..]"#), Value::Str("lo".to_string()));
    assert_eq!(eval(r#""hello"[1..=3]"#), Value::Str("ell".to_string()));
    assert_eq!(eval(r#""abcdef"[0..0]"#), Value::Str("".to_string()));
}

#[test]
fn test_assert_pass() {
    assert_eq!(eval("assert(true)"), Value::Unit);
    assert_eq!(eval("assert(1 == 1)"), Value::Unit);
}

#[test]
fn test_assert_fail() {
    let result = Engine::new().eval("assert(false)");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("assertion failed"));
}

#[test]
fn test_assert_with_message() {
    let result = Engine::new().eval(r#"assert(false, "x must be positive")"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("x must be positive"));
}

#[test]
fn test_assert_eq_pass() {
    assert_eq!(eval("assert_eq(1, 1)"), Value::Unit);
    assert_eq!(eval(r#"assert_eq("a", "a")"#), Value::Unit);
}

#[test]
fn test_assert_eq_fail() {
    let result = Engine::new().eval("assert_eq(1, 2)");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("expected 1, got 2"));
}

#[test]
fn test_assert_eq_with_message() {
    let result = Engine::new().eval(r#"assert_eq(1, 2, "values differ")"#);
    assert!(result.is_err());
    let msg = result.unwrap_err().message;
    assert!(msg.contains("values differ"));
    assert!(msg.contains("expected 1, got 2"));
}

#[test]
fn test_named_args() {
    assert_eq!(
        eval("fn add(a, b) { a - b } add(b: 10, a: 3)"),
        Value::Int(-7)
    );
}

#[test]
fn test_named_args_with_defaults() {
    assert_eq!(
        eval(
            r#"fn greet(name, greeting = "hi") { f"{greeting} {name}" } greet(greeting: "hello", name: "world")"#
        ),
        Value::Str("hello world".to_string())
    );
}

#[test]
fn test_named_args_mixed() {
    assert_eq!(
        eval("fn f(a, b, c) { a * 100 + b * 10 + c } f(1, c: 3, b: 2)"),
        Value::Int(123)
    );
}

#[test]
fn test_tuple_len() {
    assert_eq!(eval("(1, 2, 3).len()"), Value::Int(3));
}

#[test]
fn test_tuple_contains() {
    assert_eq!(eval("(1, 2, 3).contains(2)"), Value::Bool(true));
    assert_eq!(eval("(1, 2, 3).contains(5)"), Value::Bool(false));
}

#[test]
fn test_tuple_to_list() {
    assert_eq!(
        eval("(1, 2, 3).to_list()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_string_multiply() {
    assert_eq!(eval(r#""ha" * 3"#), Value::Str("hahaha".to_string()));
    assert_eq!(eval(r#"3 * "ab""#), Value::Str("ababab".to_string()));
}

#[test]
fn test_range_for_loop() {
    assert_eq!(
        eval("let mut s = 0; for i in 0..5 { s += i; } s"),
        Value::Int(10)
    );
}

#[test]
fn test_range_inclusive_for_loop() {
    assert_eq!(
        eval("let mut s = 0; for i in 1..=5 { s += i; } s"),
        Value::Int(15)
    );
}

#[test]
fn test_multiline_lambda() {
    assert_eq!(
        eval("let f = |x| { let y = x * 2; y + 1 }; f(5)"),
        Value::Int(11)
    );
}

// ============================================================
// Section: try/catch
// ============================================================

#[test]
fn test_try_catch_no_error() {
    assert_eq!(eval("try { 42 } catch e { 0 }"), Value::Int(42));
}

#[test]
fn test_try_catch_with_error() {
    assert_eq!(
        eval(r#"try { assert(false, "boom"); 1 } catch e { e }"#),
        Value::Str("boom".to_string())
    );
}

#[test]
fn test_try_catch_division_by_zero() {
    assert_eq!(eval("try { 1 / 0 } catch e { -1 }"), Value::Int(-1));
}

#[test]
fn test_try_catch_nested() {
    assert_eq!(
        eval(
            r#"
            try {
                try { assert(false, "inner") } catch e { f"caught: {e}" }
            } catch e { "outer" }
        "#
        ),
        Value::Str("caught: inner".to_string())
    );
}

#[test]
fn test_try_catch_as_expression() {
    assert_eq!(
        eval(
            r#"
            let result = try {
                let x = 10;
                let y = 0;
                x / y
            } catch e {
                -1
            };
            result
        "#
        ),
        Value::Int(-1)
    );
}

// === Unicode consistency tests ===

#[test]
fn test_string_find_char_offset() {
    // find() should return char offset, not byte offset
    assert_eq!(
        eval(r#""héllo".find("l")"#),
        Value::Option(Some(Box::new(Value::Int(2))))
    );
    assert_eq!(
        eval(r#""abc".find("c")"#),
        Value::Option(Some(Box::new(Value::Int(2))))
    );
    assert_eq!(eval(r#""abc".find("d")"#), Value::Option(None));
}

#[test]
fn test_string_slice_char_offset() {
    // slice() should use char offsets, not byte offsets
    assert_eq!(eval(r#""héllo".slice(1, 3)"#), Value::Str("él".to_string()));
    assert_eq!(
        eval(r#""hello".slice(1, 4)"#),
        Value::Str("ell".to_string())
    );
}

#[test]
fn test_string_negative_index_unicode() {
    // negative indexing should be char-based
    assert_eq!(eval(r#""héllo"[-1]"#), Value::Str("o".to_string()));
    assert_eq!(eval(r#""héllo"[-2]"#), Value::Str("l".to_string()));
}

#[test]
fn test_sort_mixed_types_error() {
    let mut engine = Engine::new();
    let result = engine.eval(r#"[1, "a", 2].sort()"#);
    assert!(result.is_err());
}

#[test]
fn test_sort_homogeneous() {
    assert_eq!(
        eval(r#"[3, 1, 2].sort()"#),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval(r#"["c", "a", "b"].sort()"#),
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
            Value::Str("c".to_string()),
        ])
    );
}

#[test]
fn test_sort_empty() {
    assert_eq!(eval(r#"[].sort()"#), Value::List(vec![]));
}

#[test]
fn test_sort_by() {
    assert_eq!(
        eval("[3, 1, 2].sort_by(|a, b| a - b)"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval("[3, 1, 2].sort_by(|a, b| b - a)"),
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn test_clamp() {
    assert_eq!(eval("math::clamp(5, 0, 3)"), Value::Int(3));
    assert_eq!(eval("math::clamp(-1, 0, 10)"), Value::Int(0));
    assert_eq!(eval("math::clamp(5, 0, 10)"), Value::Int(5));
    assert_eq!(eval("math::clamp(1.5, 0.0, 1.0)"), Value::Float(1.0));
    assert_eq!(eval("math::clamp(0.5, 0.0, 1.0)"), Value::Float(0.5));
}

#[test]
fn test_unicode_escape() {
    assert_eq!(eval(r#""\u{48}\u{49}""#), Value::Str("HI".to_string()));
    assert_eq!(eval(r#""\u{1F600}""#), Value::Str("\u{1F600}".to_string()));
    assert_eq!(eval(r#""\u{E9}""#), Value::Str("é".to_string()));
    // In f-strings
    assert_eq!(
        eval(r#"f"hello \u{1F600}""#),
        Value::Str("hello \u{1F600}".to_string())
    );
}

#[test]
fn test_unicode_escape_triple_quoted() {
    assert_eq!(
        eval(r#""""\u{2764}""""#),
        Value::Str("\u{2764}".to_string())
    );
}

#[test]
fn test_dict_map() {
    assert_eq!(
        eval(r#"#{a: 1, b: 2}.map(|k, v| v * 10)"#),
        Value::Dict(indexmap::indexmap! {
            "a".to_string() => Value::Int(10),
            "b".to_string() => Value::Int(20),
        })
    );
}

#[test]
fn test_dict_filter() {
    assert_eq!(
        eval(r#"#{a: 1, b: 2, c: 3}.filter(|k, v| v > 1)"#),
        Value::Dict(indexmap::indexmap! {
            "b".to_string() => Value::Int(2),
            "c".to_string() => Value::Int(3),
        })
    );
}

#[test]
fn test_string_contains_int() {
    assert_eq!(eval(r#""hello".contains(104)"#), Value::Bool(true)); // 'h'
    assert_eq!(eval(r#""hello".contains(122)"#), Value::Bool(false)); // 'z'
    assert_eq!(eval(r#""hello".contains("ell")"#), Value::Bool(true));
}

#[test]
fn test_to_string_method() {
    assert_eq!(
        eval(r#"let x = 42; x.to_string()"#),
        Value::Str("42".to_string()),
    );
    assert_eq!(eval(r#"true.to_string()"#), Value::Str("true".to_string()));
    assert_eq!(
        eval(r#"let x = 3.14; x.to_string()"#),
        Value::Str("3.14".to_string()),
    );
    assert_eq!(
        eval(r#"[1, 2].to_string()"#),
        Value::Str("[1, 2]".to_string())
    );
    assert_eq!(eval(r#"None.to_string()"#), Value::Str("None".to_string()));
}

#[test]
fn test_dict_zip() {
    assert_eq!(
        eval(r#"#{a: 1, b: 2}.zip(#{a: 10, b: 20})"#),
        Value::Dict(indexmap::indexmap! {
            "a".to_string() => Value::Tuple(vec![Value::Int(1), Value::Int(10)]),
            "b".to_string() => Value::Tuple(vec![Value::Int(2), Value::Int(20)]),
        })
    );
    // Only matching keys
    assert_eq!(
        eval(r#"#{a: 1, b: 2}.zip(#{b: 20, c: 30})"#),
        Value::Dict(indexmap::indexmap! {
            "b".to_string() => Value::Tuple(vec![Value::Int(2), Value::Int(20)]),
        })
    );
}

#[test]
fn test_join_builtin() {
    assert_eq!(
        eval(r#"string::join(["a", "b", "c"], ",")"#),
        Value::Str("a,b,c".to_string())
    );
    assert_eq!(
        eval(r#"string::join([1, 2, 3], " ")"#),
        Value::Str("1 2 3".to_string())
    );
    assert_eq!(
        eval(r#"string::join(["x"], "-")"#),
        Value::Str("x".to_string())
    );
}

#[test]
fn test_string_bytes_method() {
    assert_eq!(
        eval(r#""ABC".bytes()"#),
        Value::List(vec![Value::Int(65), Value::Int(66), Value::Int(67)])
    );
    assert_eq!(eval(r#""".bytes()"#), Value::List(vec![]));
}

#[test]
fn test_enumerate_string() {
    assert_eq!(
        eval(r#"enumerate("ab")"#),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Str("a".to_string())]),
            Value::Tuple(vec![Value::Int(1), Value::Str("b".to_string())]),
        ])
    );
}

#[test]
fn test_enumerate_dict() {
    assert_eq!(
        eval(r#"enumerate(#{a: 1})"#),
        Value::List(vec![Value::Tuple(vec![
            Value::Int(0),
            Value::Tuple(vec![Value::Str("a".to_string()), Value::Int(1)]),
        ])])
    );
}

#[test]
fn test_list_index_method() {
    assert_eq!(
        eval(r#"[10, 20, 30].index(20)"#),
        Value::Option(Some(Box::new(Value::Int(1))))
    );
    assert_eq!(eval(r#"[10, 20, 30].index(99)"#), Value::Option(None));
}

#[test]
fn test_list_count_method() {
    assert_eq!(eval(r#"[1, 2, 1, 3, 1].count(1)"#), Value::Int(3));
    assert_eq!(eval(r#"[1, 2, 3].count(99)"#), Value::Int(0));
}

#[test]
fn test_list_slice_method() {
    assert_eq!(
        eval(r#"[1, 2, 3, 4, 5].slice(1, 3)"#),
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval(r#"[1, 2, 3].slice(0, 2)"#),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        eval(r#"[1, 2, 3].slice(1)"#),
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_dedup_method() {
    assert_eq!(
        eval(r#"[1, 1, 2, 2, 3, 1].dedup()"#),
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(1)
        ])
    );
    assert_eq!(
        eval(r#"[1, 1, 1].dedup()"#),
        Value::List(vec![Value::Int(1)])
    );
    assert_eq!(eval(r#"[].dedup()"#), Value::List(vec![]));
}

#[test]
fn test_string_pad_start() {
    assert_eq!(
        eval(r#""42".pad_start(5, "0")"#),
        Value::Str("00042".to_string())
    );
    assert_eq!(
        eval(r#""hi".pad_start(5)"#),
        Value::Str("   hi".to_string())
    );
    assert_eq!(
        eval(r#""hello".pad_start(3, "x")"#),
        Value::Str("hello".to_string())
    );
}

#[test]
fn test_string_pad_end() {
    assert_eq!(
        eval(r#""42".pad_end(5, "0")"#),
        Value::Str("42000".to_string())
    );
    assert_eq!(eval(r#""hi".pad_end(5)"#), Value::Str("hi   ".to_string()));
}

#[test]
fn test_let_destructure_tuple() {
    assert_eq!(eval(r#"let (a, b) = (10, 20); a + b"#), Value::Int(30));
    assert_eq!(eval(r#"let (x, _, z) = (1, 2, 3); x + z"#), Value::Int(4));
}

#[test]
fn test_let_destructure_list() {
    assert_eq!(eval(r#"let [a, b] = [10, 20]; a + b"#), Value::Int(30));
    assert_eq!(
        eval(r#"let [h, ...rest] = [1, 2, 3]; rest"#),
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_list_unique() {
    assert_eq!(
        eval(r#"[1, 2, 1, 3, 2, 1].unique()"#),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(eval(r#"[].unique()"#), Value::List(vec![]));
}

#[test]
fn test_list_min() {
    assert_eq!(
        eval(r#"[3, 1, 2].min()"#),
        Value::Option(Some(Box::new(Value::Int(1))))
    );
    assert_eq!(eval(r#"[].min()"#), Value::Option(None));
    assert_eq!(
        eval(r#"["c", "a", "b"].min()"#),
        Value::Option(Some(Box::new(Value::Str("a".to_string()))))
    );
}

#[test]
fn test_list_max() {
    assert_eq!(
        eval(r#"[3, 1, 2].max()"#),
        Value::Option(Some(Box::new(Value::Int(3))))
    );
    assert_eq!(eval(r#"[].max()"#), Value::Option(None));
}

#[test]
fn test_dict_update() {
    assert_eq!(
        eval(r#"#{a: 1, b: 2}.update(#{b: 20, c: 30})"#),
        Value::Dict(indexmap::indexmap! {
            "a".to_string() => Value::Int(1),
            "b".to_string() => Value::Int(20),
            "c".to_string() => Value::Int(30),
        })
    );
}

#[test]
fn test_string_char_len() {
    assert_eq!(eval(r#""héllo".char_len()"#), Value::Int(5));
    assert_eq!(eval(r#""héllo".len()"#), Value::Int(6));
}

#[test]
fn test_list_sum() {
    assert_eq!(eval(r#"[1, 2, 3].sum()"#), Value::Int(6));
    assert_eq!(eval(r#"[].sum()"#), Value::Int(0));
    assert_eq!(eval(r#"[1, 2.5, 3].sum()"#), Value::Float(6.5));
}

#[test]
fn test_list_window() {
    assert_eq!(
        eval(r#"[1, 2, 3, 4].window(2)"#),
        Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(2), Value::Int(3)]),
            Value::List(vec![Value::Int(3), Value::Int(4)]),
        ])
    );
    assert_eq!(eval(r#"[1].window(3)"#), Value::List(vec![]));
}

#[test]
fn test_string_strip_prefix() {
    assert_eq!(
        eval(r#""hello world".strip_prefix("hello ")"#),
        Value::Str("world".to_string())
    );
    assert_eq!(
        eval(r#""hello".strip_prefix("xyz")"#),
        Value::Str("hello".to_string())
    );
}

#[test]
fn test_string_strip_suffix() {
    assert_eq!(
        eval(r#""hello.ion".strip_suffix(".ion")"#),
        Value::Str("hello".to_string())
    );
    assert_eq!(
        eval(r#""hello".strip_suffix(".ion")"#),
        Value::Str("hello".to_string())
    );
}

#[test]
fn test_dict_keys_of() {
    assert_eq!(
        eval(r#"#{a: 1, b: 2, c: 1}.keys_of(1)"#),
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("c".to_string()),
        ])
    );
    assert_eq!(eval(r#"#{a: 1}.keys_of(99)"#), Value::List(vec![]));
}

// ============================================================
// MessagePack
// ============================================================

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_int() {
    assert_eq!(
        eval("json::msgpack_decode(json::msgpack_encode(42))"),
        Value::Int(42)
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_string() {
    assert_eq!(
        eval(r#"json::msgpack_decode(json::msgpack_encode("hello"))"#),
        Value::Str("hello".to_string())
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_bytes() {
    assert_eq!(
        eval(r#"json::msgpack_decode(json::msgpack_encode(b"\xde\xad"))"#),
        Value::Bytes(vec![0xde, 0xad])
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_list() {
    assert_eq!(
        eval("json::msgpack_decode(json::msgpack_encode([1, 2, 3]))"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_dict() {
    assert_eq!(
        eval(r#"json::msgpack_decode(json::msgpack_encode(#{a: 1, b: 2}))"#),
        Value::Dict(indexmap::indexmap! {
            "a".to_string() => Value::Int(1),
            "b".to_string() => Value::Int(2),
        })
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_nested() {
    assert_eq!(
        eval(
            r#"let data = #{name: "ion", items: [1, 2], raw: b"\xff"}; json::msgpack_decode(json::msgpack_encode(data))"#
        ),
        Value::Dict(indexmap::indexmap! {
            "name".to_string() => Value::Str("ion".to_string()),
            "items".to_string() => Value::List(vec![Value::Int(1), Value::Int(2)]),
            "raw".to_string() => Value::Bytes(vec![0xff]),
        })
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_encode_returns_bytes() {
    assert_eq!(
        eval(r#"type_of(json::msgpack_encode(42))"#),
        Value::Str("bytes".to_string())
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_bool_none() {
    assert_eq!(
        eval("json::msgpack_decode(json::msgpack_encode(true))"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("json::msgpack_decode(json::msgpack_encode(None))"),
        Value::Option(None)
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn test_msgpack_round_trip_float() {
    assert_eq!(
        eval("json::msgpack_decode(json::msgpack_encode(3.14))"),
        Value::Float(3.14)
    );
}

// ============================================================
// List: chunk, reduce
// ============================================================

#[test]
fn test_list_chunk() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5].chunk(2)"),
        Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(3), Value::Int(4)]),
            Value::List(vec![Value::Int(5)]),
        ])
    );
}

#[test]
fn test_list_reduce() {
    assert_eq!(eval("[1, 2, 3, 4].reduce(|a, b| a + b)"), Value::Int(10));
}

// ============================================================
// Spread operator: [...a, ...b]
// ============================================================

#[test]
fn test_list_spread() {
    assert_eq!(
        eval("let a = [1, 2]; let b = [3, 4]; [...a, ...b]"),
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ])
    );
}

#[test]
fn test_list_spread_mixed() {
    assert_eq!(
        eval("let a = [1, 2]; [0, ...a, 3]"),
        Value::List(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ])
    );
}

// ============================================================
// Set type
// ============================================================

#[test]
fn test_set_create() {
    assert_eq!(
        eval("set([1, 2, 2, 3])"),
        Value::Set(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_set_contains() {
    assert_eq!(eval("set([1, 2, 3]).contains(2)"), Value::Bool(true));
    assert_eq!(eval("set([1, 2, 3]).contains(5)"), Value::Bool(false));
}

#[test]
fn test_set_add_remove() {
    assert_eq!(
        eval("set([1, 2]).add(3)"),
        Value::Set(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval("set([1, 2, 3]).remove(2)"),
        Value::Set(vec![Value::Int(1), Value::Int(3)])
    );
}

#[test]
fn test_set_union() {
    assert_eq!(
        eval("set([1, 2]).union(set([2, 3]))"),
        Value::Set(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_set_intersection() {
    assert_eq!(
        eval("set([1, 2, 3]).intersection(set([2, 3, 4]))"),
        Value::Set(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_set_difference() {
    assert_eq!(
        eval("set([1, 2, 3]).difference(set([2, 4]))"),
        Value::Set(vec![Value::Int(1), Value::Int(3)])
    );
}

#[test]
fn test_set_to_list() {
    assert_eq!(
        eval("set([3, 1, 2]).to_list()"),
        Value::List(vec![Value::Int(3), Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn test_set_len() {
    assert_eq!(eval("set([1, 2, 2, 3]).len()"), Value::Int(3));
}

#[test]
fn test_set_iterate() {
    assert_eq!(
        eval("let mut s = 0; for x in set([1, 2, 3]) { s = s + x; } s"),
        Value::Int(6)
    );
}

// ============================================================
// Type annotations
// ============================================================

#[test]
fn test_type_ann_valid() {
    assert_eq!(eval("let x: int = 42; x"), Value::Int(42));
    assert_eq!(
        eval(r#"let s: string = "hi"; s"#),
        Value::Str("hi".to_string())
    );
    assert_eq!(eval("let b: bool = true; b"), Value::Bool(true));
    assert_eq!(
        eval("let xs: list = [1, 2]; xs"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn test_type_ann_mismatch() {
    assert!(eval_err("let x: int = true;").contains("type mismatch"));
    assert!(eval_err(r#"let x: string = 42;"#).contains("type mismatch"));
}

#[test]
fn test_type_ann_any() {
    assert_eq!(eval("let x: any = 42; x"), Value::Int(42));
    assert_eq!(
        eval(r#"let x: any = "hi"; x"#),
        Value::Str("hi".to_string())
    );
}

#[test]
fn test_type_ann_float() {
    assert_eq!(eval("let x: float = 3.14; x"), Value::Float(3.14));
    assert!(eval_err("let x: float = 42;").contains("type mismatch"));
}

#[test]
fn test_type_ann_bytes() {
    assert_eq!(
        eval(r#"let x: bytes = b"hello"; type_of(x)"#),
        Value::Str("bytes".to_string())
    );
    assert!(eval_err(r#"let x: bytes = "hello";"#).contains("type mismatch"));
}

#[test]
fn test_type_ann_dict() {
    assert_eq!(eval(r#"let d: dict = #{"a": 1}; d["a"]"#), Value::Int(1));
    assert!(eval_err("let d: dict = [1, 2];").contains("type mismatch"));
}

#[test]
fn test_type_ann_set() {
    assert_eq!(eval("let s: set = set([1, 2, 3]); s.len()"), Value::Int(3));
    assert!(eval_err("let s: set = [1, 2];").contains("type mismatch"));
}

#[test]
fn test_type_ann_fn() {
    assert_eq!(eval("let f: fn = |x| x + 1; f(2)"), Value::Int(3));
    assert!(eval_err("let f: fn = 42;").contains("type mismatch"));
}

#[test]
fn test_type_ann_option() {
    assert_eq!(
        eval("let x: Option<int> = Some(42); x"),
        Value::Option(Some(Box::new(Value::Int(42))))
    );
    assert!(eval_err("let x: Option<int> = 42;").contains("type mismatch"));
}

#[test]
fn test_type_ann_result() {
    assert_eq!(
        eval("let x: Result<int, string> = Ok(1); x"),
        Value::Result(Ok(Box::new(Value::Int(1))))
    );
    assert!(eval_err(r#"let x: Result<int, string> = "hello";"#).contains("type mismatch"));
}

#[test]
fn test_type_ann_generic_list() {
    // Generic parameter is documentation-only; only outer type is checked
    assert_eq!(
        eval(r#"let xs: list<int> = ["a", "b"]; xs.len()"#),
        Value::Int(2)
    );
    assert!(eval_err("let xs: list<int> = 42;").contains("type mismatch"));
}

#[test]
fn test_type_ann_generic_dict() {
    assert_eq!(
        eval(r#"let d: dict<string, int> = #{"a": 1}; d["a"]"#),
        Value::Int(1)
    );
    assert!(eval_err(r#"let d: dict<string, int> = [1];"#).contains("type mismatch"));
}

#[test]
fn test_type_ann_tuple() {
    assert_eq!(
        eval("let t: tuple = (1, 2, 3); t"),
        Value::Tuple(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert!(eval_err("let t: tuple = [1, 2];").contains("type mismatch"));
}

// ---- Cell (mutable closure state) tests ----

#[test]
fn test_cell_basic() {
    assert_eq!(eval("let c = cell(0); c.get()"), Value::Int(0));
    assert_eq!(eval("let c = cell(0); c.set(42); c.get()"), Value::Int(42));
}

#[test]
fn test_cell_update() {
    assert_eq!(
        eval("let c = cell(0); c.update(|x| x + 1); c.get()"),
        Value::Int(1)
    );
}

#[test]
fn test_cell_counter_closure() {
    assert_eq!(
        eval(
            r#"
        let count = cell(0);
        let inc = || { count.update(|x| x + 1) };
        inc();
        inc();
        inc();
        count.get()
    "#
        ),
        Value::Int(3)
    );
}

#[test]
fn test_cell_shared_between_closures() {
    assert_eq!(
        eval(
            r#"
        let state = cell(0);
        let inc = || { state.update(|x| x + 1) };
        let get = || { state.get() };
        inc();
        inc();
        get()
    "#
        ),
        Value::Int(2)
    );
}

#[test]
fn test_cell_accumulator() {
    assert_eq!(
        eval(
            r#"
        let acc = cell([]);
        let add = |item| { acc.update(|xs| xs.push(item)) };
        add(1);
        add(2);
        add(3);
        acc.get()
    "#
        ),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_cell_factory_pattern() {
    assert_eq!(
        eval(
            r#"
        fn make_counter() {
            let count = cell(0);
            let inc = || { count.update(|x| x + 1) };
            let get = || { count.get() };
            (inc, get)
        }
        let (inc, get) = make_counter();
        inc();
        inc();
        inc();
        get()
    "#
        ),
        Value::Int(3)
    );
}

#[test]
fn test_cell_display() {
    assert_eq!(eval(r#"f"{cell(42)}""#), Value::Str("cell(42)".to_string()));
}

#[test]
fn test_cell_type_of() {
    assert_eq!(eval("type_of(cell(0))"), Value::Str("cell".to_string()));
}

#[test]
fn test_cell_type_annotation() {
    assert_eq!(eval("let c: cell = cell(0); c.get()"), Value::Int(0));
    assert!(eval_err("let c: cell = 42;").contains("type mismatch"));
}

#[test]
fn test_cell_update_returns_new_value() {
    assert_eq!(
        eval("let c = cell(10); c.update(|x| x * 2)"),
        Value::Int(20)
    );
}

// --- Module / Namespace System ---

fn engine_with_math_module() -> Engine {
    let mut engine = Engine::new();
    let mut math = Module::new(ion_core::h!("math"));
    math.register_fn(ion_core::h!("add"), |args: &[Value]| {
        match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            _ => Err("expected two ints".to_string()),
        }
    });
    math.set(ion_core::h!("PI"), Value::Float(std::f64::consts::PI));
    engine.register_module(math);
    engine
}

#[test]
fn test_module_path_access() {
    let mut engine = engine_with_math_module();
    assert_eq!(engine.eval("math::add(1, 2)").unwrap(), Value::Int(3));
}

#[test]
fn test_module_constant_access() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("math::PI").unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn test_use_single_import() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::add; add(3, 4)").unwrap(),
        Value::Int(7)
    );
}

#[test]
fn test_use_glob_import() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::*; add(10, 20)").unwrap(),
        Value::Int(30)
    );
}

#[test]
fn test_use_named_imports() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::{add, PI}; add(1, 2)").unwrap(),
        Value::Int(3)
    );
}

#[test]
fn test_use_named_imports_constant() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::{PI}; PI").unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn test_module_submodule() {
    let mut engine = Engine::new();
    let mut net = Module::new(ion_core::h!("net"));
    let mut http = Module::new(ion_core::h!("http"));
    http.register_fn(ion_core::h!("get"), |_args: &[Value]| {
        Ok(Value::Str("response".to_string()))
    });
    net.register_submodule(http);
    engine.register_module(net);
    assert_eq!(
        engine.eval("net::http::get()").unwrap(),
        Value::Str("response".to_string())
    );
}

#[test]
fn test_use_from_submodule() {
    let mut engine = Engine::new();
    let mut net = Module::new(ion_core::h!("net"));
    let mut http = Module::new(ion_core::h!("http"));
    http.register_fn(ion_core::h!("get"), |_args: &[Value]| {
        Ok(Value::Str("ok".to_string()))
    });
    net.register_submodule(http);
    engine.register_module(net);
    assert_eq!(
        engine.eval("use net::http::get; get()").unwrap(),
        Value::Str("ok".to_string())
    );
}

#[test]
fn test_use_glob_from_submodule() {
    let mut engine = Engine::new();
    let mut net = Module::new(ion_core::h!("net"));
    let mut http = Module::new(ion_core::h!("http"));
    http.register_fn(ion_core::h!("get"), |_args: &[Value]| {
        Ok(Value::Str("ok".to_string()))
    });
    http.set(ion_core::h!("PORT"), Value::Int(8080));
    net.register_submodule(http);
    engine.register_module(net);
    assert_eq!(
        engine.eval("use net::http::*; get()").unwrap(),
        Value::Str("ok".to_string())
    );
    assert_eq!(
        engine.eval("use net::http::*; PORT").unwrap(),
        Value::Int(8080)
    );
}

#[test]
fn test_module_undefined_error() {
    let engine_result = Engine::new().eval("foo::bar");
    assert!(engine_result.is_err());
    assert!(engine_result
        .unwrap_err()
        .message
        .contains("undefined module"));
}

#[test]
fn test_module_member_not_found_error() {
    let mut engine = engine_with_math_module();
    let result = engine.eval("math::nonexistent");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("not found in module"));
}

#[test]
fn test_use_member_not_found_error() {
    let mut engine = engine_with_math_module();
    let result = engine.eval("use math::nonexistent;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("not found in module"));
}

// --- Aliased `use` imports ---

#[test]
fn test_use_single_alias() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::add as sum; sum(3, 4)").unwrap(),
        Value::Int(7)
    );
}

#[test]
fn test_use_named_alias_partial() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine
            .eval("use math::{add as sum, PI}; sum(1, 2)")
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        engine.eval("use math::{add as sum, PI}; PI").unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn test_use_named_alias_all() {
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine
            .eval("use math::{add as sum, PI as pi}; sum(1, 2)")
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        engine.eval("use math::{add as sum, PI as pi}; pi").unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn test_use_alias_self() {
    // `as add` is allowed but redundant; should behave identically to no alias
    let mut engine = engine_with_math_module();
    assert_eq!(
        engine.eval("use math::add as add; add(2, 5)").unwrap(),
        Value::Int(7)
    );
}

#[test]
fn test_use_alias_original_unbound() {
    // After aliasing, the original name must NOT be bound in the local scope.
    let mut engine = engine_with_math_module();
    let result = engine.eval("use math::add as sum; add(1, 2)");
    assert!(result.is_err(), "expected `add` to be unbound after alias");
}

#[test]
fn test_use_alias_submodule() {
    let mut engine = Engine::new();
    let mut net = Module::new(ion_core::h!("net"));
    let mut http = Module::new(ion_core::h!("http"));
    http.register_fn(ion_core::h!("get"), |_args: &[Value]| {
        Ok(Value::Str("ok".to_string()))
    });
    net.register_submodule(http);
    engine.register_module(net);
    assert_eq!(
        engine.eval("use net::http::get as fetch; fetch()").unwrap(),
        Value::Str("ok".to_string())
    );
}

#[test]
fn test_use_alias_missing_member_reports_original_name() {
    // Lookup uses the original name; the error message must surface it,
    // not the alias.
    let mut engine = engine_with_math_module();
    let result = engine.eval("use math::nonexistent as x;");
    assert!(result.is_err());
    let err = result.unwrap_err().message;
    assert!(
        err.contains("nonexistent"),
        "error should name the original member, got: {err}"
    );
    assert!(err.contains("not found in module"));
}

#[test]
fn test_use_glob_alias_is_parse_error() {
    // `use m::* as foo;` is not a valid form.
    let mut engine = engine_with_math_module();
    assert!(engine.eval("use math::* as foo;").is_err());
}

// --- Stdlib Modules ---

#[test]
fn test_stdlib_math_constants() {
    let pi = eval("math::PI");
    assert!(matches!(pi, Value::Float(f) if (f - std::f64::consts::PI).abs() < 1e-10));
    let e = eval("math::E");
    assert!(matches!(e, Value::Float(f) if (f - std::f64::consts::E).abs() < 1e-10));
    let tau = eval("math::TAU");
    assert!(matches!(tau, Value::Float(f) if (f - std::f64::consts::TAU).abs() < 1e-10));
    assert_eq!(eval("math::INF > 999999999"), Value::Bool(true));
}

#[test]
fn test_stdlib_math_abs() {
    assert_eq!(eval("math::abs(-5)"), Value::Int(5));
    assert_eq!(eval("math::abs(3)"), Value::Int(3));
    assert_eq!(eval("math::abs(-2.5)"), Value::Float(2.5));
}

#[test]
fn test_stdlib_math_min_max() {
    assert_eq!(eval("math::min(3, 1, 2)"), Value::Int(1));
    assert_eq!(eval("math::max(3, 1, 2)"), Value::Int(3));
    assert_eq!(eval("math::min(1.5, 2.5)"), Value::Float(1.5));
    assert_eq!(eval("math::max(1.5, 2.5)"), Value::Float(2.5));
}

#[test]
fn test_stdlib_math_floor_ceil_round() {
    assert_eq!(eval("math::floor(3.7)"), Value::Float(3.0));
    assert_eq!(eval("math::ceil(3.2)"), Value::Float(4.0));
    assert_eq!(eval("math::round(3.5)"), Value::Float(4.0));
    assert_eq!(eval("math::round(3.4)"), Value::Float(3.0));
}

#[test]
fn test_stdlib_math_sqrt_pow() {
    assert_eq!(eval("math::sqrt(9.0)"), Value::Float(3.0));
    assert_eq!(eval("math::pow(2, 10)"), Value::Int(1024));
    assert_eq!(eval("math::pow(2.0, 0.5)"), Value::Float(2.0_f64.sqrt()));
}

#[test]
fn test_stdlib_math_clamp() {
    assert_eq!(eval("math::clamp(5, 1, 10)"), Value::Int(5));
    assert_eq!(eval("math::clamp(-1, 0, 10)"), Value::Int(0));
    assert_eq!(eval("math::clamp(20, 0, 10)"), Value::Int(10));
}

#[test]
fn test_stdlib_math_trig() {
    assert_eq!(eval("math::sin(0.0)"), Value::Float(0.0));
    assert_eq!(eval("math::cos(0.0)"), Value::Float(1.0));
    let tan_result = eval("math::tan(0.0)");
    assert!(matches!(tan_result, Value::Float(f) if f.abs() < 1e-10));
}

#[test]
fn test_stdlib_math_log() {
    assert_eq!(eval("math::log(1.0)"), Value::Float(0.0));
    assert_eq!(eval("math::log2(8.0)"), Value::Float(3.0));
    assert_eq!(eval("math::log10(100.0)"), Value::Float(2.0));
}

#[test]
fn test_stdlib_math_checks() {
    assert_eq!(eval("math::is_nan(math::NAN)"), Value::Bool(true));
    assert_eq!(eval("math::is_nan(1.0)"), Value::Bool(false));
    assert_eq!(eval("math::is_inf(math::INF)"), Value::Bool(true));
    assert_eq!(eval("math::is_inf(1.0)"), Value::Bool(false));
}

#[test]
fn test_stdlib_math_use_import() {
    assert_eq!(
        eval("use math::{sin, PI}; sin(PI)"),
        Value::Float(std::f64::consts::PI.sin())
    );
    assert_eq!(eval("use math::*; abs(-42)"), Value::Int(42));
}

#[test]
fn test_stdlib_json_encode_decode() {
    let val = eval(r#"json::encode(#{name: "ion", version: 1})"#);
    assert!(matches!(val, Value::Str(_)));
    let val = eval(r#"json::decode("{\"x\": 42}")"#);
    if let Value::Dict(map) = val {
        assert_eq!(map["x"], Value::Int(42));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_stdlib_json_pretty() {
    let result = eval(r#"json::pretty(#{a: 1})"#);
    match &result {
        Value::Str(s) => assert!(s.contains('\n')),
        _ => panic!("expected string"),
    }
}

#[test]
fn test_stdlib_json_use_import() {
    assert_eq!(
        eval(r#"use json::encode; encode([1, 2, 3])"#),
        Value::Str("[1,2,3]".to_string()),
    );
}

#[test]
fn test_stdlib_io_eprintln() {
    let output = Arc::new(CaptureOutput::default());
    let mut engine = Engine::with_output_handler(output.clone());

    assert_eq!(engine.eval(r#"io::eprintln("test")"#).unwrap(), Value::Unit);
    assert_eq!(output.stderr.lock().unwrap().as_str(), "test\n");
}

#[test]
fn test_stdlib_io_use_import() {
    let output = Arc::new(CaptureOutput::default());
    let mut engine = Engine::with_output_handler(output.clone());

    assert_eq!(
        engine
            .eval(r#"use io::println; println("hello", 42)"#)
            .unwrap(),
        Value::Unit
    );
    assert_eq!(output.stdout.lock().unwrap().as_str(), "hello 42\n");
}

#[test]
fn test_stdlib_io_print_requires_output_handler() {
    let mut engine = Engine::new();
    let err = engine.eval(r#"io::println("hello")"#).unwrap_err();
    assert!(err.message.contains("output handler is not configured"));
}

#[cfg(feature = "vm")]
#[test]
fn test_stdlib_io_vm_uses_output_handler() {
    let output = Arc::new(CaptureOutput::default());
    let mut engine = Engine::with_output_handler(output.clone());

    assert_eq!(engine.vm_eval(r#"io::println("vm")"#).unwrap(), Value::Unit);
    assert_eq!(output.stdout.lock().unwrap().as_str(), "vm\n");
}

// ============================================================
// register_closure — captures host state across calls
// ============================================================

#[test]
fn test_register_closure_captures_counter() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicU64::new(0));
    let counter_for_closure = counter.clone();

    let mut engine = Engine::new();
    engine.register_closure(ion_core::h!("host_tick"), move |_args| {
        let n = counter_for_closure.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(Value::Int(n as i64))
    });

    // Each call returns the post-increment value and bumps the
    // host-side counter. This proves the closure captured `counter`.
    assert_eq!(engine.eval("host_tick()").unwrap(), Value::Int(1));
    assert_eq!(engine.eval("host_tick()").unwrap(), Value::Int(2));
    assert_eq!(
        engine.eval("host_tick() + host_tick()").unwrap(),
        Value::Int(7)
    );
    assert_eq!(counter.load(Ordering::Relaxed), 4);
}

#[test]
fn test_register_closure_type_matches_fn() {
    // The closure-backed variant should satisfy `: fn` type annotations
    // exactly like a plain BuiltinFn does.
    let mut engine = Engine::new();
    engine.register_closure(ion_core::h!("noop"), |_| Ok(Value::Unit));
    assert_eq!(engine.eval("let f: fn = noop; f()").unwrap(), Value::Unit);
}

#[test]
fn test_failed_call_does_not_leak_function_scope() {
    let mut engine = Engine::new();
    engine.eval("fn f(a, b) { 1 }").unwrap();
    assert!(engine.eval("f(1)").is_err());
    let err = engine.eval("a").unwrap_err();
    assert!(
        err.message.contains("undefined variable"),
        "got: {}",
        err.message
    );
}

#[cfg(feature = "vm")]
#[test]
fn test_vm_preserves_host_builtin_shadowing() {
    let mut engine = Engine::new();
    engine.register_fn(ion_core::h!("len"), |_args| Ok(Value::Int(99)));
    assert_eq!(engine.vm_eval("len([1, 2, 3])").unwrap(), Value::Int(99));
}

// --- semver stdlib ---

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_parse() {
    let v = eval(r#"semver::parse("1.2.3-alpha.1+build.42")"#);
    let Value::Dict(map) = v else {
        panic!("expected dict, got {:?}", v);
    };
    assert_eq!(map.get("major"), Some(&Value::Int(1)));
    assert_eq!(map.get("minor"), Some(&Value::Int(2)));
    assert_eq!(map.get("patch"), Some(&Value::Int(3)));
    assert_eq!(map.get("pre"), Some(&Value::Str("alpha.1".to_string())));
    assert_eq!(map.get("build"), Some(&Value::Str("build.42".to_string())));
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_parse_invalid() {
    // Phase 7 cleanup: generic error message; underlying parse error
    // text from the `semver` crate is preserved.
    let mut engine = Engine::new();
    let result = engine.eval(r#"semver::parse("not.a.version")"#);
    assert!(result.is_err());
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_is_valid() {
    assert_eq!(eval(r#"semver::is_valid("1.2.3")"#), Value::Bool(true));
    assert_eq!(
        eval(r#"semver::is_valid("1.2.3-rc.1+meta")"#),
        Value::Bool(true)
    );
    assert_eq!(eval(r#"semver::is_valid("1.2")"#), Value::Bool(false));
    assert_eq!(eval(r#"semver::is_valid("abc")"#), Value::Bool(false));
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_format() {
    // string → string round trip preserves canonical form
    assert_eq!(
        eval(r#"semver::format("1.2.3-rc.1")"#),
        Value::Str("1.2.3-rc.1".to_string())
    );
    // dict → string
    assert_eq!(
        eval(r#"semver::format(#{major: 1, minor: 2, patch: 3, pre: "", build: ""})"#),
        Value::Str("1.2.3".to_string())
    );
    assert_eq!(
        eval(r#"semver::format(#{major: 0, minor: 1, patch: 0, pre: "alpha", build: ""})"#),
        Value::Str("0.1.0-alpha".to_string())
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_compare() {
    assert_eq!(eval(r#"semver::compare("1.2.3", "1.2.4")"#), Value::Int(-1));
    assert_eq!(eval(r#"semver::compare("1.2.3", "1.2.3")"#), Value::Int(0));
    assert_eq!(eval(r#"semver::compare("2.0.0", "1.9.9")"#), Value::Int(1));
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_compare_pre_release() {
    // Pre-release versions are *less than* the corresponding release
    assert_eq!(
        eval(r#"semver::compare("1.0.0-alpha", "1.0.0")"#),
        Value::Int(-1)
    );
    assert_eq!(
        eval(r#"semver::compare("1.0.0-alpha", "1.0.0-beta")"#),
        Value::Int(-1)
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_comparators() {
    assert_eq!(eval(r#"semver::eq("1.2.3", "1.2.3")"#), Value::Bool(true));
    assert_eq!(eval(r#"semver::eq("1.2.3", "1.2.4")"#), Value::Bool(false));
    assert_eq!(eval(r#"semver::gt("2.0.0", "1.9.9")"#), Value::Bool(true));
    assert_eq!(eval(r#"semver::gt("1.0.0", "1.0.0")"#), Value::Bool(false));
    assert_eq!(eval(r#"semver::gte("1.0.0", "1.0.0")"#), Value::Bool(true));
    assert_eq!(eval(r#"semver::lt("1.0.0", "2.0.0")"#), Value::Bool(true));
    assert_eq!(eval(r#"semver::lte("1.0.0", "1.0.0")"#), Value::Bool(true));
    assert_eq!(eval(r#"semver::lte("2.0.0", "1.0.0")"#), Value::Bool(false));
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_satisfies_caret() {
    assert_eq!(
        eval(r#"semver::satisfies("1.5.0", "^1.0")"#),
        Value::Bool(true)
    );
    assert_eq!(
        eval(r#"semver::satisfies("2.0.0", "^1.0")"#),
        Value::Bool(false)
    );
    assert_eq!(
        eval(r#"semver::satisfies("0.9.9", "^1.0")"#),
        Value::Bool(false)
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_satisfies_tilde() {
    assert_eq!(
        eval(r#"semver::satisfies("1.2.9", "~1.2")"#),
        Value::Bool(true)
    );
    assert_eq!(
        eval(r#"semver::satisfies("1.3.0", "~1.2")"#),
        Value::Bool(false)
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_satisfies_invalid_req() {
    // Phase 7 cleanup: generic error message; the offending requirement
    // text is captured via the upstream semver crate's error.
    let mut engine = Engine::new();
    let result = engine.eval(r#"semver::satisfies("1.0.0", "garbage??")"#);
    assert!(result.is_err());
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_bump_major_clears_pre_build() {
    // Pre-release and build metadata must be discarded; minor/patch zeroed
    assert_eq!(
        eval(r#"semver::bump_major("1.2.3-rc.1+meta")"#),
        Value::Str("2.0.0".to_string())
    );
    assert_eq!(
        eval(r#"semver::bump_major("0.9.9")"#),
        Value::Str("1.0.0".to_string())
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_bump_minor() {
    assert_eq!(
        eval(r#"semver::bump_minor("1.2.3")"#),
        Value::Str("1.3.0".to_string())
    );
    assert_eq!(
        eval(r#"semver::bump_minor("1.2.3-rc.1")"#),
        Value::Str("1.3.0".to_string())
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_bump_patch() {
    // Plain version → patch increments
    assert_eq!(
        eval(r#"semver::bump_patch("1.2.3")"#),
        Value::Str("1.2.4".to_string())
    );
    // Pre-release present → strip pre-release, keep numeric triple
    assert_eq!(
        eval(r#"semver::bump_patch("1.2.3-alpha")"#),
        Value::Str("1.2.3".to_string())
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_accepts_dict_arg() {
    // Comparing/formatting against a parsed dict should work the same as
    // against a string. This is the "parse once, reuse many times" path.
    assert_eq!(
        eval(r#"semver::compare(semver::parse("1.0.0"), "1.0.1")"#),
        Value::Int(-1)
    );
    assert_eq!(
        eval(r#"semver::format(semver::parse("1.2.3-rc.1+meta"))"#),
        Value::Str("1.2.3-rc.1+meta".to_string())
    );
    assert_eq!(
        eval(r#"semver::satisfies(semver::parse("1.5.0"), "^1.0")"#),
        Value::Bool(true)
    );
}

#[cfg(feature = "semver")]
#[test]
fn test_stdlib_semver_use_import() {
    // Aliased imports work — exercises both the alias feature shipped in
    // 0.5.0 and that semver functions register correctly under `use`.
    let mut engine = Engine::new();
    assert_eq!(
        engine
            .eval(r#"use semver::{satisfies as ok}; ok("1.2.3", "^1.0")"#)
            .unwrap(),
        Value::Bool(true)
    );
}

// --- os stdlib ---

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_name_nonempty() {
    let v = eval("os::name");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_arch_nonempty() {
    let v = eval("os::arch");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_family_is_known() {
    let v = eval("os::family");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(s == "unix" || s == "windows", "unexpected family: {}", s);
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_pointer_width() {
    assert!(matches!(
        eval("os::pointer_width"),
        Value::Int(32) | Value::Int(64)
    ));
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_extensions() {
    // Both extension constants are strings (possibly empty on Unix for exe).
    assert!(matches!(eval("os::dll_extension"), Value::Str(_)));
    assert!(matches!(eval("os::exe_extension"), Value::Str(_)));
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_env_var_present() {
    // Use a unique name so parallel tests don't race.
    // SAFETY: set_var is unsafe in newer Rust editions; we're on 2021.
    std::env::set_var("ION_OS_TEST_PRESENT_xK7", "hello");
    assert_eq!(
        eval(r#"os::env_var("ION_OS_TEST_PRESENT_xK7")"#),
        Value::Str("hello".to_string())
    );
    std::env::remove_var("ION_OS_TEST_PRESENT_xK7");
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_env_var_missing() {
    // Phase 7 cleanup: error literals are generic; the variable name
    // appears in the formatted error since it's user data.
    let mut engine = Engine::new();
    let result = engine.eval(r#"os::env_var("ION_DEFINITELY_UNSET_zP9_NEVER_EXISTS")"#);
    let msg = result.unwrap_err().message;
    assert!(
        msg.contains("ION_DEFINITELY_UNSET_zP9_NEVER_EXISTS"),
        "got: {}",
        msg
    );
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_env_var_default() {
    // 2-arg form returns the default when the var is absent.
    assert_eq!(
        eval(r#"os::env_var("ION_DEFINITELY_UNSET_zP9_NEVER_EXISTS", "fallback")"#),
        Value::Str("fallback".to_string())
    );
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_has_env_var() {
    std::env::set_var("ION_OS_TEST_HAS_xK7", "1");
    assert_eq!(
        eval(r#"os::has_env_var("ION_OS_TEST_HAS_xK7")"#),
        Value::Bool(true)
    );
    assert_eq!(
        eval(r#"os::has_env_var("ION_DEFINITELY_UNSET_zP9_NEVER_EXISTS")"#),
        Value::Bool(false)
    );
    std::env::remove_var("ION_OS_TEST_HAS_xK7");
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_env_vars_dict() {
    // Should return a dict; the test process always has at least one env var
    // (PATH on Unix, Path on Windows; even the Cargo runner sets a bunch).
    let v = eval("os::env_vars()");
    let Value::Dict(map) = v else {
        panic!("expected dict, got {:?}", v);
    };
    assert!(!map.is_empty(), "env_vars() returned an empty dict");
    // All values should be strings.
    for (_, val) in &map {
        assert!(
            matches!(val, Value::Str(_)),
            "env_vars value not a string: {:?}",
            val
        );
    }
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_cwd() {
    let v = eval("os::cwd()");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_pid() {
    let v = eval("os::pid()");
    let Value::Int(pid) = v else {
        panic!("expected int, got {:?}", v);
    };
    assert!(pid > 0, "pid should be positive, got {}", pid);
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_temp_dir() {
    let v = eval("os::temp_dir()");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_args_default_empty() {
    // Without `set_args`, `os::args()` returns an empty list.
    let mut engine = Engine::new();
    let result = engine.eval("os::args()").unwrap();
    assert_eq!(result, Value::List(Vec::new()));
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_args_after_set_args() {
    let mut engine = Engine::new();
    engine.set_args(vec!["alpha".into(), "beta".into(), "--flag".into()]);
    let result = engine.eval("os::args()").unwrap();
    assert_eq!(
        result,
        Value::List(vec![
            Value::Str("alpha".into()),
            Value::Str("beta".into()),
            Value::Str("--flag".into()),
        ])
    );
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_set_args_rebinds_module() {
    // After re-injection, subsequent calls see the updated args. This
    // verifies `set_args` re-registers the `os::` module rather than
    // updating stale closure captures.
    let mut engine = Engine::new();
    engine.set_args(vec!["first".into()]);
    assert_eq!(
        engine.eval("os::args()").unwrap(),
        Value::List(vec![Value::Str("first".into())])
    );
    engine.set_args(vec!["second".into(), "third".into()]);
    assert_eq!(
        engine.eval("os::args()").unwrap(),
        Value::List(vec![
            Value::Str("second".into()),
            Value::Str("third".into()),
        ])
    );
}

#[cfg(feature = "os")]
#[test]
fn test_stdlib_os_arity_errors() {
    // No-arg functions reject extra arguments.
    let mut engine = Engine::new();
    assert!(engine.eval("os::pid(1)").is_err());
    assert!(engine.eval("os::cwd(1)").is_err());
    assert!(engine.eval("os::env_vars(1)").is_err());
    assert!(engine.eval("os::env_var()").is_err());
    assert!(engine.eval(r#"os::env_var("a", "b", "c")"#).is_err());
}

// --- path stdlib ---

#[test]
fn test_stdlib_path_sep_nonempty() {
    let v = eval("path::sep");
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
}

#[test]
fn test_stdlib_path_join() {
    // Use platform-agnostic assertions: build with `path::sep` so the test
    // works on both Unix and Windows.
    let v = eval(r#"path::join("a", "b", "c")"#);
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(s.contains('a') && s.contains('b') && s.contains('c'));
    assert!(s.starts_with("a"));
}

#[test]
fn test_stdlib_path_basename_stem_extension() {
    assert_eq!(
        eval(r#"path::basename("/usr/local/bin/ion")"#),
        Value::Str("ion".to_string())
    );
    assert_eq!(
        eval(r#"path::stem("config.toml")"#),
        Value::Str("config".to_string())
    );
    assert_eq!(
        eval(r#"path::extension("config.toml")"#),
        Value::Str("toml".to_string())
    );
    assert_eq!(
        eval(r#"path::extension("README")"#),
        Value::Str("".to_string())
    );
}

#[test]
fn test_stdlib_path_with_extension() {
    assert_eq!(
        eval(r#"path::with_extension("foo.txt", "md")"#),
        Value::Str("foo.md".to_string())
    );
    // Empty extension drops the dot.
    assert_eq!(
        eval(r#"path::with_extension("foo.txt", "")"#),
        Value::Str("foo".to_string())
    );
}

#[test]
fn test_stdlib_path_parent() {
    let v = eval(r#"path::parent("/a/b/c")"#);
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(s.ends_with("b"));
}

#[test]
fn test_stdlib_path_is_absolute_relative() {
    // Use a leading separator so the test works cross-platform when paths
    // happen to be checked with the OS's notion of absolute. On Unix `/x`
    // is absolute; on Windows it's a relative-on-current-drive path. Skip
    // strict cross-platform assertion and just confirm the two predicates
    // are inverses on the same input.
    let v_abs = eval(r#"path::is_absolute("relative/dir")"#);
    let v_rel = eval(r#"path::is_relative("relative/dir")"#);
    assert_eq!(v_abs, Value::Bool(false));
    assert_eq!(v_rel, Value::Bool(true));
}

#[test]
fn test_stdlib_path_components() {
    let v = eval(r#"path::components("a/b/c")"#);
    let Value::List(items) = v else {
        panic!("expected list, got {:?}", v);
    };
    let strings: Vec<String> = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            _ => panic!("expected strings"),
        })
        .collect();
    assert_eq!(strings, vec!["a", "b", "c"]);
}

#[test]
fn test_stdlib_path_normalize() {
    assert_eq!(
        eval(r#"path::normalize("a/./b/../c")"#),
        Value::Str("a/c".to_string())
    );
    // Bare `.` collapses to `.`
    assert_eq!(eval(r#"path::normalize(".")"#), Value::Str(".".to_string()));
    // Leading `..` is preserved when there's nothing to pop
    assert_eq!(
        eval(r#"path::normalize("../a")"#),
        Value::Str("../a".to_string())
    );
}

#[test]
fn test_stdlib_path_arity_errors() {
    let mut engine = Engine::new();
    assert!(engine.eval("path::join()").is_err());
    assert!(engine.eval("path::basename()").is_err());
    assert!(engine.eval(r#"path::with_extension("a")"#).is_err());
}

// --- fs stdlib (sync build) ---
//
// Each test creates a uniquely-named temp directory and cleans up at the
// end. Names use a counter suffix to avoid races when tests run in parallel.

#[cfg(feature = "fs")]
fn fs_test_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ion_fs_test_{}_{}_{}",
        label,
        std::process::id(),
        id
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_read_write_roundtrip() {
    let dir = fs_test_dir("rw");
    let path = dir.join("hello.txt");
    let path_s = path.to_string_lossy().to_string();
    let mut engine = Engine::new();
    engine
        .eval(&format!(
            r#"fs::write("{}", "hello"); fs::read("{}")"#,
            path_s, path_s
        ))
        .unwrap();
    assert_eq!(
        engine.eval(&format!(r#"fs::read("{}")"#, path_s)).unwrap(),
        Value::Str("hello".to_string())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_read_bytes() {
    let dir = fs_test_dir("bytes");
    let path = dir.join("blob.bin");
    std::fs::write(&path, [0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    let path_s = path.to_string_lossy().to_string();
    let v = eval(&format!(r#"fs::read_bytes("{}")"#, path_s));
    let Value::Bytes(b) = v else {
        panic!("expected bytes, got {:?}", v);
    };
    assert_eq!(b, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_write_bytes() {
    let dir = fs_test_dir("writebytes");
    let path = dir.join("out.bin");
    let path_s = path.to_string_lossy().to_string();
    // Write Value::Bytes via `bytes_from_hex` builtin.
    let mut engine = Engine::new();
    engine
        .eval(&format!(
            r#"fs::write("{}", bytes_from_hex("deadbeef"))"#,
            path_s
        ))
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_append() {
    let dir = fs_test_dir("append");
    let path = dir.join("log.txt");
    let path_s = path.to_string_lossy().to_string();
    let mut engine = Engine::new();
    engine
        .eval(&format!(
            r#"fs::write("{}", "a"); fs::append("{}", "b"); fs::append("{}", "c")"#,
            path_s, path_s, path_s
        ))
        .unwrap();
    assert_eq!(
        engine.eval(&format!(r#"fs::read("{}")"#, path_s)).unwrap(),
        Value::Str("abc".to_string())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_exists_is_file_is_dir() {
    let dir = fs_test_dir("exists");
    let dir_s = dir.to_string_lossy().to_string();
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let file_s = file.to_string_lossy().to_string();
    let missing_s = dir.join("nope.txt").to_string_lossy().to_string();
    let mut engine = Engine::new();
    assert_eq!(
        engine
            .eval(&format!(r#"fs::exists("{}")"#, file_s))
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .eval(&format!(r#"fs::exists("{}")"#, missing_s))
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        engine
            .eval(&format!(r#"fs::is_file("{}")"#, file_s))
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        engine.eval(&format!(r#"fs::is_dir("{}")"#, dir_s)).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .eval(&format!(r#"fs::is_dir("{}")"#, file_s))
            .unwrap(),
        Value::Bool(false)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_list_dir() {
    let dir = fs_test_dir("list");
    let dir_s = dir.to_string_lossy().to_string();
    std::fs::write(dir.join("a"), "").unwrap();
    std::fs::write(dir.join("b"), "").unwrap();
    let v = eval(&format!(r#"fs::list_dir("{}")"#, dir_s));
    let Value::List(items) = v else {
        panic!("expected list");
    };
    let mut names: Vec<String> = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            _ => panic!("expected strings"),
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_create_remove_dirs() {
    let root = fs_test_dir("createrm");
    let nested = root.join("a/b/c");
    let nested_s = nested.to_string_lossy().to_string();
    let mut engine = Engine::new();
    engine
        .eval(&format!(r#"fs::create_dir_all("{}")"#, nested_s))
        .unwrap();
    assert!(nested.exists());
    engine
        .eval(&format!(
            r#"fs::remove_dir_all("{}")"#,
            root.to_string_lossy()
        ))
        .unwrap();
    assert!(!root.exists());
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_rename_copy() {
    let dir = fs_test_dir("renamecopy");
    let src = dir.join("src.txt");
    let dst = dir.join("dst.txt");
    let copy = dir.join("copy.txt");
    std::fs::write(&src, "data").unwrap();
    let mut engine = Engine::new();
    engine
        .eval(&format!(
            r#"fs::rename("{}", "{}")"#,
            src.to_string_lossy(),
            dst.to_string_lossy()
        ))
        .unwrap();
    assert!(!src.exists() && dst.exists());
    let n = engine
        .eval(&format!(
            r#"fs::copy("{}", "{}")"#,
            dst.to_string_lossy(),
            copy.to_string_lossy()
        ))
        .unwrap();
    assert_eq!(n, Value::Int(4));
    assert!(copy.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_metadata() {
    let dir = fs_test_dir("meta");
    let file = dir.join("m.txt");
    std::fs::write(&file, "12345").unwrap();
    let v = eval(&format!(r#"fs::metadata("{}")"#, file.to_string_lossy()));
    let Value::Dict(map) = v else {
        panic!("expected dict, got {:?}", v);
    };
    assert_eq!(map.get("size"), Some(&Value::Int(5)));
    assert_eq!(map.get("is_file"), Some(&Value::Bool(true)));
    assert_eq!(map.get("is_dir"), Some(&Value::Bool(false)));
    assert!(matches!(map.get("readonly"), Some(&Value::Bool(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_canonicalize() {
    let dir = fs_test_dir("canonicalize");
    let file = dir.join("c.txt");
    std::fs::write(&file, "").unwrap();
    let v = eval(&format!(
        r#"fs::canonicalize("{}")"#,
        file.to_string_lossy()
    ));
    let Value::Str(s) = v else {
        panic!("expected string, got {:?}", v);
    };
    assert!(!s.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_stdlib_fs_read_missing_file_errors() {
    // Phase 7 cleanup: stdlib closures emit generic errors so no
    // `mod::fn` literal lands in `.rodata`. The span carries call-site
    // info; the path itself appears in the io error string.
    let mut engine = Engine::new();
    let result = engine.eval(r#"fs::read("/this/path/should/not/exist/anywhere/xK7")"#);
    let err = result.unwrap_err();
    assert!(err.message.contains("xK7"), "got: {}", err.message);
}
