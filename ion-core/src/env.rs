use std::collections::HashMap;
use crate::value::Value;

/// Variable binding with mutability tracking.
#[derive(Debug, Clone)]
struct Binding {
    value: Value,
    mutable: bool,
}

/// Lexical scope environment using a stack of frames.
#[derive(Debug, Clone)]
pub struct Env {
    frames: Vec<HashMap<String, Binding>>,
}

impl Env {
    pub fn new() -> Self {
        Self { frames: vec![HashMap::new()] }
    }

    /// Push a new scope frame.
    pub fn push_scope(&mut self) {
        self.frames.push(HashMap::new());
    }

    /// Pop the current scope frame.
    pub fn pop_scope(&mut self) {
        self.frames.pop();
    }

    /// Define a new variable in the current scope.
    pub fn define(&mut self, name: String, value: Value, mutable: bool) {
        let frame = self.frames.last_mut().unwrap();
        frame.insert(name, Binding { value, mutable });
    }

    /// Get a variable's value by name, searching from innermost scope outward.
    pub fn get(&self, name: &str) -> Option<&Value> {
        for frame in self.frames.iter().rev() {
            if let Some(binding) = frame.get(name) {
                return Some(&binding.value);
            }
        }
        None
    }

    /// Set an existing variable's value. Returns error if not found or not mutable.
    pub fn set(&mut self, name: &str, value: Value) -> Result<(), String> {
        for frame in self.frames.iter_mut().rev() {
            if let Some(binding) = frame.get_mut(name) {
                if !binding.mutable {
                    return Err(format!(
                        "{}'{}'",
                        ion_str!("cannot assign to immutable variable "),
                        name,
                    ));
                }
                binding.value = value;
                return Ok(());
            }
        }
        Err(format!("{}{}", ion_str!("undefined variable: "), name))
    }

    /// Get all top-level bindings (for engine.get_all()).
    pub fn top_level(&self) -> HashMap<String, Value> {
        self.frames.first().map(|f| {
            f.iter().map(|(k, b)| (k.clone(), b.value.clone())).collect()
        }).unwrap_or_default()
    }

    /// Snapshot current environment for closure capture.
    pub fn capture(&self) -> HashMap<String, Value> {
        let mut captured = HashMap::new();
        for frame in &self.frames {
            for (k, b) in frame {
                captured.insert(k.clone(), b.value.clone());
            }
        }
        captured
    }
}
