//! Tests for the bytecode VM execution path.
#![cfg(feature = "vm")]

use ion_core::engine::Engine;
use ion_core::host_types::{HostEnumDef, HostStructDef, HostVariantDef};
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
    assert_eq!(
        vm_eval("let mut x = 0; while x < 5 { x += 1; } x"),
        Value::Int(5)
    );
}

#[test]
fn test_vm_for() {
    assert_eq!(
        vm_eval("let mut sum = 0; for x in [1, 2, 3] { sum += x; } sum"),
        Value::Int(6)
    );
}

#[test]
fn test_vm_continue_in_for() {
    assert_eq!(
        vm_eval(
            "
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            if x == 3 { continue; }
            sum += x;
        }
        sum
    "
        ),
        Value::Int(12)
    );
}

#[test]
fn test_vm_continue_in_while() {
    assert_eq!(
        vm_eval(
            "
        let mut sum = 0;
        let mut i = 0;
        while i < 5 {
            i += 1;
            if i == 3 { continue; }
            sum += i;
        }
        sum
    "
        ),
        Value::Int(12)
    );
}

#[test]
fn test_vm_break_in_for() {
    assert_eq!(
        vm_eval(
            "
        let mut sum = 0;
        for x in [1, 2, 3, 4, 5] {
            if x == 4 { break; }
            sum += x;
        }
        sum
    "
        ),
        Value::Int(6)
    );
}

