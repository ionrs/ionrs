use ion_core::engine::Engine;
use ion_core::stdlib::StdOutput;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = env::args().collect();

    // Parse flags
    let mut use_vm = false;
    let mut script_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            #[cfg(feature = "vm")]
            "--vm" => use_vm = true,
            arg if !arg.starts_with('-') => {
                script_path = Some(arg.to_string());
            }
            other => {
                eprintln!("Unknown flag: {}", other);
                #[cfg(feature = "vm")]
                eprintln!("Usage: ion [--vm] [script.ion]");
                #[cfg(not(feature = "vm"))]
                eprintln!("Usage: ion [script.ion]");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    match script_path {
        Some(path) => run_file(&path, use_vm),
        None => run_repl(use_vm),
    }
}

fn run_file(path: &str, use_vm: bool) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let mut engine = Engine::with_output(StdOutput);
    let result = if use_vm {
        #[cfg(feature = "vm")]
        {
            engine.vm_eval(&source)
        }
        #[cfg(not(feature = "vm"))]
        {
            engine.eval(&source)
        }
    } else {
        engine.eval(&source)
    };

    match result {
        Ok(val) => {
            if !matches!(val, ion_core::value::Value::Unit) {
                println!("{}", val);
            }
        }
        Err(e) => {
            eprint!("{}", e.format_with_source(&source));
            std::process::exit(1);
        }
    }
}

fn run_repl(use_vm: bool) {
    println!("Ion v0.1.0 \u{2014} type :quit to exit");

    let mut engine = Engine::with_output(StdOutput);
    let mut vm_mode = use_vm;
    let stdin = io::stdin();
    let mut input_buf = String::new();
    let mut brace_depth: i32 = 0;

    loop {
        // Show appropriate prompt
        if input_buf.is_empty() {
            print!("ion> ");
        } else {
            print!("...> ");
        }
        io::stdout().flush().expect("flush stdout");

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // EOF (Ctrl+D)
                println!();
                break;
            }
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
            Ok(_) => {}
        }

        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');

        // Handle REPL commands only on fresh input
        if input_buf.is_empty() {
            match trimmed {
                ":quit" | ":q" => break,
                #[cfg(feature = "vm")]
                ":vm" => {
                    vm_mode = !vm_mode;
                    println!("VM mode: {}", if vm_mode { "on" } else { "off" });
                    continue;
                }
                _ => {}
            }
        }

        // Accumulate input
        if !input_buf.is_empty() {
            input_buf.push('\n');
        }
        input_buf.push_str(trimmed);

        // Update brace depth
        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        // Continue reading if braces are unbalanced or line ends with '{'
        if brace_depth > 0 || trimmed.ends_with('{') {
            continue;
        }

        // Reset depth for next input
        brace_depth = 0;

        let source = input_buf.clone();
        input_buf.clear();

        if source.trim().is_empty() {
            continue;
        }

        let result = if vm_mode {
            #[cfg(feature = "vm")]
            {
                engine.vm_eval(&source)
            }
            #[cfg(not(feature = "vm"))]
            {
                engine.eval(&source)
            }
        } else {
            engine.eval(&source)
        };

        match result {
            Ok(val) => {
                if !matches!(val, ion_core::value::Value::Unit) {
                    println!("{}", val);
                }
            }
            Err(e) => {
                eprint!("\x1b[31m{}\x1b[0m", e.format_with_source(&source));
            }
        }
    }
}
