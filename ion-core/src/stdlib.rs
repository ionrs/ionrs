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

/// Build the `io` stdlib module with a host-provided output handler — sync
/// build. Calls into [`OutputHandler::write`] directly.
#[cfg(not(feature = "async-runtime"))]
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

/// Build the `io` stdlib module with a host-provided output handler — async
/// build. Each `io::print*` call dispatches the (still-sync) handler write
/// onto Tokio's blocking thread pool via `spawn_blocking`, so a slow stdout
/// can't stall the executor running other Ion tasks.
#[cfg(feature = "async-runtime")]
pub fn io_module_with_output(output: Arc<dyn OutputHandler>) -> Module {
    use crate::error::IonError;
    let mut m = Module::new("io");

    let stdout = Arc::clone(&output);
    m.register_async_fn("print", move |args| {
        let stdout = Arc::clone(&stdout);
        async move {
            let text = format_output_args(&args);
            tokio::task::spawn_blocking(move || stdout.write(OutputStream::Stdout, &text))
                .await
                .map_err(|e| IonError::runtime(format!("io::print: join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(format!("io::print: {}", e), 0, 0))?;
            Ok(Value::Unit)
        }
    });

    let stdout = Arc::clone(&output);
    m.register_async_fn("println", move |args| {
        let stdout = Arc::clone(&stdout);
        async move {
            let mut text = format_output_args(&args);
            text.push('\n');
            tokio::task::spawn_blocking(move || stdout.write(OutputStream::Stdout, &text))
                .await
                .map_err(|e| IonError::runtime(format!("io::println: join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(format!("io::println: {}", e), 0, 0))?;
            Ok(Value::Unit)
        }
    });

    let stderr = Arc::clone(&output);
    m.register_async_fn("eprintln", move |args| {
        let stderr = Arc::clone(&stderr);
        async move {
            let mut text = format_output_args(&args);
            text.push('\n');
            tokio::task::spawn_blocking(move || stderr.write(OutputStream::Stderr, &text))
                .await
                .map_err(|e| IonError::runtime(format!("io::eprintln: join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(format!("io::eprintln: {}", e), 0, 0))?;
            Ok(Value::Unit)
        }
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

// ── os ───────────────────────────────────────────────────────────────────
//
// The `os::` module is feature-gated. OS / arch detection values are baked
// in at build time (from `std::env::consts`); env-var and process-info
// helpers call `std` directly. Script args are host-injected via
// `Engine::set_args` and reach scripts as `os::args()`.

/// Build the `os` stdlib module with the given script args (used by
/// `os::args()`). Pass `Arc::new(Vec::new())` for the default empty list.
#[cfg(feature = "os")]
pub fn os_module_with_args(args: Arc<Vec<String>>) -> Module {
    let mut m = Module::new("os");

    // Detection constants
    m.set("name", Value::Str(std::env::consts::OS.to_string()));
    m.set("arch", Value::Str(std::env::consts::ARCH.to_string()));
    m.set("family", Value::Str(std::env::consts::FAMILY.to_string()));
    m.set(
        "dll_extension",
        Value::Str(std::env::consts::DLL_EXTENSION.to_string()),
    );
    m.set(
        "exe_extension",
        Value::Str(std::env::consts::EXE_EXTENSION.to_string()),
    );
    m.set(
        "pointer_width",
        Value::Int((std::mem::size_of::<usize>() * 8) as i64),
    );

    m.register_fn("env_var", |args: &[Value]| {
        if args.is_empty() || args.len() > 2 {
            return Err(ion_str!(
                "os::env_var takes 1 or 2 arguments: name, [default]"
            ));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("os::env_var: name must be a string"))?;
        match std::env::var(name) {
            Ok(v) => Ok(Value::Str(v)),
            Err(_) if args.len() == 2 => Ok(args[1].clone()),
            Err(e) => Err(format!("os::env_var('{}'): {}", name, e)),
        }
    });

    m.register_fn("has_env_var", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("os::has_env_var takes 1 argument"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("os::has_env_var: name must be a string"))?;
        Ok(Value::Bool(std::env::var_os(name).is_some()))
    });

    m.register_fn("env_vars", |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("os::env_vars takes no arguments"));
        }
        let mut d = indexmap::IndexMap::new();
        for (k, v) in std::env::vars() {
            d.insert(k, Value::Str(v));
        }
        Ok(Value::Dict(d))
    });

    m.register_fn("cwd", |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("os::cwd takes no arguments"));
        }
        std::env::current_dir()
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| format!("os::cwd: {}", e))
    });

    m.register_fn("pid", |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("os::pid takes no arguments"));
        }
        Ok(Value::Int(std::process::id() as i64))
    });

    m.register_fn("temp_dir", |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("os::temp_dir takes no arguments"));
        }
        Ok(Value::Str(
            std::env::temp_dir().to_string_lossy().into_owned(),
        ))
    });

    let args_arc = Arc::clone(&args);
    m.register_closure("args", move |call_args: &[Value]| {
        if !call_args.is_empty() {
            return Err(ion_str!("os::args takes no arguments"));
        }
        Ok(Value::List(
            args_arc.iter().map(|s| Value::Str(s.clone())).collect(),
        ))
    });

    m
}

