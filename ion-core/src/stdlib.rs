//! Built-in standard library modules.
//!
//! These modules are automatically registered in every Engine instance
//! and provide namespaced access to common functions and constants.
//! The same functions remain available as top-level builtins for
//! backwards compatibility.

use std::sync::Arc;

use crate::module::Module;
use crate::value::Value;

#[cfg(feature = "semver")]
use semver::{BuildMetadata, Prerelease, Version, VersionReq};

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

/// Build the `log` stdlib module with a handler whose threshold is the
/// shared atomic level (used by `log::set_level`/`log::level`).
///
/// Functions: trace, debug, info, warn, error, set_level, level, enabled.
pub fn log_module_with_handler(
    handler: Arc<dyn crate::log::LogHandler>,
    level: Arc<crate::log::AtomicLogLevel>,
) -> Module {
    use crate::log::{AtomicLogLevel, LogLevel};

    let mut m = Module::new("log");

    fn register_level(
        m: &mut Module,
        name: &'static str,
        level: LogLevel,
        handler: Arc<dyn crate::log::LogHandler>,
    ) {
        m.register_closure(name, move |args: &[Value]| {
            // Pre-flight check — handlers can short-circuit before any
            // formatting work happens for callsites that survived
            // compile-time stripping but fail the runtime threshold.
            if !handler.enabled(level) {
                return Ok(Value::Unit);
            }
            let (message, fields) = match args.len() {
                1 => (extract_message(&args[0])?, Vec::new()),
                2 => (extract_message(&args[0])?, extract_fields(&args[1])?),
                _ => {
                    return Err(format!(
                        "log::{} requires 1 or 2 arguments: message, [fields]",
                        level.as_str()
                    ));
                }
            };
            handler.log(level, &message, &fields);
            Ok(Value::Unit)
        });
    }

    register_level(&mut m, "trace", LogLevel::Trace, Arc::clone(&handler));
    register_level(&mut m, "debug", LogLevel::Debug, Arc::clone(&handler));
    register_level(&mut m, "info", LogLevel::Info, Arc::clone(&handler));
    register_level(&mut m, "warn", LogLevel::Warn, Arc::clone(&handler));
    register_level(&mut m, "error", LogLevel::Error, Arc::clone(&handler));

    let level_for_set = Arc::clone(&level);
    m.register_closure("set_level", move |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("log::set_level takes 1 argument: name"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("log::set_level requires a string"))?;
        let parsed = LogLevel::from_str_ci(name).ok_or_else(|| {
            format!(
                "log::set_level: unknown level '{}' (expected off|error|warn|info|debug|trace)",
                name
            )
        })?;
        level_for_set.set(parsed);
        Ok(Value::Unit)
    });

    let level_for_get = Arc::clone(&level);
    m.register_closure("level", move |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("log::level takes no arguments"));
        }
        Ok(Value::Str(level_for_get.get().as_str().to_string()))
    });

    let handler_for_enabled = Arc::clone(&handler);
    m.register_closure("enabled", move |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("log::enabled takes 1 argument: name"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("log::enabled requires a string"))?;
        let parsed = LogLevel::from_str_ci(name).ok_or_else(|| {
            format!(
                "log::enabled: unknown level '{}' (expected off|error|warn|info|debug|trace)",
                name
            )
        })?;
        Ok(Value::Bool(handler_for_enabled.enabled(parsed)))
    });

    // Avoid `unused` warning when no extra references are made later.
    let _ = AtomicLogLevel::default_runtime;
    let _ = level;

    m
}

/// Build the default `log` stdlib module — uses [`StdLogHandler`] writing to
/// stderr, with a fresh runtime threshold (honouring `ION_LOG`).
pub fn log_module() -> Module {
    let level = crate::log::AtomicLogLevel::default_runtime();
    let handler: Arc<dyn crate::log::LogHandler> =
        Arc::new(crate::log::StdLogHandler::with_threshold(Arc::clone(&level)));
    log_module_with_handler(handler, level)
}

fn extract_message(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Ok(format!("{}", other)),
    }
}

