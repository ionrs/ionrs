#![allow(clippy::result_large_err)]

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
pub mod parser;
pub mod token;
pub mod value;
#[cfg(feature = "vm")]
pub mod vm;

#[cfg(feature = "derive")]
pub use ion_derive::IonType;
