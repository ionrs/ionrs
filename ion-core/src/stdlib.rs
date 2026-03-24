//! Built-in standard library modules.
//!
//! These modules are automatically registered in every Engine instance
//! and provide namespaced access to common functions and constants.
//! The same functions remain available as top-level builtins for
//! backwards compatibility.

use crate::module::Module;
use crate::value::Value;

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
            return Err(ion_str!("math::clamp requires 3 arguments: value, min, max"));
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

    m
}

/// Build the `io` stdlib module.
///
/// Functions: print, println, input (placeholder)
pub fn io_module() -> Module {
    let mut m = Module::new("io");

    m.register_fn("print", |args: &[Value]| {
        let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        print!("{}", parts.join(" "));
        Ok(Value::Unit)
    });

    m.register_fn("println", |args: &[Value]| {
        let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        println!("{}", parts.join(" "));
        Ok(Value::Unit)
    });

    m.register_fn("eprintln", |args: &[Value]| {
        let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        eprintln!("{}", parts.join(" "));
        Ok(Value::Unit)
    });

    m
}

/// Register all stdlib modules in the given environment.
pub fn register_stdlib(env: &mut crate::env::Env) {
    let math = math_module();
    env.define(math.name.clone(), math.to_value(), false);

    let json = json_module();
    env.define(json.name.clone(), json.to_value(), false);

    let io = io_module();
    env.define(io.name.clone(), io.to_value(), false);
}
