#![allow(clippy::result_large_err)]
//! Ion — an embeddable scripting language for Rust.
//!
//! Ion is a small, strongly-typed scripting language inspired by Starlark,
//! designed for embedding in Rust applications. It features a tree-walk
//! interpreter and an optional bytecode VM for better performance.
//!
//! # Quick Start
//!
//! ```rust
//! use ion_core::engine::Engine;
//!
//! let mut engine = Engine::new();
//! let result = engine.eval("1 + 2").unwrap();
//! assert_eq!(result, ion_core::value::Value::Int(3));
//! ```
//!
//! # Features
//!
//! - **`vm`** (default) — Bytecode compiler and stack-based VM
//! - **`optimize`** (default) — Peephole optimizer, constant folding,
//!   dead-code elimination, tail-call optimization
//! - **`derive`** (default) — `#[derive(IonType)]` for host type injection
//! - **`concurrency`** — Structured concurrency: `async`/`spawn`/`.await`/
//!   `select`/`channel`, cooperative cancellation, tokio-friendly
//!   embedding via [`engine::Engine::register_closure`]
//! - **`msgpack`** — `Value::to_msgpack()` / `from_msgpack()` via `rmpv`
//! - **`obfuscate`** — String obfuscation via `obfstr`
//! - **`rewrite`** — Source rewriter at [`rewrite::replace_global`]

/// Macro for string obfuscation. Returns a `String`.
/// When the `obfuscate` feature is enabled, strings are encrypted at compile
/// time and decrypted at runtime via `obfstr`. Without the feature, they
/// pass through as regular `String`s.
#[cfg(feature = "obfuscate")]
macro_rules! ion_str {
    ($s:literal) => {{
        let _tmp: String = obfstr::obfstr!($s).to_string();
        _tmp
    }};
}

#[cfg(not(feature = "obfuscate"))]
macro_rules! ion_str {
    ($s:literal) => {
        String::from($s)
    };
}

/// Same as `ion_str!` but returns `&str` (non-obfuscated in obfuscate mode
/// for contexts requiring `&'static str` like type_name()).
/// These strings are short type names that are low-value for obfuscation.
macro_rules! ion_static_str {
    ($s:literal) => {
        $s
    };
}

pub mod ast;
#[cfg(feature = "concurrency")]
pub mod async_rt;
#[cfg(feature = "concurrency")]
pub mod async_rt_std;
#[cfg(feature = "async-runtime")]
pub mod async_runtime;
#[cfg(feature = "vm")]
pub mod bytecode;
#[cfg(feature = "vm")]
pub mod compiler;
pub mod engine;
pub mod env;
pub mod error;
pub mod host_types;
pub mod intern;
pub mod interpreter;
pub mod lexer;
pub mod module;
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
