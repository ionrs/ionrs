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
#[derive(Debug, Clone)]
pub struct Env {
    frames: Vec<Vec<Binding>>,
    pool: StringPool,
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
        }
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
    pub fn get(&self, name: &str) -> Option<&Value> {
        let sym = self.pool.map_get(name)?;
        self.get_sym(sym)
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
