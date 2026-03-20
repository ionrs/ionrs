use ion_core::engine::Engine;
use ion_core::host_types::{HostStructDef, HostEnumDef, HostVariantDef, IonType};
use ion_core::interpreter::Limits;
use ion_core::value::Value;
use ion_core::IonType;

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
    assert!(msg.contains("immutable"), "expected immutable error, got: {}", msg);
}

#[test]
fn test_shadowing() {
    assert_eq!(eval("let x = 1; let x = 2; x"), Value::Int(2));
}

#[test]
fn test_shadowing_type_change() {
    assert_eq!(eval(r#"let x = 1; let x = "hello"; x"#), Value::Str("hello".into()));
}

#[test]
fn test_shadowing_freeze() {
    let msg = eval_err("let mut x = 1; let x = x; x = 3;");
    assert!(msg.contains("immutable"), "expected immutable error, got: {}", msg);
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
    assert!(msg.contains("undefined"), "expected undefined error, got: {}", msg);
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
    assert_eq!(eval(r#""hello" + " " + "world""#), Value::Str("hello world".into()));
}

#[test]
fn test_type_error_add() {
    let msg = eval_err(r#"1 + "hello""#);
    assert!(msg.contains("cannot apply"), "expected type error, got: {}", msg);
}

#[test]
fn test_division_by_zero() {
    let msg = eval_err("1 / 0");
    assert!(msg.contains("division by zero"), "expected div zero, got: {}", msg);
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
    assert_eq!(eval("
        fn f(x) {
            if x > 0 { return x; }
            0 - x
        }
        f(5)
    "), Value::Int(5));
    assert_eq!(eval("
        fn f(x) {
            if x > 0 { return x; }
            0 - x
        }
        f(-5)
    "), Value::Int(5));
}

#[test]
fn test_fn_default_args() {
    assert_eq!(eval("fn greet(name, greeting = \"hello\") { greeting + \" \" + name } greet(\"world\")"),
        Value::Str("hello world".into()));
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
    assert_eq!(eval("let x = 1; let f = |y| x + y; let x = 100; f(0)"), Value::Int(1));
}

#[test]
fn test_higher_order_fn() {
    assert_eq!(eval("fn apply(f, x) { f(x) } apply(|x| x * 3, 5)"), Value::Int(15));
}

#[test]
fn test_recursion() {
    assert_eq!(eval("
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "), Value::Int(55));
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
    assert_eq!(eval("let x = if 1 > 0 { 10 } else { 20 }; x"), Value::Int(10));
}

#[test]
fn test_else_if() {
    assert_eq!(eval("
        let x = 5;
        if x > 10 { \"big\" } else if x > 3 { \"medium\" } else { \"small\" }
    "), Value::Str("medium".into()));
}

#[test]
fn test_block_expr_returns_last() {
    assert_eq!(eval("let x = { let a = 1; let b = 2; a + b }; x"), Value::Int(3));
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
    assert_eq!(eval("
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            sum = sum + x;
        }
        sum
    "), Value::Int(15));
}

#[test]
fn test_for_range() {
    assert_eq!(eval("
        let mut sum = 0;
        for i in 0..5 {
            sum = sum + i;
        }
        sum
    "), Value::Int(10));
}

#[test]
fn test_while_loop() {
    assert_eq!(eval("
        let mut i = 0;
        while i < 5 {
            i = i + 1;
        }
        i
    "), Value::Int(5));
}

#[test]
fn test_loop_break() {
    assert_eq!(eval("
        let mut i = 0;
        loop {
            if i >= 3 { break; }
            i = i + 1;
        }
        i
    "), Value::Int(3));
}

#[test]
fn test_loop_break_value() {
    assert_eq!(eval("
        let result = loop {
            break 42;
        };
        result
    "), Value::Int(42));
}

#[test]
fn test_for_continue() {
    assert_eq!(eval("
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            if x == 3 { continue; }
            sum = sum + x;
        }
        sum
    "), Value::Int(12));
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
    assert!(msg.contains("immutable"), "expected immutable error, got: {}", msg);
}

// ============================================================
// Section 9: Match
// ============================================================

#[test]
fn test_match_int() {
    assert_eq!(eval(r#"match 2 { 1 => "one", 2 => "two", _ => "other" }"#),
        Value::Str("two".into()));
}

#[test]
fn test_match_wildcard() {
    assert_eq!(eval(r#"match 99 { 1 => "one", _ => "other" }"#),
        Value::Str("other".into()));
}

#[test]
fn test_match_with_guard() {
    assert_eq!(eval(r#"
        let score = 85;
        match score {
            s if s >= 90 => "A",
            s if s >= 80 => "B",
            _ => "C",
        }
    "#), Value::Str("B".into()));
}

#[test]
fn test_match_option() {
    assert_eq!(eval(r#"
        let x = Some(42);
        match x {
            Some(v) => v,
            None => 0,
        }
    "#), Value::Int(42));
}

#[test]
fn test_match_result() {
    assert_eq!(eval(r#"
        let x = Ok(10);
        match x {
            Ok(v) => v * 2,
            Err(e) => 0,
        }
    "#), Value::Int(20));
}

#[test]
fn test_match_nested() {
    assert_eq!(eval(r#"
        let x = Ok(Some(5));
        match x {
            Ok(Some(v)) => v,
            Ok(None) => 0,
            Err(e) => -1,
        }
    "#), Value::Int(5));
}

#[test]
fn test_non_exhaustive_match_error() {
    let msg = eval_err("match 5 { 1 => 10 }");
    assert!(msg.contains("non-exhaustive"), "expected non-exhaustive, got: {}", msg);
}

// ============================================================
// Section 10: Option & Result
// ============================================================

#[test]
fn test_some_none() {
    assert_eq!(eval("Some(42)"), Value::Option(Some(Box::new(Value::Int(42)))));
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
    // ? on Err should propagate
    let result = Engine::new().eval(r#"
        fn inner() { let x = Err("fail"); x? }
        inner()
    "#);
    assert!(result.is_err());
}

#[test]
fn test_question_mark_some() {
    assert_eq!(eval("fn f() { let x = Some(10); x? } f()"), Value::Int(10));
}

#[test]
fn test_question_mark_none_propagation() {
    let result = Engine::new().eval("fn f() { let x = None; x? } f()");
    assert!(result.is_err());
}

#[test]
fn test_question_mark_type_error() {
    let result = Engine::new().eval("fn f() { let x = 42; x? } f()");
    assert!(result.is_err());
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
    assert!(msg.contains("value missing"), "expected expect msg, got: {}", msg);
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
    assert_eq!(eval("
        let x = Some(42);
        if let Some(v) = x { v } else { 0 }
    "), Value::Int(42));
}

#[test]
fn test_if_let_no_match() {
    assert_eq!(eval("
        let x = None;
        if let Some(v) = x { v } else { 0 }
    "), Value::Int(0));
}

#[test]
fn test_while_let() {
    // Simulates popping from a list
    assert_eq!(eval("
        let mut items = [1, 2, 3];
        let mut sum = 0;
        while let [first, ...rest] = items {
            sum = sum + first;
            items = rest;
        }
        sum
    "), Value::Int(6));
}

// ============================================================
// Section 12: Lists
// ============================================================

#[test]
fn test_list_literal() {
    assert_eq!(eval("[1, 2, 3]"), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_list_map() {
    assert_eq!(eval("[1, 2, 3].map(|x| x * 2)"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)]));
}

#[test]
fn test_list_filter() {
    assert_eq!(eval("[1, 2, 3, 4, 5].filter(|x| x > 3)"),
        Value::List(vec![Value::Int(4), Value::Int(5)]));
}

#[test]
fn test_list_fold() {
    assert_eq!(eval("[1, 2, 3, 4].fold(0, |acc, x| acc + x)"), Value::Int(10));
}

#[test]
fn test_list_push_returns_new() {
    assert_eq!(eval("
        let a = [1, 2];
        let b = a.push(3);
        a
    "), Value::List(vec![Value::Int(1), Value::Int(2)]));
}

#[test]
fn test_list_push_result() {
    assert_eq!(eval("[1, 2].push(3)"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_list_len() {
    assert_eq!(eval("[1, 2, 3].len()"), Value::Int(3));
}

#[test]
fn test_list_first_last() {
    assert_eq!(eval("[10, 20, 30].first()"), Value::Option(Some(Box::new(Value::Int(10)))));
    assert_eq!(eval("[10, 20, 30].last()"), Value::Option(Some(Box::new(Value::Int(30)))));
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
    assert_eq!(eval("[1, 2, 3].reverse()"),
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)]));
}

#[test]
fn test_list_sort() {
    assert_eq!(eval("[3, 1, 2].sort()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_list_contains() {
    assert_eq!(eval("[1, 2, 3].contains(2)"), Value::Bool(true));
    assert_eq!(eval("[1, 2, 3].contains(5)"), Value::Bool(false));
}

#[test]
fn test_list_flatten() {
    assert_eq!(eval("[[1, 2], [3, 4]].flatten()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)]));
}

#[test]
fn test_list_zip() {
    assert_eq!(eval("[1, 2].zip([3, 4])"),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(1), Value::Int(3)]),
            Value::Tuple(vec![Value::Int(2), Value::Int(4)]),
        ]));
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
    assert_eq!(eval(r#"let d = #{ "x": 42 }; d["x"]"#),
        Value::Option(Some(Box::new(Value::Int(42)))));
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
    assert_eq!(eval(r#"#{ "a": 1, "b": 2 }.keys()"#),
        Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]));
    assert_eq!(eval(r#"#{ "a": 1, "b": 2 }.values()"#),
        Value::List(vec![Value::Int(1), Value::Int(2)]));
}

#[test]
fn test_dict_insert_returns_new() {
    let val = eval(r#"
        let d = #{ "a": 1 };
        d.insert("b", 2)
    "#);
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 2);
        assert_eq!(map["b"], Value::Int(2));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_remove_returns_new() {
    let val = eval(r#"
        let d = #{ "a": 1, "b": 2 };
        d.remove("a")
    "#);
    if let Value::Dict(map) = val {
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key("a"));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_dict_merge() {
    let val = eval(r#"
        let a = #{ "x": 1 };
        let b = #{ "y": 2 };
        a.merge(b)
    "#);
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
    assert_eq!(eval("(1, 2, 3)"), Value::Tuple(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
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
    assert_eq!(eval(r#"let name = "world"; f"hello {name}""#),
        Value::Str("hello world".into()));
}

#[test]
fn test_fstring_expr() {
    assert_eq!(eval(r#"f"result = {1 + 2}""#),
        Value::Str("result = 3".into()));
}

#[test]
fn test_regular_string_no_interp() {
    assert_eq!(eval(r#""hello {name}""#),
        Value::Str("hello {name}".into()));
}

// ============================================================
// Section 16: Ranges
// ============================================================

#[test]
fn test_range_exclusive() {
    assert_eq!(eval("0..3"),
        Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2)]));
}

#[test]
fn test_range_inclusive() {
    assert_eq!(eval("0..=3"),
        Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2), Value::Int(3)]));
}

// ============================================================
// Section 17: Pipe Operator
// ============================================================

#[test]
fn test_pipe_basic() {
    assert_eq!(eval("
        fn double(x) { x * 2 }
        5 |> double()
    "), Value::Int(10));
}

#[test]
fn test_pipe_chain() {
    assert_eq!(eval("
        fn add(x, y) { x + y }
        fn double(x) { x * 2 }
        5 |> add(3) |> double()
    "), Value::Int(16));
}

#[test]
fn test_pipe_bare_fn() {
    assert_eq!(eval("
        fn double(x) { x * 2 }
        5 |> double
    "), Value::Int(10));
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
    assert_eq!(eval("range(3)"),
        Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2)]));
    assert_eq!(eval("range(2, 5)"),
        Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)]));
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
    assert_eq!(eval(r#""hello world".contains("world")"#), Value::Bool(true));
    assert_eq!(eval(r#""hello".starts_with("hel")"#), Value::Bool(true));
    assert_eq!(eval(r#""hello".ends_with("llo")"#), Value::Bool(true));
    assert_eq!(eval(r#""  hello  ".trim()"#), Value::Str("hello".into()));
    assert_eq!(eval(r#""hello".to_upper()"#), Value::Str("HELLO".into()));
    assert_eq!(eval(r#""HELLO".to_lower()"#), Value::Str("hello".into()));
}

#[test]
fn test_string_split() {
    assert_eq!(eval(r#""a,b,c".split(",")"#),
        Value::List(vec![Value::Str("a".into()), Value::Str("b".into()), Value::Str("c".into())]));
}

#[test]
fn test_string_replace() {
    assert_eq!(eval(r#""hello world".replace("world", "ion")"#),
        Value::Str("hello ion".into()));
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
    engine.register_fn("square", |args: &[Value]| {
        match &args[0] {
            Value::Int(n) => Ok(Value::Int(n * n)),
            _ => Err("expected int".to_string()),
        }
    });
    assert_eq!(engine.eval("square(5)").unwrap(), Value::Int(25));
}

// ============================================================
// Section 21: Complex Programs
// ============================================================

#[test]
fn test_fibonacci_functional() {
    assert_eq!(eval("
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(|n| fib(n))
    "), Value::List(vec![
        Value::Int(0), Value::Int(1), Value::Int(1), Value::Int(2),
        Value::Int(3), Value::Int(5), Value::Int(8), Value::Int(13),
        Value::Int(21), Value::Int(34), Value::Int(55),
    ]));
}

#[test]
fn test_dict_pipeline() {
    let val = eval(r#"
        let data = #{ "name": "Alice", "age": 30 };
        let updated = data.insert("role", "admin");
        updated.get("role")
    "#);
    assert_eq!(val, Value::Option(Some(Box::new(Value::Str("admin".into())))));
}

#[test]
fn test_error_propagation_chain() {
    // Chain of ? operators
    let result = Engine::new().eval(r#"
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
    "#).unwrap();
    assert_eq!(result, Value::Result(Ok(Box::new(Value::Int(84)))));
}

#[test]
fn test_error_propagation_failure() {
    let result = Engine::new().eval(r#"
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
    "#);
    assert!(result.is_err(), "expected error propagation");
}

#[test]
fn test_nested_closures() {
    assert_eq!(eval("
        fn make_adder(x) {
            |y| x + y
        }
        let add5 = make_adder(5);
        let add10 = make_adder(10);
        add5(3) + add10(3)
    "), Value::Int(21));
}

#[test]
fn test_for_dict_iteration() {
    assert_eq!(eval(r#"
        let mut sum = 0;
        for (key, val) in #{ "a": 1, "b": 2, "c": 3 } {
            sum = sum + val;
        }
        sum
    "#), Value::Int(6));
}

#[test]
fn test_list_of_dicts() {
    let val = eval(r#"
        let people = [
            #{ "name": "Alice", "age": 30 },
            #{ "name": "Bob", "age": 25 },
        ];
        people.map(|p| p["name"])
    "#);
    assert_eq!(val, Value::List(vec![
        Value::Option(Some(Box::new(Value::Str("Alice".into())))),
        Value::Option(Some(Box::new(Value::Str("Bob".into())))),
    ]));
}

// ============================================================
// Section 22: List Comprehensions
// ============================================================

#[test]
fn test_list_comp_basic() {
    assert_eq!(eval("[x * 2 for x in [1, 2, 3]]"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)]));
}

#[test]
fn test_list_comp_with_filter() {
    assert_eq!(eval("[x for x in [1, 2, 3, 4, 5] if x > 3]"),
        Value::List(vec![Value::Int(4), Value::Int(5)]));
}

#[test]
fn test_list_comp_with_transform_and_filter() {
    assert_eq!(eval("[x * x for x in [1, 2, 3, 4, 5] if x % 2 == 0]"),
        Value::List(vec![Value::Int(4), Value::Int(16)]));
}

#[test]
fn test_list_comp_tuple_pattern() {
    assert_eq!(eval(r#"
        let pairs = [(1, "a"), (2, "b"), (3, "c")];
        [n for (n, _s) in pairs if n > 1]
    "#), Value::List(vec![Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_list_comp_over_range() {
    assert_eq!(eval("[x * x for x in range(5)]"),
        Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(4), Value::Int(9), Value::Int(16)]));
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
    let val = eval(r#"
        let base = #{ "a": 1, "b": 2 };
        #{ ...base, "c": 3 }
    "#);
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
    let val = eval(r#"
        let base = #{ "a": 1, "b": 2 };
        #{ ...base, "b": 99 }
    "#);
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
    let val = eval(r#"
        let a = #{ "x": 1 };
        let b = #{ "y": 2 };
        #{ ...a, ...b, "z": 3 }
    "#);
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
    assert_eq!(eval("json_encode(42)"), Value::Str("42".into()));
}

#[test]
fn test_json_encode_dict() {
    let val = eval(r#"json_encode(#{ "name": "Ion", "version": 1 })"#);
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
    assert_eq!(eval("json_encode([1, 2, 3])"), Value::Str("[1,2,3]".into()));
}

#[test]
fn test_json_decode_object() {
    let val = eval(r#"json_decode("{\"a\": 1, \"b\": 2}")"#);
    if let Value::Dict(map) = val {
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(2));
    } else {
        panic!("expected dict, got: {:?}", val);
    }
}

#[test]
fn test_json_decode_array() {
    assert_eq!(eval(r#"json_decode("[1, 2, 3]")"#),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_json_roundtrip() {
    let val = eval(r#"
        let data = #{ "name": "test", "values": [1, 2, 3] };
        let encoded = json_encode(data);
        json_decode(encoded)
    "#);
    if let Value::Dict(map) = val {
        assert_eq!(map["name"], Value::Str("test".into()));
        assert_eq!(map["values"], Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_json_decode_invalid() {
    let err = eval_err(r#"json_decode("not json")"#);
    assert!(err.contains("json_decode error"), "got: {}", err);
}

// ============================================================
// Section 26: Execution Limits (Sandboxing)
// ============================================================

#[test]
fn test_max_call_depth() {
    let mut engine = Engine::new();
    engine.set_limits(Limits { max_call_depth: 10, max_loop_iters: 1_000_000 });
    let err = engine.eval("
        fn recurse(n) { recurse(n + 1) }
        recurse(0)
    ").unwrap_err();
    assert!(err.message.contains("maximum call depth"), "got: {}", err.message);
}

#[test]
fn test_max_loop_iters() {
    let mut engine = Engine::new();
    engine.set_limits(Limits { max_call_depth: 512, max_loop_iters: 100 });
    let err = engine.eval("
        let mut i = 0;
        while true { i = i + 1; }
    ").unwrap_err();
    assert!(err.message.contains("maximum loop iterations"), "got: {}", err.message);
}

#[test]
fn test_loop_within_limit() {
    let mut engine = Engine::new();
    engine.set_limits(Limits { max_call_depth: 512, max_loop_iters: 100 });
    let result = engine.eval("
        let mut sum = 0;
        let mut i = 0;
        while i < 50 { sum = sum + i; i = i + 1; }
        sum
    ").unwrap();
    assert_eq!(result, Value::Int(1225));
}

// ============================================================
// Section 27: Stdlib Builtins
// ============================================================

#[test]
fn test_abs() {
    assert_eq!(eval("abs(-5)"), Value::Int(5));
    assert_eq!(eval("abs(5)"), Value::Int(5));
    assert_eq!(eval("abs(-3.14)"), Value::Float(3.14));
}

#[test]
fn test_min_max() {
    assert_eq!(eval("min(3, 1, 2)"), Value::Int(1));
    assert_eq!(eval("max(3, 1, 2)"), Value::Int(3));
    assert_eq!(eval("min(1.5, 2.5)"), Value::Float(1.5));
    assert_eq!(eval("max(1, 2.5)"), Value::Float(2.5));
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
        name: "Config".into(),
        fields: vec!["host".into(), "port".into(), "debug".into()],
    });
    engine.register_enum(HostEnumDef {
        name: "Color".into(),
        variants: vec![
            HostVariantDef { name: "Red".into(), arity: 0 },
            HostVariantDef { name: "Green".into(), arity: 0 },
            HostVariantDef { name: "Blue".into(), arity: 0 },
            HostVariantDef { name: "Custom".into(), arity: 3 },
        ],
    });
    engine
}

#[test]
fn test_host_struct_construct() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"Config { host: "localhost", port: 8080, debug: true }"#).unwrap();
    if let Value::HostStruct { type_name, fields } = &val {
        assert_eq!(type_name, "Config");
        assert_eq!(fields["host"], Value::Str("localhost".into()));
        assert_eq!(fields["port"], Value::Int(8080));
        assert_eq!(fields["debug"], Value::Bool(true));
    } else {
        panic!("expected HostStruct, got: {:?}", val);
    }
}

#[test]
fn test_host_struct_field_access() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"
        let cfg = Config { host: "localhost", port: 8080, debug: false };
        cfg.host
    "#).unwrap();
    assert_eq!(val, Value::Str("localhost".into()));
}

#[test]
fn test_host_struct_missing_field_error() {
    let mut engine = engine_with_types();
    let err = engine.eval(r#"Config { host: "localhost" }"#).unwrap_err();
    assert!(err.message.contains("missing field"), "got: {}", err.message);
}

#[test]
fn test_host_struct_unknown_field_error() {
    let mut engine = engine_with_types();
    let err = engine.eval(r#"Config { host: "x", port: 80, debug: true, extra: 1 }"#).unwrap_err();
    assert!(err.message.contains("unknown field"), "got: {}", err.message);
}

#[test]
fn test_host_struct_pattern_match() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"
        let cfg = Config { host: "localhost", port: 8080, debug: true };
        match cfg {
            Config { host, port } => f"{host}:{port}",
        }
    "#).unwrap();
    assert_eq!(val, Value::Str("localhost:8080".into()));
}

#[test]
fn test_host_struct_spread() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"
        let base = Config { host: "localhost", port: 8080, debug: false };
        let updated = Config { ...base, debug: true };
        updated.debug
    "#).unwrap();
    assert_eq!(val, Value::Bool(true));
}

// ============================================================
// Section 29: Host Types — Enums
// ============================================================

#[test]
fn test_host_enum_unit_variant() {
    let mut engine = engine_with_types();
    let val = engine.eval("Color::Red").unwrap();
    assert_eq!(val, Value::HostEnum {
        enum_name: "Color".into(),
        variant: "Red".into(),
        data: vec![],
    });
}

#[test]
fn test_host_enum_data_variant() {
    let mut engine = engine_with_types();
    let val = engine.eval("Color::Custom(255, 128, 0)").unwrap();
    assert_eq!(val, Value::HostEnum {
        enum_name: "Color".into(),
        variant: "Custom".into(),
        data: vec![Value::Int(255), Value::Int(128), Value::Int(0)],
    });
}

#[test]
fn test_host_enum_unknown_variant_error() {
    let mut engine = engine_with_types();
    let err = engine.eval("Color::Yellow").unwrap_err();
    assert!(err.message.contains("unknown variant"), "got: {}", err.message);
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
    let val = engine.eval(r#"
        let c = Color::Custom(255, 128, 0);
        match c {
            Color::Red => "red",
            Color::Custom(r, g, b) => f"rgb({r},{g},{b})",
            _ => "other",
        }
    "#).unwrap();
    assert_eq!(val, Value::Str("rgb(255,128,0)".into()));
}

#[test]
fn test_host_enum_match_unit_variant() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"
        let c = Color::Green;
        match c {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue => "blue",
            _ => "other",
        }
    "#).unwrap();
    assert_eq!(val, Value::Str("green".into()));
}

#[test]
fn test_host_struct_display() {
    let mut engine = engine_with_types();
    let val = engine.eval(r#"
        let cfg = Config { host: "localhost", port: 8080, debug: true };
        f"{cfg}"
    "#).unwrap();
    if let Value::Str(s) = &val {
        assert!(s.contains("Config"), "got: {}", s);
        assert!(s.contains("localhost"), "got: {}", s);
    } else {
        panic!("expected string");
    }
}

#[test]
fn test_host_enum_display() {
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

// ============================================================
// Section 30: Extended Stdlib
// ============================================================

#[test]
fn test_floor_ceil_round() {
    assert_eq!(eval("floor(3.7)"), Value::Float(3.0));
    assert_eq!(eval("ceil(3.2)"), Value::Float(4.0));
    assert_eq!(eval("round(3.5)"), Value::Float(4.0));
    assert_eq!(eval("round(3.4)"), Value::Float(3.0));
    assert_eq!(eval("floor(5)"), Value::Int(5));
}

#[test]
fn test_pow() {
    assert_eq!(eval("pow(2, 10)"), Value::Int(1024));
    assert_eq!(eval("pow(2.0, 0.5)"), Value::Float(2.0_f64.sqrt()));
}

#[test]
fn test_sqrt() {
    assert_eq!(eval("sqrt(16)"), Value::Float(4.0));
    assert_eq!(eval("sqrt(2.0)"), Value::Float(2.0_f64.sqrt()));
}

#[test]
fn test_list_join() {
    assert_eq!(eval(r#"["a", "b", "c"].join(", ")"#), Value::Str("a, b, c".into()));
    assert_eq!(eval(r#"[1, 2, 3].join("-")"#), Value::Str("1-2-3".into()));
}

#[test]
fn test_list_enumerate() {
    assert_eq!(eval(r#"["a", "b"].enumerate()"#), Value::List(vec![
        Value::Tuple(vec![Value::Int(0), Value::Str("a".into())]),
        Value::Tuple(vec![Value::Int(1), Value::Str("b".into())]),
    ]));
}

#[test]
fn test_enumerate_builtin() {
    assert_eq!(eval(r#"enumerate(["x", "y"])"#), Value::List(vec![
        Value::Tuple(vec![Value::Int(0), Value::Str("x".into())]),
        Value::Tuple(vec![Value::Int(1), Value::Str("y".into())]),
    ]));
}

#[test]
fn test_json_encode_pretty() {
    let val = eval(r#"json_encode_pretty(#{ "a": 1 })"#);
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
    if let Value::HostStruct { type_name, fields } = &val {
        assert_eq!(type_name, "Point");
        assert_eq!(fields["x"], Value::Float(1.0));
        assert_eq!(fields["y"], Value::Float(2.0));
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
    let val = engine.eval("
        let p = Point { x: 10.0, y: 20.0 };
        p.x + p.y
    ").unwrap();
    assert_eq!(val, Value::Float(30.0));
}

#[test]
fn test_derive_set_typed_get_typed() {
    let mut engine = Engine::new();
    engine.register_type::<UserProfile>();
    let profile = UserProfile { name: "Alice".into(), age: 30, active: true };
    engine.set_typed("user", &profile);
    let val = engine.eval(r#"f"{user.name} is {user.age}""#).unwrap();
    assert_eq!(val, Value::Str("Alice is 30".into()));

    engine.eval("let result = UserProfile { name: \"Bob\", age: 25, active: false };").unwrap();
    let result: UserProfile = engine.get_typed("result").unwrap();
    assert_eq!(result.name, "Bob");
    assert_eq!(result.age, 25);
    assert_eq!(result.active, false);
}

#[test]
fn test_derive_enum_to_ion() {
    let s = Shape::Circle(5.0);
    let val = s.to_ion();
    assert_eq!(val, Value::HostEnum {
        enum_name: "Shape".into(),
        variant: "Circle".into(),
        data: vec![Value::Float(5.0)],
    });
}

#[test]
fn test_derive_enum_from_ion() {
    let val = Value::HostEnum {
        enum_name: "Shape".into(),
        variant: "Rect".into(),
        data: vec![Value::Float(3.0), Value::Float(4.0)],
    };
    let s = Shape::from_ion(&val).unwrap();
    match s {
        Shape::Rect(w, h) => { assert_eq!(w, 3.0); assert_eq!(h, 4.0); }
        _ => panic!("expected Rect"),
    }
}

#[test]
fn test_derive_enum_in_script() {
    let mut engine = Engine::new();
    engine.register_type::<Shape>();
    let val = engine.eval(r#"
        let s = Shape::Circle(5.0);
        match s {
            Shape::Circle(r) => r * r * 3.14,
            Shape::Rect(w, h) => w * h,
            Shape::Empty => 0.0,
        }
    "#).unwrap();
    assert_eq!(val, Value::Float(78.5));
}

#[test]
fn test_derive_enum_unit_variant_in_script() {
    let mut engine = Engine::new();
    engine.register_type::<Shape>();
    let val = engine.eval(r#"
        let s = Shape::Empty;
        match s {
            Shape::Circle(r) => r,
            Shape::Empty => 0.0,
            _ => -1.0,
        }
    "#).unwrap();
    assert_eq!(val, Value::Float(0.0));
}

#[test]
fn test_derive_roundtrip_typed() {
    let mut engine = Engine::new();
    engine.register_type::<Point>();
    let original = Point { x: 42.0, y: 99.0 };
    engine.set_typed("p", &original);
    engine.eval("let p2 = Point { x: p.x * 2.0, y: p.y * 2.0 };").unwrap();
    let result: Point = engine.get_typed("p2").unwrap();
    assert_eq!(result.x, 84.0);
    assert_eq!(result.y, 198.0);
}
