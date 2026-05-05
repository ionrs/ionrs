use std::collections::HashMap;
#[cfg(feature = "async-runtime")]
use std::future::Future;
use std::sync::Arc;

use crate::error::IonError;
use crate::host_types::{HostEnumDef, HostStructDef, IonType, IonTypeDef};
use crate::interpreter::{Interpreter, Limits};
use crate::lexer::Lexer;
use crate::log::{AtomicLogLevel, LogHandler, LogLevel, StdLogHandler};
use crate::module::Module;
use crate::parser::Parser;
use crate::stdlib::OutputHandler;
use crate::value::{HostSignature, Value};

/// The public embedding API for the Ion interpreter.
pub struct Engine {
    interpreter: Interpreter,
    output: Arc<dyn OutputHandler>,
    log_handler: Arc<dyn LogHandler>,
    log_level: Arc<AtomicLogLevel>,
    script_args: Arc<Vec<String>>,
    #[cfg(feature = "async-runtime")]
    external_queue: crate::async_runtime::ExternalQueue,
}

impl Engine {
    pub fn new() -> Self {
        Self::with_output_handler(crate::stdlib::missing_output_handler())
    }

    /// Create an engine with a host-provided output handler for `io::print*`.
    pub fn with_output<H>(output: H) -> Self
    where
        H: OutputHandler + 'static,
    {
        Self::with_output_handler(Arc::new(output))
    }

    /// Create an engine with a shared host-provided output handler. Uses the
    /// default [`StdLogHandler`] wired to the engine-wide log level (so
    /// `log::set_level` works as expected).
    pub fn with_output_handler(output: Arc<dyn OutputHandler>) -> Self {
        let log_level = AtomicLogLevel::default_runtime();
        let log_handler: Arc<dyn LogHandler> =
            Arc::new(StdLogHandler::with_threshold(Arc::clone(&log_level)));
        Self::build(output, log_handler, log_level)
    }

    /// Create an engine with both a host output handler and a log handler.
    /// The log handler manages its own filtering — `log::set_level` still
    /// updates the engine-wide threshold but the supplied handler can ignore
    /// it (e.g. [`crate::log::TracingLogHandler`] defers to `tracing`).
    pub fn with_handlers(output: Arc<dyn OutputHandler>, log_handler: Arc<dyn LogHandler>) -> Self {
        Self::build(output, log_handler, AtomicLogLevel::default_runtime())
    }

    fn build(
        output: Arc<dyn OutputHandler>,
        log_handler: Arc<dyn LogHandler>,
        log_level: Arc<AtomicLogLevel>,
    ) -> Self {
        let interpreter = Interpreter::with_handlers(
            Arc::clone(&output),
            Arc::clone(&log_handler),
            Arc::clone(&log_level),
        );
        Self {
            interpreter,
            output,
            log_handler,
            log_level,
            script_args: Arc::new(Vec::new()),
            #[cfg(feature = "async-runtime")]
            external_queue: crate::async_runtime::ExternalQueue::new(),
        }
    }

    /// Evaluate a script, returning the last expression's value.
    ///
    /// Removed when the `async-runtime` feature is enabled — async builds
    /// must use [`Engine::eval_async`]. The two runtimes are deliberately
    /// mutually exclusive so non-coloured stdlib functions (`fs::*`,
    /// `io::print*`) have one implementation per build.
    #[cfg(not(feature = "async-runtime"))]
    pub fn eval(&mut self, source: &str) -> Result<Value, IonError> {
        self.eval_sync_internal(source)
    }