/// Build the `os::` stdlib module with no script args. Use
/// `Engine::set_args` afterwards to inject argv reachable as `os::args()`.
#[cfg(feature = "os")]
pub fn os_module() -> Module {
    os_module_with_args(Arc::new(Vec::new()))
}

// ── path ─────────────────────────────────────────────────────────────────
//
// Pure string-level path manipulation. No I/O, no async, no feature gate —
// this is composition glue that should always be available. Operates on
// strings and returns strings; `Value::Bytes` paths are not supported.

/// Build the `path::` stdlib module.
pub fn path_module() -> Module {
    use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};

    let mut m = Module::new("path");

    m.set("sep", Value::Str(MAIN_SEPARATOR_STR.to_string()));

    m.register_fn("join", |args: &[Value]| {
        if args.is_empty() {
            return Err(ion_str!("path::join takes at least 1 argument"));
        }
        let mut buf = PathBuf::new();
        for (i, arg) in args.iter().enumerate() {
            let s = arg.as_str().ok_or_else(|| {
                format!("path::join: argument {} must be a string", i + 1)
            })?;
            buf.push(s);
        }
        Ok(Value::Str(buf.to_string_lossy().into_owned()))
    });

    m.register_fn("parent", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::parent takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::parent requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn("basename", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::basename takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::basename requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn("stem", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::stem takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::stem requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn("extension", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::extension takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::extension requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .extension()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn("with_extension", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("path::with_extension takes 2 arguments: path, ext"));
        }
        let p = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::with_extension: path must be a string"))?;
        let ext = args[1]
            .as_str()
            .ok_or_else(|| ion_str!("path::with_extension: ext must be a string"))?;
        Ok(Value::Str(
            Path::new(p)
                .with_extension(ext)
                .to_string_lossy()
                .into_owned(),
        ))
    });

    m.register_fn("is_absolute", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::is_absolute takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::is_absolute requires a string"))?;
        Ok(Value::Bool(Path::new(s).is_absolute()))
    });

    m.register_fn("is_relative", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::is_relative takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::is_relative requires a string"))?;
        Ok(Value::Bool(Path::new(s).is_relative()))
    });

    m.register_fn("components", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("path::components takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::components requires a string"))?;
        let comps: Vec<Value> = Path::new(s)
            .components()
            .map(|c| Value::Str(c.as_os_str().to_string_lossy().into_owned()))
            .collect();
        Ok(Value::List(comps))
    });

    m.register_fn("normalize", |args: &[Value]| {
        // Lexical normalisation: collapse `.` and `..` without consulting the
        // filesystem. Mirrors `path.Clean` from Go / Node's `path.normalize`.
        if args.len() != 1 {
            return Err(ion_str!("path::normalize takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path::normalize requires a string"))?;
        use std::path::Component;
        let mut out = PathBuf::new();
        for c in Path::new(s).components() {
            match c {
                Component::Prefix(p) => {
                    out.push(p.as_os_str());
                }
                Component::RootDir => {
                    out.push(std::path::Component::RootDir);
                }
                Component::CurDir => { /* skip */ }
                Component::ParentDir => {
                    // Pop the last normal component if any, otherwise keep the `..`
                    let popped = match out.components().next_back() {
                        Some(Component::Normal(_)) => out.pop(),
                        _ => false,
                    };
                    if !popped {
                        out.push("..");
                    }
                }
                Component::Normal(n) => out.push(n),
            }
        }
        let result = if out.as_os_str().is_empty() {
            ".".to_string()
        } else {
            out.to_string_lossy().into_owned()
        };
        Ok(Value::Str(result))
    });

    m
}

