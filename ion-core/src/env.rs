use crate::hash::h;
use crate::intern::{StringPool, Symbol};
use crate::value::Value;
use std::collections::HashMap;

/// Variable binding with mutability tracking.
#[derive(Debug, Clone)]
struct Binding {
    sym: Symbol,
    value: Value,
    mutable: bool,
}

/// Lexical scope environment using a stack of frames.
///
/// Each frame is a Vec of bindings, scanned linearly. For typical scope sizes
/// (<20 variables), linear scan with Symbol (u32) comparison is faster than
/// HashMap due to cache locality.
///
/// `globals_h` is a parallel, hash-keyed top-level table for host-registered
/// names (modules, builtin functions). Lookups by `&str` hash on the fly and
/// fall back to this table when a binding isn't found in any scope frame.
/// This keeps host names entirely outside the `StringPool` so the source
/// identifier never lives at runtime — see HIDE_NAMES_PLAN.md.
#[derive(Debug, Clone)]
pub struct Env {
    frames: Vec<Vec<Binding>>,
    pool: StringPool,
    globals_h: HashMap<u64, Value>,
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

impl Env {
    pub fn new() -> Self {
        Self {
            frames: vec![Vec::new()],
            pool: StringPool::new(),
            globals_h: HashMap::new(),
        }
    }

    /// Define a host-registered binding by hash. Used for modules, builtin
    /// functions, and any name the host registers without exposing the
    /// source string at runtime.
    pub fn define_h(&mut self, name_hash: u64, value: Value) {
        self.globals_h.insert(name_hash, value);
    }

    /// Look up a host-registered binding by hash directly. Bypasses all
    /// scope frames and the string pool.
    pub fn get_h(&self, name_hash: u64) -> Option<&Value> {
        self.globals_h.get(&name_hash)
    }

    /// Iterate every host-registered binding (used by closure capture and
    /// for `use mod::*` over `Value::Module`).
    pub fn globals_h(&self) -> impl Iterator<Item = (u64, &Value)> {
        self.globals_h.iter().map(|(k, v)| (*k, v))
    }

    /// Get a reference to the string pool.
    pub fn pool(&self) -> &StringPool {
        &self.pool
    }

    /// Get a mutable reference to the string pool.
    pub fn pool_mut(&mut self) -> &mut StringPool {
        &mut self.pool
    }

    /// Intern a string and return its symbol.
    pub fn intern(&mut self, s: &str) -> Symbol {
        self.pool.intern(s)
    }

    /// Resolve a symbol to its string.
    pub fn resolve(&self, sym: Symbol) -> &str {
        self.pool.resolve(sym)
    }

    /// Push a new scope frame.
    pub fn push_scope(&mut self) {
        self.frames.push(Vec::new());
    }

    /// Pop the current scope frame.
    pub fn pop_scope(&mut self) {
        self.frames.pop();
    }

    /// Define a new variable in the current scope.
    pub fn define(&mut self, name: String, value: Value, mutable: bool) {
        let sym = self.pool.intern(&name);
        self.define_sym(sym, value, mutable);
    }

    /// Define a variable by symbol in the current scope.
    pub fn define_sym(&mut self, sym: Symbol, value: Value, mutable: bool) {
        let frame = self.frames.last_mut().unwrap();
        // Check if already defined in this scope (overwrite)
        for binding in frame.iter_mut() {
            if binding.sym == sym {
                binding.value = value;
                binding.mutable = mutable;
                return;
            }
        }
        frame.push(Binding {
            sym,
            value,
            mutable,
        });
    }

    /// Get a variable's value by name, searching from innermost scope outward.
    /// Falls back to host-registered globals (`globals_h`) if no scope frame
    /// matches — that table is the source of truth for module names and
    /// builtin functions whose identifiers were never interned.
    pub fn get(&self, name: &str) -> Option<&Value> {
        if let Some(sym) = self.pool.map_get(name) {
            if let Some(v) = self.get_sym(sym) {
                return Some(v);
            }
        }
        self.globals_h.get(&h(name))
    }

    /// Get a variable's value by symbol.
    pub fn get_sym(&self, sym: Symbol) -> Option<&Value> {
        for frame in self.frames.iter().rev() {
            for binding in frame.iter().rev() {
                if binding.sym == sym {
                    return Some(&binding.value);
                }
            }
        }
        None
    }

    /// Like [`Env::get_sym`] but falls back to the host-registered hash table
    /// when the symbol is not bound in any scope frame. Used by the VM's
    /// `Op::GetGlobal` so script-side lookups of `len`, `range`, `set`, etc.
    /// resolve to the builtins registered via `define_h` / `Engine::register_fn`.
    pub fn get_sym_or_global(&self, sym: Symbol) -> Option<&Value> {
        if let Some(v) = self.get_sym(sym) {
            return Some(v);
        }
        let name = self.pool.resolve(sym);
        self.globals_h.get(&h(name))
    }

    /// Set an existing variable's value. Returns error if not found or not mutable.
    pub fn set(&mut self, name: &str, value: Value) -> Result<(), String> {
        let sym = self.pool.intern(name);
        self.set_sym(sym, value)
    }

    /// Set a variable by symbol.
    pub fn set_sym(&mut self, sym: Symbol, value: Value) -> Result<(), String> {
        for frame in self.frames.iter_mut().rev() {
            for binding in frame.iter_mut().rev() {
                if binding.sym == sym {
                    if !binding.mutable {
                        return Err(format!(
                            "{}'{}'",
                            ion_str!("cannot assign to immutable variable "),
                            self.pool.resolve(sym),
                        ));
                    }
                    binding.value = value;
                    return Ok(());
                }
            }
        }
        Err(format!(
            "{}{}",
            ion_str!("undefined variable: "),
            self.pool.resolve(sym)
        ))
    }

    /// Get all top-level bindings (for engine.get_all()).
    pub fn top_level(&self) -> HashMap<String, Value> {
        self.frames
            .first()
            .map(|f| {
                f.iter()
                    .map(|b| (self.pool.resolve(b.sym).to_string(), b.value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Snapshot current environment for closure capture.
    /// Host-registered globals (`globals_h`) are intentionally not captured —
    /// they're shared across the whole engine and don't need cloning into
    /// each closure's environment.
    pub fn capture(&self) -> HashMap<String, Value> {
        let mut captured = HashMap::new();
        for frame in &self.frames {
            for b in frame {
                captured.insert(self.pool.resolve(b.sym).to_string(), b.value.clone());
            }
        }
        captured
    }
}
