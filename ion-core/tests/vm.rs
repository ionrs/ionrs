//! Tests for the bytecode VM execution path.

use ion_core::engine::Engine;
use ion_core::value::Value;

fn vm_eval(src: &str) -> Value {
    let mut engine = Engine::new();
    engine.vm_eval(src).unwrap()
}

fn vm_eval_err(src: &str) -> String {
    let mut engine = Engine::new();
    engine.vm_eval(src).unwrap_err().message
}

// ============================================================
// Literals and basic expressions
// ============================================================

#[test]
fn test_vm_int() {
    assert_eq!(vm_eval("42"), Value::Int(42));
}

#[test]
fn test_vm_float() {
    assert_eq!(vm_eval("3.14"), Value::Float(3.14));
}

#[test]
fn test_vm_bool() {
    assert_eq!(vm_eval("true"), Value::Bool(true));
    assert_eq!(vm_eval("false"), Value::Bool(false));
}

#[test]
fn test_vm_string() {
    assert_eq!(vm_eval("\"hello\""), Value::Str("hello".to_string()));
}

#[test]
fn test_vm_unit() {
    assert_eq!(vm_eval("()"), Value::Unit);
}

#[test]
fn test_vm_none() {
    assert_eq!(vm_eval("None"), Value::Option(None));
}

// ============================================================
// Arithmetic
// ============================================================

#[test]
fn test_vm_add() {
    assert_eq!(vm_eval("1 + 2"), Value::Int(3));
    assert_eq!(vm_eval("1.5 + 2.5"), Value::Float(4.0));
}

#[test]
fn test_vm_sub() {
    assert_eq!(vm_eval("10 - 3"), Value::Int(7));
}

#[test]
fn test_vm_mul() {
    assert_eq!(vm_eval("4 * 5"), Value::Int(20));
}

#[test]
fn test_vm_div() {
    assert_eq!(vm_eval("10 / 3"), Value::Int(3));
    assert_eq!(vm_eval("10.0 / 3.0"), Value::Float(10.0 / 3.0));
}

#[test]
fn test_vm_mod() {
    assert_eq!(vm_eval("10 % 3"), Value::Int(1));
}

#[test]
fn test_vm_neg() {
    assert_eq!(vm_eval("-5"), Value::Int(-5));
}

#[test]
fn test_vm_complex_arithmetic() {
    assert_eq!(vm_eval("(2 + 3) * 4 - 1"), Value::Int(19));
}

// ============================================================
// Comparison
// ============================================================

#[test]
fn test_vm_comparisons() {
    assert_eq!(vm_eval("1 == 1"), Value::Bool(true));
    assert_eq!(vm_eval("1 != 2"), Value::Bool(true));
    assert_eq!(vm_eval("1 < 2"), Value::Bool(true));
    assert_eq!(vm_eval("2 > 1"), Value::Bool(true));
    assert_eq!(vm_eval("1 <= 1"), Value::Bool(true));
    assert_eq!(vm_eval("1 >= 1"), Value::Bool(true));
}

// ============================================================
// Logic
// ============================================================

#[test]
fn test_vm_not() {
    assert_eq!(vm_eval("!true"), Value::Bool(false));
    assert_eq!(vm_eval("!false"), Value::Bool(true));
}

#[test]
fn test_vm_and_or() {
    assert_eq!(vm_eval("true && false"), Value::Bool(false));
    assert_eq!(vm_eval("true || false"), Value::Bool(true));
    assert_eq!(vm_eval("false || true"), Value::Bool(true));
}

#[test]
fn test_vm_short_circuit() {
    // `false && <anything>` should short-circuit
    assert_eq!(vm_eval("false && (1 / 0 == 0)"), Value::Bool(false));
}

// ============================================================
// Variables
// ============================================================

#[test]
fn test_vm_let() {
    assert_eq!(vm_eval("let x = 42; x"), Value::Int(42));
}

#[test]
fn test_vm_let_mut() {
    assert_eq!(vm_eval("let mut x = 1; x = 2; x"), Value::Int(2));
}

#[test]
fn test_vm_compound_assign() {
    assert_eq!(vm_eval("let mut x = 10; x += 5; x"), Value::Int(15));
    assert_eq!(vm_eval("let mut x = 10; x -= 3; x"), Value::Int(7));
    assert_eq!(vm_eval("let mut x = 10; x *= 2; x"), Value::Int(20));
    assert_eq!(vm_eval("let mut x = 10; x /= 2; x"), Value::Int(5));
}

#[test]
fn test_vm_immutable_error() {
    let err = vm_eval_err("let x = 1; x = 2; x");
    assert!(err.contains("immutable"), "got: {}", err);
}

// ============================================================
// Control flow
// ============================================================

#[test]
fn test_vm_if() {
    assert_eq!(vm_eval("if true { 1 } else { 2 }"), Value::Int(1));
    assert_eq!(vm_eval("if false { 1 } else { 2 }"), Value::Int(2));
}

#[test]
fn test_vm_if_no_else() {
    assert_eq!(vm_eval("if true { 42 }"), Value::Int(42));
    assert_eq!(vm_eval("if false { 42 }"), Value::Unit);
}

#[test]
fn test_vm_block() {
    assert_eq!(vm_eval("{ let x = 10; x + 5 }"), Value::Int(15));
}

#[test]
fn test_vm_while() {
    assert_eq!(vm_eval("let mut x = 0; while x < 5 { x += 1; } x"), Value::Int(5));
}

#[test]
fn test_vm_for() {
    assert_eq!(vm_eval("let mut sum = 0; for x in [1, 2, 3] { sum += x; } sum"), Value::Int(6));
}

// ============================================================
// Functions
// ============================================================