fn extract_fields(v: &Value) -> Result<Vec<(String, Value)>, String> {
    match v {
        Value::Dict(map) => Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        other => Err(format!(
            "log fields must be a dict, got {}",
            other.type_name()
        )),
    }
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

// ── semver ───────────────────────────────────────────────────────────────
//
// The `semver::` module is feature-gated. Versions cross the language
// boundary as dicts shaped `#{major, minor, patch, pre, build}` so scripts
// can inspect fields directly and round-trip through `json::encode`.

#[cfg(feature = "semver")]
fn semver_version_to_dict(v: &Version) -> Value {
    let mut d = indexmap::IndexMap::new();
    d.insert("major".to_string(), Value::Int(v.major as i64));
    d.insert("minor".to_string(), Value::Int(v.minor as i64));
    d.insert("patch".to_string(), Value::Int(v.patch as i64));
    d.insert("pre".to_string(), Value::Str(v.pre.as_str().to_string()));
    d.insert("build".to_string(), Value::Str(v.build.as_str().to_string()));
    Value::Dict(d)
}

/// Coerce a `Value` (string or dict) into a parsed `Version`. Used by every
/// semver function that compares or rewrites a version.
#[cfg(feature = "semver")]
fn semver_parse_arg(v: &Value, fn_name: &str) -> Result<Version, String> {
    match v {
        Value::Str(s) => {
            Version::parse(s).map_err(|e| format!("semver::{}: {}", fn_name, e))
        }
        Value::Dict(map) => {
            let major = map
                .get("major")
                .and_then(Value::as_int)
                .ok_or_else(|| format!("semver::{}: dict missing integer 'major'", fn_name))?;
            let minor = map
                .get("minor")
                .and_then(Value::as_int)
                .ok_or_else(|| format!("semver::{}: dict missing integer 'minor'", fn_name))?;
            let patch = map
                .get("patch")
                .and_then(Value::as_int)
                .ok_or_else(|| format!("semver::{}: dict missing integer 'patch'", fn_name))?;
            if major < 0 || minor < 0 || patch < 0 {
                return Err(format!(
                    "semver::{}: version components must be non-negative",
                    fn_name
                ));
            }
            let pre_str = map.get("pre").and_then(Value::as_str).unwrap_or("");
            let build_str = map.get("build").and_then(Value::as_str).unwrap_or("");
            let pre = if pre_str.is_empty() {
                Prerelease::EMPTY
            } else {
                Prerelease::new(pre_str).map_err(|e| {
                    format!("semver::{}: invalid pre-release '{}': {}", fn_name, pre_str, e)
                })?
            };
            let build = if build_str.is_empty() {
                BuildMetadata::EMPTY
            } else {
                BuildMetadata::new(build_str).map_err(|e| {
                    format!(
                        "semver::{}: invalid build metadata '{}': {}",
                        fn_name, build_str, e
                    )
                })?
            };
            Ok(Version {
                major: major as u64,
                minor: minor as u64,
                patch: patch as u64,
                pre,
                build,
            })
        }
        _ => Err(format!(
            "semver::{}: expected string or dict, got {}",
            fn_name,
            v.type_name()
        )),
    }
}

/// Build the `semver` stdlib module.
///
/// Functions: parse, is_valid, format, compare, eq, gt, gte, lt, lte,
///            satisfies, bump_major, bump_minor, bump_patch
#[cfg(feature = "semver")]
pub fn semver_module() -> Module {
    let mut m = Module::new("semver");

    m.register_fn("parse", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::parse takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("semver::parse requires a string"))?;
        let v = Version::parse(s).map_err(|e| format!("semver::parse: {}", e))?;
        Ok(semver_version_to_dict(&v))
    });

    m.register_fn("is_valid", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::is_valid takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("semver::is_valid requires a string"))?;
        Ok(Value::Bool(Version::parse(s).is_ok()))
    });

    m.register_fn("format", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::format takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0], "format")?;
        Ok(Value::Str(v.to_string()))
    });

    m.register_fn("compare", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::compare takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "compare")?;
        let b = semver_parse_arg(&args[1], "compare")?;
        Ok(Value::Int(match a.cmp(&b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }))
    });

    m.register_fn("eq", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::eq takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "eq")?;
        let b = semver_parse_arg(&args[1], "eq")?;
        Ok(Value::Bool(a == b))
    });

    m.register_fn("gt", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::gt takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "gt")?;
        let b = semver_parse_arg(&args[1], "gt")?;
        Ok(Value::Bool(a > b))
    });

    m.register_fn("gte", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::gte takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "gte")?;
        let b = semver_parse_arg(&args[1], "gte")?;
        Ok(Value::Bool(a >= b))
    });

    m.register_fn("lt", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::lt takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "lt")?;
        let b = semver_parse_arg(&args[1], "lt")?;
        Ok(Value::Bool(a < b))
    });

    m.register_fn("lte", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("semver::lte takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0], "lte")?;
        let b = semver_parse_arg(&args[1], "lte")?;
        Ok(Value::Bool(a <= b))
    });

    m.register_fn("satisfies", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!(
                "semver::satisfies takes 2 arguments: version, requirement"
            ));
        }
        let v = semver_parse_arg(&args[0], "satisfies")?;
        let req_str = args[1]
            .as_str()
            .ok_or_else(|| ion_str!("semver::satisfies requirement must be a string"))?;
        let req = VersionReq::parse(req_str)
            .map_err(|e| format!("semver::satisfies: invalid requirement: {}", e))?;
        Ok(Value::Bool(req.matches(&v)))
    });

    m.register_fn("bump_major", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::bump_major takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0], "bump_major")?;
        let bumped = Version::new(v.major + 1, 0, 0);
        Ok(Value::Str(bumped.to_string()))
    });

    m.register_fn("bump_minor", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::bump_minor takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0], "bump_minor")?;
        let bumped = Version::new(v.major, v.minor + 1, 0);
        Ok(Value::Str(bumped.to_string()))
    });

    m.register_fn("bump_patch", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("semver::bump_patch takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0], "bump_patch")?;
        // If the input has a pre-release, the bumped value is the same
        // numeric triple with the pre-release stripped (1.2.3-alpha → 1.2.3).
        // Otherwise increment patch.
        let bumped = if v.pre.is_empty() {
            Version::new(v.major, v.minor, v.patch + 1)
        } else {
            Version::new(v.major, v.minor, v.patch)
        };
        Ok(Value::Str(bumped.to_string()))
    });

    m
}

