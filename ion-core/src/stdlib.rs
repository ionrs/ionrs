//! Built-in standard library modules.
//!
//! These modules are automatically registered in every Engine instance
//! and provide namespaced access to common functions and constants.
//! The same functions remain available as top-level builtins for
//! backwards compatibility.

use std::sync::Arc;

use crate::module::Module;
use crate::value::Value;

/// Output stream requested by Ion's `io` stdlib module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Host-side output handler for Ion's `io::print*` functions.
///
/// Embedders install an implementation with `Engine::set_output` or
/// `Engine::with_output`. The core runtime never writes directly to process
/// stdout/stderr through `io::print*`.
pub trait OutputHandler: Send + Sync {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String>;
}

/// Output handler that writes to the process stdout/stderr streams.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdOutput;

impl OutputHandler for StdOutput {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String> {
        use std::io::Write;

        match stream {
            OutputStream::Stdout => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(text.as_bytes()).map_err(|e| e.to_string())
            }
            OutputStream::Stderr => {
                let mut stderr = std::io::stderr().lock();
                stderr.write_all(text.as_bytes()).map_err(|e| e.to_string())
            }
        }
    }
}

struct MissingOutputHandler;

impl OutputHandler for MissingOutputHandler {
    fn write(&self, _stream: OutputStream, _text: &str) -> Result<(), String> {
        Err(ion_str!(
            "io output handler is not configured; call Engine::set_output"
        ))
    }
}

pub(crate) fn missing_output_handler() -> Arc<dyn OutputHandler> {
    Arc::new(MissingOutputHandler)
}

