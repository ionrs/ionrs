//! Edge case and adversarial tests for correctness. Sync-build only —
//! async builds use `Engine::eval_async`.

#![cfg(not(feature = "async-runtime"))]

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
// Empty collections
// ============================================================

#[test]
fn edge_empty_list() {
    assert_eq!(eval("[]"), Value::List(vec![]));
    assert_eq!(eval("[].len()"), Value::Int(0));
}

#[test]
fn edge_empty_tuple() {
    assert_eq!(eval("()"), Value::Unit); // () is unit, not empty tuple
}

#[test]
fn edge_empty_dict() {
    if let Value::Dict(d) = eval("#{}") {
        assert!(d.is_empty());
    } else {
        panic!("expected Dict");
    }
}

#[test]
fn edge_empty_string() {
    assert_eq!(eval(r#""".len()"#), Value::Int(0));
    assert_eq!(eval(r#""".contains("")"#), Value::Bool(true));
}

// ============================================================
// Division by zero
// ============================================================

#[test]
fn edge_div_by_zero_int() {
    let msg = eval_err("1 / 0");
    assert!(
        msg.contains("division by zero") || msg.contains("divide"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_mod_by_zero() {
    let msg = eval_err("1 % 0");
    assert!(
        msg.contains("modulo by zero") || msg.contains("zero"),
        "got: {}",
        msg
    );
}

// ============================================================
// Integer overflow (should not panic)
// ============================================================

#[test]
fn edge_large_int_arithmetic() {
    // Large but not overflowing
    assert_eq!(eval("1000000 * 1000000"), Value::Int(1_000_000_000_000));
}

// ============================================================
// Deeply nested expressions
// ============================================================

#[test]
fn edge_deeply_nested_if() {
    assert_eq!(eval("if true { if true { if true { if true { 42 } else { 0 } } else { 0 } } else { 0 } } else { 0 }"), Value::Int(42));
}

#[test]
fn edge_deeply_nested_blocks() {
    assert_eq!(eval("{ { { { 99 } } } }"), Value::Int(99));
}

#[test]
fn edge_deeply_nested_parens() {
    assert_eq!(eval("((((((1 + 2))))))"), Value::Int(3));
}

// ============================================================
// Scope edge cases
// ============================================================

#[test]
fn edge_shadow_variable() {
    assert_eq!(eval("let x = 1; let x = 2; x"), Value::Int(2));
}

#[test]
fn edge_scope_isolation() {
    assert_eq!(eval("let x = 1; { let x = 99; } x"), Value::Int(1));
}

#[test]
fn edge_many_scopes() {
    assert_eq!(
        eval("let mut x = 0; { x += 1; { x += 1; { x += 1; } } } x"),
        Value::Int(3)
    );
}

// ============================================================
// Function edge cases
// ============================================================

#[test]
fn edge_fn_no_args() {
    assert_eq!(eval("fn f() { 42 } f()"), Value::Int(42));
}

#[test]
fn edge_fn_many_args() {
    assert_eq!(
        eval("fn f(a, b, c, d, e) { a + b + c + d + e } f(1, 2, 3, 4, 5)"),
        Value::Int(15)
    );
}

#[test]
fn edge_fn_returns_fn() {
    assert_eq!(eval("fn f() { fn g() { 42 } g } f()()"), Value::Int(42));
}

#[test]
fn edge_immediate_call_lambda() {
    assert_eq!(eval("(|x| x * 2)(21)"), Value::Int(42));
}

// ============================================================
// Match edge cases
// ============================================================

#[test]
fn edge_match_wildcard_only() {
    assert_eq!(eval("match 5 { _ => 99 }"), Value::Int(99));
}

#[test]
fn edge_match_many_arms() {
    assert_eq!(
        eval("match 5 { 1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50, _ => 0 }"),
        Value::Int(50)
    );
}

#[test]
fn edge_match_string() {
    assert_eq!(
        eval(r#"match "hello" { "hello" => 1, _ => 0 }"#),
        Value::Int(1)
    );
}

// ============================================================
// Loop edge cases
// ============================================================

#[test]
fn edge_while_false() {
    // Loop body never executes
    assert_eq!(
        eval("let mut x = 0; while false { x = 99; } x"),
        Value::Int(0)
    );
}

#[test]
fn edge_for_empty_list() {
    // Loop body never executes
    assert_eq!(
        eval("let mut x = 0; for i in [] { x = 99; } x"),
        Value::Int(0)
    );
}

#[test]
fn edge_for_single_element() {
    assert_eq!(
        eval("let mut x = 0; for i in [42] { x = i; } x"),
        Value::Int(42)
    );
}

#[test]
fn edge_break_immediately() {
    assert_eq!(
        eval("let mut x = 0; for i in [1, 2, 3] { break; x = 99; } x"),
        Value::Int(0)
    );
}

#[test]
fn edge_continue_every_iteration() {
    assert_eq!(
        eval("let mut x = 0; for i in [1, 2, 3] { continue; x += i; } x"),
        Value::Int(0)
    );
}

// ============================================================
// String edge cases
// ============================================================

#[test]
fn edge_string_escape() {
    assert_eq!(eval(r#""\n".len()"#), Value::Int(1));
    assert_eq!(eval(r#""\t".len()"#), Value::Int(1));
    assert_eq!(eval(r#""\\".len()"#), Value::Int(1));
}

#[test]
fn edge_fstring_empty_expr() {
    assert_eq!(
        eval(r#"let x = ""; f"({x})""#),
        Value::Str("()".to_string())
    );
}

// ============================================================
// Option/Result edge cases
// ============================================================

#[test]
fn edge_nested_option() {
    assert_eq!(
        eval("Some(Some(1))"),
        Value::Option(Some(Box::new(Value::Option(Some(Box::new(Value::Int(1)))))))
    );
}

#[test]
fn edge_option_comparison() {
    assert_eq!(eval("None == None"), Value::Bool(true));
    assert_eq!(eval("Some(1) == Some(1)"), Value::Bool(true));
    assert_eq!(eval("Some(1) == Some(2)"), Value::Bool(false));
    assert_eq!(eval("Some(1) == None"), Value::Bool(false));
}

// ============================================================
// Error handling
// ============================================================

#[test]
fn edge_undefined_variable() {
    let msg = eval_err("x");
    assert!(
        msg.contains("undefined") || msg.contains("not found"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_type_mismatch_add() {
    let msg = eval_err(r#"1 + "hello""#);
    assert!(!msg.is_empty(), "expected error for int + string");
}

#[test]
fn edge_immutable_reassign() {
    let msg = eval_err("let x = 1; x = 2;");
    assert!(
        msg.contains("immutable") || msg.contains("cannot") || msg.contains("mutable"),
        "got: {}",
        msg
    );
}

// ============================================================
// Large constant pools (>256 constants)
// ============================================================

#[cfg(feature = "vm")]
#[test]
fn edge_many_constants() {
    // Generate a program with many string constants
    let mut src = String::new();
    for i in 0..300 {
        src.push_str(&format!("let x{} = {};\n", i, i));
    }
    src.push_str("x299");
    let mut engine = Engine::new();
    assert_eq!(engine.vm_eval(&src).unwrap(), Value::Int(299));
}

// ============================================================
// Recursive depth (tested in integration.rs — skip here to
// avoid stack overflow in small test thread stacks)
// ============================================================

// ============================================================
// Chained methods
// ============================================================

#[test]
fn edge_chained_list_methods() {
    assert_eq!(
        eval("[1, 2, 3, 4, 5].filter(|x| x > 2).map(|x| x * 10).len()"),
        Value::Int(3),
    );
}

// ============================================================
// Multi-line programs
// ============================================================

#[test]
fn edge_multiline_program() {
    assert_eq!(
        eval(
            r#"
        let a = 1;
        let b = 2;
        let c = a + b;
        let d = c * 2;
        let e = d - 1;
        e
    "#
        ),
        Value::Int(5)
    );
}

// ============================================================
// Bool arithmetic (should error or handle gracefully)
// ============================================================

#[test]
fn edge_bool_equality() {
    assert_eq!(eval("true == true"), Value::Bool(true));
    assert_eq!(eval("true == false"), Value::Bool(false));
    assert_eq!(eval("true != false"), Value::Bool(true));
}

// ============================================================
// Dict edge cases
// ============================================================

#[test]
fn edge_dict_overwrite_key() {
    assert_eq!(eval(r#"#{"a": 1, "a": 2}.a"#), Value::Int(2));
}

#[test]
fn edge_dict_missing_key() {
    assert_eq!(eval(r#"#{"a": 1}.b"#), Value::Option(None));
}

// ============================================================
// Shift operator bounds
// ============================================================

#[test]
fn edge_shift_left_valid() {
    assert_eq!(eval("1 << 0"), Value::Int(1));
    assert_eq!(eval("1 << 10"), Value::Int(1024));
    assert_eq!(eval("1 << 63"), Value::Int(i64::MIN));
}

#[test]
fn edge_shift_right_valid() {
    assert_eq!(eval("1024 >> 5"), Value::Int(32));
    assert_eq!(eval("-1 >> 1"), Value::Int(-1));
}

#[test]
fn edge_shift_left_out_of_range() {
    let msg = eval_err("1 << 64");
    assert!(msg.contains("out of range"), "got: {}", msg);
}

#[test]
fn edge_shift_right_out_of_range() {
    let msg = eval_err("1 >> -1");
    assert!(msg.contains("out of range"), "got: {}", msg);
}

#[test]
fn edge_shift_left_negative() {
    let msg = eval_err("1 << -1");
    assert!(msg.contains("out of range"), "got: {}", msg);
}

// ============================================================
// Arity errors
// ============================================================

#[test]
fn edge_len_no_args() {
    let msg = eval_err("len()");
    assert!(
        msg.contains("requires 1 argument") || msg.contains("arity"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_type_of_no_args() {
    let msg = eval_err("type_of()");
    assert!(
        msg.contains("requires 1 argument") || msg.contains("arity"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_dict_insert_arity() {
    let msg = eval_err(r#"let d = #{"a": 1}; d.insert("b")"#);
    assert!(
        msg.contains("insert") || msg.contains("argument"),
        "got: {}",
        msg
    );
}

// ============================================================
// bytes / bytes_from_hex errors
// ============================================================

#[test]
fn edge_bytes_allocation_cap() {
    let msg = eval_err("bytes(100000000)");
    // 0.9.0 cleanup: error literal genericised — accepts either old or new wording.
    assert!(
        msg.contains("out of range") || msg.contains("invalid byte count"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_bytes_negative() {
    let msg = eval_err("bytes(-1)");
    assert!(
        msg.contains("out of range") || msg.contains("invalid byte count"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_bytes_from_hex_non_ascii() {
    let msg = eval_err(r#"bytes_from_hex("café")"#);
    assert!(
        msg.contains("ASCII") || msg.contains("invalid"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_bytes_from_hex_odd_length() {
    let msg = eval_err(r#"bytes_from_hex("abc")"#);
    assert!(
        msg.contains("length") || msg.contains("odd") || msg.contains("invalid"),
        "got: {}",
        msg
    );
}

// ============================================================
// window(0) error
// ============================================================

#[test]
fn edge_window_zero() {
    let msg = eval_err("[1, 2, 3].window(0)");
    assert!(
        msg.contains("must be > 0") || msg.contains("window"),
        "got: {}",
        msg
    );
}

// ============================================================
// reduce on empty list
// ============================================================

#[test]
fn edge_reduce_empty_list() {
    let msg = eval_err("[].reduce(|a, b| a + b)");
    assert!(
        msg.contains("empty") || msg.contains("reduce"),
        "got: {}",
        msg
    );
}

// ============================================================
// Float division by zero
// ============================================================

#[test]
fn edge_float_div_by_zero() {
    let result = eval("1.0 / 0.0");
    match result {
        Value::Float(f) => assert!(f.is_infinite(), "expected infinity, got: {}", f),
        other => panic!("expected Float, got: {:?}", other),
    }
}

#[test]
fn edge_float_zero_div_zero() {
    let result = eval("0.0 / 0.0");
    match result {
        Value::Float(f) => assert!(f.is_nan(), "expected NaN, got: {}", f),
        other => panic!("expected Float, got: {:?}", other),
    }
}

// ============================================================
// Break/continue outside loop (compile error)
// ============================================================

#[test]
fn edge_break_outside_loop() {
    let msg = eval_err("break;");
    assert!(
        msg.contains("outside") || msg.contains("loop") || msg.contains("break"),
        "got: {}",
        msg
    );
}

#[test]
fn edge_continue_outside_loop() {
    let msg = eval_err("continue;");
    assert!(
        msg.contains("outside") || msg.contains("loop") || msg.contains("continue"),
        "got: {}",
        msg
    );
}

// ============================================================
// Labeled break / continue
// ============================================================

#[test]
fn label_break_outer_for() {
    let src = r#"
        let mut hits = 0;
        'outer: for i in 0..5 {
            for j in 0..5 {
                hits += 1;
                if i == 2 && j == 1 { break 'outer; }
            }
        }
        hits
    "#;
    // i=0,1: 5 inner iters each = 10. i=2: j=0 (hits=1) + j=1 (hits=1, then break 'outer) = 2.
    // Total: 10 + 2 = 12.
    assert_eq!(eval(src), Value::Int(12));
}

#[test]
fn label_break_outer_marks_side_effect() {
    // We can't observe break-with-value through a statement-position loop
    // (compile_program emits Unit after the last statement), so instead
    // verify that the labeled break exits the outer loop by tracking
    // iteration counts across both loops.
    let src = r#"
        let mut outer_iters = 0;
        let mut inner_iters = 0;
        'outer: loop {
            outer_iters += 1;
            loop {
                inner_iters += 1;
                if inner_iters == 3 { break 'outer; }
            }
        }
        outer_iters * 100 + inner_iters
    "#;
    // outer=1; inner=1. inner=2. inner=3 → break 'outer. So 1*100+3 = 103.
    assert_eq!(eval(src), Value::Int(103));
}

#[test]
fn label_continue_outer_for() {
    let src = r#"
        let mut sum = 0;
        'outer: for i in 0..3 {
            for j in 0..3 {
                if j == 1 { continue 'outer; }
                sum += 1;
            }
            sum += 100;
        }
        sum
    "#;
    // each outer iter does inner j=0 (sum+=1) then j=1 → continue 'outer.
    // Outer post-body increment never runs. 3 outer iters × 1 = 3.
    assert_eq!(eval(src), Value::Int(3));
}

#[test]
fn label_inner_break_only_exits_inner() {
    let src = r#"
        let mut hits = 0;
        'outer: for i in 0..2 {
            for j in 0..3 {
                hits += 1;
                if j == 0 { break; }
            }
            hits += 100;
        }
        hits
    "#;
    // Each outer iter: inner j=0 → hits=1 → break inner; hits+=100. ×2 = 202.
    assert_eq!(eval(src), Value::Int(202));
}

#[test]
fn label_break_unknown_errors() {
    let msg = eval_err("for i in 0..3 { break 'nope; }");
    assert!(
        msg.contains("'nope") || msg.contains("nope"),
        "got: {}",
        msg
    );
}

#[test]
fn label_continue_unknown_errors() {
    let msg = eval_err("for i in 0..3 { continue 'nope; }");
    assert!(
        msg.contains("'nope") || msg.contains("nope"),
        "got: {}",
        msg
    );
}

#[test]
fn label_shadowing_inner_wins() {
    let src = r#"
        let mut hits = 0;
        'a: for i in 0..3 {
            'a: for j in 0..3 {
                hits += 1;
                if j == 1 { break 'a; }
            }
            hits += 100;
        }
        hits
    "#;
    // Inner 'a shadows outer. break 'a exits inner only.
    // Outer iter: hits 0,1 (j=0,1) then break inner; hits += 100. ×3 outer = (2+100)*3 = 306.
    assert_eq!(eval(src), Value::Int(306));
}

#[test]
fn label_on_while() {
    let src = r#"
        let mut i = 0;
        let mut hits = 0;
        'outer: while i < 5 {
            let mut j = 0;
            while j < 5 {
                hits += 1;
                if i == 1 && j == 2 { break 'outer; }
                j += 1;
            }
            i += 1;
        }
        hits
    "#;
    // i=0 inner runs 5 (5). i=1: j=0,1,2 → break outer at j=2 (3). Total 8.
    assert_eq!(eval(src), Value::Int(8));
}

#[test]
fn label_on_loop_continue() {
    // 'outer continue should restart outer's loop; we count side-effects to
    // confirm we re-entered outer rather than continuing the inner loop.
    let src = r#"
        let mut outer_hits = 0;
        let mut inner_hits = 0;
        'outer: loop {
            outer_hits += 1;
            if outer_hits == 3 { break 'outer; }
            loop {
                inner_hits += 1;
                if inner_hits >= 5 { break; }
                continue 'outer;
            }
        }
        outer_hits * 1000 + inner_hits
    "#;
    // Iter1: outer=1, inner=1, continue outer. Iter2: outer=2, inner=2, continue outer.
    // Iter3: outer=3, break outer. inner_hits=2.
    assert_eq!(eval(src), Value::Int(3 * 1000 + 2));
}
