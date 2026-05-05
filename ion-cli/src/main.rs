use ion_core::engine::Engine;
use ion_core::lexer::Lexer;
use ion_core::parser::Parser;
use ion_core::stdlib::StdOutput;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Read, Write};

fn main() {
    let args: Vec<String> = env::args().collect();

    // Parse flags. Once a script path is found, everything after it
    // (including dash-prefixed args) is captured as script args reachable
    // from Ion as `os::args()`. This matches Python/Node behaviour.
    let mut use_vm = false;
    let mut script_path: Option<String> = None;
    let mut script_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if script_path.is_some() {
            // After the script path, capture verbatim — flags belong to the script.
            script_args.push(arg.clone());
        } else if arg == "--vm" {
            #[cfg(feature = "vm")]
            {
                use_vm = true;
            }
            #[cfg(not(feature = "vm"))]
            {
                eprintln!("--vm flag is unavailable: built without the `vm` feature");
                std::process::exit(1);
            }
        } else if arg == "--check" {
            // Parse-only mode. Lex + parse the source and report errors;
            // never evaluate. Used by the docs-site CI to verify `.ion`
            // code blocks compile.
            //   ion --check <file>   parse from a file
            //   ion --check -        parse from stdin
            i += 1;
            let target = args.get(i).map(String::as_str);
            std::process::exit(check_source(target));
        } else if !arg.starts_with('-') {
            script_path = Some(arg.clone());
        } else {
            eprintln!("Unknown flag: {}", arg);
            #[cfg(feature = "vm")]
            eprintln!("Usage: ion [--vm] [script.ion [args...]] | ion --check <file|->");
            #[cfg(not(feature = "vm"))]
            eprintln!("Usage: ion [script.ion [args...]] | ion --check <file|->");
            std::process::exit(1);
        }
        i += 1;
    }

    match script_path {
        Some(path) => run_file(&path, use_vm, script_args),
        None => run_repl(use_vm),
    }
}

fn check_source(target: Option<&str>) -> i32 {
    let (source, label) = match target {
        Some("-") => {
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                print_read_stdin_error(e);
                return 1;
            }
            (buf, "<stdin>".to_string())
        }
        Some(path) => match fs::read_to_string(path) {
            Ok(s) => (s, path.to_string()),
            Err(e) => {
                print_check_file_read_error(path, e);
                return 1;
            }
        },
        None => {
            eprintln!("ion --check: missing argument (file path or `-` for stdin)");
            return 1;
        }
    };

    let tokens = match Lexer::new(&source).tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{label}: lex error: {e}");
            return 1;
        }
    };
    let output = Parser::new(tokens).parse_program_recovering();
    if !output.errors.is_empty() {
        for err in &output.errors {
            eprintln!("{label}: {err}");
        }
        return 1;
    }
    0
}

fn run_file(path: &str, use_vm: bool, script_args: Vec<String>) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            print_script_read_error(path, e);
            std::process::exit(1);
        }
    };

    let mut engine = Engine::with_output(StdOutput);
    engine.set_args(script_args);

    // The runtime mode is fixed at build time — `async-runtime` and the
    // default sync build are mutually exclusive in ion-core, so exactly one
    // arm here is compiled.
    #[cfg(not(feature = "async-runtime"))]
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

    #[cfg(feature = "async-runtime")]
    let result = {
        // `use_vm` is meaningless under async-runtime — the async path always
        // goes through the bytecode VM internally.
        let _ = use_vm;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime for ion CLI");
        rt.block_on(engine.eval_async(&source))
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

    // Build a tokio runtime once for the whole REPL session under
    // async-runtime; each line is `block_on`-ed against it.
    #[cfg(feature = "async-runtime")]
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime for ion REPL");
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
                print_repl_read_error(e);
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

        #[cfg(not(feature = "async-runtime"))]
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

        #[cfg(feature = "async-runtime")]
        let result = {
            // `vm_mode` doesn't apply under async — the async runtime always
            // compiles to bytecode. Read the binding so the REPL toggle still
            // type-checks.
            let _ = vm_mode;
            rt.block_on(engine.eval_async(&source))
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

fn print_read_stdin_error(err: impl fmt::Display) {
    #[cfg(debug_assertions)]
    eprintln!(
        "ion --check: failed reading stdin: {}",
        redacted_error::display(err)
    );
    #[cfg(not(debug_assertions))]
    {
        let _ = err;
        eprintln!(
            "{}",
            redacted_error::message!("ion --check: failed reading stdin")
        );
    }
}

fn print_check_file_read_error(path: &str, err: impl fmt::Display) {
    #[cfg(debug_assertions)]
    eprintln!(
        "ion --check: cannot read {path}: {}",
        redacted_error::display(err)
    );
    #[cfg(not(debug_assertions))]
    {
        let _ = (path, err);
        eprintln!(
            "{}",
            redacted_error::message!("ion --check: cannot read input")
        );
    }
}

fn print_script_read_error(path: &str, err: impl fmt::Display) {
    #[cfg(debug_assertions)]
    eprintln!("Error reading {path}: {}", redacted_error::display(err));
    #[cfg(not(debug_assertions))]
    {
        let _ = (path, err);
        eprintln!("{}", redacted_error::message!("Error reading input"));
    }
}

fn print_repl_read_error(err: impl fmt::Display) {
    #[cfg(debug_assertions)]
    eprintln!("Read error: {}", redacted_error::display(err));
    #[cfg(not(debug_assertions))]
    {
        let _ = err;
        eprintln!("{}", redacted_error::message!("Read error"));
    }
}