/// Build the `math` stdlib module.
///
/// Functions: abs, min, max, floor, ceil, round, sqrt, pow, clamp, log, log2, log10, sin, cos, tan, atan2
/// Constants: PI, E, INF, NAN, TAU
pub fn math_module() -> Module {
    let mut m = Module::new("math");

    // Constants
    m.set("PI", Value::Float(std::f64::consts::PI));
    m.set("E", Value::Float(std::f64::consts::E));
    m.set("TAU", Value::Float(std::f64::consts::TAU));
    m.set("INF", Value::Float(f64::INFINITY));
    m.set("NAN", Value::Float(f64::NAN));

    m.register_fn("abs", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("math::abs takes 1 argument"));
        }
        match &args[0] {
            Value::Int(n) => Ok(Value::Int(n.abs())),
            Value::Float(n) => Ok(Value::Float(n.abs())),
            _ => Err(format!(
                "{}{}",
                ion_str!("math::abs not supported for "),
                args[0].type_name()
            )),
        }
    });

    m.register_fn("min", |args: &[Value]| {
        if args.len() < 2 {
            return Err(ion_str!("math::min requires at least 2 arguments"));
        }
        let mut best = args[0].clone();
        for arg in &args[1..] {
            match (&best, arg) {
                (Value::Int(a), Value::Int(b)) if b < a => best = arg.clone(),
                (Value::Float(a), Value::Float(b)) if b < a => best = arg.clone(),
                (Value::Int(a), Value::Float(b)) if *b < (*a as f64) => best = arg.clone(),
                (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => best = arg.clone(),
                (Value::Int(_), Value::Int(_))
                | (Value::Float(_), Value::Float(_))
                | (Value::Int(_), Value::Float(_))
                | (Value::Float(_), Value::Int(_)) => {}
                _ => return Err(ion_str!("math::min requires numeric arguments")),
            }
        }
        Ok(best)
    });

    m.register_fn("max", |args: &[Value]| {
        if args.len() < 2 {
            return Err(ion_str!("math::max requires at least 2 arguments"));
        }
        let mut best = args[0].clone();
        for arg in &args[1..] {
            match (&best, arg) {
                (Value::Int(a), Value::Int(b)) if b > a => best = arg.clone(),
                (Value::Float(a), Value::Float(b)) if b > a => best = arg.clone(),
                (Value::Int(a), Value::Float(b)) if *b > (*a as f64) => best = arg.clone(),
                (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => best = arg.clone(),
                (Value::Int(_), Value::Int(_))
                | (Value::Float(_), Value::Float(_))
                | (Value::Int(_), Value::Float(_))
                | (Value::Float(_), Value::Int(_)) => {}
                _ => return Err(ion_str!("math::max requires numeric arguments")),
            }
        }
        Ok(best)
    });

    m.register_fn("floor", |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.floor())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("math::floor not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn("ceil", |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.ceil())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("math::ceil not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn("round", |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.round())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("math::round not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn("sqrt", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::sqrt requires a number"))?;
        Ok(Value::Float(n.sqrt()))
    });

    m.register_fn("pow", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("math::pow takes 2 arguments"));
        }
        match (&args[0], &args[1]) {
            (Value::Int(base), Value::Int(exp)) => {
                if *exp >= 0 {
                    Ok(Value::Int(base.pow(*exp as u32)))
                } else {
                    Ok(Value::Float((*base as f64).powi(*exp as i32)))
                }
            }
            _ => {
                let b = args[0]
                    .as_float()
                    .ok_or(ion_str!("math::pow requires numeric arguments"))?;
                let e = args[1]
                    .as_float()
                    .ok_or(ion_str!("math::pow requires numeric arguments"))?;
                Ok(Value::Float(b.powf(e)))
            }
        }
    });

    m.register_fn("clamp", |args: &[Value]| {
        if args.len() != 3 {
            return Err(ion_str!(
                "math::clamp requires 3 arguments: value, min, max"
            ));
        }
        match (&args[0], &args[1], &args[2]) {
            (Value::Int(v), Value::Int(lo), Value::Int(hi)) => Ok(Value::Int(*v.max(lo).min(hi))),
            (Value::Float(v), Value::Float(lo), Value::Float(hi)) => {
                Ok(Value::Float(v.max(*lo).min(*hi)))
            }
            _ => {
                let v = args[0]
                    .as_float()
                    .ok_or(ion_str!("math::clamp requires numeric arguments"))?;
                let lo = args[1]
                    .as_float()
                    .ok_or(ion_str!("math::clamp requires numeric arguments"))?;
                let hi = args[2]
                    .as_float()
                    .ok_or(ion_str!("math::clamp requires numeric arguments"))?;
                Ok(Value::Float(v.max(lo).min(hi)))
            }
        }
    });

    // Trigonometry
    m.register_fn("sin", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::sin requires a number"))?;
        Ok(Value::Float(n.sin()))
    });

    m.register_fn("cos", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::cos requires a number"))?;
        Ok(Value::Float(n.cos()))
    });

    m.register_fn("tan", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::tan requires a number"))?;
        Ok(Value::Float(n.tan()))
    });

    m.register_fn("atan2", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("math::atan2 takes 2 arguments"));
        }
        let y = args[0]
            .as_float()
            .ok_or(ion_str!("math::atan2 requires numeric arguments"))?;
        let x = args[1]
            .as_float()
            .ok_or(ion_str!("math::atan2 requires numeric arguments"))?;
        Ok(Value::Float(y.atan2(x)))
    });

    // Logarithms
    m.register_fn("log", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::log requires a number"))?;
        Ok(Value::Float(n.ln()))
    });

    m.register_fn("log2", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::log2 requires a number"))?;
        Ok(Value::Float(n.log2()))
    });

    m.register_fn("log10", |args: &[Value]| {
        let n = args[0]
            .as_float()
            .ok_or(ion_str!("math::log10 requires a number"))?;
        Ok(Value::Float(n.log10()))
    });

    // Rounding/check
    m.register_fn("is_nan", |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Bool(n.is_nan())),
        Value::Int(_) => Ok(Value::Bool(false)),
        _ => Err(format!(
            "{}{}",
            ion_str!("math::is_nan not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn("is_inf", |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Bool(n.is_infinite())),
        Value::Int(_) => Ok(Value::Bool(false)),
        _ => Err(format!(
            "{}{}",
            ion_str!("math::is_inf not supported for "),
            args[0].type_name()
        )),
    });

    m
}