#[test]
fn test_vm_nested_continue() {
    assert_eq!(
        vm_eval(
            "
        let mut sum = 0;
        for x in [1, 2, 3] {
            for y in [10, 20, 30] {
                if y == 20 { continue; }
                sum += y;
            }
            sum += x;
        }
        sum
    "
        ),
        Value::Int(126)
    );
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
    assert_eq!(
        vm_eval(
            "
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "
        ),
        Value::Int(55)
    );
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
    assert_eq!(
        vm_eval("[1, 2, 3]"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_vm_tuple() {
    assert_eq!(
        vm_eval("(1, 2)"),
        Value::Tuple(vec![Value::Int(1), Value::Int(2)])
    );
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
    assert_eq!(
        vm_eval("\"hello\" + \" \" + \"world\""),
        Value::Str("hello world".to_string())
    );
}

#[test]
fn test_vm_fstring() {
    assert_eq!(
        vm_eval("let name = \"world\"; f\"hello {name}\""),
        Value::Str("hello world".to_string())
    );
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
    assert_eq!(
        vm_eval("\"hello\".to_upper()"),
        Value::Str("HELLO".to_string())
    );
    assert_eq!(vm_eval("\"  hi  \".trim()"), Value::Str("hi".to_string()));
    assert_eq!(vm_eval("\"hello\".len()"), Value::Int(5));
}

#[test]
fn test_vm_dict_methods() {
    assert_eq!(vm_eval("#{\"a\": 1, \"b\": 2}.len()"), Value::Int(2));
    assert_eq!(
        vm_eval("#{\"a\": 1}.contains_key(\"a\")"),
        Value::Bool(true)
    );
}

// ============================================================
// Option / Result
// ============================================================

#[test]
fn test_vm_some() {
    assert_eq!(
        vm_eval("Some(42)"),
        Value::Option(Some(Box::new(Value::Int(42))))
    );
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
    assert_eq!(
        vm_eval(
            "
        fn get_val() { Some(42)? }
        get_val()
    "
        ),
        Value::Int(42)
    );
}

// ============================================================
// Range
// ============================================================

#[test]
fn test_vm_range() {
    assert_eq!(
        vm_eval("let mut sum = 0; for x in 1..4 { sum += x; } sum"),
        Value::Int(6)
    );
}

#[test]
fn test_vm_range_inclusive() {
    assert_eq!(
        vm_eval("let mut sum = 0; for x in 1..=3 { sum += x; } sum"),
        Value::Int(6)
    );
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

// ============================================================
// Match expressions (VM-native)
// ============================================================

#[test]
fn test_vm_match_int() {
    assert_eq!(
        vm_eval("match 1 { 1 => \"one\", 2 => \"two\", _ => \"other\" }"),
        Value::Str("one".to_string())
    );
    assert_eq!(
        vm_eval("match 2 { 1 => \"one\", 2 => \"two\", _ => \"other\" }"),
        Value::Str("two".to_string())
    );
    assert_eq!(
        vm_eval("match 99 { 1 => \"one\", 2 => \"two\", _ => \"other\" }"),
        Value::Str("other".to_string())
    );
}

#[test]
fn test_vm_match_bool() {
    assert_eq!(
        vm_eval("match true { true => 1, false => 0 }"),
        Value::Int(1)
    );
    assert_eq!(
        vm_eval("match false { true => 1, false => 0 }"),
        Value::Int(0)
    );
}

#[test]
fn test_vm_match_string() {
    assert_eq!(
        vm_eval("match \"hi\" { \"hi\" => 1, \"bye\" => 2, _ => 0 }"),
        Value::Int(1)
    );
}

#[test]
fn test_vm_match_binding() {
    assert_eq!(vm_eval("match 42 { x => x + 1 }"), Value::Int(43));
    assert_eq!(
        vm_eval("match 10 { 1 => \"one\", x => x * 2 }"),
        Value::Int(20)
    );
}

#[test]
fn test_vm_match_option() {
    assert_eq!(
        vm_eval("match Some(5) { Some(x) => x, None => 0 }"),
        Value::Int(5)
    );
    assert_eq!(
        vm_eval("match None { Some(x) => x, None => 0 }"),
        Value::Int(0)
    );
}

#[test]
fn test_vm_match_result() {
    assert_eq!(
        vm_eval("match Ok(10) { Ok(x) => x, Err(e) => 0 }"),
        Value::Int(10)
    );
    assert_eq!(
        vm_eval("match Err(\"fail\") { Ok(x) => 0, Err(e) => e }"),
        Value::Str("fail".to_string())
    );
}

#[test]
fn test_vm_match_tuple() {
    assert_eq!(vm_eval("match (1, 2) { (a, b) => a + b }"), Value::Int(3));
    assert_eq!(
        vm_eval("match (1, 2) { (1, x) => x, _ => 0 }"),
        Value::Int(2)
    );
}

#[test]
fn test_vm_match_wildcard() {
    assert_eq!(
        vm_eval("match 42 { _ => \"any\" }"),
        Value::Str("any".to_string())
    );
}

// ============================================================
// Closures / Lambdas (VM-native)
// ============================================================

#[test]
fn test_vm_lambda_basic() {
    assert_eq!(vm_eval("let f = |x| x + 1; f(10)"), Value::Int(11));
}

#[test]
fn test_vm_lambda_multi_param() {
    assert_eq!(vm_eval("let add = |a, b| a + b; add(3, 4)"), Value::Int(7));
}

#[test]
fn test_vm_lambda_closure_capture() {
    assert_eq!(
        vm_eval("let n = 10; let f = |x| x + n; f(5)"),
        Value::Int(15)
    );
}

#[test]
fn test_vm_lambda_in_list() {
    assert_eq!(
        vm_eval("let ops = [|x| x + 1, |x| x * 2]; ops[1](5)"),
        Value::Int(10)
    );
}

#[test]
fn test_vm_lambda_passed_to_fn() {
    assert_eq!(
        vm_eval("fn apply(f, x) { f(x) } apply(|x| x * 3, 7)"),
        Value::Int(21)
    );
}

// ============================================================
// List / Dict comprehensions (VM-native)
// ============================================================

#[test]
fn test_vm_list_comp_basic() {
    assert_eq!(
        vm_eval("[x * 2 for x in [1, 2, 3]]"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
    );
}

#[test]
fn test_vm_list_comp_with_filter() {
    assert_eq!(
        vm_eval("[x for x in [1, 2, 3, 4, 5] if x > 2]"),
        Value::List(vec![Value::Int(3), Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn test_vm_list_comp_transform_and_filter() {
    assert_eq!(
        vm_eval("[x * x for x in [1, 2, 3, 4] if x % 2 == 0]"),
        Value::List(vec![Value::Int(4), Value::Int(16)])
    );
}

#[test]
fn test_vm_list_comp_string() {
    assert_eq!(
        vm_eval("[c for c in \"abc\"]"),
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
            Value::Str("c".to_string())
        ])
    );
}

#[test]
fn test_vm_dict_comp() {
    let result = vm_eval("#{f\"{k}!\": v * 2 for (k, v) in #{\"a\": 1, \"b\": 2}}");
    match result {
        Value::Dict(map) => {
            assert_eq!(map.get("a!"), Some(&Value::Int(2)));
            assert_eq!(map.get("b!"), Some(&Value::Int(4)));
        }
        other => panic!("expected dict, got {:?}", other),
    }
}

// ============================================================
// If-let / While-let (VM-native)
// ============================================================

#[test]
fn test_vm_if_let_some() {
    assert_eq!(
        vm_eval("if let Some(x) = Some(42) { x } else { 0 }"),
        Value::Int(42)
    );
    assert_eq!(
        vm_eval("if let Some(x) = None { x } else { 0 }"),
        Value::Int(0)
    );
}

#[test]
fn test_vm_if_let_ok() {
    assert_eq!(
        vm_eval("if let Ok(x) = Ok(10) { x + 1 } else { 0 }"),
        Value::Int(11)
    );
    assert_eq!(
        vm_eval("if let Ok(x) = Err(\"bad\") { x } else { -1 }"),
        Value::Int(-1)
    );
}

#[test]
fn test_vm_if_let_no_else() {
    assert_eq!(vm_eval("if let Some(x) = Some(5) { x }"), Value::Int(5));
    assert_eq!(vm_eval("if let Some(x) = None { x }"), Value::Unit);
}

#[test]
fn test_vm_while_let() {
    assert_eq!(
        vm_eval(
            r#"
        let mut items = [Some(1), Some(2), Some(3), None];
        let mut sum = 0;
        let mut i = 0;
        while let Some(x) = items[i] {
            sum += x;
            i += 1;
        }
        sum
    "#
        ),
        Value::Int(6)
    );
}

// ============================================================
// Index/field assignment (VM-native)
// ============================================================

#[test]
fn test_vm_list_index_assign() {
    assert_eq!(
        vm_eval("let mut a = [1, 2, 3]; a[0] = 10; a"),
        Value::List(vec![Value::Int(10), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_vm_list_index_compound_assign() {
    assert_eq!(
        vm_eval("let mut a = [1, 2, 3]; a[1] += 10; a"),
        Value::List(vec![Value::Int(1), Value::Int(12), Value::Int(3)])
    );
}

#[test]
fn test_vm_dict_field_assign() {
    let result = vm_eval("let mut d = #{\"x\": 1}; d.x = 42; d.x");
    assert_eq!(result, Value::Int(42));
}

#[test]
fn test_vm_dict_index_assign() {
    let result = vm_eval("let mut d = #{\"a\": 1, \"b\": 2}; d[\"a\"] = 99; d[\"a\"]");
    assert_eq!(result, Value::Int(99));
}

#[test]
fn test_vm_dict_field_compound_assign() {
    let result = vm_eval("let mut d = #{\"x\": 10}; d.x += 5; d.x");
    assert_eq!(result, Value::Int(15));
}

// ============================================================
// Closure-based methods (VM-native)
// ============================================================

#[test]
fn test_vm_list_map() {
    assert_eq!(
        vm_eval("[1, 2, 3].map(|x| x * 2)"),
        Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
    );
}

#[test]
fn test_vm_list_filter() {
    assert_eq!(
        vm_eval("[1, 2, 3, 4, 5].filter(|x| x > 2)"),
        Value::List(vec![Value::Int(3), Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn test_vm_list_fold() {
    assert_eq!(
        vm_eval("[1, 2, 3, 4].fold(0, |acc, x| acc + x)"),
        Value::Int(10)
    );
}

#[test]
fn test_vm_list_flat_map() {
    assert_eq!(
        vm_eval("[1, 2, 3].flat_map(|x| [x, x * 10])"),
        Value::List(vec![
            Value::Int(1),
            Value::Int(10),
            Value::Int(2),
            Value::Int(20),
            Value::Int(3),
            Value::Int(30)
        ])
    );
}

#[test]
fn test_vm_list_any_all() {
    assert_eq!(vm_eval("[1, 2, 3].any(|x| x > 2)"), Value::Bool(true));
    assert_eq!(vm_eval("[1, 2, 3].any(|x| x > 5)"), Value::Bool(false));
    assert_eq!(vm_eval("[1, 2, 3].all(|x| x > 0)"), Value::Bool(true));
    assert_eq!(vm_eval("[1, 2, 3].all(|x| x > 2)"), Value::Bool(false));
}

#[test]
fn test_vm_option_map() {
    assert_eq!(
        vm_eval("Some(5).map(|x| x * 2)"),
        Value::Option(Some(Box::new(Value::Int(10))))
    );
    assert_eq!(vm_eval("None.map(|x| x * 2)"), Value::Option(None));
}

#[test]
fn test_vm_option_and_then() {
    assert_eq!(
        vm_eval("Some(5).and_then(|x| Some(x + 1))"),
        Value::Option(Some(Box::new(Value::Int(6))))
    );
    assert_eq!(
        vm_eval("None.and_then(|x| Some(x + 1))"),
        Value::Option(None)
    );
}

#[test]
fn test_vm_result_map() {
    assert_eq!(
        vm_eval("Ok(10).map(|x| x + 1)"),
        Value::Result(Ok(Box::new(Value::Int(11))))
    );
}

#[test]
fn test_vm_result_map_err() {
    assert_eq!(
        vm_eval("Err(\"bad\").map_err(|e| f\"error: {e}\")"),
        Value::Result(Err(Box::new(Value::Str("error: bad".to_string()))))
    );
}

// ============================================================
// List pattern matching (VM-native)
// ============================================================

#[test]
fn test_vm_match_list() {
    assert_eq!(
        vm_eval("match [1, 2, 3] { [a, b, c] => a + b + c, _ => 0 }"),
        Value::Int(6)
    );
}

#[test]
fn test_vm_match_list_rest() {
    assert_eq!(
        vm_eval("match [1, 2, 3, 4] { [first, ...rest] => first, _ => 0 }"),
        Value::Int(1)
    );
}

#[test]
fn test_vm_match_list_rest_binding() {
    assert_eq!(
        vm_eval("match [1, 2, 3, 4] { [h, ...t] => t }"),
        Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
    );
}

#[test]
fn test_vm_match_list_empty() {
    assert_eq!(
        vm_eval("match [] { [] => \"empty\", _ => \"nonempty\" }"),
        Value::Str("empty".to_string())
    );
    assert_eq!(
        vm_eval("match [1] { [] => \"empty\", _ => \"nonempty\" }"),
        Value::Str("nonempty".to_string())
    );
}

#[test]
fn test_vm_match_list_length_mismatch() {
    assert_eq!(
        vm_eval("match [1, 2] { [a, b, c] => 0, [a, b] => a + b, _ => -1 }"),
        Value::Int(3)
    );
}

#[test]
fn test_vm_let_list_destructure() {
    assert_eq!(vm_eval("let [a, b, c] = [10, 20, 30]; b"), Value::Int(20));
}

#[test]
fn test_vm_let_list_rest() {
    assert_eq!(
        vm_eval("let [h, ...t] = [1, 2, 3, 4]; t"),
        Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
    );
}

#[test]
fn test_vm_for_list_destructure() {
    assert_eq!(
        vm_eval("let mut sum = 0; for [a, b] in [[1, 2], [3, 4]] { sum += a + b; } sum"),
        Value::Int(10)
    );
}

// ============================================================
// String interning / perf (VM-native)
// ============================================================

#[test]
fn test_vm_many_variable_accesses() {
    // Exercises the interned symbol lookup path heavily
    assert_eq!(
        vm_eval(
            r#"
        let mut x = 0;
        let mut y = 0;
        let mut z = 0;
        for i in 0..100 {
            x += 1;
            y += 2;
            z += 3;
        }
        x + y + z
    "#
        ),
        Value::Int(600)
    );
}

#[test]
fn test_vm_nested_scopes_many_vars() {
    assert_eq!(
        vm_eval(
            r#"
        let a = 1;
        let b = 2;
        let result = {
            let a = 10;
            let c = 3;
            a + b + c
        };
        result + a
    "#
        ),
        Value::Int(16)
    );
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

// ============================================================
// Missing stdlib methods (VM path)
// ============================================================

#[test]
fn test_vm_list_first_last() {
    assert_eq!(
        vm_eval("[1, 2, 3].first()"),
        Value::Option(Some(Box::new(Value::Int(1))))
    );
    assert_eq!(
        vm_eval("[1, 2, 3].last()"),
        Value::Option(Some(Box::new(Value::Int(3))))
    );
    assert_eq!(vm_eval("[].first()"), Value::Option(None));
    assert_eq!(vm_eval("[].last()"), Value::Option(None));
}

#[test]
fn test_vm_list_sort() {
    assert_eq!(
        vm_eval("[3, 1, 2].sort()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        vm_eval(r#"["banana", "apple", "cherry"].sort()"#),
        Value::List(vec![
            Value::Str("apple".into()),
            Value::Str("banana".into()),
            Value::Str("cherry".into())
        ])
    );
}

#[test]
fn test_vm_list_flatten() {
    assert_eq!(
        vm_eval("[[1, 2], [3, 4], [5]].flatten()"),
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
            Value::Int(5)
        ])
    );
    assert_eq!(
        vm_eval("[[1], 2, [3]].flatten()"),
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_vm_list_zip() {
    assert_eq!(
        vm_eval("[1, 2, 3].zip([\"a\", \"b\", \"c\"])"),
        Value::List(vec![
            Value::Tuple(vec![Value::Int(1), Value::Str("a".into())]),
            Value::Tuple(vec![Value::Int(2), Value::Str("b".into())]),
            Value::Tuple(vec![Value::Int(3), Value::Str("c".into())]),
        ])
    );
}

#[test]
fn test_vm_dict_entries() {
    let result = vm_eval(r#"let d = #{"a": 1}; d.entries()"#);
    assert_eq!(
        result,
        Value::List(vec![Value::Tuple(vec![
            Value::Str("a".into()),
            Value::Int(1)
        ])])
    );
}

#[test]
fn test_vm_dict_insert() {
    let result = vm_eval(r#"let d = #{"a": 1}; d.insert("b", 2)"#);
    if let Value::Dict(map) = result {
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("b"), Some(&Value::Int(2)));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_vm_dict_remove() {
    let result = vm_eval(r#"let d = #{"a": 1, "b": 2}; d.remove("a")"#);
    if let Value::Dict(map) = result {
        assert_eq!(map.len(), 1);
        assert!(map.get("a").is_none());
        assert_eq!(map.get("b"), Some(&Value::Int(2)));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_vm_dict_merge() {
    let result = vm_eval(r#"let a = #{"x": 1}; let b = #{"y": 2}; a.merge(b)"#);
    if let Value::Dict(map) = result {
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("x"), Some(&Value::Int(1)));
        assert_eq!(map.get("y"), Some(&Value::Int(2)));
    } else {
        panic!("expected dict");
    }
}

// ============================================================
// Compilation caching (VM path)
// ============================================================

#[test]
fn test_vm_fn_cache_recursive() {
    // fibonacci exercises the cache: same function compiled once, called many times
    assert_eq!(
        vm_eval(
            r#"
        fn fib(n) { if n <= 1 { n } else { fib(n - 1) + fib(n - 2) } }
        fib(15)
    "#
        ),
        Value::Int(610)
    );
}

#[test]
fn test_vm_fn_cache_repeated_calls() {
    assert_eq!(
        vm_eval(
            r#"
        fn double(x) { x * 2 }
        let a = double(1);
        let b = double(2);
        let c = double(3);
        a + b + c
    "#
        ),
        Value::Int(12)
    );
}

// ============================================================
// Error source spans (VM path)
// ============================================================

#[test]
fn test_vm_error_has_col_info() {
    use ion_core::error::IonError;
    let mut engine = Engine::new();
    let err: IonError = engine.vm_eval("1 / 0").unwrap_err();
    assert_eq!(err.message, "division by zero");
    assert_eq!(err.line, 1);
    assert!(err.col > 0, "expected non-zero column, got {}", err.col);
}

#[test]
fn test_vm_error_field_access_col() {
    use ion_core::error::IonError;
    let mut engine = Engine::new();
    let err: IonError = engine.vm_eval("let x = 42; x.foo").unwrap_err();
    assert_eq!(err.line, 1);
    assert!(
        err.col > 0,
        "expected non-zero column for field error, got {}",
        err.col
    );
}

// ============================================================
// Tail call optimization (VM path)
// ============================================================

#[test]
#[cfg(feature = "optimize")]
fn test_vm_tail_call_deep_recursion() {
    // This would stack overflow without TCO — 10,000 recursive calls
    assert_eq!(
        vm_eval(
            r#"
        fn countdown(n) {
            if n <= 0 { 0 } else { countdown(n - 1) }
        }
        countdown(10000)
    "#
        ),
        Value::Int(0)
    );
}

#[test]
#[cfg(feature = "optimize")]
fn test_vm_tail_call_accumulator() {
    // Tail-recursive sum with accumulator
    assert_eq!(
        vm_eval(
            r#"
        fn sum_acc(n, acc) {
            if n <= 0 { acc } else { sum_acc(n - 1, acc + n) }
        }
        sum_acc(10000, 0)
    "#
        ),
        Value::Int(50005000)
    );
}

// ============================================================
// Dict spread
// ============================================================

#[test]
fn test_vm_dict_spread_basic() {
    let val = vm_eval(
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
fn test_vm_dict_spread_override() {
    let val = vm_eval(
        r#"
        let base = #{ "a": 1, "b": 2 };
        #{ ...base, "b": 99 }
    "#,
    );
    if let Value::Dict(map) = val {
        assert_eq!(map["a"], Value::Int(1));
        assert_eq!(map["b"], Value::Int(99));
    } else {
        panic!("expected dict");
    }
}

#[test]
fn test_vm_dict_spread_multiple() {
    let val = vm_eval(
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
fn test_vm_dict_spread_non_dict_error() {
    let err = vm_eval_err(r#"#{ ...[1, 2, 3] }"#);
    assert!(err.contains("spread requires a dict"), "got: {}", err);
}

// ============================================================
// Constant folding
// ============================================================

#[test]
fn test_vm_const_fold_arithmetic() {
    assert_eq!(vm_eval("2 + 3"), Value::Int(5));
    assert_eq!(vm_eval("10 - 3"), Value::Int(7));
    assert_eq!(vm_eval("4 * 5"), Value::Int(20));
    assert_eq!(vm_eval("10 / 3"), Value::Int(3));
    assert_eq!(vm_eval("10 % 3"), Value::Int(1));
}

#[test]
fn test_vm_const_fold_float() {
    assert_eq!(vm_eval("1.5 + 2.5"), Value::Float(4.0));
    assert_eq!(vm_eval("2 + 1.5"), Value::Float(3.5));
    assert_eq!(vm_eval("1.5 * 2"), Value::Float(3.0));
}

#[test]
fn test_vm_const_fold_string_concat() {
    assert_eq!(
        vm_eval(r#""hello" + " world""#),
        Value::Str("hello world".into())
    );
}

#[test]
fn test_vm_const_fold_comparison() {
    assert_eq!(vm_eval("3 > 2"), Value::Bool(true));
    assert_eq!(vm_eval("1 == 2"), Value::Bool(false));
    assert_eq!(vm_eval("5 <= 5"), Value::Bool(true));
}

#[test]
fn test_vm_const_fold_bool_logic() {
    assert_eq!(vm_eval("true && false"), Value::Bool(false));
    assert_eq!(vm_eval("true || false"), Value::Bool(true));
}

#[test]
fn test_vm_const_fold_unary() {
    assert_eq!(vm_eval("-42"), Value::Int(-42));
    assert_eq!(vm_eval("!true"), Value::Bool(false));
    assert_eq!(vm_eval("-3.14"), Value::Float(-3.14));
}

#[test]
fn test_vm_const_fold_nested() {
    // Nested constant expressions should fold through AST structure
    // (2 + 3) is folded to 5, then 5 * 4 can't fold because AST is (2+3)*4
    // but the inner fold still helps
    assert_eq!(vm_eval("(2 + 3) * 4"), Value::Int(20));
}

// ============================================================
// Host types
// ============================================================

fn vm_engine_with_types() -> Engine {
    let mut engine = Engine::new();
    engine.register_struct(HostStructDef {
        name: "Config".into(),
        fields: vec!["host".into(), "port".into(), "debug".into()],
    });
    engine.register_enum(HostEnumDef {
        name: "Color".into(),
        variants: vec![
            HostVariantDef {
                name: "Red".into(),
                arity: 0,
            },
            HostVariantDef {
                name: "Custom".into(),
                arity: 3,
            },
        ],
    });
    engine
}

#[test]
fn test_vm_host_struct_construct() {
    let mut engine = vm_engine_with_types();
    let val = engine
        .vm_eval(r#"Config { host: "localhost", port: 8080, debug: true }"#)
        .unwrap();
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
fn test_vm_host_struct_field_access() {
    let mut engine = vm_engine_with_types();
    let val = engine
        .vm_eval(
            r#"
        let c = Config { host: "localhost", port: 8080, debug: false };
        c.port
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Int(8080));
}

#[test]
fn test_vm_host_struct_spread() {
    let mut engine = vm_engine_with_types();
    let val = engine
        .vm_eval(
            r#"
        let base = Config { host: "localhost", port: 8080, debug: false };
        let updated = Config { ...base, debug: true };
        updated.debug
    "#,
        )
        .unwrap();
    assert_eq!(val, Value::Bool(true));
}

#[test]
fn test_vm_host_enum_variant() {
    let mut engine = vm_engine_with_types();
    let val = engine.vm_eval("Color::Red").unwrap();
    if let Value::HostEnum {
        enum_name,
        variant,
        data,
    } = &val
    {
        assert_eq!(enum_name, "Color");
        assert_eq!(variant, "Red");
        assert!(data.is_empty());
    } else {
        panic!("expected HostEnum, got: {:?}", val);
    }
}

#[test]
fn test_vm_dead_code_after_return() {
    // Code after return should be eliminated — function should still work
    assert_eq!(
        vm_eval(
            r#"
        fn foo() {
            return 42;
            let x = 100;
            x + 1
        }
        foo()
    "#
        ),
        Value::Int(42)
    );
}

#[test]
fn test_vm_dead_code_after_break() {
    assert_eq!(
        vm_eval(
            r#"
        let mut sum = 0;
        for i in [1, 2, 3, 4, 5] {
            if i > 3 {
                break;
                sum = sum + 999;
            }
            sum = sum + i;
        }
        sum
    "#
        ),
        Value::Int(6)
    );
}

#[test]
fn test_vm_host_enum_variant_with_data() {
    let mut engine = vm_engine_with_types();
    let val = engine.vm_eval("Color::Custom(255, 128, 0)").unwrap();
    if let Value::HostEnum {
        enum_name,
        variant,
        data,
    } = &val
    {
        assert_eq!(enum_name, "Color");
        assert_eq!(variant, "Custom");
        assert_eq!(data, &vec![Value::Int(255), Value::Int(128), Value::Int(0)]);
    } else {
        panic!("expected HostEnum, got: {:?}", val);
    }
}

// ============================================================
// Peephole optimization (correctness under optimization)
// ============================================================

#[test]
fn test_vm_peephole_double_not() {
    assert_eq!(vm_eval("!!true"), Value::Bool(true));
    assert_eq!(vm_eval("!!false"), Value::Bool(false));
    assert_eq!(vm_eval("let x = 5; !!(x > 3)"), Value::Bool(true));
}

#[test]
fn test_vm_peephole_double_neg() {
    assert_eq!(vm_eval("--5"), Value::Int(5));
    assert_eq!(vm_eval("let x = 42; --x"), Value::Int(42));
}

#[test]
fn test_vm_peephole_if_else_correctness() {
    assert_eq!(vm_eval("if true { 1 } else { 2 }"), Value::Int(1));
    assert_eq!(vm_eval("if false { 1 } else { 2 }"), Value::Int(2));
    assert_eq!(
        vm_eval("let x = if true { 10 } else { 20 }; x"),
        Value::Int(10)
    );
}

#[test]
fn test_vm_peephole_match_correctness() {
    assert_eq!(
        vm_eval("match Some(5) { Some(v) => v * 2, None => 0 }"),
        Value::Int(10)
    );
    assert_eq!(
        vm_eval("match None { Some(v) => v, None => 99 }"),
        Value::Int(99)
    );
    assert_eq!(
        vm_eval("match Ok(7) { Ok(v) => v, Err(e) => 0 }"),
        Value::Int(7)
    );
}
