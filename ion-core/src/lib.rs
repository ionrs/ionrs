#![allow(clippy::result_large_err)]
#![cfg_attr(not(debug_assertions), allow(dead_code, unused_variables))]
//! Ion — an embeddable scripting language for Rust.
//!
//! Ion is a small, strongly-typed scripting language inspired by Starlark,
//! designed for embedding in Rust applications. It features a tree-walk
//! interpreter and an optional bytecode VM for better performance.
//!
//! # Quick Start
//!
//! ```rust
//! # #[cfg(not(feature = "async-runtime"))]
//! # fn main() {
//! use ion_core::engine::Engine;
//!
//! let mut engine = Engine::new();
//! let result = engine.eval("1 + 2").unwrap();
//! assert_eq!(result, ion_core::value::Value::Int(3));
//! # }
//! # #[cfg(feature = "async-runtime")]
//! # fn main() {}
//! ```
//!
//! # Features
//!
//! - **`vm`** (default) — Bytecode compiler and stack-based VM with
//!   peephole optimization, constant folding, dead-code elimination, and
//!   tail-call optimization
//! - **`derive`** (default) — `#[derive(IonType)]` for host type injection
//! - **`async-runtime`** — Native Tokio async evaluation via
//!   [`engine::Engine::eval_async`], async host functions, `spawn`/`.await`/
//!   `select`, timers, and channels
//! - **`legacy-threaded-concurrency`** — Legacy sync-eval backend using OS
//!   threads and crossbeam channels
//! - **`msgpack`** — `Value::to_msgpack()` / `from_msgpack()` via `rmpv`
//! - **`rewrite`** — Source rewriter at [`rewrite::replace_global`]

/// Public error/message string facade. In debug builds this preserves the
/// literal for diagnostics; release-only dynamic detail is stripped by
/// `ion_format!` and the error constructors in [`error`].
#[cfg(debug_assertions)]
macro_rules! ion_str {
    ($s:literal) => {
        redacted_error::message_string!($s)
    };
}

#[cfg(not(debug_assertions))]
macro_rules! ion_str {
    ($s:literal) => {
        redacted_error::message_string!("runtime error")
    };
}

#[cfg(debug_assertions)]
macro_rules! ion_format {
    ($($arg:tt)*) => {
        format!($($arg)*)
    };
}

#[cfg(not(debug_assertions))]
macro_rules! ion_format {
    ($($arg:tt)*) => {
        redacted_error::message_string!("runtime error")
    };
}

macro_rules! ion_obf_string {
    ($s:literal) => {
        redacted_error::message_string!($s)
    };
}

/// Same as `ion_str!` but returns `&str` for contexts requiring `&'static str`
/// like type names. Release builds collapse these diagnostics to a generic
/// public word.
#[cfg(debug_assertions)]
macro_rules! ion_static_str {
    ($s:literal) => {
        $s
    };
}

#[cfg(not(debug_assertions))]
macro_rules! ion_static_str {
    ($s:literal) => {
        "value"
    };
}

/// Define a top-level global built-in function in an `Env`. The name literal
/// is hashed at compile time via `h!`, so the source identifier never reaches
/// `.rodata`. Equivalent to writing `env.define_h(h!(name), Value::BuiltinFn
/// { qualified_hash: h!(name), func: $f })` by hand.
macro_rules! global_builtin {
    ($env:expr, $name:literal, $f:expr) => {{
        let __h: u64 = $crate::h!($name);
        $env.define_h(
            __h,
            $crate::value::Value::BuiltinFn {
                qualified_hash: __h,
                func: $f,
                signature: None,
            },
        );
    }};
}

pub mod ast;
#[cfg(all(
    feature = "legacy-threaded-concurrency",
    not(feature = "async-runtime")
))]
pub mod async_rt;
#[cfg(all(
    feature = "legacy-threaded-concurrency",
    not(feature = "async-runtime")
))]
pub mod async_rt_std;
#[cfg(feature = "async-runtime")]
pub mod async_runtime;
#[cfg(feature = "vm")]
pub mod bytecode;
pub mod call;
#[cfg(feature = "vm")]
pub mod compiler;
pub mod engine;
pub mod env;
pub mod error;
pub mod hash;
pub mod host_types;
pub mod intern;
pub mod interpreter;
pub mod lexer;
pub mod log;
pub mod module;
pub mod names;
pub mod parser;
#[cfg(feature = "rewrite")]
pub mod rewrite;
pub mod stdlib;
pub mod token;
pub mod value;
#[cfg(feature = "vm")]
pub mod vm;

pub use engine::Engine;
pub use value::Value;

#[cfg(feature = "derive")]
pub use ion_derive::IonType;

/// Canonical doc manifest for the Ion standard library, in the
/// `ionDocVersion: 2` format (`IonDocManifest`). Embedded at compile time
/// via `include_str!` so it ships with every `ion-core` build.
///
/// Both the LSP (for hover/completion) and the docs site read from this
/// single source — keeping editor tooltips and the published reference in
/// sync without an extra build step.
#[cfg(any(test, feature = "embedded-stdlib-docs"))]
pub const STDLIB_DOCS_JSON: &str = include_str!("stdlib-docs.json");

#[cfg(test)]
mod stdlib_docs_tests {
    use super::STDLIB_DOCS_JSON;

    #[test]
    fn embedded_manifest_is_valid_v2_json() {
        let v: serde_json::Value =
            serde_json::from_str(STDLIB_DOCS_JSON).expect("stdlib-docs.json parses");
        assert_eq!(v["ionDocVersion"], 2, "must be ionDocVersion: 2");
        let modules = v["modules"].as_array().expect("modules is an array");
        assert!(
            modules.iter().any(|m| m["name"] == "core"),
            "core (global builtins) module is required"
        );
        assert!(
            modules.iter().any(|m| m["name"] == "types"),
            "types (built-in types) module is required"
        );
        for m in modules {
            assert!(m["name"].is_string(), "every module needs a name");
            for member in m["members"].as_array().unwrap_or(&Vec::new()) {
                let kind = member["kind"].as_str().unwrap_or("");
                assert!(
                    matches!(
                        kind,
                        "function" | "constant" | "method" | "type" | "builtin"
                    ),
                    "unknown member kind: {kind}"
                );
            }
        }
    }
}