#[test]
fn test_vm_fn_decl_and_call() {
    assert_eq!(vm_eval("fn add(a, b) { a + b } add(3, 4)"), Value::Int(7));
}

#[test]
fn test_vm_recursive_fn() {
    assert_eq!(vm_eval("
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "), Value::Int(55));
}

#[test]
fn test_vm_builtin_fn() {
    assert_eq!(vm_eval("len([1, 2, 3])"), Value::Int(3));
    assert_eq!(vm_eval("abs(-5)"), Value::Int(5));
}

// ============================================================
// Collections
// ============================================================

#[test]
fn test_vm_list() {
    assert_eq!(vm_eval("[1, 2, 3]"), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
}

#[test]
fn test_vm_tuple() {
    assert_eq!(vm_eval("(1, 2)"), Value::Tuple(vec![Value::Int(1), Value::Int(2)]));
}

#[test]
fn test_vm_dict() {
    assert_eq!(vm_eval("let d = #{\"a\": 1}; d[\"a\"]"), Value::Int(1));
}

#[test]
fn test_vm_list_index() {
    assert_eq!(vm_eval("[10, 20, 30][1]"), Value::Int(20));
}

#[test]
fn test_vm_negative_index() {
    assert_eq!(vm_eval("[10, 20, 30][-1]"), Value::Int(30));
}

// ============================================================
// String operations
// ============================================================

#[test]
fn test_vm_string_concat() {
    assert_eq!(vm_eval("\"hello\" + \" \" + \"world\""), Value::Str("hello world".to_string()));
}

#[test]
fn test_vm_fstring() {
    assert_eq!(vm_eval("let name = \"world\"; f\"hello {name}\""), Value::Str("hello world".to_string()));
}

// ============================================================
// Method calls
// ============================================================

#[test]
fn test_vm_list_methods() {
    assert_eq!(vm_eval("[1, 2, 3].len()"), Value::Int(3));
    assert_eq!(vm_eval("[1, 2, 3].contains(2)"), Value::Bool(true));
    assert_eq!(vm_eval("[].is_empty()"), Value::Bool(true));
}

#[test]
fn test_vm_string_methods() {
    assert_eq!(vm_eval("\"hello\".to_upper()"), Value::Str("HELLO".to_string()));
    assert_eq!(vm_eval("\"  hi  \".trim()"), Value::Str("hi".to_string()));
    assert_eq!(vm_eval("\"hello\".len()"), Value::Int(5));
}

#[test]
fn test_vm_dict_methods() {
    assert_eq!(vm_eval("#{\"a\": 1, \"b\": 2}.len()"), Value::Int(2));
    assert_eq!(vm_eval("#{\"a\": 1}.contains_key(\"a\")"), Value::Bool(true));
}

// ============================================================
// Option / Result
// ============================================================

#[test]
fn test_vm_some() {
    assert_eq!(vm_eval("Some(42)"), Value::Option(Some(Box::new(Value::Int(42)))));
}

#[test]
fn test_vm_option_methods() {
    assert_eq!(vm_eval("Some(42).is_some()"), Value::Bool(true));
    assert_eq!(vm_eval("None.is_none()"), Value::Bool(true));
    assert_eq!(vm_eval("None.unwrap_or(0)"), Value::Int(0));
}

#[test]
fn test_vm_ok_err() {
    assert_eq!(vm_eval("Ok(1).is_ok()"), Value::Bool(true));
    assert_eq!(vm_eval("Err(\"bad\").is_err()"), Value::Bool(true));
}

#[test]
fn test_vm_try_operator() {
    assert_eq!(vm_eval("
        fn get_val() { Some(42)? }
        get_val()
    "), Value::Int(42));
}

// ============================================================
// Range
// ============================================================

#[test]
fn test_vm_range() {
    assert_eq!(vm_eval("let mut sum = 0; for x in 1..4 { sum += x; } sum"), Value::Int(6));
}

#[test]
fn test_vm_range_inclusive() {
    assert_eq!(vm_eval("let mut sum = 0; for x in 1..=3 { sum += x; } sum"), Value::Int(6));
}

// ============================================================
// Pipe operator
// ============================================================

#[test]
fn test_vm_pipe() {
    assert_eq!(vm_eval("-5 |> abs"), Value::Int(5));
}

// ============================================================
// Scoping
// ============================================================

#[test]
fn test_vm_scope_isolation() {
    assert_eq!(vm_eval("let x = 1; { let x = 2; }; x"), Value::Int(1));
}

// ============================================================
// Tuple destructuring
// ============================================================

#[test]
fn test_vm_tuple_destructure() {
    assert_eq!(vm_eval("let (a, b) = (10, 20); a + b"), Value::Int(30));
}

// ============================================================
// Fallback to tree-walk for unsupported features
// ============================================================

#[test]
fn test_vm_fallback_match() {
    // match is unsupported in VM, should fall back to tree-walk
    assert_eq!(vm_eval("match 1 { 1 => \"one\", _ => \"other\" }"), Value::Str("one".to_string()));
}

#[test]
fn test_vm_fallback_list_comp() {
    assert_eq!(vm_eval("[x * 2 for x in [1, 2, 3]]"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)]));
}

// ============================================================
// Bitwise operators (VM path)
// ============================================================

#[test]
fn test_vm_bitwise() {
    assert_eq!(vm_eval("12 & 10"), Value::Int(8));
    assert_eq!(vm_eval("12 | 10"), Value::Int(14));
    assert_eq!(vm_eval("12 ^ 10"), Value::Int(6));
    assert_eq!(vm_eval("1 << 4"), Value::Int(16));
    assert_eq!(vm_eval("16 >> 2"), Value::Int(4));
}
