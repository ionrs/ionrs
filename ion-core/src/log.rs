//! Leveled logging for Ion scripts.
//!
//! Ion exposes `log::trace`, `log::debug`, `log::info`, `log::warn`, and
//! `log::error` as host functions in the `log::` module. Embedders install a
//! [`LogHandler`] (typically the [`TracingLogHandler`] under the `tracing`
//! feature) and the runtime dispatches each call into it.
//!
//! Two filtering layers cooperate:
//!
//! 1. **Compile-time cap** ([`COMPILE_LOG_CAP`]). The bytecode compiler strips
//!    every `log::<level>(...)` callsite whose level sits above the cap —
//!    arguments and all — so they cost nothing at runtime. The cap is selected
//!    by Cargo features that mirror `tracing`'s `release_max_level_*` flags;
//!    when none are enabled it defaults to `Trace` under `debug_assertions`
//!    and `Info` otherwise. Embedders compose this with `tracing`'s own
//!    feature flags by enabling the matching `ion-core/log_max_level_*` and
//!    `tracing/release_max_level_*` together.
//!
//! 2. **Runtime threshold** ([`LogHandler::enabled`]). For surviving callsites
//!    the handler can short-circuit before any string formatting happens. The
//!    default [`StdLogHandler`] honours the threshold set by
//!    [`Engine::set_log_level`] / `ION_LOG`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::value::Value;

/// Severity levels for [`LogHandler`]. Ordered so that lower-priority levels
/// compare *greater* (e.g. `Trace > Debug > Info > Warn > Error > Off`),
/// matching the sense of "level above the cap is dropped".
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl LogLevel {
    /// Parse a level name (case-insensitive). Returns `None` on unknown input.
    pub fn from_str_ci(s: &str) -> Option<Self> {
        fn lower(byte: u8) -> u8 {
            if byte.is_ascii_uppercase() {
                byte + 32
            } else {
                byte
            }
        }

        let bytes = s.as_bytes();
        match bytes.len() {
            3 => match (lower(bytes[0]), lower(bytes[1]), lower(bytes[2])) {
                (111, 102, 102) => Some(Self::Off),
                (101, 114, 114) => Some(Self::Error),
                _ => None,
            },
            4 => match (
                lower(bytes[0]),
                lower(bytes[1]),
                lower(bytes[2]),
                lower(bytes[3]),
            ) {
                (110, 111, 110, 101) => Some(Self::Off),
                (119, 97, 114, 110) => Some(Self::Warn),
                (105, 110, 102, 111) => Some(Self::Info),
                _ => None,
            },
            5 => match (
                lower(bytes[0]),
                lower(bytes[1]),
                lower(bytes[2]),
                lower(bytes[3]),
                lower(bytes[4]),
            ) {
                (101, 114, 114, 111, 114) => Some(Self::Error),
                (100, 101, 98, 117, 103) => Some(Self::Debug),
                (116, 114, 97, 99, 101) => Some(Self::Trace),
                _ => None,
            },
            7 => match (
                lower(bytes[0]),
                lower(bytes[1]),
                lower(bytes[2]),
                lower(bytes[3]),
                lower(bytes[4]),
                lower(bytes[5]),
                lower(bytes[6]),
            ) {
                (119, 97, 114, 110, 105, 110, 103) => Some(Self::Warn),
                _ => None,
            },
            _ => None,
        }
    }

    #[cfg(debug_assertions)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    #[cfg(not(debug_assertions))]
    pub const fn as_str(self) -> &'static str {
        let _ = self;
        ion_static_str!("level")
    }

    /// True when emitting at `self` should be allowed under the threshold
    /// `cap` (i.e. `self <= cap`).
    pub const fn allowed_under(self, cap: LogLevel) -> bool {
        (self as u8) <= (cap as u8)
    }
}

#[cfg(not(debug_assertions))]
impl std::fmt::Debug for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = self;
        f.write_str(ion_static_str!("LogLevel"))
    }
}

/// Compile-time level cap derived from Cargo features. Set explicitly by
/// `ion-core/log_max_level_*`; otherwise `Trace` under `debug_assertions`,
/// `Info` for release builds. Mirrors `tracing`'s `release_max_level_*`.
pub const COMPILE_LOG_CAP: LogLevel = compute_compile_cap();

const fn compute_compile_cap() -> LogLevel {
    if cfg!(feature = "log_max_level_off") {
        LogLevel::Off
    } else if cfg!(feature = "log_max_level_error") {
        LogLevel::Error
    } else if cfg!(feature = "log_max_level_warn") {
        LogLevel::Warn
    } else if cfg!(feature = "log_max_level_info") {
        LogLevel::Info
    } else if cfg!(feature = "log_max_level_debug") {
        LogLevel::Debug
    } else if cfg!(feature = "log_max_level_trace") {
        LogLevel::Trace
    } else if cfg!(debug_assertions) {
        LogLevel::Trace
    } else {
        LogLevel::Info
    }
}

/// Sink for `log::*` calls that survived compile-time stripping.
///
/// Embedders implement this to route Ion log records into their own
/// observability stack. The default [`StdLogHandler`] writes to stderr; the
/// optional [`TracingLogHandler`] (under the `tracing` feature) forwards to
/// `tracing::event!`.
pub trait LogHandler: Send + Sync {
    /// Emit a log record. `fields` are the entries from the optional dict
    /// argument (or empty). The handler should not mutate the slice.
    fn log(&self, level: LogLevel, message: &str, fields: &[(String, Value)]);

    /// Pre-flight check used by `log::*` host functions: when this returns
    /// `false`, the runtime skips formatting and the [`Self::log`] call.
    /// Default implementation always returns `true`.
    fn enabled(&self, _level: LogLevel) -> bool {
        true
    }
}

