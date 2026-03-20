use std::collections::HashMap;

use crate::error::IonError;
use crate::interpreter::Interpreter;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::value::Value;

/// The public embedding API for the Ion interpreter.
pub struct Engine {
    interpreter: Interpreter,
}

impl Engine {
    pub fn new() -> Self {
        Self { interpreter: Interpreter::new() }
    }

    /// Evaluate a script, returning the last expression's value.
    pub fn eval(&mut self, source: &str) -> Result<Value, IonError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program()?;
        self.interpreter.eval_program(&program)
    }

    /// Inject a value into the script scope.
    pub fn set(&mut self, name: &str, value: Value) {
        self.interpreter.env.define(name.to_string(), value, false);
    }

    /// Read a variable from the script scope.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.interpreter.env.get(name).cloned()
    }

    /// Try to get a variable, returning None if it doesn't exist.
    pub fn try_get(&self, name: &str) -> Option<Value> {
        self.interpreter.env.get(name).cloned()
    }

    /// Get all top-level bindings.
    pub fn get_all(&self) -> HashMap<String, Value> {
        self.interpreter.env.top_level()
    }

    /// Register a built-in function.
    pub fn register_fn(&mut self, name: &str, func: fn(&[Value]) -> Result<Value, String>) {
        self.interpreter.env.define(
            name.to_string(),
            Value::BuiltinFn(name.to_string(), func),
            false,
        );
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
