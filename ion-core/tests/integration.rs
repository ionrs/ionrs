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