// ── fs ───────────────────────────────────────────────────────────────────
//
// Filesystem I/O. The script-level surface (`fs::read`, `fs::write`, …) is
// identical regardless of build mode; only the underlying impl changes. In
// a sync build (`async-runtime` off) operations call `std::fs` directly. In
// an async build they're registered via `register_async_fn` and call
// `tokio::fs`, so they cooperate with the executor instead of blocking it.
//
// `read_bytes` returns `bytes`; everything else returns strings or unit.

#[cfg(feature = "fs")]
fn fs_metadata_to_dict(md: &std::fs::Metadata) -> Value {
    let mut d = indexmap::IndexMap::new();
    d.insert("size".to_string(), Value::Int(md.len() as i64));
    d.insert("is_file".to_string(), Value::Bool(md.is_file()));
    d.insert("is_dir".to_string(), Value::Bool(md.is_dir()));
    d.insert("readonly".to_string(), Value::Bool(md.permissions().readonly()));
    let modified = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| Value::Int(d.as_secs() as i64))
        .unwrap_or(Value::Unit);
    d.insert("modified".to_string(), modified);
    Value::Dict(d)
}

#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
fn fs_arg_str<'a>(args: &'a [Value], fn_name: &str, idx: usize) -> Result<&'a str, String> {
    args[idx]
        .as_str()
        .ok_or_else(|| format!("fs::{}: argument {} must be a string", fn_name, idx + 1))
}

/// Build the `fs::` stdlib module — sync impl backed by `std::fs`. Used when
/// the `async-runtime` feature is **not** enabled.
#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
pub fn fs_module() -> Module {
    let mut m = Module::new("fs");

    m.register_fn("read", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::read takes 1 argument"));
        }
        let path = fs_arg_str(args, "read", 0)?;
        std::fs::read_to_string(path)
            .map(Value::Str)
            .map_err(|e| format!("fs::read('{}'): {}", path, e))
    });

    m.register_fn("read_bytes", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::read_bytes takes 1 argument"));
        }
        let path = fs_arg_str(args, "read_bytes", 0)?;
        std::fs::read(path)
            .map(Value::Bytes)
            .map_err(|e| format!("fs::read_bytes('{}'): {}", path, e))
    });

    m.register_fn("write", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("fs::write takes 2 arguments: path, contents"));
        }
        let path = fs_arg_str(args, "write", 0)?;
        let result = match &args[1] {
            Value::Str(s) => std::fs::write(path, s.as_bytes()),
            Value::Bytes(b) => std::fs::write(path, b),
            other => {
                return Err(format!(
                    "fs::write: contents must be string or bytes, got {}",
                    other.type_name()
                ));
            }
        };
        result
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::write('{}'): {}", path, e))
    });

    m.register_fn("append", |args: &[Value]| {
        use std::io::Write;
        if args.len() != 2 {
            return Err(ion_str!("fs::append takes 2 arguments: path, contents"));
        }
        let path = fs_arg_str(args, "append", 0)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("fs::append('{}'): {}", path, e))?;
        let bytes: &[u8] = match &args[1] {
            Value::Str(s) => s.as_bytes(),
            Value::Bytes(b) => b,
            other => {
                return Err(format!(
                    "fs::append: contents must be string or bytes, got {}",
                    other.type_name()
                ));
            }
        };
        f.write_all(bytes)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::append('{}'): {}", path, e))
    });

    m.register_fn("exists", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::exists takes 1 argument"));
        }
        let path = fs_arg_str(args, "exists", 0)?;
        Ok(Value::Bool(std::path::Path::new(path).exists()))
    });

    m.register_fn("is_file", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::is_file takes 1 argument"));
        }
        let path = fs_arg_str(args, "is_file", 0)?;
        Ok(Value::Bool(std::path::Path::new(path).is_file()))
    });

    m.register_fn("is_dir", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::is_dir takes 1 argument"));
        }
        let path = fs_arg_str(args, "is_dir", 0)?;
        Ok(Value::Bool(std::path::Path::new(path).is_dir()))
    });

    m.register_fn("list_dir", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::list_dir takes 1 argument"));
        }
        let path = fs_arg_str(args, "list_dir", 0)?;
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("fs::list_dir('{}'): {}", path, e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("fs::list_dir('{}'): {}", path, e))?;
            names.push(Value::Str(entry.file_name().to_string_lossy().into_owned()));
        }
        Ok(Value::List(names))
    });

    m.register_fn("create_dir", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::create_dir takes 1 argument"));
        }
        let path = fs_arg_str(args, "create_dir", 0)?;
        std::fs::create_dir(path)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::create_dir('{}'): {}", path, e))
    });

    m.register_fn("create_dir_all", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::create_dir_all takes 1 argument"));
        }
        let path = fs_arg_str(args, "create_dir_all", 0)?;
        std::fs::create_dir_all(path)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::create_dir_all('{}'): {}", path, e))
    });

    m.register_fn("remove_file", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::remove_file takes 1 argument"));
        }
        let path = fs_arg_str(args, "remove_file", 0)?;
        std::fs::remove_file(path)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::remove_file('{}'): {}", path, e))
    });

    m.register_fn("remove_dir", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::remove_dir takes 1 argument"));
        }
        let path = fs_arg_str(args, "remove_dir", 0)?;
        std::fs::remove_dir(path)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::remove_dir('{}'): {}", path, e))
    });

    m.register_fn("remove_dir_all", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::remove_dir_all takes 1 argument"));
        }
        let path = fs_arg_str(args, "remove_dir_all", 0)?;
        std::fs::remove_dir_all(path)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::remove_dir_all('{}'): {}", path, e))
    });

    m.register_fn("rename", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("fs::rename takes 2 arguments: from, to"));
        }
        let from = fs_arg_str(args, "rename", 0)?;
        let to = fs_arg_str(args, "rename", 1)?;
        std::fs::rename(from, to)
            .map(|_| Value::Unit)
            .map_err(|e| format!("fs::rename('{}' -> '{}'): {}", from, to, e))
    });

    m.register_fn("copy", |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("fs::copy takes 2 arguments: from, to"));
        }
        let from = fs_arg_str(args, "copy", 0)?;
        let to = fs_arg_str(args, "copy", 1)?;
        std::fs::copy(from, to)
            .map(|n| Value::Int(n as i64))
            .map_err(|e| format!("fs::copy('{}' -> '{}'): {}", from, to, e))
    });

    m.register_fn("metadata", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::metadata takes 1 argument"));
        }
        let path = fs_arg_str(args, "metadata", 0)?;
        let md = std::fs::metadata(path)
            .map_err(|e| format!("fs::metadata('{}'): {}", path, e))?;
        Ok(fs_metadata_to_dict(&md))
    });

    m.register_fn("canonicalize", |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("fs::canonicalize takes 1 argument"));
        }
        let path = fs_arg_str(args, "canonicalize", 0)?;
        std::fs::canonicalize(path)
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| format!("fs::canonicalize('{}'): {}", path, e))
    });

    m
}