    /// Crate-private sync evaluator. Used by both the public [`Engine::eval`]
    /// (in sync builds) and by the async runtime's pure-sync fallback for
    /// programs that don't touch async host functions.
    pub(crate) fn eval_sync_internal(&mut self, source: &str) -> Result<Value, IonError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program()?;
        self.interpreter.eval_program(&program)
    }

    /// Evaluate a script through the async-runtime entry point.
    ///
    /// Pure synchronous scripts execute through the existing evaluator.
    /// Compilable scripts that reference async host functions run through
    /// the pollable bytecode continuation runtime.
    #[cfg(feature = "async-runtime")]
    pub fn eval_async<'a>(
        &'a mut self,
        source: &'a str,
    ) -> crate::async_runtime::IonEvalFuture<'a> {
        crate::async_runtime::IonEvalFuture::new(self, source)
    }

    /// Return a cloneable handle that host async code can use to schedule
    /// callbacks into the async runtime.
    #[cfg(feature = "async-runtime")]
    pub fn handle(&self) -> crate::async_runtime::EngineHandle {
        self.external_queue.handle()
    }

    #[cfg(feature = "async-runtime")]
    #[allow(dead_code)]
    pub(crate) fn external_queue(&self) -> crate::async_runtime::ExternalQueue {
        self.external_queue.clone()
    }

    #[cfg(feature = "async-runtime")]
    pub(crate) fn interpreter_mut(&mut self) -> &mut Interpreter {
        &mut self.interpreter
    }

    #[cfg(feature = "async-runtime")]
    pub(crate) fn interpreter(&self) -> &Interpreter {
        &self.interpreter
    }

    #[cfg(feature = "async-runtime")]
    pub(crate) fn output_handler(&self) -> Arc<dyn OutputHandler> {
        Arc::clone(&self.output)
    }

    /// Drain externally scheduled requests.
    ///
    /// This is exposed while the async runtime is scaffolded so tests and
    /// future runtime code can validate the non-reentrant handle path.
    #[cfg(feature = "async-runtime")]
    #[doc(hidden)]
    pub fn drain_external_requests(&self) -> Vec<crate::async_runtime::ExternalRequest> {
        self.external_queue.drain()
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

    /// Set the host output handler used by `io::print`, `io::println`, and
    /// `io::eprintln`.
    pub fn set_output<H>(&mut self, output: H)
    where
        H: OutputHandler + 'static,
    {
        self.set_output_handler(Arc::new(output));
    }

    /// Set a shared host output handler used by `io::print*`.
    pub fn set_output_handler(&mut self, output: Arc<dyn OutputHandler>) {
        self.output = Arc::clone(&output);
        let io = crate::stdlib::io_module_with_output(output);
        let h = io.name_hash();
        self.interpreter.env.define_h(h, io.into_value());
    }

    /// Inject script-level arguments reachable from Ion as `os::args()`.
    /// Re-registers the `os::` module so subsequent `os::args()` calls
    /// see the new list. The default (when this is never called) is `[]`.
    pub fn set_args(&mut self, args: Vec<String>) {
        self.script_args = Arc::new(args);
        #[cfg(feature = "os")]
        {
            let os = crate::stdlib::os_module_with_args(Arc::clone(&self.script_args));
            let h = os.name_hash();
            self.interpreter.env.define_h(h, os.into_value());
        }
    }

    /// Builder counterpart of [`Engine::set_args`].
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.set_args(args);
        self
    }

    /// Currently configured script args (what `os::args()` returns).
    pub fn args(&self) -> &[String] {
        &self.script_args
    }

    /// Install a host log handler. Surviving `log::*` callsites dispatch into
    /// this handler. The shared runtime threshold (controlled by
    /// `log::set_level` / [`Engine::set_log_level`]) is preserved.
    pub fn set_log_handler<H>(&mut self, handler: H)
    where
        H: LogHandler + 'static,
    {
        self.set_log_handler_arc(Arc::new(handler));
    }

    /// Install a shared host log handler.
    pub fn set_log_handler_arc(&mut self, handler: Arc<dyn LogHandler>) {
        self.log_handler = Arc::clone(&handler);
        let log = crate::stdlib::log_module_with_handler(handler, Arc::clone(&self.log_level));
        let h = log.name_hash();
        self.interpreter.env.define_h(h, log.into_value());
    }

    /// Set the runtime log level used by the default `StdLogHandler`.
    /// Custom handlers may consult this via [`Engine::log_level`] but are
    /// not required to.
    pub fn set_log_level(&mut self, level: LogLevel) {
        self.log_level.set(level);
    }

    pub fn log_level(&self) -> Arc<AtomicLogLevel> {
        Arc::clone(&self.log_level)
    }

    /// Register a top-level builtin function under `name_hash`. Use the
    /// `h!()` macro at the call site so the source identifier is hashed at
    /// compile time and never appears in the binary.
    pub fn register_fn(&mut self, name_hash: u64, func: fn(&[Value]) -> Result<Value, String>) {
        self.interpreter.env.define_h(
            name_hash,
            Value::BuiltinFn {
                qualified_hash: name_hash,
                func,
                signature: None,
            },
        );
    }

    pub fn register_fn_sig(
        &mut self,
        name_hash: u64,
        signature: HostSignature,
        func: fn(&[Value]) -> Result<Value, String>,
    ) {
        self.interpreter.env.define_h(
            name_hash,
            Value::BuiltinFn {
                qualified_hash: name_hash,
                func,
                signature: Some(Arc::new(signature)),
            },
        );
    }

    /// Register a closure-backed top-level builtin. Captures host-side
    /// state — `tokio::runtime::Handle`, DB pool, counters, etc.
    pub fn register_closure<F>(&mut self, name_hash: u64, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        self.interpreter.env.define_h(
            name_hash,
            Value::BuiltinClosure {
                qualified_hash: name_hash,
                func: crate::value::BuiltinClosureFn::new(func),
                signature: None,
            },
        );
    }

    pub fn register_closure_sig<F>(&mut self, name_hash: u64, signature: HostSignature, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        self.interpreter.env.define_h(
            name_hash,
            Value::BuiltinClosure {
                qualified_hash: name_hash,
                func: crate::value::BuiltinClosureFn::new(func),
                signature: Some(Arc::new(signature)),
            },
        );
    }

    /// Register a top-level async builtin. Callable only under `eval_async`.
    #[cfg(feature = "async-runtime")]
    pub fn register_async_fn<F, Fut>(&mut self, name_hash: u64, func: F)
    where
        F: Fn(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Value, IonError>> + 'static,
    {
        self.interpreter.env.define_h(
            name_hash,
            Value::AsyncBuiltinClosure {
                qualified_hash: name_hash,
                func: crate::value::AsyncBuiltinClosureFn::new(func),
                signature: None,
            },
        );
    }

    #[cfg(feature = "async-runtime")]
    pub fn register_async_fn_sig<F, Fut>(
        &mut self,
        name_hash: u64,
        signature: HostSignature,
        func: F,
    ) where
        F: Fn(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Value, IonError>> + 'static,
    {
        self.interpreter.env.define_h(
            name_hash,
            Value::AsyncBuiltinClosure {
                qualified_hash: name_hash,
                func: crate::value::AsyncBuiltinClosureFn::new(func),
                signature: Some(Arc::new(signature)),
            },
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

    /// Register a module that scripts can access via `module::name` or
    /// `use module::*`. Stored under the module's `name_hash`.
    pub fn register_module(&mut self, module: Module) {
        let name_hash = module.name_hash();
        let value = module.into_value();
        self.interpreter.env.define_h(name_hash, value);
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
            .ok_or_else(|| ion_format!("variable '{}' not found", name))?;
        T::from_ion(val)
    }

    /// Evaluate a script via the bytecode VM. Falls back to tree-walk for
    /// unsupported features (concurrency).
    ///
    /// Removed under `async-runtime` (see [`Engine::eval`]).
    #[cfg(all(feature = "vm", not(feature = "async-runtime")))]
    pub fn vm_eval(&mut self, source: &str) -> Result<Value, IonError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program()?;

        // Try the bytecode path first
        let compiler = crate::compiler::Compiler::new();
        match compiler.compile_program(&program) {
            Ok((chunk, fn_chunks)) => {
                let mut vm = crate::vm::Vm::with_env_and_output(
                    std::mem::take(&mut self.interpreter.env),
                    Arc::clone(&self.output),
                );
                // Pre-populate the VM's function cache with compiled chunks
                vm.preload_fn_chunks(fn_chunks);
                // Pass host type registry to VM
                vm.set_types(self.interpreter.types.clone());
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
