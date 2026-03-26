//! Cross-validation: run scripts through both tree-walk and VM,
//! assert identical results. Catches divergence between execution paths.
#![cfg(feature = "vm")]

use ion_core::engine::Engine;
use ion_core::value::Value;

/// Run source through both tree-walk and VM, assert same result.
fn assert_both(src: &str) {
    let mut tw = Engine::new();
    let mut vm = Engine::new();
    let tw_result = tw.eval(src);
    let vm_result = vm.vm_eval(src);
    match (tw_result, vm_result) {
        (Ok(tw_val), Ok(vm_val)) => {
            assert_eq!(tw_val, vm_val, "divergence on: {}", src);
        }
        (Err(tw_err), Err(vm_err)) => {
            // Both errored — OK (error messages may differ)
            let _ = (tw_err, vm_err);
        }
        (Ok(tw_val), Err(vm_err)) => {
            panic!(
                "tree-walk OK ({:?}) but VM errored: {} | src: {}",
                tw_val, vm_err.message, src
            );
        }
        (Err(tw_err), Ok(vm_val)) => {
            panic!(
                "tree-walk errored ({}) but VM OK: {:?} | src: {}",
                tw_err.message, vm_val, src
            );
        }
    }
}

/// Same as assert_both but checks specific expected value.
fn assert_both_eq(src: &str, expected: Value) {
    let mut tw = Engine::new();
    let mut vm = Engine::new();
    let tw_val = tw.eval(src).unwrap();
    let vm_val = vm.vm_eval(src).unwrap();
    assert_eq!(tw_val, expected, "tree-walk mismatch: {}", src);
    assert_eq!(vm_val, expected, "VM mismatch: {}", src);
}

// ============================================================
// Literals
// ============================================================

#[test]
fn cross_int_literals() {
    assert_both_eq("42", Value::Int(42));
    assert_both_eq("-1", Value::Int(-1));
    assert_both_eq("0", Value::Int(0));
}

#[test]
fn cross_float_literals() {
    assert_both_eq("3.14", Value::Float(3.14));
    assert_both_eq("-0.5", Value::Float(-0.5));
}

#[test]
fn cross_bool_literals() {
    assert_both_eq("true", Value::Bool(true));
    assert_both_eq("false", Value::Bool(false));
}