/// Register all stdlib modules in the given environment.
pub fn register_stdlib(env: &mut crate::env::Env) {
    register_stdlib_with_output(env, missing_output_handler());
}

/// Register all stdlib modules with a host-provided output handler.
pub fn register_stdlib_with_output(env: &mut crate::env::Env, output: Arc<dyn OutputHandler>) {
    let level = crate::log::AtomicLogLevel::default_runtime();
    let log_handler: Arc<dyn crate::log::LogHandler> = Arc::new(
        crate::log::StdLogHandler::with_threshold(Arc::clone(&level)),
    );
    register_stdlib_with_handlers(env, output, log_handler, level);
}

/// Register all stdlib modules with both a host output handler and a log
/// handler. The shared `level` is used by `log::set_level`/`log::level` to
/// gate the default `StdLogHandler`. Custom handlers are free to ignore it.
pub fn register_stdlib_with_handlers(
    env: &mut crate::env::Env,
    output: Arc<dyn OutputHandler>,
    log_handler: Arc<dyn crate::log::LogHandler>,
    level: Arc<crate::log::AtomicLogLevel>,
) {
    let math = math_module();
    env.define(math.name.clone(), math.to_value(), false);

    let json = json_module();
    env.define(json.name.clone(), json.to_value(), false);

    let io = io_module_with_output(output);
    env.define(io.name.clone(), io.to_value(), false);

    let string_mod = string_module();
    env.define(string_mod.name.clone(), string_mod.to_value(), false);

    let log = log_module_with_handler(log_handler, level);
    env.define(log.name.clone(), log.to_value(), false);

    #[cfg(feature = "semver")]
    {
        let s = semver_module();
        env.define(s.name.clone(), s.to_value(), false);
    }
}
