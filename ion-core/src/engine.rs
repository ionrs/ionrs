use std::collections::HashMap;

use crate::error::IonError;
use crate::host_types::{HostEnumDef, HostStructDef, IonType, IonTypeDef};
use crate::interpreter::{Interpreter, Limits};
use crate::lexer::Lexer;
use crate::module::Module;
use crate::parser::Parser;
use crate::value::Value;

/// The public embedding API for the Ion interpreter.
pub struct Engine {
    interpreter: Interpreter,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            interpreter: Interpreter::new(),
        }
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


    /// Get all top-level bindings.
    pub fn get_all(&self) -> HashMap<String, Value> {
        self.interpreter.env.top_level()
    }

    /// Set execution limits.
    pub fn set_limits(&mut self, limits: Limits) {
        self.interpreter.limits = limits;
    }

    /// Register a built-in function.
    pub fn register_fn(&mut self, name: &str, func: fn(&[Value]) -> Result<Value, String>) {
        self.interpreter.env.define(
            name.to_string(),
            Value::BuiltinFn(name.to_string(), func),
            false,
        );
    }

    /// Register a built-in backed by a closure. Unlike `register_fn`,
    /// this accepts any `Fn` — including closures that capture
    /// host-side state such as a `tokio::runtime::Handle`, a database
    /// pool, or shared counters. See `docs/concurrency.md` for the
    /// tokio embedding pattern.
    pub fn register_closure<F>(&mut self, name: &str, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        self.interpreter.env.define(
            name.to_string(),
            Value::BuiltinClosure(
                name.to_string(),
                crate::value::BuiltinClosureFn::new(func),
            ),
            false,
        );
    }

    /// Register a host struct type that scripts can construct and match on.
    pub fn register_struct(&mut self, def: HostStructDef) {
        self.interpreter.types.register_struct(def);
    }

    /// Register a host enum type that scripts can construct and match on.
    pub fn register_enum(&mut self, def: HostEnumDef) {
        self.interpreter.types.register_enum(def);
    }

    /// Register a module that scripts can access via `module::name` or `use module::*`.
    pub fn register_module(&mut self, module: Module) {
        let name = module.name.clone();
        let value = module.to_value();
        self.interpreter.env.define(name, value, false);
    }

    /// Register a type via the IonType trait (used with `#[derive(IonType)]`).
    pub fn register_type<T: IonType>(&mut self) {
        match T::ion_type_def() {
            IonTypeDef::Struct(def) => self.interpreter.types.register_struct(def),
            IonTypeDef::Enum(def) => self.interpreter.types.register_enum(def),
        }
    }

    /// Inject a typed Rust value into the script scope.
    pub fn set_typed<T: IonType>(&mut self, name: &str, value: &T) {
        self.interpreter
            .env
            .define(name.to_string(), value.to_ion(), false);
    }

    /// Extract a typed Rust value from the script scope.
    pub fn get_typed<T: IonType>(&self, name: &str) -> Result<T, String> {
        let val = self
            .interpreter
            .env
            .get(name)
            .ok_or_else(|| format!("{}{}{}", ion_str!("variable '"), name, ion_str!("' not found")))?;
        T::from_ion(val)
    }

    /// Evaluate a script via the bytecode VM. Falls back to tree-walk for
    /// unsupported features (concurrency).
    #[cfg(feature = "vm")]
    pub fn vm_eval(&mut self, source: &str) -> Result<Value, IonError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program()?;

        // Try the bytecode path first
        let compiler = crate::compiler::Compiler::new();
        match compiler.compile_program(&program) {
            Ok((chunk, fn_chunks)) => {
                let mut vm = crate::vm::Vm::with_env(std::mem::take(&mut self.interpreter.env));
                // Pre-populate the VM's function cache with compiled chunks
                vm.preload_fn_chunks(fn_chunks);
                // Pass host type registry to VM
                vm.set_types(self.interpreter.types.clone());
                // Register builtins in VM env
                crate::interpreter::register_builtins(vm.env_mut());
                let result = vm.execute(&chunk);
                // Restore env back to interpreter
                self.interpreter.env = std::mem::take(vm.env_mut());
                result
            }
            Err(_) => {
                // Compilation failed (unsupported feature) — fall back to tree-walk
                self.interpreter.eval_program(&program)
            }
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