/// Built-in [`LogHandler`] that writes to stderr in the format
/// `LEVEL message [k1=v1 k2=v2]`. Threshold is controlled via the shared
/// [`AtomicLogLevel`].
pub struct StdLogHandler {
    threshold: Arc<AtomicLogLevel>,
}

impl StdLogHandler {
    pub fn new() -> Self {
        Self::with_threshold(AtomicLogLevel::default_runtime())
    }

    pub fn with_threshold(threshold: Arc<AtomicLogLevel>) -> Self {
        Self { threshold }
    }

    pub fn threshold(&self) -> Arc<AtomicLogLevel> {
        Arc::clone(&self.threshold)
    }
}

impl Default for StdLogHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl LogHandler for StdLogHandler {
    fn enabled(&self, level: LogLevel) -> bool {
        level.allowed_under(self.threshold.get())
    }

    fn log(&self, level: LogLevel, message: &str, fields: &[(String, Value)]) {
        use std::io::Write;
        let mut out = format!("{} {}", level.as_str().to_uppercase(), message);
        if !fields.is_empty() {
            out.push_str(" [");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(k);
                out.push('=');
                out.push_str(&format_field_value(v));
            }
            out.push(']');
        }
        out.push('\n');
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(out.as_bytes());
    }
}

fn format_field_value(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        other => format!("{}", other),
    }
}

/// Bridge to the [`tracing`](https://docs.rs/tracing) crate. Available under
/// the `tracing` Cargo feature. Each Ion level maps to the corresponding
/// `tracing::Level`. Field dicts are emitted as a single
/// `fields = "k1=v1 k2=v2"` event field (kept simple — embedders that want
/// structured fields with named keys should write a custom handler).
#[cfg(feature = "tracing")]
pub struct TracingLogHandler;

#[cfg(feature = "tracing")]
impl TracingLogHandler {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "tracing")]
impl Default for TracingLogHandler {
    fn default() -> Self {
        Self
    }
}

#[cfg(feature = "tracing")]
impl LogHandler for TracingLogHandler {
    fn enabled(&self, level: LogLevel) -> bool {
        // Defer to tracing's runtime filter via `event_enabled!`.
        match level {
            LogLevel::Off => false,
            LogLevel::Error => tracing::event_enabled!(tracing::Level::ERROR),
            LogLevel::Warn => tracing::event_enabled!(tracing::Level::WARN),
            LogLevel::Info => tracing::event_enabled!(tracing::Level::INFO),
            LogLevel::Debug => tracing::event_enabled!(tracing::Level::DEBUG),
            LogLevel::Trace => tracing::event_enabled!(tracing::Level::TRACE),
        }
    }

    fn log(&self, level: LogLevel, message: &str, fields: &[(String, Value)]) {
        let rendered = render_fields(fields);
        match level {
            LogLevel::Off => {}
            LogLevel::Error => tracing::error!(fields = %rendered, "{}", message),
            LogLevel::Warn => tracing::warn!(fields = %rendered, "{}", message),
            LogLevel::Info => tracing::info!(fields = %rendered, "{}", message),
            LogLevel::Debug => tracing::debug!(fields = %rendered, "{}", message),
            LogLevel::Trace => tracing::trace!(fields = %rendered, "{}", message),
        }
    }
}

#[cfg(feature = "tracing")]
fn render_fields(fields: &[(String, Value)]) -> String {
    let mut s = String::new();
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(k);
        s.push('=');
        s.push_str(&format_field_value(v));
    }
    s
}

/// Shared atomic level used to gate the default [`StdLogHandler`] at runtime.
#[derive(Debug)]
pub struct AtomicLogLevel(AtomicU8);

impl AtomicLogLevel {
    pub fn new(level: LogLevel) -> Self {
        Self(AtomicU8::new(level as u8))
    }

    pub fn get(&self) -> LogLevel {
        match self.0.load(Ordering::Relaxed) {
            1 => LogLevel::Error,
            2 => LogLevel::Warn,
            3 => LogLevel::Info,
            4 => LogLevel::Debug,
            5 => LogLevel::Trace,
            _ => LogLevel::Off,
        }
    }

    pub fn set(&self, level: LogLevel) {
        self.0.store(level as u8, Ordering::Relaxed);
    }

    /// Default runtime threshold. Debug builds honor `ION_LOG`; release builds
    /// keep the environment variable name out of the binary.
    pub fn default_runtime() -> Arc<Self> {
        #[cfg(debug_assertions)]
        let level = std::env::var("ION_LOG")
            .ok()
            .and_then(|s| LogLevel::from_str_ci(s.trim()))
            .unwrap_or_else(|| {
                if cfg!(debug_assertions) {
                    LogLevel::Debug
                } else {
                    LogLevel::Info
                }
            });

        #[cfg(not(debug_assertions))]
        let level = LogLevel::Info;

        Arc::new(Self::new(level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering() {
        assert!(LogLevel::Error.allowed_under(LogLevel::Info));
        assert!(LogLevel::Info.allowed_under(LogLevel::Info));
        assert!(!LogLevel::Debug.allowed_under(LogLevel::Info));
        assert!(!LogLevel::Trace.allowed_under(LogLevel::Off));
    }

    #[test]
    fn parse_level() {
        assert_eq!(LogLevel::from_str_ci("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str_ci("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str_ci("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str_ci("err"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str_ci("nope"), None);
    }

    #[test]
    fn atomic_round_trip() {
        let lvl = AtomicLogLevel::new(LogLevel::Warn);
        assert_eq!(lvl.get(), LogLevel::Warn);
        lvl.set(LogLevel::Trace);
        assert_eq!(lvl.get(), LogLevel::Trace);
    }
}
