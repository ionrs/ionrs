//! Host-registered modules and their runtime representation.
//!
//! `Module` is the builder API; `ModuleTable` is the immutable value embedded
//! in `Value::Module(Arc<ModuleTable>)`. Both are keyed by FNV-1a 64-bit
//! name hashes — no identifier strings touch the binary. See
//! `HIDE_NAMES_PLAN.md` for the design.

use indexmap::IndexMap;
use std::sync::Arc;

#[cfg(feature = "async-runtime")]
use std::future::Future;

#[cfg(feature = "async-runtime")]
use crate::error::IonError;
use crate::hash::mix;
#[cfg(feature = "async-runtime")]
use crate::value::AsyncBuiltinClosureFn;
use crate::value::{BuiltinClosureFn, BuiltinFn, Value};

fn register_qualified_name(qualified_hash: u64, module_hash: u64, name_hash: u64) {
    if crate::names::lookup(qualified_hash).is_some() {
        return;
    }
    let (Some(module_name), Some(item_name)) = (
        crate::names::lookup(module_hash),
        crate::names::lookup(name_hash),
    ) else {
        return;
    };
    let joined: &'static str =
        Box::leak(format!("{}::{}", module_name, item_name).into_boxed_str());
    crate::names::register(qualified_hash, joined);
}

/// Frozen, hash-keyed module table embedded in `Value::Module`.
///
/// `items` holds functions, constants, and submodules indexed by their
/// (unqualified) name hash. Functions are stored as `Value::BuiltinFn`/
/// `BuiltinClosure` whose `qualified_hash` was precomputed at registration
/// as `mix(module_hash, fn_name_hash)`.
#[derive(Debug)]
pub struct ModuleTable {
    pub name_hash: u64,
    /// Insertion-ordered map of name_hash → value.
    pub items: IndexMap<u64, Value>,
}

/// Builder for a host-registered module. Call `.into_value()` to freeze
/// it into a `Value::Module(Arc<ModuleTable>)`.
pub struct Module {
    name_hash: u64,
    items: IndexMap<u64, Value>,
}

impl Module {
    /// Create a new empty module identified by its precomputed name hash.
    /// Use the `h!()` macro at the call site so the source identifier is
    /// hashed at compile time.
    pub fn new(name_hash: u64) -> Self {
        Self {
            name_hash,
            items: IndexMap::new(),
        }
    }

    /// Module's own name hash (e.g. `h!("math")`).
    pub fn name_hash(&self) -> u64 {
        self.name_hash
    }

    /// Register a `fn`-pointer builtin under `name_hash`. The
    /// `qualified_hash` stored on the resulting `Value::BuiltinFn` is
    /// `mix(module_hash, name_hash)`.
    pub fn register_fn(&mut self, name_hash: u64, func: BuiltinFn) {
        let qualified_hash = mix(self.name_hash, name_hash);
        register_qualified_name(qualified_hash, self.name_hash, name_hash);
        let prev = self.items.insert(
            name_hash,
            Value::BuiltinFn {
                qualified_hash,
                func,
            },
        );
        assert!(
            prev.is_none(),
            "duplicate or colliding name in module #{:016x}",
            self.name_hash
        );
    }

    /// Register a closure-backed builtin (captures host-side state).
    pub fn register_closure<F>(&mut self, name_hash: u64, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        let qualified_hash = mix(self.name_hash, name_hash);
        register_qualified_name(qualified_hash, self.name_hash, name_hash);
        let prev = self.items.insert(
            name_hash,
            Value::BuiltinClosure {
                qualified_hash,
                func: BuiltinClosureFn::new(func),
            },
        );
        assert!(
            prev.is_none(),
            "duplicate or colliding name in module #{:016x}",
            self.name_hash
        );
    }

    /// Register an async closure-backed builtin. Only callable under the
    /// async runtime; sync `eval` rejects calls explicitly.
    #[cfg(feature = "async-runtime")]
    pub fn register_async_fn<F, Fut>(&mut self, name_hash: u64, func: F)
    where
        F: Fn(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Value, IonError>> + 'static,
    {
        let qualified_hash = mix(self.name_hash, name_hash);
        register_qualified_name(qualified_hash, self.name_hash, name_hash);
        let prev = self.items.insert(
            name_hash,
            Value::AsyncBuiltinClosure {
                qualified_hash,
                func: AsyncBuiltinClosureFn::new(func),
            },
        );
        assert!(
            prev.is_none(),
            "duplicate or colliding name in module #{:016x}",
            self.name_hash
        );
    }

    /// Register a constant value under `name_hash`.
    pub fn set(&mut self, name_hash: u64, value: Value) {
        let prev = self.items.insert(name_hash, value);
        assert!(
            prev.is_none(),
            "duplicate or colliding name in module #{:016x}",
            self.name_hash
        );
    }

    /// Register a submodule. Indexed by the submodule's own `name_hash`.
    pub fn register_submodule(&mut self, sub: Module) {
        let sub_hash = sub.name_hash;
        let prev = self.items.insert(sub_hash, sub.into_value());
        assert!(
            prev.is_none(),
            "duplicate or colliding submodule in module #{:016x}",
            self.name_hash
        );
    }

    /// Freeze the builder into a `Value::Module(Arc<ModuleTable>)`.
    pub fn into_value(self) -> Value {
        Value::Module(Arc::new(ModuleTable {
            name_hash: self.name_hash,
            items: self.items,
        }))
    }

    /// Iterate registered name hashes (used by `use mod::*` glob imports).
    pub fn name_hashes(&self) -> Vec<u64> {
        self.items.keys().copied().collect()
    }
}
