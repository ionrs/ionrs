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
            panic!("tree-walk OK ({:?}) but VM errored: {} | src: {}", tw_val, vm_err.message, src);
        }
        (Err(tw_err), Ok(vm_val)) => {
            panic!("tree-walk errored ({}) but VM OK: {:?} | src: {}", tw_err.message, vm_val, src);
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
    assert_both_eq("let x = 5; if x < 0 { -1 } else if x == 0 { 0 } else { 1 }", Value::Int(1));
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
    assert_both_eq("let mut sum = 0; for x in [1, 2, 3] { sum += x; } sum", Value::Int(6));
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
        "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(10)",
        Value::Int(55),
    );
}

#[test]
fn cross_fn_default_params() {
    assert_both_eq(r#"fn greet(name = "world") { name } greet()"#, Value::Str("world".to_string()));
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
    assert_both_eq("[1, 2, 3]", Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
    assert_both_eq("[1, 2, 3].len()", Value::Int(3));
}

#[test]
fn cross_tuple() {
    assert_both_eq("(1, 2, 3)", Value::Tuple(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
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
    assert_both_eq("match Some(5) { Some(v) => v * 2, None => 0 }", Value::Int(10));
    assert_both_eq("match None { Some(v) => v, None => 99 }", Value::Int(99));
}

#[test]
fn cross_match_result() {
    assert_both_eq("match Ok(7) { Ok(v) => v, Err(e) => 0 }", Value::Int(7));
    assert_both_eq(r#"match Err("fail") { Ok(v) => 0, Err(e) => e }"#, Value::Str("fail".to_string()));
}

#[test]
fn cross_match_tuple() {
    assert_both_eq("match (1, 2) { (a, b) => a + b, _ => 0 }", Value::Int(3));
}

#[test]
fn cross_match_list() {
    assert_both_eq("match [1, 2, 3] { [a, b, c] => a + b + c, _ => 0 }", Value::Int(6));
    assert_both_eq("match [] { [] => 1, _ => 0 }", Value::Int(1));
}

// ============================================================
// String operations
// ============================================================

#[test]
fn cross_fstring() {
    assert_both_eq(r#"let x = 42; f"val={x}""#, Value::Str("val=42".to_string()));
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
    assert_both_eq(r#""hello".find("ell")"#, Value::Option(Some(Box::new(Value::Int(1)))));
    assert_both_eq(r#""hello".find("xyz")"#, Value::Option(None));
}

#[test]
fn cross_string_split_replace() {
    assert_both_eq(r#""a,b,c".split(",")"#, Value::List(vec![
        Value::Str("a".to_string()), Value::Str("b".to_string()), Value::Str("c".to_string()),
    ]));
    assert_both_eq(r#""hello".replace("l", "r")"#, Value::Str("herro".to_string()));
}

#[test]
fn cross_string_chars() {
    assert_both_eq(r#""hi".chars()"#, Value::List(vec![
        Value::Str("h".to_string()), Value::Str("i".to_string()),
    ]));
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
    assert_both_eq("let x = 1; let y = { let x = 2; x + 10 }; x + y", Value::Int(13));
}

// ============================================================
// Complex programs
// ============================================================

#[test]
fn cross_fibonacci() {
    assert_both_eq(
        "fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } } fib(10)",
        Value::Int(55),
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
    assert_both_eq(r#"
        fn classify(n) {
            match n % 3 {
                0 => "fizz",
                1 => "one",
                _ => "other",
            }
        }
        classify(9)
    "#, Value::Str("fizz".to_string()));
}

#[test]
fn cross_nested_loops() {
    assert_both_eq(r#"
        let mut sum = 0;
        for i in [1, 2, 3] {
            for j in [10, 20] {
                sum += i * j;
            }
        }
        sum
    "#, Value::Int(180));
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
fn cross_dict_keys_values() {
    assert_both_eq(r#"#{"a": 1, "b": 2}.keys()"#, Value::List(vec![
        Value::Str("a".to_string()), Value::Str("b".to_string()),
    ]));
    assert_both_eq(r#"#{"a": 1, "b": 2}.values()"#, Value::List(vec![Value::Int(1), Value::Int(2)]));
}

#[test]
fn cross_dict_entries() {
    assert_both_eq(r#"#{"a": 1}.entries()"#, Value::List(vec![
        Value::Tuple(vec![Value::Str("a".to_string()), Value::Int(1)]),
    ]));
}

// ============================================================
// Math builtins
// ============================================================

#[test]
fn cross_math_builtins() {
    assert_both_eq("abs(-5)", Value::Int(5));
    assert_both_eq("abs(3.5)", Value::Float(3.5));
    assert_both_eq("min(3, 7)", Value::Int(3));
    assert_both_eq("max(3, 7)", Value::Int(7));
    assert_both_eq("floor(3.7)", Value::Float(3.0));
    assert_both_eq("ceil(3.2)", Value::Float(4.0));
    assert_both_eq("round(3.5)", Value::Float(4.0));
    assert_both_eq("sqrt(16.0)", Value::Float(4.0));
    assert_both_eq("pow(2, 10)", Value::Int(1024));
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
