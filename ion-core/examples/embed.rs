//! Example: embedding Ion in a Rust application.
//!
//! Run with: cargo run --example embed

use ion_core::engine::Engine;
use ion_core::value::Value;

fn main() {
    let mut engine = Engine::new();

    // Basic evaluation
    let result = engine.eval("1 + 2 * 3").unwrap();
    println!("1 + 2 * 3 = {}", result);

    // Inject values
    engine.set("player_hp", Value::Int(100));
    engine.set("damage", Value::Int(30));
    let alive = engine.eval("player_hp - damage > 0").unwrap();
    println!("Player alive: {}", alive);

    // Define functions in script
    engine
        .eval(
            r#"
        fn fibonacci(n) {
            if n <= 1 { n }
            else { fibonacci(n - 1) + fibonacci(n - 2) }
        }
    "#,
        )
        .unwrap();
    let fib10 = engine.eval("fibonacci(10)").unwrap();
    println!("fibonacci(10) = {}", fib10);

    // Use the pipe operator
    let result = engine
        .eval(
            r#"
        fn double(x) { x * 2 }
        fn add_one(x) { x + 1 }
        5 |> double |> add_one
    "#,
        )
        .unwrap();
    println!("5 |> double |> add_one = {}", result);

    // Register a Rust function
    engine.register_fn("square", |args: &[Value]| match &args[0] {
        Value::Int(n) => Ok(Value::Int(n * n)),
        _ => Err("expected int".to_string()),
    });
    let sq = engine.eval("square(7)").unwrap();
    println!("square(7) = {}", sq);

    // Error handling with format_with_source
    let bad_src = "let x = 1 / 0;";
    match engine.eval(bad_src) {
        Ok(val) => println!("Result: {}", val),
        Err(e) => eprint!("{}", e.format_with_source(bad_src)),
    }
}