#[test]
fn cross_string_literals() {
    assert_both_eq(r#""hello""#, Value::Str("hello".to_string()));
    assert_both_eq(r#""""#, Value::Str(String::new()));
}

#[test]
fn cross_unit() {
    assert_both_eq("()", Value::Unit);
}

// ============================================================
// Arithmetic
// ============================================================

#[test]
fn cross_arithmetic() {
    assert_both_eq("2 + 3", Value::Int(5));
    assert_both_eq("10 - 4", Value::Int(6));
    assert_both_eq("3 * 7", Value::Int(21));
    assert_both_eq("15 / 3", Value::Int(5));
    assert_both_eq("17 % 5", Value::Int(2));
    assert_both_eq("2.0 + 3.0", Value::Float(5.0));
}

#[test]
fn cross_unary() {
    assert_both_eq("-5", Value::Int(-5));
    assert_both_eq("!true", Value::Bool(false));
    assert_both_eq("!false", Value::Bool(true));
}

// ============================================================
// Comparison & Logic
// ============================================================

#[test]
fn cross_comparison() {
    assert_both_eq("1 < 2", Value::Bool(true));
    assert_both_eq("2 > 1", Value::Bool(true));
    assert_both_eq("1 == 1", Value::Bool(true));
    assert_both_eq("1 != 2", Value::Bool(true));
    assert_both_eq("3 <= 3", Value::Bool(true));
    assert_both_eq("4 >= 5", Value::Bool(false));
}

#[test]
fn cross_logic() {
    assert_both_eq("true && true", Value::Bool(true));
    assert_both_eq("true && false", Value::Bool(false));
    assert_both_eq("false || true", Value::Bool(true));
    assert_both_eq("false || false", Value::Bool(false));
}

// ============================================================
// Variables & Assignment
// ============================================================

#[test]
fn cross_let_bindings() {
    assert_both_eq("let x = 10; x", Value::Int(10));
    assert_both_eq("let mut x = 1; x = 2; x", Value::Int(2));
    assert_both_eq("let mut x = 0; x += 5; x", Value::Int(5));
}

// ============================================================
// If/else
// ============================================================

#[test]
fn cross_if_else() {
    assert_both_eq("if true { 1 } else { 2 }", Value::Int(1));
    assert_both_eq("if false { 1 } else { 2 }", Value::Int(2));
    assert_both_eq("if 1 > 0 { 10 } else { 20 }", Value::Int(10));
}

#[test]
fn cross_if_chain() {
    assert_both_eq(
        "let x = 5; if x < 0 { -1 } else if x == 0 { 0 } else { 1 }",
        Value::Int(1),
    );
}

// ============================================================
// Loops
// ============================================================

#[test]
fn cross_while_loop() {
    assert_both_eq("let mut x = 0; while x < 5 { x += 1; } x", Value::Int(5));
}

#[test]
fn cross_for_loop() {
    assert_both_eq(
        "let mut sum = 0; for x in [1, 2, 3] { sum += x; } sum",
        Value::Int(6),
    );
}

#[test]
fn cross_for_continue() {
    assert_both_eq(
        "let mut sum = 0; for x in [1, 2, 3, 4, 5] { if x == 3 { continue; } sum += x; } sum",
        Value::Int(12),
    );
}

#[test]
fn cross_for_break() {
    assert_both_eq(
        "let mut sum = 0; for x in [1, 2, 3, 4, 5] { if x == 4 { break; } sum += x; } sum",
        Value::Int(6),
    );
}

// ============================================================
// Functions
// ============================================================

#[test]
fn cross_fn_basic() {
    assert_both_eq("fn add(a, b) { a + b } add(3, 4)", Value::Int(7));
}

#[test]
fn cross_fn_recursive() {
    assert_both_eq(
        "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(6)",
        Value::Int(8),
    );
}

#[test]
fn cross_fn_default_params() {
    assert_both_eq(
        r#"fn greet(name = "world") { name } greet()"#,
        Value::Str("world".to_string()),
    );
}

#[test]
fn cross_closure() {
    assert_both_eq("let x = 10; fn add_x(y) { x + y } add_x(5)", Value::Int(15));
}

#[test]
fn cross_lambda() {
    assert_both_eq("let double = |x| x * 2; double(5)", Value::Int(10));
}

// ============================================================
// Collections
// ============================================================

#[test]
fn cross_list() {
    assert_both_eq(
        "[1, 2, 3]",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    assert_both_eq("[1, 2, 3].len()", Value::Int(3));
}

#[test]
fn cross_tuple() {
    assert_both_eq(
        "(1, 2, 3)",
        Value::Tuple(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_dict() {
    assert_both("let d = #{a: 1, b: 2}; d.a");
}

#[test]
fn cross_index() {
    assert_both_eq("[10, 20, 30][1]", Value::Int(20));
}

#[test]
fn cross_field_access() {
    assert_both_eq(r#"#{"name": "ion"}.name"#, Value::Str("ion".to_string()));
}

// ============================================================
// Option / Result
// ============================================================

#[test]
fn cross_option() {
    assert_both_eq("Some(42)", Value::Option(Some(Box::new(Value::Int(42)))));
    assert_both_eq("None", Value::Option(None));
}

#[test]
fn cross_result() {
    assert_both_eq("Ok(1)", Value::Result(Ok(Box::new(Value::Int(1)))));
}

// ============================================================
// ? operator
// ============================================================

#[test]
fn cross_try_ok() {
    assert_both_eq("fn f() { let x = Ok(42); x? } f()", Value::Int(42));
}

#[test]
fn cross_try_some() {
    assert_both_eq("fn f() { let x = Some(10); x? } f()", Value::Int(10));
}

#[test]
fn cross_try_err_propagates() {
    assert_both(r#"fn f() { let x = Err("fail"); x? } f()"#);
}

#[test]
fn cross_try_none_propagates() {
    assert_both("fn f() { let x = None; x? } f()");
}

// ============================================================
// Pattern matching
// ============================================================

#[test]
fn cross_match_literal() {
    assert_both_eq("match 5 { 5 => 10, _ => 0 }", Value::Int(10));
    assert_both_eq("match 3 { 5 => 10, _ => 0 }", Value::Int(0));
}

#[test]
fn cross_match_option() {
    assert_both_eq(
        "match Some(5) { Some(v) => v * 2, None => 0 }",
        Value::Int(10),
    );
    assert_both_eq("match None { Some(v) => v, None => 99 }", Value::Int(99));
}

#[test]
fn cross_match_result() {
    assert_both_eq("match Ok(7) { Ok(v) => v, Err(e) => 0 }", Value::Int(7));
    assert_both_eq(
        r#"match Err("fail") { Ok(v) => 0, Err(e) => e }"#,
        Value::Str("fail".to_string()),
    );
}

#[test]
fn cross_match_tuple() {
    assert_both_eq("match (1, 2) { (a, b) => a + b, _ => 0 }", Value::Int(3));
}

#[test]
fn cross_match_list() {
    assert_both_eq(
        "match [1, 2, 3] { [a, b, c] => a + b + c, _ => 0 }",
        Value::Int(6),
    );
    assert_both_eq("match [] { [] => 1, _ => 0 }", Value::Int(1));
}

// ============================================================
// String operations
// ============================================================

#[test]
fn cross_fstring() {
    assert_both_eq(
        r#"let x = 42; f"val={x}""#,
        Value::Str("val=42".to_string()),
    );
}

#[test]
fn cross_string_methods() {
    assert_both_eq(r#""hello".len()"#, Value::Int(5));
    assert_both_eq(r#""hello".contains("ell")"#, Value::Bool(true));
    assert_both_eq(r#""hello".starts_with("hel")"#, Value::Bool(true));
    assert_both_eq(r#""hello".ends_with("llo")"#, Value::Bool(true));
    assert_both_eq(r#""  hi  ".trim()"#, Value::Str("hi".to_string()));
    assert_both_eq(r#""  hi  ".trim_start()"#, Value::Str("hi  ".to_string()));
    assert_both_eq(r#""  hi  ".trim_end()"#, Value::Str("  hi".to_string()));
    assert_both_eq(r#""hi".to_upper()"#, Value::Str("HI".to_string()));
    assert_both_eq(r#""HI".to_lower()"#, Value::Str("hi".to_string()));
    assert_both_eq(r#""ab".repeat(3)"#, Value::Str("ababab".to_string()));
    assert_both_eq(r#""hello".reverse()"#, Value::Str("olleh".to_string()));
    assert_both_eq(r#""hello".is_empty()"#, Value::Bool(false));
    assert_both_eq(r#""".is_empty()"#, Value::Bool(true));
}

#[test]
fn cross_string_find() {
    assert_both_eq(
        r#""hello".find("ell")"#,
        Value::Option(Some(Box::new(Value::Int(1)))),
    );
    assert_both_eq(r#""hello".find("xyz")"#, Value::Option(None));
}

#[test]
fn cross_string_split_replace() {
    assert_both_eq(
        r#""a,b,c".split(",")"#,
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
            Value::Str("c".to_string()),
        ]),
    );
    assert_both_eq(
        r#""hello".replace("l", "r")"#,
        Value::Str("herro".to_string()),
    );
}

#[test]
fn cross_string_chars() {
    assert_both_eq(
        r#""hi".chars()"#,
        Value::List(vec![
            Value::Str("h".to_string()),
            Value::Str("i".to_string()),
        ]),
    );
}

#[test]
fn cross_string_slice() {
    assert_both_eq(r#""hello".slice(1, 4)"#, Value::Str("ell".to_string()));
}

// ============================================================
// Pipe operator
// ============================================================

#[test]
fn cross_pipe() {
    assert_both_eq("fn double(x) { x * 2 } 5 |> double", Value::Int(10));
}

// ============================================================
// Bitwise operations
// ============================================================

#[test]
fn cross_bitwise() {
    assert_both_eq("255 & 15", Value::Int(15));
    assert_both_eq("15 | 240", Value::Int(255));
    assert_both_eq("255 ^ 15", Value::Int(240));
    assert_both_eq("1 << 4", Value::Int(16));
    assert_both_eq("16 >> 2", Value::Int(4));
}

// ============================================================
// If-let / While-let
// ============================================================

#[test]
fn cross_if_let() {
    assert_both_eq(
        "let x = Some(10); if let Some(v) = x { v + 1 } else { 0 }",
        Value::Int(11),
    );
    assert_both_eq(
        "let x = None; if let Some(v) = x { v } else { 99 }",
        Value::Int(99),
    );
}

// ============================================================
// Comprehensions
// ============================================================

#[test]
fn cross_list_comprehension() {
    assert_both_eq(
        "[x * 2 for x in [1, 2, 3]]",
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)]),
    );
}

#[test]
fn cross_filtered_comprehension() {
    assert_both_eq(
        "[x for x in [1, 2, 3, 4, 5] if x % 2 == 0]",
        Value::List(vec![Value::Int(2), Value::Int(4)]),
    );
}

// ============================================================
// Index/field assignment
// ============================================================

#[test]
fn cross_list_assign() {
    assert_both_eq(
        "let mut a = [1, 2, 3]; a[0] = 10; a",
        Value::List(vec![Value::Int(10), Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_dict_assign() {
    assert_both("let mut d = #{x: 1}; d.x = 2; d.x");
}

// ============================================================
// Scope
// ============================================================

#[test]
fn cross_block_scope() {
    assert_both_eq(
        "let x = 1; let y = { let x = 2; x + 10 }; x + y",
        Value::Int(13),
    );
}

// ============================================================
// Complex programs
// ============================================================

#[test]
fn cross_fibonacci() {
    assert_both_eq(
        "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(6)",
        Value::Int(8),
    );
}

#[test]
fn cross_nested_closures() {
    assert_both_eq(
        "fn make_adder(x) { |y| x + y } let add5 = make_adder(5); add5(10)",
        Value::Int(15),
    );
}

#[test]
fn cross_map_filter() {
    assert_both_eq(
        "[1, 2, 3, 4, 5].filter(|x| x > 2).map(|x| x * 10)",
        Value::List(vec![Value::Int(30), Value::Int(40), Value::Int(50)]),
    );
}

#[test]
fn cross_complex_match() {
    assert_both_eq(
        r#"
        fn classify(n) {
            match n % 3 {
                0 => "fizz",
                1 => "one",
                _ => "other",
            }
        }
        classify(9)
    "#,
        Value::Str("fizz".to_string()),
    );
}

#[test]
fn cross_nested_loops() {
    assert_both_eq(
        r#"
        let mut sum = 0;
        for i in [1, 2, 3] {
            for j in [10, 20] {
                sum += i * j;
            }
        }
        sum
    "#,
        Value::Int(180),
    );
}

// ============================================================
// Dict methods
// ============================================================

#[test]
fn cross_dict_methods() {
    assert_both_eq(r#"#{"a": 1, "b": 2}.len()"#, Value::Int(2));
    assert_both_eq(r#"#{"a": 1, "b": 2}.contains_key("a")"#, Value::Bool(true));
    assert_both_eq(r#"#{"a": 1, "b": 2}.contains_key("c")"#, Value::Bool(false));
    assert_both_eq(r#"#{"a": 1}.is_empty()"#, Value::Bool(false));
    assert_both_eq(r#"#{}.is_empty()"#, Value::Bool(true));
}

#[test]
fn cross_dict_shorthand_keys() {
    assert_both_eq("#{a: 1, b: 2}.len()", Value::Int(2));
    assert_both(r#"#{a: 1}.a"#);
}

// ============================================================
// List methods
// ============================================================

#[test]
fn cross_list_methods() {
    assert_both_eq(
        "[3, 1, 2].sort()",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    assert_both_eq(
        "[1, 2, 3].reverse()",
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)]),
    );
    assert_both_eq("[1, 2, 3].contains(2)", Value::Bool(true));
    assert_both_eq("[1, 2, 3].contains(5)", Value::Bool(false));
    assert_both_eq("[1, 2, 3].is_empty()", Value::Bool(false));
    assert_both_eq("[].is_empty()", Value::Bool(true));
    assert_both_eq(r#"[1, 2, 3].join(",")"#, Value::Str("1,2,3".to_string()));
}

#[test]
fn cross_list_first_last() {
    assert_both_eq(
        "[1, 2, 3].first()",
        Value::Option(Some(Box::new(Value::Int(1)))),
    );
    assert_both_eq(
        "[1, 2, 3].last()",
        Value::Option(Some(Box::new(Value::Int(3)))),
    );
    assert_both_eq("[].first()", Value::Option(None));
}

#[test]
fn cross_list_closure_methods() {
    assert_both_eq(
        "[1, 2, 3].map(|x| x * 2)",
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)]),
    );
    assert_both_eq(
        "[1, 2, 3, 4].filter(|x| x > 2)",
        Value::List(vec![Value::Int(3), Value::Int(4)]),
    );
    assert_both_eq("[1, 2, 3].fold(0, |acc, x| acc + x)", Value::Int(6));
    assert_both_eq("[1, 2, 3].any(|x| x > 2)", Value::Bool(true));
    assert_both_eq("[1, 2, 3].any(|x| x > 5)", Value::Bool(false));
    assert_both_eq("[1, 2, 3].all(|x| x > 0)", Value::Bool(true));
    assert_both_eq("[1, 2, 3].all(|x| x > 1)", Value::Bool(false));
}

#[test]
fn cross_list_flatten_zip() {
    assert_both_eq(
        "[[1, 2], [3, 4]].flatten()",
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]),
    );
    assert_both_eq(
        "[1, 2].zip([3, 4])",
        Value::List(vec![
            Value::Tuple(vec![Value::Int(1), Value::Int(3)]),
            Value::Tuple(vec![Value::Int(2), Value::Int(4)]),
        ]),
    );
}

#[test]
fn cross_list_enumerate() {
    assert_both_eq(
        "[10, 20].enumerate()",
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Int(10)]),
            Value::Tuple(vec![Value::Int(1), Value::Int(20)]),
        ]),
    );
}

#[test]
fn cross_dict_get() {
    assert_both_eq(
        r#"#{"a": 1}.get("a")"#,
        Value::Option(Some(Box::new(Value::Int(1)))),
    );
    assert_both_eq(r#"#{"a": 1}.get("b")"#, Value::Option(None));
}

#[test]
fn cross_dict_keys_values() {
    assert_both_eq(
        r#"#{"a": 1, "b": 2}.keys()"#,
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
        ]),
    );
    assert_both_eq(
        r#"#{"a": 1, "b": 2}.values()"#,
        Value::List(vec![Value::Int(1), Value::Int(2)]),
    );
}

#[test]
fn cross_dict_entries() {
    assert_both_eq(
        r#"#{"a": 1}.entries()"#,
        Value::List(vec![Value::Tuple(vec![
            Value::Str("a".to_string()),
            Value::Int(1),
        ])]),
    );
}

// ============================================================
// Math builtins
// ============================================================

#[test]
fn cross_math_builtins() {
    assert_both_eq("math::abs(-5)", Value::Int(5));
    assert_both_eq("math::abs(3.5)", Value::Float(3.5));
    assert_both_eq("math::min(3, 7)", Value::Int(3));
    assert_both_eq("math::max(3, 7)", Value::Int(7));
    assert_both_eq("math::floor(3.7)", Value::Float(3.0));
    assert_both_eq("math::ceil(3.2)", Value::Float(4.0));
    assert_both_eq("math::round(3.5)", Value::Float(4.0));
    assert_both_eq("math::sqrt(16.0)", Value::Float(4.0));
    assert_both_eq("math::pow(2, 10)", Value::Int(1024));
}

// ============================================================
// Type checking
// ============================================================

#[test]
fn cross_type_of() {
    assert_both_eq(r#"type_of(42)"#, Value::Str("int".to_string()));
    assert_both_eq(r#"type_of("hello")"#, Value::Str("string".to_string()));
    assert_both_eq(r#"type_of(true)"#, Value::Str("bool".to_string()));
    assert_both_eq(r#"type_of([])"#, Value::Str("list".to_string()));
    assert_both_eq(r#"type_of(#{})"#, Value::Str("dict".to_string()));
}

#[test]
fn cross_flat_map() {
    assert_both_eq(
        "[1, 2, 3].flat_map(|x| [x, x * 10])",
        Value::List(vec![
            Value::Int(1),
            Value::Int(10),
            Value::Int(2),
            Value::Int(20),
            Value::Int(3),
            Value::Int(30),
        ]),
    );
}

#[test]
fn cross_string_index() {
    assert_both_eq(r#""hello"[0]"#, Value::Str("h".to_string()));
    assert_both_eq(r#""hello"[4]"#, Value::Str("o".to_string()));
    assert_both_eq(r#""hello"[-1]"#, Value::Str("o".to_string()));
}

#[test]
fn cross_tuple_methods() {
    assert_both_eq("(1, 2, 3).len()", Value::Int(3));
    assert_both_eq("(1, 2, 3).contains(2)", Value::Bool(true));
    assert_both_eq("(1, 2, 3).contains(9)", Value::Bool(false));
    assert_both_eq(
        "(10, 20).to_list()",
        Value::List(vec![Value::Int(10), Value::Int(20)]),
    );
}

#[test]
fn cross_triple_string() {
    assert_both_eq("\"\"\"hello\"\"\"", Value::Str("hello".to_string()));
}

#[test]
fn cross_string_multiply() {
    assert_both_eq(r#""ha" * 3"#, Value::Str("hahaha".to_string()));
    assert_both_eq(r#"3 * "ab""#, Value::Str("ababab".to_string()));
}

#[test]
fn cross_range_iteration() {
    assert_both_eq("let mut s = 0; for i in 0..5 { s += i; } s", Value::Int(10));
    assert_both_eq("let mut s = 0; for i in 1..=3 { s += i; } s", Value::Int(6));
}

#[test]
fn cross_multiline_lambda() {
    assert_both_eq("let f = |x| { let y = x * 2; y + 1 }; f(5)", Value::Int(11));
}

#[test]
fn cross_string_slice_char() {
    assert_both_eq(r#""hello"[1..3]"#, Value::Str("el".to_string()));
    assert_both_eq(r#""hello"[..2]"#, Value::Str("he".to_string()));
    assert_both_eq(r#""hello"[3..]"#, Value::Str("lo".to_string()));
    assert_both_eq(r#""hello"[1..=3]"#, Value::Str("ell".to_string()));
}

#[test]
fn cross_try_top_level() {
    assert_both_eq(
        r#"let x = Err("oops"); x?"#,
        Value::Result(Err(Box::new(Value::Str("oops".to_string())))),
    );
    assert_both_eq("let x = None; x?", Value::Option(None));
    assert_both_eq("let x = Ok(42); x?", Value::Int(42));
    assert_both_eq("let x = Some(10); x?", Value::Int(10));
}

#[test]
fn cross_string_negative_index_unicode() {
    assert_both_eq(r#""héllo"[-1]"#, Value::Str("o".to_string()));
    assert_both_eq(r#""héllo"[-2]"#, Value::Str("l".to_string()));
    assert_both_eq(r#""héllo"[0]"#, Value::Str("h".to_string()));
    assert_both_eq(r#""héllo"[1]"#, Value::Str("é".to_string()));
}

#[test]
fn cross_string_slice_unicode() {
    assert_both_eq(r#""héllo".slice(1, 3)"#, Value::Str("él".to_string()));
    assert_both_eq(r#""hello".slice(0, 3)"#, Value::Str("hel".to_string()));
}

#[test]
fn cross_string_find_char_offset() {
    assert_both_eq(
        r#""héllo".find("l")"#,
        Value::Option(Some(Box::new(Value::Int(2)))),
    );
    assert_both_eq(r#""hello".find("z")"#, Value::Option(None));
}

#[test]
fn cross_sort_homogeneous() {
    assert_both_eq(
        r#"[3, 1, 2].sort()"#,
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    assert_both_eq(r#"[].sort()"#, Value::List(vec![]));
}

#[test]
fn cross_sort_by() {
    assert_both_eq(
        "[3, 1, 2].sort_by(|a, b| a - b)",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    assert_both_eq(
        "[3, 1, 2].sort_by(|a, b| b - a)",
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)]),
    );
}

#[test]
fn cross_clamp() {
    assert_both_eq("math::clamp(5, 0, 3)", Value::Int(3));
    assert_both_eq("math::clamp(-1, 0, 10)", Value::Int(0));
    assert_both_eq("math::clamp(5, 0, 10)", Value::Int(5));
}

#[test]
fn cross_bytes_basic() {
    assert_both_eq(r#"b"hello".len()"#, Value::Int(5));
    assert_both_eq(r#"b"hello"[0]"#, Value::Int(104));
    assert_both_eq(r#"b"hello"[-1]"#, Value::Int(111));
    assert_both_eq(r#"b"hello".to_hex()"#, Value::Str("68656c6c6f".to_string()));
}

#[test]
fn cross_dict_map() {
    // Just check both engines agree (dict equality via assert_both)
    assert_both(r#"#{a: 1, b: 2}.map(|k, v| v * 10)"#);
}

#[test]
fn cross_dict_filter() {
    assert_both(r#"#{a: 1, b: 2, c: 3}.filter(|k, v| v > 1)"#);
}

#[test]
fn cross_unicode_escape() {
    assert_both_eq(r#""\u{48}\u{49}""#, Value::Str("HI".to_string()));
    assert_both_eq(r#""\u{E9}""#, Value::Str("é".to_string()));
}

#[test]
fn cross_string_contains_int() {
    assert_both_eq(r#""hello".contains(104)"#, Value::Bool(true));
    assert_both_eq(r#""hello".contains(122)"#, Value::Bool(false));
}

#[test]
fn cross_to_string() {
    assert_both_eq(r#"let x = 42; x.to_string()"#, Value::Str("42".to_string()));
    assert_both_eq(r#"true.to_string()"#, Value::Str("true".to_string()));
    assert_both_eq(r#"None.to_string()"#, Value::Str("None".to_string()));
}

#[test]
fn cross_dict_zip() {
    assert_both(r#"#{a: 1, b: 2}.zip(#{a: 10, b: 20})"#);
}

#[test]
fn cross_join_builtin() {
    assert_both_eq(r#"string::join(["a", "b"], ",")"#, Value::Str("a,b".to_string()));
}

#[test]
fn cross_string_bytes() {
    assert_both_eq(
        r#""AB".bytes()"#,
        Value::List(vec![Value::Int(65), Value::Int(66)]),
    );
}

#[test]
fn cross_enumerate_string() {
    assert_both_eq(
        r#"enumerate("ab")"#,
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Str("a".to_string())]),
            Value::Tuple(vec![Value::Int(1), Value::Str("b".to_string())]),
        ]),
    );
}

#[test]
fn cross_list_index() {
    assert_both_eq(
        r#"[10, 20, 30].index(20)"#,
        Value::Option(Some(Box::new(Value::Int(1)))),
    );
    assert_both_eq(r#"[10, 20, 30].index(99)"#, Value::Option(None));
}

#[test]
fn cross_list_count() {
    assert_both_eq(r#"[1, 2, 1, 3, 1].count(1)"#, Value::Int(3));
}

#[test]
fn cross_list_slice() {
    assert_both_eq(
        r#"[1, 2, 3, 4, 5].slice(1, 3)"#,
        Value::List(vec![Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_list_dedup() {
    assert_both_eq(
        r#"[1, 1, 2, 2, 3].dedup()"#,
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_string_pad_start() {
    assert_both_eq(r#""42".pad_start(5, "0")"#, Value::Str("00042".to_string()));
}

#[test]
fn cross_string_pad_end() {
    assert_both_eq(r#""42".pad_end(5, "0")"#, Value::Str("42000".to_string()));
}

#[test]
fn cross_let_destructure_tuple() {
    assert_both_eq(r#"let (a, b) = (10, 20); a + b"#, Value::Int(30));
}

#[test]
fn cross_let_destructure_list() {
    assert_both_eq(r#"let [a, b] = [10, 20]; a + b"#, Value::Int(30));
}

#[test]
fn cross_list_unique() {
    assert_both_eq(
        r#"[1, 2, 1, 3, 2].unique()"#,
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_list_min() {
    assert_both_eq(
        r#"[3, 1, 2].min()"#,
        Value::Option(Some(Box::new(Value::Int(1)))),
    );
    assert_both_eq(r#"[].min()"#, Value::Option(None));
}

#[test]
fn cross_list_max() {
    assert_both_eq(
        r#"[3, 1, 2].max()"#,
        Value::Option(Some(Box::new(Value::Int(3)))),
    );
}

#[test]
fn cross_dict_update() {
    assert_both(r#"#{a: 1, b: 2}.update(#{b: 20, c: 30})"#);
}

#[test]
fn cross_string_char_len() {
    assert_both_eq(r#""héllo".char_len()"#, Value::Int(5));
}

#[test]
fn cross_list_sum() {
    assert_both_eq(r#"[1, 2, 3].sum()"#, Value::Int(6));
    assert_both_eq(r#"[1, 2.5, 3].sum()"#, Value::Float(6.5));
}

#[test]
fn cross_list_window() {
    assert_both_eq(
        r#"[1, 2, 3].window(2)"#,
        Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(2), Value::Int(3)]),
        ]),
    );
}

#[test]
fn cross_string_strip_prefix() {
    assert_both_eq(
        r#""hello world".strip_prefix("hello ")"#,
        Value::Str("world".to_string()),
    );
}

#[test]
fn cross_string_strip_suffix() {
    assert_both_eq(
        r#""hello.ion".strip_suffix(".ion")"#,
        Value::Str("hello".to_string()),
    );
}

#[test]
fn cross_dict_keys_of() {
    assert_both_eq(
        r#"#{a: 1, b: 2, c: 1}.keys_of(1)"#,
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("c".to_string()),
        ]),
    );
}

#[test]
fn cross_try_catch_no_error() {
    assert_both_eq("try { 42 } catch e { e }", Value::Int(42));
}

#[test]
fn cross_try_catch_with_error() {
    assert_both_eq(
        r#"try { assert(false, "boom"); 1 } catch e { e }"#,
        Value::Str("boom".to_string()),
    );
}

#[test]
fn cross_try_catch_division_by_zero() {
    assert_both_eq(
        r#"try { 1 / 0 } catch e { "caught" }"#,
        Value::Str("caught".to_string()),
    );
}

#[test]
fn cross_try_catch_as_expression() {
    assert_both_eq("let x = try { 10 } catch e { 0 }; x + 1", Value::Int(11));
}

#[test]
fn cross_named_args() {
    assert_both_eq(
        r#"fn greet(name, greeting) { f"{greeting} {name}" } greet(greeting: "hi", name: "world")"#,
        Value::Str("hi world".to_string()),
    );
}

#[test]
fn cross_named_args_with_default() {
    assert_both_eq(
        r#"fn greet(name, greeting = "hello") { f"{greeting} {name}" } greet(name: "world")"#,
        Value::Str("hello world".to_string()),
    );
}

#[test]
fn cross_json_encode_decode() {
    assert_both_eq(
        r#"json::decode(json::encode(#{a: 1, b: "two"}))"#,
        Value::Dict(indexmap::indexmap! {
            "a".to_string() => Value::Int(1),
            "b".to_string() => Value::Str("two".to_string()),
        }),
    );
}

#[test]
fn cross_assert_no_error() {
    assert_both_eq("assert(true); 42", Value::Int(42));
}

#[test]
fn cross_assert_eq_pass() {
    assert_both_eq("assert_eq(1 + 1, 2); true", Value::Bool(true));
}

#[test]
fn cross_loop_as_expr() {
    assert_both_eq(
        "let mut i = 0; let x = loop { i = i + 1; if i >= 5 { break i; } }; x",
        Value::Int(5),
    );
}

#[test]
fn cross_enumerate_list() {
    assert_both_eq(
        "enumerate([10, 20])",
        Value::List(vec![
            Value::Tuple(vec![Value::Int(0), Value::Int(10)]),
            Value::Tuple(vec![Value::Int(1), Value::Int(20)]),
        ]),
    );
}

#[test]
fn cross_range() {
    assert_both_eq(
        "let mut s = 0; for i in range(1, 4) { s = s + i; } s",
        Value::Int(6),
    );
}

#[test]
fn cross_option_methods() {
    assert_both_eq("Some(42).unwrap()", Value::Int(42));
    assert_both_eq("None.unwrap_or(99)", Value::Int(99));
}

#[test]
fn cross_result_methods() {
    assert_both_eq("Ok(42).unwrap()", Value::Int(42));
    assert_both_eq(r#"Err("bad").unwrap_or(0)"#, Value::Int(0));
}

// ============================================================
// MessagePack
// ============================================================

#[cfg(feature = "msgpack")]
#[test]
fn cross_msgpack_int() {
    assert_both_eq("json::msgpack_decode(json::msgpack_encode(42))", Value::Int(42));
}

#[cfg(feature = "msgpack")]
#[test]
fn cross_msgpack_string() {
    assert_both_eq(
        r#"json::msgpack_decode(json::msgpack_encode("hello"))"#,
        Value::Str("hello".to_string()),
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn cross_msgpack_bytes() {
    assert_both_eq(
        r#"json::msgpack_decode(json::msgpack_encode(b"\xde\xad"))"#,
        Value::Bytes(vec![0xde, 0xad]),
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn cross_msgpack_list() {
    assert_both_eq(
        "json::msgpack_decode(json::msgpack_encode([1, 2, 3]))",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

#[cfg(feature = "msgpack")]
#[test]
fn cross_msgpack_dict() {
    assert_both_eq(
        r#"let d = #{a: 1}; json::msgpack_decode(json::msgpack_encode(d))"#,
        Value::Dict(indexmap::indexmap! { "a".to_string() => Value::Int(1) }),
    );
}

// ============================================================
// List chunk/reduce
// ============================================================

#[test]
fn cross_list_chunk() {
    assert_both_eq(
        "[1, 2, 3, 4].chunk(2)",
        Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(3), Value::Int(4)]),
        ]),
    );
}

#[test]
fn cross_list_reduce() {
    assert_both_eq("[1, 2, 3].reduce(|a, b| a + b)", Value::Int(6));
}

// ============================================================
// Spread in lists
// ============================================================

#[test]
fn cross_list_spread() {
    assert_both_eq(
        "let a = [1, 2]; [...a, 3]",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

// ============================================================
// Set type
// ============================================================

#[test]
fn cross_set_basic() {
    assert_both_eq("set([1, 2, 2]).len()", Value::Int(2));
}

#[test]
fn cross_set_union() {
    assert_both_eq("set([1, 2]).union(set([2, 3])).len()", Value::Int(3));
}

// ============================================================
// Type annotations
// ============================================================

#[test]
fn cross_type_ann() {
    assert_both_eq("let x: int = 42; x", Value::Int(42));
}

#[test]
fn cross_type_ann_list() {
    assert_both_eq("let xs: list = [1, 2]; xs.len()", Value::Int(2));
}

#[test]
fn cross_type_ann_float() {
    assert_both_eq("let x: float = 3.14; x", Value::Float(3.14));
}

#[test]
fn cross_type_ann_option() {
    assert_both_eq(
        "let x: Option<int> = Some(42); x",
        Value::Option(Some(Box::new(Value::Int(42)))),
    );
}

#[test]
fn cross_type_ann_result() {
    assert_both_eq(
        "let x: Result<int, string> = Ok(1); x",
        Value::Result(Ok(Box::new(Value::Int(1)))),
    );
}

#[test]
fn cross_type_ann_generic_list() {
    // Inner type is documentation-only; only outer type checked
    assert_both_eq(r#"let xs: list<int> = ["a"]; xs.len()"#, Value::Int(1));
}

#[test]
fn cross_type_ann_dict() {
    assert_both_eq(r#"let d: dict = #{"a": 1}; d["a"]"#, Value::Int(1));
}

#[test]
fn cross_type_ann_fn() {
    assert_both_eq("let f: fn = |x| x + 1; f(2)", Value::Int(3));
}

#[test]
fn cross_type_ann_any() {
    assert_both_eq("let x: any = 42; x", Value::Int(42));
}

#[test]
fn cross_type_ann_mismatch() {
    // Both engines should error on type mismatch
    assert_both("let x: int = true;");
}

// ---- Cell (mutable closure state) cross-validation ----

#[test]
fn cross_cell_basic() {
    assert_both_eq("let c = cell(0); c.get()", Value::Int(0));
    assert_both_eq("let c = cell(0); c.set(42); c.get()", Value::Int(42));
}

#[test]
fn cross_cell_update() {
    assert_both_eq(
        "let c = cell(0); c.update(|x| x + 1); c.get()",
        Value::Int(1),
    );
}

#[test]
fn cross_cell_counter_closure() {
    assert_both_eq(
        r#"
        let count = cell(0);
        let inc = || { count.update(|x| x + 1) };
        inc();
        inc();
        inc();
        count.get()
    "#,
        Value::Int(3),
    );
}

#[test]
fn cross_cell_shared_closures() {
    assert_both_eq(
        r#"
        let state = cell(0);
        let inc = || { state.update(|x| x + 1) };
        let get = || { state.get() };
        inc();
        inc();
        get()
    "#,
        Value::Int(2),
    );
}

#[test]
fn cross_cell_factory() {
    assert_both_eq(
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
        get()
    "#,
        Value::Int(2),
    );
}

#[test]
fn cross_cell_accumulator() {
    assert_both_eq(
        r#"
        let acc = cell([]);
        let add = |item| { acc.update(|xs| xs.push(item)) };
        add(1);
        add(2);
        acc.get()
    "#,
        Value::List(vec![Value::Int(1), Value::Int(2)]),
    );
}

#[test]
fn cross_cell_type_of() {
    assert_both_eq("type_of(cell(0))", Value::Str("cell".to_string()));
}

#[test]
fn cross_cell_update_returns_value() {
    assert_both_eq("let c = cell(10); c.update(|x| x * 2)", Value::Int(20));
}

// ============================================================
// Set methods (expanded)
// ============================================================

#[test]
fn cross_set_intersection() {
    assert_both_eq(
        "set([1, 2, 3]).intersection(set([2, 3, 4])).len()",
        Value::Int(2),
    );
}

#[test]
fn cross_set_difference() {
    assert_both_eq(
        "set([1, 2, 3]).difference(set([2, 3, 4])).len()",
        Value::Int(1),
    );
}

#[test]
fn cross_set_contains() {
    assert_both_eq("set([1, 2, 3]).contains(2)", Value::Bool(true));
    assert_both_eq("set([1, 2, 3]).contains(5)", Value::Bool(false));
}

#[test]
fn cross_set_add_remove() {
    // set.add/remove mutate in-place; both engines should agree
    assert_both("let mut s = set([1, 2]); s.add(3); s.len()");
    assert_both("let mut s = set([1, 2, 3]); s.remove(2); s.len()");
}

#[test]
fn cross_set_add_duplicate() {
    assert_both("let mut s = set([1, 2]); s.add(2); s.len()");
}

#[test]
fn cross_set_to_list() {
    assert_both_eq("set([3, 1, 2]).to_list().len()", Value::Int(3));
}

#[test]
fn cross_set_is_empty() {
    assert_both_eq("set([]).is_empty()", Value::Bool(true));
    assert_both_eq("set([1]).is_empty()", Value::Bool(false));
}

// ============================================================
// Spread (expanded)
// ============================================================

#[test]
fn cross_spread_multiple() {
    assert_both_eq(
        "let a = [1, 2]; let b = [3, 4]; [...a, ...b]",
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]),
    );
}

#[test]
fn cross_spread_with_elements() {
    assert_both_eq(
        "let a = [2, 3]; [1, ...a, 4]",
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]),
    );
}

#[test]
fn cross_spread_empty() {
    assert_both_eq(
        "let a = []; [1, ...a, 2]",
        Value::List(vec![Value::Int(1), Value::Int(2)]),
    );
}

// ============================================================
// Pipe operator (expanded)
// ============================================================

#[test]
fn cross_pipe_chain() {
    assert_both_eq(
        "fn double(x) { x * 2 } fn inc(x) { x + 1 } 5 |> double |> inc",
        Value::Int(11),
    );
}


// ============================================================
// Range (expanded)
// ============================================================

#[test]
fn cross_range_in_list() {
    assert_both_eq(
        "let mut r = []; for i in 0..3 { r = r.push(i); } r",
        Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2)]),
    );
}

#[test]
fn cross_range_inclusive_boundary() {
    assert_both_eq(
        "let mut r = []; for i in 1..=1 { r = r.push(i); } r",
        Value::List(vec![Value::Int(1)]),
    );
}

// ============================================================
// Match (expanded)
// ============================================================

#[test]
fn cross_match_nested_option() {
    assert_both_eq(
        "match Some(Some(42)) { Some(Some(v)) => v, _ => 0 }",
        Value::Int(42),
    );
}

#[test]
fn cross_match_string() {
    assert_both_eq(
        r#"match "hello" { "hello" => 1, "world" => 2, _ => 0 }"#,
        Value::Int(1),
    );
}

#[test]
fn cross_match_bool() {
    assert_both_eq("match true { true => 1, false => 0 }", Value::Int(1));
}

#[test]
fn cross_match_in_function() {
    assert_both_eq(
        r#"
        fn describe(opt) {
            match opt {
                Some(v) => f"has {v}",
                None => "empty",
            }
        }
        describe(Some(42))
    "#,
        Value::Str("has 42".to_string()),
    );
}

// ============================================================
// List methods (expanded)
// ============================================================

#[test]
fn cross_list_push() {
    assert_both_eq(
        "let a = [1, 2].push(3); a",
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}

#[test]
fn cross_list_pop() {
    // pop returns Option(Some(last))
    assert_both("let a = [1, 2, 3].pop(); a");
}

#[test]
fn cross_list_negative_index() {
    assert_both_eq("[10, 20, 30][-1]", Value::Int(30));
    assert_both_eq("[10, 20, 30][-2]", Value::Int(20));
}

#[test]
fn cross_list_find() {
    assert_both("[1, 2, 3, 4].find(|x| x > 2)");
    assert_both("[1, 2, 3].find(|x| x > 5)");
}

// ============================================================
// String methods (expanded)
// ============================================================

#[test]
fn cross_string_to_int() {
    // to_int returns Result
    assert_both(r#""42".to_int()"#);
}

#[test]
fn cross_string_to_float() {
    // to_float returns Result
    assert_both(r#""3.14".to_float()"#);
}

// ============================================================
// Option/Result methods (expanded)
// ============================================================

#[test]
fn cross_option_map() {
    assert_both_eq(
        "Some(5).map(|x| x * 2)",
        Value::Option(Some(Box::new(Value::Int(10)))),
    );
    assert_both_eq("None.map(|x| x * 2)", Value::Option(None));
}

#[test]
fn cross_option_is_some_none() {
    assert_both_eq("Some(1).is_some()", Value::Bool(true));
    assert_both_eq("Some(1).is_none()", Value::Bool(false));
    assert_both_eq("None.is_some()", Value::Bool(false));
    assert_both_eq("None.is_none()", Value::Bool(true));
}

#[test]
fn cross_result_is_ok_err() {
    assert_both_eq("Ok(1).is_ok()", Value::Bool(true));
    assert_both_eq("Ok(1).is_err()", Value::Bool(false));
    assert_both_eq(r#"Err("x").is_ok()"#, Value::Bool(false));
    assert_both_eq(r#"Err("x").is_err()"#, Value::Bool(true));
}

#[test]
fn cross_result_map() {
    assert_both_eq(
        "Ok(5).map(|x| x + 1)",
        Value::Result(Ok(Box::new(Value::Int(6)))),
    );
    assert_both_eq(
        r#"Err("bad").map(|x| x + 1)"#,
        Value::Result(Err(Box::new(Value::Str("bad".to_string())))),
    );
}

// ============================================================
// For-in destructuring
// ============================================================

#[test]
fn cross_for_in_tuple_destructure() {
    assert_both_eq(
        "let mut sum = 0; for (i, v) in [(0, 10), (1, 20)] { sum += v; } sum",
        Value::Int(30),
    );
}

#[test]
fn cross_for_in_enumerate_destructure() {
    assert_both_eq(
        r#"let mut s = ""; for (i, c) in "ab".chars().enumerate() { s = s + c; } s"#,
        Value::Str("ab".to_string()),
    );
}

// ============================================================
// While-let
// ============================================================

#[test]
fn cross_while_let() {
    assert_both_eq(
        r#"
        let mut items = [Some(1), Some(2), None];
        let mut sum = 0;
        let mut i = 0;
        while i < items.len() {
            if let Some(v) = items[i] { sum += v; }
            i += 1;
        }
        sum
    "#,
        Value::Int(3),
    );
}

// ============================================================
// Nested data structures
// ============================================================

#[test]
fn cross_nested_dict_access() {
    assert_both_eq(
        r#"let d = #{a: #{b: 42}}; d.a.b"#,
        Value::Int(42),
    );
}

#[test]
fn cross_nested_list_index() {
    assert_both_eq("[[1, 2], [3, 4]][1][0]", Value::Int(3));
}

// ============================================================
// Closure edge cases
// ============================================================

#[test]
fn cross_closure_captures_loop_var() {
    assert_both_eq(
        r#"
        let mut fns = [];
        for i in [1, 2, 3] {
            fns = fns.push(|x| x + i);
        }
        fns[0](10)
    "#,
        Value::Int(11),
    );
}

#[test]
fn cross_closure_returning_closure() {
    assert_both_eq(
        "fn make(x) { |y| |z| x + y + z } make(1)(2)(3)",
        Value::Int(6),
    );
}

// ============================================================
// Compound assignment operators
// ============================================================

#[test]
fn cross_compound_assign() {
    assert_both("let mut x = 10; x -= 3; x");
    assert_both("let mut x = 4; x *= 5; x");
    assert_both("let mut x = 20; x /= 4; x");
    assert_both("let mut x = 17; x %= 5; x");
}

// ============================================================
// Type conversions
// ============================================================

#[test]
fn cross_int_to_float() {
    assert_both_eq("int(3.7)", Value::Int(3));
    assert_both_eq("float(42)", Value::Float(42.0));
}

#[test]
fn cross_string_to_int_builtin() {
    assert_both_eq(r#"int("123")"#, Value::Int(123));
    assert_both_eq(r#"float("3.14")"#, Value::Float(3.14));
}

// ============================================================
// Complex programs (expanded)
// ============================================================

#[test]
fn cross_accumulator_pattern() {
    assert_both_eq(
        r#"
        let mut result = [];
        for i in 0..5 {
            if i % 2 == 0 {
                result = result.push(i * i);
            }
        }
        result
    "#,
        Value::List(vec![Value::Int(0), Value::Int(4), Value::Int(16)]),
    );
}

#[test]
fn cross_nested_match_in_loop() {
    assert_both(
        r#"
        let items = [Some(1), None, Some(3)];
        let mut sum = 0;
        for item in items {
            match item {
                Some(v) => { sum += v; }
                None => {}
            }
        }
        sum
    "#,
    );
}

#[test]
fn cross_functional_pipeline() {
    assert_both_eq(
        "[1, 2, 3, 4, 5].filter(|x| x % 2 == 1).map(|x| x * x).reduce(|a, b| a + b)",
        Value::Int(35),
    );
}

#[test]
fn cross_dict_comprehension() {
    assert_both(r#"#{x: x * x for x in [1, 2, 3]}"#);
}

#[test]
fn cross_early_return() {
    assert_both_eq(
        r#"
        fn find_first_even(lst) {
            for x in lst {
                if x % 2 == 0 { return x; }
            }
            -1
        }
        find_first_even([1, 3, 4, 6])
    "#,
        Value::Int(4),
    );
}

#[test]
fn cross_recursive_sum() {
    assert_both_eq(
        r#"
        fn sum(lst) {
            if lst.len() == 0 { 0 }
            else { lst[0] + sum(lst.slice(1, lst.len())) }
        }
        sum([1, 2, 3, 4, 5])
    "#,
        Value::Int(15),
    );
}

// ============================================================
// Shift operators
// ============================================================

#[test]
fn cross_shift_left() {
    assert_both_eq("1 << 10", Value::Int(1024));
    assert_both_eq("3 << 4", Value::Int(48));
}

#[test]
fn cross_shift_right() {
    assert_both_eq("1024 >> 5", Value::Int(32));
    assert_both_eq("255 >> 4", Value::Int(15));
}

#[test]
fn cross_shift_out_of_range() {
    // Both should error on out-of-range shift counts
    assert_both("1 << 64");
    assert_both("1 >> -1");
    assert_both("1 << -1");
}

// ============================================================
// Match exhaustiveness
// ============================================================

#[test]
fn cross_match_non_exhaustive() {
    // Both should error on non-exhaustive match
    assert_both(r#"match 42 { 1 => "one", 2 => "two" }"#);
}

// ============================================================
// Module path access
// ============================================================

#[test]
fn cross_math_module() {
    assert_both_eq("math::abs(-5)", Value::Int(5));
    assert_both_eq("math::max(3, 7)", Value::Int(7));
    assert_both_eq("math::min(3, 7)", Value::Int(3));
}

#[test]
fn cross_math_constants() {
    assert_both("math::PI");
    assert_both("math::E");
}

#[test]
fn cross_string_join() {
    assert_both_eq(
        r#"string::join(["a", "b", "c"], ", ")"#,
        Value::Str("a, b, c".to_string()),
    );
}

// ============================================================
// Window / chunk edge cases
// ============================================================

#[test]
fn cross_window() {
    assert_both_eq(
        "[1, 2, 3, 4].window(2)",
        Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(2), Value::Int(3)]),
            Value::List(vec![Value::Int(3), Value::Int(4)]),
        ]),
    );
}

#[test]
fn cross_window_zero_error() {
    assert_both("[1, 2, 3].window(0)");
}
