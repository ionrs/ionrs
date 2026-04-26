use indexmap::IndexMap;

use crate::value::{BuiltinClosureFn, BuiltinFn, Value};

enum ModuleFn {
    Function(BuiltinFn),
    Closure(BuiltinClosureFn),
}

/// A named collection of functions and values that can be registered
/// with an Engine and accessed via `module::name` syntax in Ion scripts.
pub struct Module {
    pub name: String,
    functions: IndexMap<String, (String, ModuleFn)>,
    values: IndexMap<String, Value>,
    submodules: IndexMap<String, Module>,
}

impl Module {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            functions: IndexMap::new(),
            values: IndexMap::new(),
            submodules: IndexMap::new(),
        }
    }

    /// Register a builtin function in this module.
    pub fn register_fn(&mut self, name: &str, func: BuiltinFn) {
        let qualified = format!("{}::{}", self.name, name);
        self.functions
            .insert(name.to_string(), (qualified, ModuleFn::Function(func)));
    }

    /// Register a closure-backed builtin function in this module.
    pub fn register_closure<F>(&mut self, name: &str, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        let qualified = format!("{}::{}", self.name, name);
        self.functions.insert(
            name.to_string(),
            (qualified, ModuleFn::Closure(BuiltinClosureFn::new(func))),
        );
    }

    /// Register a constant value in this module.
    pub fn set(&mut self, name: &str, value: Value) {
        self.values.insert(name.to_string(), value);
    }

    /// Register a submodule (e.g., `net::http`).
    pub fn register_submodule(&mut self, sub: Module) {
        self.submodules.insert(sub.name.clone(), sub);
    }

    /// Convert this module into a `Value::Dict` for use in the interpreter env.
    /// Functions become `Value::BuiltinFn`, submodules become nested dicts.
    pub fn to_value(&self) -> Value {
        let mut map = IndexMap::new();

        for (name, (qualified, func)) in &self.functions {
            let value = match func {
                ModuleFn::Function(func) => Value::BuiltinFn(qualified.clone(), *func),
                ModuleFn::Closure(func) => Value::BuiltinClosure(qualified.clone(), func.clone()),
            };
            map.insert(name.clone(), value);
        }

        for (name, value) in &self.values {
            map.insert(name.clone(), value.clone());
        }

        for (name, sub) in &self.submodules {
            map.insert(name.clone(), sub.to_value());
        }

        Value::Dict(map)
    }

    /// Get all exported names (for `use mod::*`).
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.functions.keys().cloned().collect();
        names.extend(self.values.keys().cloned());
        names.extend(self.submodules.keys().cloned());
        names
    }
}
