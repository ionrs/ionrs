use ion_core::engine::Engine;
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: ion <script.ion>");
        std::process::exit(1);
    }

    let source = match fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", args[1], e);
            std::process::exit(1);
        }
    };

    let mut engine = Engine::new();
    match engine.eval(&source) {
        Ok(val) => {
            // Print the return value if it's not unit
            if !matches!(val, ion_core::value::Value::Unit) {
                println!("{}", val);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}