/// Build the `fs::` stdlib module — async impl backed by `tokio::fs`. Used
/// when the `async-runtime` feature is enabled. Surface matches the sync
/// build exactly; scripts call these the same way under `Engine::eval_async`.
#[cfg(all(feature = "fs", feature = "async-runtime"))]
pub fn fs_module() -> Module {
    use crate::error::IonError;

    fn arg_str(args: &[Value], fn_name: &str, idx: usize) -> Result<String, IonError> {
        args.get(idx)
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(|| {
                IonError::runtime(
                    format!("fs::{}: argument {} must be a string", fn_name, idx + 1),
                    0,
                    0,
                )
            })
    }

    fn io_err(fn_name: &str, target: &str, e: std::io::Error) -> IonError {
        IonError::runtime(format!("fs::{}('{}'): {}", fn_name, target, e), 0, 0)
    }

    let mut m = Module::new("fs");

    m.register_async_fn("read", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::read takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "read", 0)?;
        tokio::fs::read_to_string(&path)
            .await
            .map(Value::Str)
            .map_err(|e| io_err("read", &path, e))
    });

    m.register_async_fn("read_bytes", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::read_bytes takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "read_bytes", 0)?;
        tokio::fs::read(&path)
            .await
            .map(Value::Bytes)
            .map_err(|e| io_err("read_bytes", &path, e))
    });

    m.register_async_fn("write", |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                "fs::write takes 2 arguments: path, contents",
                0,
                0,
            ));
        }
        let path = arg_str(&args, "write", 0)?;
        let bytes: Vec<u8> = match &args[1] {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Bytes(b) => b.clone(),
            other => {
                return Err(IonError::runtime(
                    format!(
                        "fs::write: contents must be string or bytes, got {}",
                        other.type_name()
                    ),
                    0,
                    0,
                ));
            }
        };
        tokio::fs::write(&path, &bytes)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("write", &path, e))
    });

    m.register_async_fn("append", |args| async move {
        use tokio::io::AsyncWriteExt;
        if args.len() != 2 {
            return Err(IonError::runtime(
                "fs::append takes 2 arguments: path, contents",
                0,
                0,
            ));
        }
        let path = arg_str(&args, "append", 0)?;
        let bytes: Vec<u8> = match &args[1] {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Bytes(b) => b.clone(),
            other => {
                return Err(IonError::runtime(
                    format!(
                        "fs::append: contents must be string or bytes, got {}",
                        other.type_name()
                    ),
                    0,
                    0,
                ));
            }
        };
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| io_err("append", &path, e))?;
        f.write_all(&bytes)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("append", &path, e))
    });

    m.register_async_fn("exists", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::exists takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "exists", 0)?;
        // `tokio::fs::try_exists` on stable. Fall back to a metadata check so
        // we behave the same as `Path::exists()` did in the sync impl.
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(Value::Bool(true)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err("exists", &path, e)),
        }
    });

    m.register_async_fn("is_file", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::is_file takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "is_file", 0)?;
        match tokio::fs::metadata(&path).await {
            Ok(md) => Ok(Value::Bool(md.is_file())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err("is_file", &path, e)),
        }
    });

    m.register_async_fn("is_dir", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::is_dir takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "is_dir", 0)?;
        match tokio::fs::metadata(&path).await {
            Ok(md) => Ok(Value::Bool(md.is_dir())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err("is_dir", &path, e)),
        }
    });

    m.register_async_fn("list_dir", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::list_dir takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "list_dir", 0)?;
        let mut rd = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| io_err("list_dir", &path, e))?;
        let mut names = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => {
                    names.push(Value::Str(entry.file_name().to_string_lossy().into_owned()));
                }
                Ok(None) => break,
                Err(e) => return Err(io_err("list_dir", &path, e)),
            }
        }
        Ok(Value::List(names))
    });

    m.register_async_fn("create_dir", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::create_dir takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "create_dir", 0)?;
        tokio::fs::create_dir(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("create_dir", &path, e))
    });

    m.register_async_fn("create_dir_all", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(
                "fs::create_dir_all takes 1 argument",
                0,
                0,
            ));
        }
        let path = arg_str(&args, "create_dir_all", 0)?;
        tokio::fs::create_dir_all(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("create_dir_all", &path, e))
    });

    m.register_async_fn("remove_file", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::remove_file takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "remove_file", 0)?;
        tokio::fs::remove_file(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("remove_file", &path, e))
    });

    m.register_async_fn("remove_dir", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::remove_dir takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "remove_dir", 0)?;
        tokio::fs::remove_dir(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("remove_dir", &path, e))
    });

    m.register_async_fn("remove_dir_all", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(
                "fs::remove_dir_all takes 1 argument",
                0,
                0,
            ));
        }
        let path = arg_str(&args, "remove_dir_all", 0)?;
        tokio::fs::remove_dir_all(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err("remove_dir_all", &path, e))
    });

    m.register_async_fn("rename", |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                "fs::rename takes 2 arguments: from, to",
                0,
                0,
            ));
        }
        let from = arg_str(&args, "rename", 0)?;
        let to = arg_str(&args, "rename", 1)?;
        tokio::fs::rename(&from, &to)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| {
                IonError::runtime(format!("fs::rename('{}' -> '{}'): {}", from, to, e), 0, 0)
            })
    });

    m.register_async_fn("copy", |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                "fs::copy takes 2 arguments: from, to",
                0,
                0,
            ));
        }
        let from = arg_str(&args, "copy", 0)?;
        let to = arg_str(&args, "copy", 1)?;
        tokio::fs::copy(&from, &to)
            .await
            .map(|n| Value::Int(n as i64))
            .map_err(|e| {
                IonError::runtime(format!("fs::copy('{}' -> '{}'): {}", from, to, e), 0, 0)
            })
    });

    m.register_async_fn("metadata", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::metadata takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "metadata", 0)?;
        let md = tokio::fs::metadata(&path)
            .await
            .map_err(|e| io_err("metadata", &path, e))?;
        Ok(fs_metadata_to_dict(&md))
    });

    m.register_async_fn("canonicalize", |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime("fs::canonicalize takes 1 argument", 0, 0));
        }
        let path = arg_str(&args, "canonicalize", 0)?;
        tokio::fs::canonicalize(&path)
            .await
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| io_err("canonicalize", &path, e))
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

    #[cfg(feature = "os")]
    {
        let os = os_module();
        env.define(os.name.clone(), os.to_value(), false);
    }

    let path = path_module();
    env.define(path.name.clone(), path.to_value(), false);

    #[cfg(feature = "fs")]
    {
        let fs = fs_module();
        env.define(fs.name.clone(), fs.to_value(), false);
    }
}