/// Build the `json` stdlib module.
///
/// Functions: encode, decode, pretty
pub fn json_module() -> Module {
    let mut m = Module::new("json");

    m.register_fn("encode", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("json::encode takes 1 argument"));
        }
        let json = args[0].to_json();
        Ok(Value::Str(json.to_string()))
    });

    m.register_fn("decode", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("json::decode takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("json::decode requires a string"))?;
        let json: serde_json::Value = serde_json::from_str(s)
            .map_err(|e| format!("{}{}", ion_str!("json::decode error: "), e))?;
        Ok(Value::from_json(json))
    });

    m.register_fn("pretty", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("json::pretty takes 1 argument"));
        }
        let json = args[0].to_json();
        serde_json::to_string_pretty(&json)
            .map(Value::Str)
            .map_err(|e| format!("{}{}", ion_str!("json::pretty error: "), e))
    });

    #[cfg(feature = "msgpack")]
    m.register_fn("msgpack_encode", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("json::msgpack_encode takes 1 argument"));
        }
        args[0].to_msgpack().map(Value::Bytes)
    });

    #[cfg(feature = "msgpack")]
    m.register_fn("msgpack_decode", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("json::msgpack_decode takes 1 argument"));
        }
        let data = match &args[0] {
            Value::Bytes(b) => b,
            _ => return Err(ion_str!("json::msgpack_decode requires bytes")),
        };
        Value::from_msgpack(data)
    });

    m
}

fn format_output_args(args: &[Value]) -> String {
    args.iter()
        .map(|arg| arg.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the `io` stdlib module.
///
/// Functions: print, println, eprintln
pub fn io_module() -> Module {
    io_module_with_output(missing_output_handler())
}

/// Build the `io` stdlib module with a host-provided output handler.
pub fn io_module_with_output(output: Arc<dyn OutputHandler>) -> Module {
    let mut m = Module::new("io");

    let stdout = Arc::clone(&output);
    m.register_closure("print", move |args: &[Value]| {
        stdout.write(OutputStream::Stdout, &format_output_args(args))?;
        Ok(Value::Unit)
    });

    let stdout = Arc::clone(&output);
    m.register_closure("println", move |args: &[Value]| {
        let mut text = format_output_args(args);
        text.push('\n');
        stdout.write(OutputStream::Stdout, &text)?;
        Ok(Value::Unit)
    });

    m.register_closure("eprintln", move |args: &[Value]| {
        let mut text = format_output_args(args);
        text.push('\n');
        output.write(OutputStream::Stderr, &text)?;
        Ok(Value::Unit)
    });

    m
}

/// Build the `str` stdlib module.
///
/// Functions: join
pub fn string_module() -> Module {
    let mut m = Module::new("string");

    m.register_fn("join", |args: &[Value]| {
        if args.is_empty() || args.len() > 2 {
            return Err(ion_str!(
                "string::join requires 1-2 arguments: list, [separator]"
            ));
        }
        let items = match &args[0] {
            Value::List(items) => items,
            _ => return Err(ion_str!("string::join requires a list as first argument")),
        };
        let sep = if args.len() > 1 {
            args[1].as_str().unwrap_or("").to_string()
        } else {
            String::new()
        };
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::Str(parts.join(&sep)))
    });

    m
}

/// Register all stdlib modules in the given environment.
pub fn register_stdlib(env: &mut crate::env::Env) {
    register_stdlib_with_output(env, missing_output_handler());
}

/// Register all stdlib modules with a host-provided output handler.
pub fn register_stdlib_with_output(env: &mut crate::env::Env, output: Arc<dyn OutputHandler>) {
    let math = math_module();
    env.define(math.name.clone(), math.to_value(), false);

    let json = json_module();
    env.define(json.name.clone(), json.to_value(), false);

    let io = io_module_with_output(output);
    env.define(io.name.clone(), io.to_value(), false);

    let string_mod = string_module();
    env.define(string_mod.name.clone(), string_mod.to_value(), false);
}
