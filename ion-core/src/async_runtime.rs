//! Pollable async runtime.
//!
//! Pure synchronous scripts still use the existing evaluator. Compilable
//! scripts that reference async host functions run through the pollable
//! bytecode continuation runtime. Unsupported async-host programs return
//! compiler errors instead of falling back to a second interpreter.

use std::future::Future;
use std::pin::Pin;
use std::task::Waker;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex},
};

use indexmap::IndexMap;

use crate::ast::{
    BinOp, DictEntry, Expr, ExprKind, FStrPart, ListEntry, Pattern, Program, Span, Stmt, StmtKind,
    UnaryOp,
};
use crate::bytecode::{Chunk, Op};
use crate::compiler::Compiler;
use crate::engine::Engine;
use crate::env::Env;
use crate::error::{ErrorKind, IonError};
use crate::host_types::TypeRegistry;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::stdlib::{missing_output_handler, OutputHandler, OutputStream};
use crate::value::{BoxIonFuture, FnChunkCache, Value};

/// Request submitted from host async code back into the Ion runtime.
pub enum ExternalRequest {
    Call {
        fn_name: String,
        args: Vec<Value>,
        result_tx: tokio::sync::oneshot::Sender<Result<Value, IonError>>,
    },
}

/// Shared external queue used by `EngineHandle`.
#[derive(Clone, Default)]
pub struct ExternalQueue {
    inner: Rc<RefCell<ExternalQueueInner>>,
}

#[derive(Default)]
struct ExternalQueueInner {
    requests: VecDeque<ExternalRequest>,
    waker: Option<Waker>,
}

impl ExternalQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&self) -> EngineHandle {
        EngineHandle {
            queue: self.clone(),
        }
    }

    pub fn register_waker(&self, waker: &Waker) {
        self.inner.borrow_mut().waker = Some(waker.clone());
    }

    pub fn push(&self, request: ExternalRequest) {
        let waker = {
            let mut inner = self.inner.borrow_mut();
            inner.requests.push_back(request);
            inner.waker.clone()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub fn drain(&self) -> Vec<ExternalRequest> {
        self.inner.borrow_mut().requests.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Cloneable handle that host futures can use to schedule Ion work.
#[derive(Clone)]
pub struct EngineHandle {
    queue: ExternalQueue,
}

impl EngineHandle {
    pub fn call_async(&self, fn_name: &str, args: Vec<Value>) -> EngineCallFuture {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        self.queue.push(ExternalRequest::Call {
            fn_name: fn_name.to_string(),
            args,
            result_tx,
        });
        EngineCallFuture { result_rx }
    }
}

/// Future resolved when an externally scheduled Ion call completes.
pub struct EngineCallFuture {
    result_rx: tokio::sync::oneshot::Receiver<Result<Value, IonError>>,
}

impl Future for EngineCallFuture {
    type Output = Result<Value, IonError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.result_rx).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(IonError::runtime(
                "scheduled Ion call was cancelled".to_string(),
                0,
                0,
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct ExternalCallTask {
    future: BoxIonFuture,
    result_tx: Option<tokio::sync::oneshot::Sender<Result<Value, IonError>>>,
}

/// Temporary task identifier used by the async-runtime scaffolding.
///
/// The final runtime may replace this with a slotmap key, but host
/// futures already need a stable waiter handle for cancellation tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

/// Stable identifier for a pending host future.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FutureId {
    slot: usize,
    generation: u64,
}

/// Local async Ion task used by the `eval_async` bridge.
#[derive(Clone)]
pub struct AsyncTask {
    inner: Rc<RefCell<AsyncTaskInner>>,
}

struct AsyncTaskInner {
    future: Option<BoxIonFuture>,
    result: Option<Result<Value, IonError>>,
}

impl std::fmt::Debug for AsyncTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncTask").finish_non_exhaustive()
    }
}

impl AsyncTask {
    fn new(future: BoxIonFuture) -> Self {
        Self {
            inner: Rc::new(RefCell::new(AsyncTaskInner {
                future: Some(future),
                result: None,
            })),
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    fn poll_result(&self, cx: &mut Context<'_>) -> Poll<Result<Value, IonError>> {
        let mut inner = self.inner.borrow_mut();
        if let Some(result) = &inner.result {
            return Poll::Ready(result.clone());
        }

        let Some(future) = inner.future.as_mut() else {
            return Poll::Ready(Err(IonError::runtime(
                "async task completed without a result",
                0,
                0,
            )));
        };

        match future.as_mut().poll(cx) {
            Poll::Ready(result) => {
                inner.future = None;
                inner.result = Some(result.clone());
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Native async-runtime channel sender endpoint.
#[derive(Clone)]
pub struct NativeChannelSender {
    inner: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Value>>>>,
}

impl std::fmt::Debug for NativeChannelSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeChannelSender")
            .finish_non_exhaustive()
    }
}

impl NativeChannelSender {
    fn new(sender: tokio::sync::mpsc::Sender<Value>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(sender))),
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn sender(&self) -> Option<tokio::sync::mpsc::Sender<Value>> {
        self.inner.lock().unwrap().clone()
    }

    fn close(&self) {
        *self.inner.lock().unwrap() = None;
    }
}

/// Native async-runtime channel receiver endpoint.
#[derive(Clone)]
pub struct NativeChannelReceiver {
    inner: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Value>>>,
}

impl std::fmt::Debug for NativeChannelReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeChannelReceiver")
            .finish_non_exhaustive()
    }
}

impl NativeChannelReceiver {
    fn new(receiver: tokio::sync::mpsc::Receiver<Value>) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(receiver)),
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// State a lightweight Ion task can be parked in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    WaitingHostFuture(FutureId),
    WaitingTask(TaskId),
    Done,
}

/// Result of executing one VM step or bounded compound operation.
#[derive(Debug, Clone)]
pub enum StepOutcome {
    Continue,
    Yield,
    Suspended(TaskState),
    InstructionError(IonError),
    Done(Result<Value, IonError>),
}

/// Minimal task scaffold used by the budgeted runner.
#[derive(Debug, Clone)]
pub struct IonTask {
    pub state: TaskState,
    pub cancel_requested: bool,
    waiters: Vec<TaskId>,
    result: Option<Result<Value, IonError>>,
    resumed_value: Option<Value>,
    pending_error: Option<IonError>,
}

/// Explicit VM call frame used by the future async VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallFrame {
    pub chunk: ChunkId,
    pub ip: usize,
    pub locals_base: usize,
    captured_scope: bool,
}

/// Minimal continuation-shaped VM state.
pub struct VmContinuation {
    pub frames: Vec<CallFrame>,
    pub stack: Vec<Value>,
    env: Env,
    types: TypeRegistry,
    output: Arc<dyn OutputHandler>,
    spawned_tasks: Vec<AsyncTask>,
    iterators: Vec<Box<dyn Iterator<Item = Value>>>,
    locals: Vec<ContinuationLocalSlot>,
    local_frames: Vec<usize>,
    fn_chunks: HashMap<u64, ChunkId>,
    exception_handlers: Vec<ContinuationExceptionHandler>,
    pending_method: Option<MethodContinuation>,
}

impl Default for VmContinuation {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            stack: Vec::new(),
            env: Env::new(),
            types: TypeRegistry::default(),
            output: missing_output_handler(),
            spawned_tasks: Vec::new(),
            iterators: Vec::new(),
            locals: Vec::new(),
            local_frames: Vec::new(),
            fn_chunks: HashMap::new(),
            exception_handlers: Vec::new(),
            pending_method: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ContinuationLocalSlot {
    value: Value,
    mutable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContinuationExceptionHandler {
    catch_ip: usize,
    stack_depth: usize,
}

struct MethodContinuation {
    parent_frame_depth: usize,
    line: usize,
    col: usize,
    method: MethodContinuationKind,
}

enum MethodContinuationKind {
    Complete(Value),
    SingleCallback {
        func: Value,
        args: Vec<Value>,
        started: bool,
        after: MethodSingleCallbackAfter,
    },
    ListMap {
        func: Value,
        items: Vec<Value>,
        index: usize,
        result: Vec<Value>,
    },
    ListFilter {
        func: Value,
        items: Vec<Value>,
        index: usize,
        result: Vec<Value>,
    },
    ListAny {
        func: Value,
        items: Vec<Value>,
        index: usize,
        found: bool,
    },
    ListAll {
        func: Value,
        items: Vec<Value>,
        index: usize,
        failed: bool,
    },
    ListFlatMap {
        func: Value,
        items: Vec<Value>,
        index: usize,
        result: Vec<Value>,
    },
    ListFold {
        func: Value,
        items: Vec<Value>,
        index: usize,
        acc: Value,
    },
    ListSortBy {
        func: Value,
        items: Vec<Value>,
        pass: usize,
        index: usize,
    },
    DictMap {
        func: Value,
        entries: Vec<(String, Value)>,
        index: usize,
        result: IndexMap<String, Value>,
    },
    DictFilter {
        func: Value,
        entries: Vec<(String, Value)>,
        index: usize,
        result: IndexMap<String, Value>,
    },
}

enum MethodSingleCallbackAfter {
    Direct,
    WrapSome,
    WrapOk,
    WrapErr,
    CellSet(std::sync::Arc<std::sync::Mutex<Value>>),
}

impl VmContinuation {
    pub fn new(root: ChunkId) -> Self {
        Self {
            frames: vec![CallFrame {
                chunk: root,
                ip: 0,
                locals_base: 0,
                captured_scope: false,
            }],
            ..Self::default()
        }
    }

    pub fn with_env(root: ChunkId, env: Env) -> Self {
        Self {
            env,
            ..Self::new(root)
        }
    }

    pub fn take_env(&mut self) -> Env {
        std::mem::take(&mut self.env)
    }

    pub fn register_fn_chunk(&mut self, fn_id: u64, chunk: ChunkId) {
        self.fn_chunks.insert(fn_id, chunk);
    }

    pub fn types_mut(&mut self) -> &mut TypeRegistry {
        &mut self.types
    }

    pub fn set_output_handler(&mut self, output: Arc<dyn OutputHandler>) {
        self.output = output;
    }

    pub fn define_global(&mut self, name: impl Into<String>, value: Value, mutable: bool) {
        self.env.define(name.into(), value, mutable);
    }

    pub fn resume_host_result(&mut self, result: Result<Value, IonError>) -> StepOutcome {
        match result {
            Ok(value) => {
                self.stack.push(value);
                StepOutcome::Continue
            }
            Err(err) => route_continuation_error(self, err),
        }
    }
}

/// Execute one bytecode instruction or bounded compound operation for the
/// async VM continuation runtime.
pub fn step_task(arena: &ChunkArena, cont: &mut VmContinuation) -> StepOutcome {
    step_task_inner(arena, cont, None, None)
}

pub fn step_task_with_host_futures(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    task: TaskId,
    host_futures: &mut HostFutureTable,
) -> StepOutcome {
    step_task_inner(arena, cont, Some(task), Some(host_futures))
}

fn step_task_inner(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    task: Option<TaskId>,
    mut host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    if let Some(outcome) =
        step_pending_method_continuation(arena, cont, task, host_futures.as_deref_mut())
    {
        return outcome;
    }

    let frame_index = cont.frames.len().saturating_sub(1);
    let Some(frame) = cont.frames.last().copied() else {
        return StepOutcome::Done(Ok(cont.stack.pop().unwrap_or(Value::Unit)));
    };

    let Some(chunk) = arena.get(frame.chunk) else {
        return StepOutcome::InstructionError(IonError::runtime("missing chunk", 0, 0));
    };

    if frame.ip >= chunk.code.len() {
        if let Some(frame) = cont.frames.pop() {
            cleanup_continuation_frame(cont, frame);
        }
        if cont.frames.is_empty() {
            return StepOutcome::Done(Ok(cont.stack.pop().unwrap_or(Value::Unit)));
        }
        return StepOutcome::Continue;
    }

    let op_byte = chunk.code[frame.ip];
    let line = chunk.lines.get(frame.ip).copied().unwrap_or(0);
    let col = chunk.cols.get(frame.ip).copied().unwrap_or(0);
    let mut next_ip = frame.ip + 1;

    let mut outcome = if op_byte == Op::Unit as u8 {
        cont.stack.push(Value::Unit);
        StepOutcome::Continue
    } else if op_byte == Op::True as u8 {
        cont.stack.push(Value::Bool(true));
        StepOutcome::Continue
    } else if op_byte == Op::False as u8 {
        cont.stack.push(Value::Bool(false));
        StepOutcome::Continue
    } else if op_byte == Op::None as u8 {
        cont.stack.push(Value::Option(None));
        StepOutcome::Continue
    } else if op_byte == Op::Constant as u8 {
        if next_ip + 1 >= chunk.code.len() {
            StepOutcome::InstructionError(IonError::runtime(
                "truncated constant operand",
                line,
                col,
            ))
        } else {
            let idx = chunk.read_u16(next_ip) as usize;
            next_ip += 2;
            match chunk.constants.get(idx) {
                Some(value) => {
                    cont.stack.push(value.clone());
                    StepOutcome::Continue
                }
                None => StepOutcome::InstructionError(IonError::runtime(
                    "constant index out of bounds",
                    line,
                    col,
                )),
            }
        }
    } else if op_byte == Op::Add as u8 {
        binary_stack_op(cont, line, col, scaffold_add)
    } else if op_byte == Op::Sub as u8 {
        binary_stack_op(cont, line, col, scaffold_sub)
    } else if op_byte == Op::Mul as u8 {
        binary_stack_op(cont, line, col, scaffold_mul)
    } else if op_byte == Op::Div as u8 {
        binary_stack_op(cont, line, col, scaffold_div)
    } else if op_byte == Op::Mod as u8 {
        binary_stack_op(cont, line, col, scaffold_mod)
    } else if op_byte == Op::Neg as u8 {
        unary_stack_op(cont, line, col, scaffold_neg)
    } else if op_byte == Op::BitAnd as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_bitwise(left, right, line, col, "&", |x, y| x & y)
        })
    } else if op_byte == Op::BitOr as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_bitwise(left, right, line, col, "|", |x, y| x | y)
        })
    } else if op_byte == Op::BitXor as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_bitwise(left, right, line, col, "^", |x, y| x ^ y)
        })
    } else if op_byte == Op::Shl as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_shift(left, right, line, col, "<<", |x, y| x << y)
        })
    } else if op_byte == Op::Shr as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_shift(left, right, line, col, ">>", |x, y| x >> y)
        })
    } else if op_byte == Op::Eq as u8 {
        binary_stack_op(cont, line, col, |left, right, _, _| {
            Ok(Value::Bool(left == right))
        })
    } else if op_byte == Op::NotEq as u8 {
        binary_stack_op(cont, line, col, |left, right, _, _| {
            Ok(Value::Bool(left != right))
        })
    } else if op_byte == Op::Lt as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_compare_lt(&left, &right, line, col).map(Value::Bool)
        })
    } else if op_byte == Op::Gt as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_compare_lt(&right, &left, line, col).map(Value::Bool)
        })
    } else if op_byte == Op::LtEq as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_compare_lt(&right, &left, line, col).map(|value| Value::Bool(!value))
        })
    } else if op_byte == Op::GtEq as u8 {
        binary_stack_op(cont, line, col, |left, right, line, col| {
            scaffold_compare_lt(&left, &right, line, col).map(|value| Value::Bool(!value))
        })
    } else if op_byte == Op::Not as u8 {
        match pop_stack(cont, line, col) {
            Ok(value) => {
                cont.stack.push(Value::Bool(!value.is_truthy()));
                StepOutcome::Continue
            }
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::And as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated and operand")
            .map(|offset| {
                next_ip += 2;
                match cont.stack.last() {
                    Some(value) if !value.is_truthy() => next_ip += offset as usize,
                    Some(_) => {}
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "stack underflow (peek)",
                            line,
                            col,
                        ));
                    }
                }
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Or as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated or operand")
            .map(|offset| {
                next_ip += 2;
                match cont.stack.last() {
                    Some(value) if value.is_truthy() => next_ip += offset as usize,
                    Some(_) => {}
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "stack underflow (peek)",
                            line,
                            col,
                        ));
                    }
                }
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Pop as u8 {
        match pop_stack(cont, line, col) {
            Ok(_) => StepOutcome::Continue,
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::Dup as u8 {
        match cont.stack.last().cloned() {
            Some(value) => {
                cont.stack.push(value);
                StepOutcome::Continue
            }
            None => StepOutcome::InstructionError(IonError::runtime(
                "stack underflow (peek)",
                line,
                col,
            )),
        }
    } else if op_byte == Op::PushScope as u8 {
        cont.env.push_scope();
        cont.local_frames.push(cont.locals.len());
        StepOutcome::Continue
    } else if op_byte == Op::PopScope as u8 {
        cont.env.pop_scope();
        match cont.local_frames.pop() {
            Some(depth) => {
                cont.locals.truncate(depth);
                StepOutcome::Continue
            }
            None => {
                StepOutcome::InstructionError(IonError::runtime("local scope underflow", line, col))
            }
        }
    } else if op_byte == Op::BuildFString as u8 {
        build_f_string(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::Pipe as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated pipe operand")
            .map(|_| {
                next_ip += 1;
                StepOutcome::InstructionError(IonError::runtime(
                    "pipe opcode should not be executed directly",
                    line,
                    col,
                ))
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::DefineLocal as u8 {
        define_env_local(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::GetLocal as u8 || op_byte == Op::GetGlobal as u8 {
        get_env_local(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::SetLocal as u8 || op_byte == Op::SetGlobal as u8 {
        set_env_local(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::DefineLocalSlot as u8 {
        read_u8_operand(
            chunk,
            next_ip,
            line,
            col,
            "truncated define-local-slot operand",
        )
        .map(|mutable| {
            next_ip += 1;
            match pop_stack(cont, line, col) {
                Ok(value) => {
                    cont.locals.push(ContinuationLocalSlot {
                        value,
                        mutable: mutable != 0,
                    });
                    StepOutcome::Continue
                }
                Err(err) => StepOutcome::InstructionError(err),
            }
        })
        .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::ImportGlob as u8 {
        import_glob(cont, line, col)
    } else if op_byte == Op::GetLocalSlot as u8 {
        read_u16_operand(
            chunk,
            next_ip,
            line,
            col,
            "truncated get-local-slot operand",
        )
        .map(|slot| {
            next_ip += 2;
            let slot = frame.locals_base + slot as usize;
            match cont.locals.get(slot) {
                Some(local) => {
                    cont.stack.push(local.value.clone());
                    StepOutcome::Continue
                }
                None => StepOutcome::InstructionError(IonError::runtime(
                    "local slot out of bounds",
                    line,
                    col,
                )),
            }
        })
        .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::SetLocalSlot as u8 {
        read_u16_operand(
            chunk,
            next_ip,
            line,
            col,
            "truncated set-local-slot operand",
        )
        .map(|slot| {
            next_ip += 2;
            let slot = frame.locals_base + slot as usize;
            let value = match pop_stack(cont, line, col) {
                Ok(value) => value,
                Err(err) => return StepOutcome::InstructionError(err),
            };
            let Some(local) = cont.locals.get_mut(slot) else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "local slot out of bounds",
                    line,
                    col,
                ));
            };
            if !local.mutable {
                return StepOutcome::InstructionError(IonError::runtime(
                    "cannot assign to immutable variable",
                    line,
                    col,
                ));
            }
            local.value = value.clone();
            cont.stack.push(value);
            StepOutcome::Continue
        })
        .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::WrapSome as u8 {
        wrap_stack(cont, line, col, |value| {
            Value::Option(Some(Box::new(value)))
        })
    } else if op_byte == Op::WrapOk as u8 {
        wrap_stack(cont, line, col, |value| Value::Result(Ok(Box::new(value))))
    } else if op_byte == Op::WrapErr as u8 {
        wrap_stack(cont, line, col, |value| Value::Result(Err(Box::new(value))))
    } else if op_byte == Op::Try as u8 {
        match pop_stack(cont, line, col) {
            Ok(Value::Option(Some(value))) => {
                cont.stack.push(*value);
                StepOutcome::Continue
            }
            Ok(Value::Option(None)) => {
                StepOutcome::InstructionError(IonError::propagated_none(line, 0))
            }
            Ok(Value::Result(Ok(value))) => {
                cont.stack.push(*value);
                StepOutcome::Continue
            }
            Ok(Value::Result(Err(value))) => StepOutcome::InstructionError(
                IonError::propagated_err(value.to_string(), line, col),
            ),
            Ok(value) => StepOutcome::InstructionError(IonError::type_err(
                format!(
                    "{}{}",
                    "? operator requires Option or Result, got ",
                    value.type_name()
                ),
                line,
                col,
            )),
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::BuildList as u8 {
        build_sequence(chunk, cont, &mut next_ip, line, col, Value::List)
    } else if op_byte == Op::BuildTuple as u8 {
        build_sequence(chunk, cont, &mut next_ip, line, col, Value::Tuple)
    } else if op_byte == Op::BuildDict as u8 {
        build_dict(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::GetField as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated get-field operand")
            .map(|idx| {
                next_ip += 2;
                let field = match chunk.constants.get(idx as usize) {
                    Some(value) => match scaffold_const_as_str(value, line, col) {
                        Ok(field) => field,
                        Err(err) => return StepOutcome::InstructionError(err),
                    },
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "field constant index out of bounds",
                            line,
                            col,
                        ));
                    }
                };
                match pop_stack(cont, line, col)
                    .and_then(|object| scaffold_get_field(object, &field, line, col))
                {
                    Ok(value) => {
                        cont.stack.push(value);
                        StepOutcome::Continue
                    }
                    Err(err) => StepOutcome::InstructionError(err),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::GetIndex as u8 {
        let index = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        let object = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        match scaffold_get_index(object, index, line, col) {
            Ok(value) => {
                cont.stack.push(value);
                StepOutcome::Continue
            }
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::SetField as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated set-field operand")
            .map(|idx| {
                next_ip += 2;
                let field = match chunk.constants.get(idx as usize) {
                    Some(value) => match scaffold_const_as_str(value, line, col) {
                        Ok(field) => field,
                        Err(err) => return StepOutcome::InstructionError(err),
                    },
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "field constant index out of bounds",
                            line,
                            col,
                        ));
                    }
                };
                let value = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                let object = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                match scaffold_set_field(object, &field, value, line, col) {
                    Ok(value) => {
                        cont.stack.push(value);
                        StepOutcome::Continue
                    }
                    Err(err) => StepOutcome::InstructionError(err),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::SetIndex as u8 {
        let value = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        let index = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        let object = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        match scaffold_set_index(object, index, value, line, col) {
            Ok(value) => {
                cont.stack.push(value);
                StepOutcome::Continue
            }
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::MethodCall as u8 {
        read_method_call_operands(chunk, next_ip, line, col)
            .map(|(method, arg_count)| {
                next_ip += 3;
                if cont.stack.len() < arg_count + 1 {
                    return StepOutcome::InstructionError(IonError::runtime(
                        "stack underflow",
                        line,
                        col,
                    ));
                }
                let start = cont.stack.len() - arg_count;
                let args: Vec<Value> = cont.stack.drain(start..).collect();
                let receiver = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                if scaffold_is_closure_method(&receiver, &method) {
                    return start_continuation_method_call(
                        arena,
                        cont,
                        receiver,
                        &method,
                        args,
                        line,
                        col,
                        task,
                        host_futures.as_deref_mut(),
                    );
                }
                if let Some(outcome) = call_native_async_channel_method(
                    cont,
                    receiver.clone(),
                    &method,
                    &args,
                    line,
                    col,
                    task,
                    host_futures.as_deref_mut(),
                ) {
                    return outcome;
                }
                match scaffold_call_method(receiver, &method, &args, line, col) {
                    Ok(value) => {
                        cont.stack.push(value);
                        StepOutcome::Continue
                    }
                    Err(err) => StepOutcome::InstructionError(err),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Jump as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated jump operand")
            .map(|offset| {
                next_ip += 2 + offset as usize;
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::JumpIfFalse as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated jump-if-false operand")
            .map(|offset| {
                next_ip += 2;
                match cont.stack.last() {
                    Some(value) if !value.is_truthy() => next_ip += offset as usize,
                    Some(_) => {}
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "stack underflow (peek)",
                            line,
                            col,
                        ));
                    }
                }
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Loop as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated loop operand")
            .map(|offset| {
                next_ip += 2;
                next_ip = next_ip.saturating_sub(offset as usize);
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::BuildRange as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated range operand")
            .map(|inclusive| {
                next_ip += 1;
                let end = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                let start = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                let Some(start) = start.as_int() else {
                    return StepOutcome::InstructionError(IonError::type_err(
                        "range start must be int",
                        line,
                        col,
                    ));
                };
                let Some(end) = end.as_int() else {
                    return StepOutcome::InstructionError(IonError::type_err(
                        "range end must be int",
                        line,
                        col,
                    ));
                };
                cont.stack.push(Value::Range {
                    start,
                    end,
                    inclusive: inclusive != 0,
                });
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::ConstructStruct as u8 {
        construct_host_struct(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::ConstructEnum as u8 {
        construct_host_enum(chunk, cont, &mut next_ip, line, col)
    } else if op_byte == Op::MatchBegin as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated match-begin operand")
            .map(|kind| {
                next_ip += 1;
                let value = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                let matched = match kind {
                    1 => matches!(value, Value::Option(Some(_))),
                    2 => matches!(value, Value::Result(Ok(_))),
                    3 => matches!(value, Value::Result(Err(_))),
                    4 => {
                        let expected = match read_u8_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated tuple match operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 1;
                        matches!(&value, Value::Tuple(items) if items.len() == expected)
                    }
                    5 => {
                        let min_len = match read_u8_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated list match length operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        let has_rest = match read_u8_operand(
                            chunk,
                            next_ip + 1,
                            line,
                            col,
                            "truncated list match rest operand",
                        ) {
                            Ok(value) => value != 0,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 2;
                        match &value {
                            Value::List(items) if has_rest => items.len() >= min_len,
                            Value::List(items) => items.len() == min_len,
                            _ => false,
                        }
                    }
                    6 => {
                        let type_idx = match read_u16_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated struct match type operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 2;
                        let expected = match chunk.constants.get(type_idx) {
                            Some(Value::Str(value)) => value,
                            Some(_) => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "expected string constant",
                                    line,
                                    col,
                                ));
                            }
                            None => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "type constant index out of bounds",
                                    line,
                                    col,
                                ));
                            }
                        };
                        let want = crate::hash::h(expected);
                        matches!(&value, Value::HostStruct { type_hash, .. } if *type_hash == want)
                    }
                    7 => {
                        let enum_idx = match read_u16_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated enum match enum operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        let variant_idx = match read_u16_operand(
                            chunk,
                            next_ip + 2,
                            line,
                            col,
                            "truncated enum match variant operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        let expected_arity = match read_u8_operand(
                            chunk,
                            next_ip + 4,
                            line,
                            col,
                            "truncated enum match arity operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 5;
                        let expected_enum = match chunk.constants.get(enum_idx) {
                            Some(Value::Str(value)) => value,
                            Some(_) => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "expected string constant",
                                    line,
                                    col,
                                ));
                            }
                            None => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "enum constant index out of bounds",
                                    line,
                                    col,
                                ));
                            }
                        };
                        let expected_variant = match chunk.constants.get(variant_idx) {
                            Some(Value::Str(value)) => value,
                            Some(_) => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "expected string constant",
                                    line,
                                    col,
                                ));
                            }
                            None => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "variant constant index out of bounds",
                                    line,
                                    col,
                                ));
                            }
                        };
                        let want_enum = crate::hash::h(expected_enum);
                        let want_variant = crate::hash::h(expected_variant);
                        matches!(
                            &value,
                            Value::HostEnum {
                                enum_hash,
                                variant_hash,
                                data,
                            } if *enum_hash == want_enum
                                && *variant_hash == want_variant
                                && data.len() == expected_arity
                        )
                    }
                    _ => false,
                };
                cont.stack.push(value);
                cont.stack.push(Value::Bool(matched));
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::MatchArm as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated match-arm operand")
            .map(|kind| {
                next_ip += 1;
                match kind {
                    1 => match pop_stack(cont, line, col) {
                        Ok(Value::Option(Some(value))) => cont.stack.push(*value),
                        Ok(value) => cont.stack.push(value),
                        Err(err) => return StepOutcome::InstructionError(err),
                    },
                    2 => match pop_stack(cont, line, col) {
                        Ok(Value::Result(Ok(value))) => cont.stack.push(*value),
                        Ok(value) => cont.stack.push(value),
                        Err(err) => return StepOutcome::InstructionError(err),
                    },
                    3 => match pop_stack(cont, line, col) {
                        Ok(Value::Result(Err(value))) => cont.stack.push(*value),
                        Ok(value) => cont.stack.push(value),
                        Err(err) => return StepOutcome::InstructionError(err),
                    },
                    4 | 5 | 7 => {
                        let index = match read_u8_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated indexed match-arm operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 1;
                        match (kind, cont.stack.last()) {
                            (4 | 5, Some(Value::Tuple(items)))
                            | (4 | 5, Some(Value::List(items))) => {
                                cont.stack
                                    .push(items.get(index).cloned().unwrap_or(Value::Unit));
                            }
                            (7, Some(Value::HostEnum { data, .. })) => {
                                cont.stack
                                    .push(data.get(index).cloned().unwrap_or(Value::Unit));
                            }
                            (_, Some(_)) => cont.stack.push(Value::Unit),
                            (_, None) => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "stack underflow (peek)",
                                    line,
                                    col,
                                ));
                            }
                        }
                    }
                    6 => {
                        let field_idx = match read_u16_operand(
                            chunk,
                            next_ip,
                            line,
                            col,
                            "truncated struct field match-arm operand",
                        ) {
                            Ok(value) => value as usize,
                            Err(err) => return StepOutcome::InstructionError(err),
                        };
                        next_ip += 2;
                        let field = match chunk.constants.get(field_idx) {
                            Some(Value::Str(value)) => value,
                            Some(_) => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "expected string constant",
                                    line,
                                    col,
                                ));
                            }
                            None => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "field constant index out of bounds",
                                    line,
                                    col,
                                ));
                            }
                        };
                        match cont.stack.last() {
                            Some(Value::HostStruct { fields, .. }) => {
                                let fh = crate::hash::h(field);
                                let field_value = fields.get(&fh).cloned();
                                cont.stack.push(Value::Option(field_value.map(Box::new)));
                            }
                            Some(_) => cont.stack.push(Value::Option(None)),
                            None => {
                                return StepOutcome::InstructionError(IonError::runtime(
                                    "stack underflow (peek)",
                                    line,
                                    col,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::MatchEnd as u8 {
        StepOutcome::InstructionError(IonError::runtime("non-exhaustive match", line, col))
    } else if op_byte == Op::CheckType as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated check-type operand")
            .map(|idx| {
                next_ip += 2;
                let type_name = match chunk.constants.get(idx as usize) {
                    Some(Value::Str(type_name)) => type_name.as_str(),
                    Some(_) => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "expected string constant",
                            line,
                            col,
                        ));
                    }
                    None => {
                        return StepOutcome::InstructionError(IonError::runtime(
                            "type constant index out of bounds",
                            line,
                            col,
                        ));
                    }
                };
                let Some(value) = cont.stack.last() else {
                    return StepOutcome::InstructionError(IonError::runtime(
                        "CheckType: empty stack",
                        line,
                        col,
                    ));
                };
                match scaffold_check_type(value, type_name, line, col) {
                    Ok(()) => StepOutcome::Continue,
                    Err(err) => StepOutcome::InstructionError(err),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Slice as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated slice operand")
            .map(|flags| {
                next_ip += 1;
                let has_start = flags & 1 != 0;
                let has_end = flags & 2 != 0;
                let inclusive = flags & 4 != 0;
                let end = if has_end {
                    match pop_stack(cont, line, col) {
                        Ok(value) => Some(value),
                        Err(err) => return StepOutcome::InstructionError(err),
                    }
                } else {
                    None
                };
                let start = if has_start {
                    match pop_stack(cont, line, col) {
                        Ok(value) => Some(value),
                        Err(err) => return StepOutcome::InstructionError(err),
                    }
                } else {
                    None
                };
                let object = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                match scaffold_slice_access(object, start, end, inclusive, line, col) {
                    Ok(value) => {
                        cont.stack.push(value);
                        StepOutcome::Continue
                    }
                    Err(err) => StepOutcome::InstructionError(err),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::ListAppend as u8 {
        match scaffold_list_append(cont, line, col) {
            Ok(()) => StepOutcome::Continue,
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::ListExtend as u8 {
        match scaffold_list_extend(cont, line, col) {
            Ok(()) => StepOutcome::Continue,
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::DictInsert as u8 {
        match scaffold_dict_insert(cont, line, col) {
            Ok(()) => StepOutcome::Continue,
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::DictMerge as u8 {
        match scaffold_dict_merge(cont, line, col) {
            Ok(()) => StepOutcome::Continue,
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::IterInit as u8 {
        match pop_stack(cont, line, col).and_then(|value| scaffold_value_iterator(value, line, col))
        {
            Ok(iterator) => {
                cont.iterators.push(iterator);
                cont.stack.push(Value::Unit);
                StepOutcome::Continue
            }
            Err(err) => StepOutcome::InstructionError(err),
        }
    } else if op_byte == Op::IterNext as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated iter-next operand")
            .map(|offset| {
                next_ip += 2;
                if let Err(err) = pop_stack(cont, line, col) {
                    return StepOutcome::InstructionError(err);
                }
                let Some(iterator) = cont.iterators.last_mut() else {
                    return StepOutcome::InstructionError(IonError::runtime(
                        "no active iterator",
                        line,
                        col,
                    ));
                };
                match iterator.next() {
                    Some(value) => cont.stack.push(value),
                    None => {
                        cont.iterators.pop();
                        cont.stack.push(Value::Unit);
                        next_ip += offset as usize;
                    }
                }
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::IterDrop as u8 {
        cont.iterators.pop();
        StepOutcome::Continue
    } else if op_byte == Op::TryBegin as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated try-begin operand")
            .map(|offset| {
                next_ip += 2;
                cont.exception_handlers.push(ContinuationExceptionHandler {
                    catch_ip: next_ip + offset as usize,
                    stack_depth: cont.stack.len(),
                });
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::TryEnd as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated try-end operand")
            .map(|offset| {
                next_ip += 2;
                cont.exception_handlers.pop();
                next_ip += offset as usize;
                StepOutcome::Continue
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Closure as u8 {
        read_u16_operand(chunk, next_ip, line, col, "truncated closure operand")
            .map(|idx| {
                next_ip += 2;
                match chunk.constants.get(idx as usize) {
                    Some(value) => {
                        cont.stack.push(value.clone());
                        StepOutcome::Continue
                    }
                    None => StepOutcome::InstructionError(IonError::runtime(
                        "closure constant index out of bounds",
                        line,
                        col,
                    )),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Call as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated call operand")
            .map(|arg_count| {
                next_ip += 1;
                call_continuation_function(
                    arena,
                    cont,
                    arg_count as usize,
                    line,
                    col,
                    task,
                    host_futures.as_deref_mut(),
                    None,
                )
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::CallNamed as u8 {
        read_call_named_operands(chunk, next_ip, line, col)
            .map(|(arg_count, named_args, operand_len)| {
                next_ip += operand_len;
                call_continuation_function_named(
                    arena,
                    cont,
                    arg_count,
                    &named_args,
                    line,
                    col,
                    task,
                    host_futures.as_deref_mut(),
                )
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::TailCall as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated tail-call operand")
            .map(|arg_count| {
                next_ip += 1;
                call_continuation_function(
                    arena,
                    cont,
                    arg_count as usize,
                    line,
                    col,
                    task,
                    host_futures.as_deref_mut(),
                    Some((frame_index, chunk.code.len())),
                )
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::SpawnCall as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated spawn-call operand")
            .map(|arg_count| {
                next_ip += 1;
                spawn_continuation_call_task(arena, cont, arg_count as usize, line, col)
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::SpawnCallNamed as u8 {
        read_call_named_operands(chunk, next_ip, line, col)
            .map(|(arg_count, named_args, operand_len)| {
                next_ip += operand_len;
                spawn_continuation_call_task_named(arena, cont, arg_count, &named_args, line, col)
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::AwaitTask as u8 {
        await_continuation_task(cont, task, host_futures.as_deref_mut(), line, col)
    } else if op_byte == Op::SelectTasks as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated select-tasks operand")
            .map(|branch_count| {
                next_ip += 1;
                select_continuation_tasks(
                    cont,
                    branch_count as usize,
                    task,
                    host_futures.as_deref_mut(),
                    line,
                    col,
                )
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else if op_byte == Op::Return as u8 {
        let value = cont.stack.pop().unwrap_or(Value::Unit);
        if let Some(frame) = cont.frames.pop() {
            cleanup_continuation_frame(cont, frame);
        }
        if cont.frames.is_empty() {
            StepOutcome::Done(Ok(value))
        } else {
            cont.stack.push(value);
            StepOutcome::Continue
        }
    } else if op_byte == Op::Print as u8 {
        read_u8_operand(chunk, next_ip, line, col, "truncated print operand")
            .map(|newline| {
                next_ip += 1;
                let value = match pop_stack(cont, line, col) {
                    Ok(value) => value,
                    Err(err) => return StepOutcome::InstructionError(err),
                };
                let text = if newline != 0 {
                    format!("{value}\n")
                } else {
                    value.to_string()
                };
                match cont.output.write(OutputStream::Stdout, &text) {
                    Ok(()) => {
                        cont.stack.push(Value::Unit);
                        StepOutcome::Continue
                    }
                    Err(err) => StepOutcome::InstructionError(IonError::runtime(err, line, col)),
                }
            })
            .unwrap_or_else(StepOutcome::InstructionError)
    } else {
        StepOutcome::InstructionError(IonError::runtime(
            "opcode not supported by async VM scaffold",
            line,
            col,
        ))
    };

    if let StepOutcome::InstructionError(err) = &outcome {
        if matches!(
            err.kind,
            ErrorKind::PropagatedErr | ErrorKind::PropagatedNone
        ) && cont.frames.len() > 1
        {
            let err = err.clone();
            if let Some(frame) = cont.frames.pop() {
                cleanup_continuation_frame(cont, frame);
            }
            match err.kind {
                ErrorKind::PropagatedErr => {
                    cont.stack
                        .push(Value::Result(Err(Box::new(Value::Str(err.message)))));
                }
                ErrorKind::PropagatedNone => {
                    cont.stack.push(Value::Option(None));
                }
                _ => unreachable!(),
            }
            outcome = StepOutcome::Continue;
        } else if err.kind != ErrorKind::PropagatedErr && err.kind != ErrorKind::PropagatedNone {
            if let Some(handler) = cont.exception_handlers.pop() {
                cont.stack.truncate(handler.stack_depth);
                cont.stack.push(Value::Str(err.message.clone()));
                next_ip = handler.catch_ip;
                outcome = StepOutcome::Continue;
            }
        }
    }

    if matches!(outcome, StepOutcome::Continue | StepOutcome::Suspended(_)) {
        if let Some(current) = cont.frames.get_mut(frame_index) {
            if current.chunk == frame.chunk && current.ip == frame.ip {
                current.ip = next_ip;
            }
        }
    }

    outcome
}

fn cleanup_continuation_frame(cont: &mut VmContinuation, frame: CallFrame) {
    cont.locals.truncate(frame.locals_base);
    cont.local_frames
        .retain(|depth| *depth <= frame.locals_base);
    if frame.captured_scope {
        cont.env.pop_scope();
    }
}

fn route_continuation_error(cont: &mut VmContinuation, err: IonError) -> StepOutcome {
    if err.kind != ErrorKind::PropagatedErr && err.kind != ErrorKind::PropagatedNone {
        if let Some(handler) = cont.exception_handlers.pop() {
            cont.stack.truncate(handler.stack_depth);
            cont.stack.push(Value::Str(err.message));
            if let Some(frame) = cont.frames.last_mut() {
                frame.ip = handler.catch_ip;
            }
            return StepOutcome::Continue;
        }
    }
    StepOutcome::InstructionError(err)
}

fn pop_stack(cont: &mut VmContinuation, line: usize, col: usize) -> Result<Value, IonError> {
    cont.stack
        .pop()
        .ok_or_else(|| IonError::runtime("stack underflow", line, col))
}

fn unary_stack_op(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
    op: impl FnOnce(Value, usize, usize) -> Result<Value, IonError>,
) -> StepOutcome {
    match pop_stack(cont, line, col).and_then(|value| op(value, line, col)) {
        Ok(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(err),
    }
}

fn binary_stack_op(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
    op: impl FnOnce(Value, Value, usize, usize) -> Result<Value, IonError>,
) -> StepOutcome {
    let right = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let left = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return StepOutcome::InstructionError(err),
    };

    match op(left, right, line, col) {
        Ok(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(err),
    }
}

fn wrap_stack(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
    wrap: impl FnOnce(Value) -> Value,
) -> StepOutcome {
    match pop_stack(cont, line, col) {
        Ok(value) => {
            cont.stack.push(wrap(value));
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(err),
    }
}

fn read_u16_operand(
    chunk: &Chunk,
    offset: usize,
    line: usize,
    col: usize,
    message: &'static str,
) -> Result<u16, IonError> {
    if offset + 1 >= chunk.code.len() {
        return Err(IonError::runtime(message, line, col));
    }
    Ok(chunk.read_u16(offset))
}

fn read_u8_operand(
    chunk: &Chunk,
    offset: usize,
    line: usize,
    col: usize,
    message: &'static str,
) -> Result<u8, IonError> {
    chunk
        .code
        .get(offset)
        .copied()
        .ok_or_else(|| IonError::runtime(message, line, col))
}

fn read_call_named_operands(
    chunk: &Chunk,
    offset: usize,
    line: usize,
    col: usize,
) -> Result<(usize, Vec<(usize, String)>, usize), IonError> {
    let arg_count = read_u8_operand(chunk, offset, line, col, "truncated named-call arg count")?;
    let named_count = read_u8_operand(
        chunk,
        offset + 1,
        line,
        col,
        "truncated named-call named count",
    )?;
    let mut cursor = offset + 2;
    let mut named_args = Vec::with_capacity(named_count as usize);
    for _ in 0..named_count {
        let position = read_u8_operand(
            chunk,
            cursor,
            line,
            col,
            "truncated named-call argument position",
        )? as usize;
        let name_idx = read_u16_operand(
            chunk,
            cursor + 1,
            line,
            col,
            "truncated named-call argument name",
        )? as usize;
        cursor += 3;
        let Some(value) = chunk.constants.get(name_idx) else {
            return Err(IonError::runtime(
                "named-call name constant index out of bounds",
                line,
                col,
            ));
        };
        named_args.push((position, scaffold_const_as_str(value, line, col)?));
    }
    Ok((arg_count as usize, named_args, cursor - offset))
}

fn read_method_call_operands(
    chunk: &Chunk,
    offset: usize,
    line: usize,
    col: usize,
) -> Result<(String, usize), IonError> {
    let method_idx =
        read_u16_operand(chunk, offset, line, col, "truncated method-call name")? as usize;
    let arg_count = read_u8_operand(
        chunk,
        offset + 2,
        line,
        col,
        "truncated method-call argument count",
    )? as usize;
    let Some(value) = chunk.constants.get(method_idx) else {
        return Err(IonError::runtime(
            "method name constant index out of bounds",
            line,
            col,
        ));
    };
    Ok((scaffold_const_as_str(value, line, col)?, arg_count))
}

fn call_continuation_function(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    arg_count: usize,
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
    tail_call: Option<(usize, usize)>,
) -> StepOutcome {
    if cont.stack.len() < arg_count + 1 {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let args_start = cont.stack.len() - arg_count;
    let func_idx = args_start - 1;
    let func = cont.stack[func_idx].clone();
    let args: Vec<Value> = cont.stack[args_start..].to_vec();
    cont.stack.truncate(func_idx);

    let Value::Fn(ion_fn) = func else {
        if let Value::BuiltinFn { func: builtin, .. } = func {
            return match builtin(&args).map_err(|err| IonError::runtime(err, line, col)) {
                Ok(value) => {
                    cont.stack.push(value);
                    StepOutcome::Continue
                }
                Err(err) => StepOutcome::InstructionError(err),
            };
        }

        if let Value::BuiltinClosure { func: builtin, .. } = func {
            return match builtin
                .call(&args)
                .map_err(|err| IonError::runtime(err, line, col))
            {
                Ok(value) => {
                    cont.stack.push(value);
                    StepOutcome::Continue
                }
                Err(err) => StepOutcome::InstructionError(err),
            };
        }

        #[cfg(feature = "async-runtime")]
        if let Value::AsyncBuiltinClosure {
            qualified_hash,
            func: async_fn,
        } = func
        {
            let Some(task) = task else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "async host function call requires a runtime task",
                    line,
                    col,
                ));
            };
            let Some(host_futures) = host_futures else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "async host function call requires a host future table",
                    line,
                    col,
                ));
            };
            let future = if qualified_hash == crate::hash::h("timeout") {
                match timeout_future(arena, cont, args, line, col) {
                    Ok(future) => future,
                    Err(err) => return StepOutcome::InstructionError(err),
                }
            } else {
                async_fn.call(args)
            };
            let future_id = host_futures.insert(task, future);
            if let Some((frame_index, return_ip)) = tail_call {
                if let Some(frame) = cont.frames.get_mut(frame_index) {
                    frame.ip = return_ip;
                }
            }
            return StepOutcome::Suspended(TaskState::WaitingHostFuture(future_id));
        }

        return StepOutcome::InstructionError(IonError::type_err(
            format!("cannot call {}", func.type_name()),
            line,
            col,
        ));
    };

    start_continuation_ion_function(
        cont,
        &ion_fn,
        args.into_iter().map(Some).collect(),
        arg_count,
        line,
        col,
        tail_call.map(|(frame_index, _)| frame_index),
    )
}

fn spawn_continuation_call_task(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    arg_count: usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    if cont.stack.len() < arg_count + 1 {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let args_start = cont.stack.len() - arg_count;
    let func_idx = args_start - 1;
    let func = cont.stack[func_idx].clone();
    let args: Vec<Value> = cont.stack[args_start..].to_vec();
    cont.stack.truncate(func_idx);

    let future: BoxIonFuture = match func {
        Value::AsyncBuiltinClosure { func: async_fn, .. } => async_fn.call(args),
        Value::BuiltinFn { func: builtin, .. } => {
            Box::pin(async move { builtin(&args).map_err(|err| IonError::runtime(err, line, col)) })
        }
        Value::BuiltinClosure { func: builtin, .. } => Box::pin(async move {
            builtin
                .call(&args)
                .map_err(|err| IonError::runtime(err, line, col))
        }),
        Value::Fn(ion_fn) => {
            match spawned_ion_function_future(arena, cont, ion_fn, args, line, col) {
                Ok(future) => future,
                Err(err) => return StepOutcome::InstructionError(err),
            }
        }
        other => {
            return StepOutcome::InstructionError(IonError::type_err(
                format!("cannot spawn {}", other.type_name()),
                line,
                col,
            ));
        }
    };

    let task = AsyncTask::new(future);
    cont.spawned_tasks.push(task.clone());
    cont.stack.push(Value::AsyncTask(task));
    StepOutcome::Continue
}

fn spawn_continuation_call_task_named(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    arg_count: usize,
    named_args: &[(usize, String)],
    line: usize,
    col: usize,
) -> StepOutcome {
    if cont.stack.len() < arg_count + 1 {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let args_start = cont.stack.len() - arg_count;
    let func_idx = args_start - 1;
    let func = cont.stack[func_idx].clone();
    let raw_args: Vec<Value> = cont.stack[args_start..].to_vec();
    cont.stack.truncate(func_idx);

    let Value::Fn(ion_fn) = &func else {
        cont.stack.push(func);
        cont.stack.extend(raw_args);
        return spawn_continuation_call_task(arena, cont, arg_count, line, col);
    };

    let mut ordered = vec![None; ion_fn.params.len()];
    let mut positional = 0usize;
    for (index, value) in raw_args.into_iter().enumerate() {
        if let Some((_, name)) = named_args.iter().find(|(position, _)| *position == index) {
            let Some(param_index) = ion_fn.params.iter().position(|param| &param.name == name)
            else {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!(
                        "unknown parameter '{}' for function '{}'",
                        name, ion_fn.name
                    ),
                    line,
                    col,
                ));
            };
            if ordered[param_index].is_some() {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!("duplicate argument '{}'", name),
                    line,
                    col,
                ));
            }
            ordered[param_index] = Some(value);
        } else {
            while positional < ordered.len() && ordered[positional].is_some() {
                positional += 1;
            }
            if positional >= ordered.len() {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!(
                        "function '{}' expected {} arguments, got {}",
                        ion_fn.name,
                        ion_fn.params.len(),
                        arg_count
                    ),
                    line,
                    col,
                ));
            }
            ordered[positional] = Some(value);
            positional += 1;
        }
    }

    match spawned_ion_function_future_from_supplied(
        arena,
        cont,
        ion_fn.clone(),
        ordered,
        arg_count,
        line,
        col,
    ) {
        Ok(future) => {
            let task = AsyncTask::new(future);
            cont.spawned_tasks.push(task.clone());
            cont.stack.push(Value::AsyncTask(task));
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(err),
    }
}

fn spawned_ion_function_future(
    arena: &ChunkArena,
    parent: &VmContinuation,
    ion_fn: crate::value::IonFn,
    args: Vec<Value>,
    line: usize,
    col: usize,
) -> Result<BoxIonFuture, IonError> {
    let supplied_count = args.len();
    let supplied = args.into_iter().map(Some).collect();
    spawned_ion_function_future_from_supplied(
        arena,
        parent,
        ion_fn,
        supplied,
        supplied_count,
        line,
        col,
    )
}

fn spawned_ion_function_future_from_supplied(
    arena: &ChunkArena,
    parent: &VmContinuation,
    ion_fn: crate::value::IonFn,
    supplied: Vec<Option<Value>>,
    supplied_count: usize,
    line: usize,
    col: usize,
) -> Result<BoxIonFuture, IonError> {
    let mut child = VmContinuation {
        env: parent.env.clone(),
        types: parent.types.clone(),
        output: Arc::clone(&parent.output),
        fn_chunks: parent.fn_chunks.clone(),
        ..VmContinuation::default()
    };
    match start_continuation_ion_function(
        &mut child,
        &ion_fn,
        supplied,
        supplied_count,
        line,
        col,
        None,
    ) {
        StepOutcome::Continue => {}
        StepOutcome::InstructionError(err) => return Err(err),
        StepOutcome::Done(result) => return Ok(Box::pin(async move { result })),
        StepOutcome::Yield | StepOutcome::Suspended(_) => {
            return Err(IonError::runtime(
                "unexpected suspension while starting spawned function",
                line,
                col,
            ));
        }
    }

    let arena = arena.clone();
    Ok(Box::pin(async move {
        poll_spawned_ion_continuation(arena, child).await
    }))
}

async fn poll_spawned_ion_continuation(
    arena: ChunkArena,
    mut cont: VmContinuation,
) -> Result<Value, IonError> {
    let task = TaskId(0);
    let mut host_futures = HostFutureTable::new();
    let mut waiting = None;
    let mut final_result = None;

    std::future::poll_fn(move |cx| {
        const STEP_BUDGET: usize = 1024;

        loop {
            for _ in 0..STEP_BUDGET {
                if waiting.is_some() {
                    match resume_spawned_continuation_host_future(
                        cx,
                        &mut cont,
                        task,
                        &mut host_futures,
                        &mut waiting,
                        &mut final_result,
                    ) {
                        SpawnedContinuationPoll::Pending => return Poll::Pending,
                        SpawnedContinuationPoll::Continue => {}
                        SpawnedContinuationPoll::Done(result) => return Poll::Ready(result),
                    }
                }

                match step_task_with_host_futures(&arena, &mut cont, task, &mut host_futures) {
                    StepOutcome::Continue => {}
                    StepOutcome::Yield => {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    StepOutcome::Suspended(state) => {
                        waiting = Some(state);
                    }
                    StepOutcome::InstructionError(err) => return Poll::Ready(Err(err)),
                    StepOutcome::Done(result) => {
                        if cont.spawned_tasks.is_empty() {
                            return Poll::Ready(result);
                        }
                        let future_id = host_futures
                            .insert(task, join_spawned_tasks_future(cont.spawned_tasks.clone()));
                        waiting = Some(TaskState::WaitingHostFuture(future_id));
                        final_result = Some(result);
                    }
                }
            }

            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
    })
    .await
}

enum SpawnedContinuationPoll {
    Pending,
    Continue,
    Done(Result<Value, IonError>),
}

fn resume_spawned_continuation_host_future(
    cx: &mut Context<'_>,
    cont: &mut VmContinuation,
    task: TaskId,
    host_futures: &mut HostFutureTable,
    waiting: &mut Option<TaskState>,
    final_result: &mut Option<Result<Value, IonError>>,
) -> SpawnedContinuationPoll {
    let ready = host_futures.poll_ready(cx);
    let mut resumed = false;
    for item in ready {
        if item.waiter != task {
            continue;
        }
        if let Some(result) = final_result.take() {
            *waiting = None;
            return SpawnedContinuationPoll::Done(match item.result {
                Ok(_) => result,
                Err(err) => Err(err),
            });
        }
        match cont.resume_host_result(item.result) {
            StepOutcome::Continue => {
                *waiting = None;
                resumed = true;
            }
            StepOutcome::InstructionError(err) => {
                return SpawnedContinuationPoll::Done(Err(err));
            }
            StepOutcome::Done(result) => {
                return SpawnedContinuationPoll::Done(result);
            }
            StepOutcome::Yield | StepOutcome::Suspended(_) => {
                resumed = true;
            }
        }
    }

    if resumed {
        SpawnedContinuationPoll::Continue
    } else {
        SpawnedContinuationPoll::Pending
    }
}

fn await_continuation_task(
    cont: &mut VmContinuation,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
    line: usize,
    col: usize,
) -> StepOutcome {
    let value = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let Value::AsyncTask(target) = value else {
        return StepOutcome::InstructionError(IonError::type_err(
            format!("cannot await {}", value.type_name()),
            line,
            col,
        ));
    };
    let Some(task) = task else {
        return StepOutcome::InstructionError(IonError::runtime(
            "await requires a runtime task",
            line,
            col,
        ));
    };
    let Some(host_futures) = host_futures else {
        return StepOutcome::InstructionError(IonError::runtime(
            "await requires a host future table",
            line,
            col,
        ));
    };

    let spawned = cont.spawned_tasks.clone();
    let future: BoxIonFuture = Box::pin(async move {
        std::future::poll_fn(|cx| {
            for spawned_task in &spawned {
                if !spawned_task.ptr_eq(&target) {
                    let _ = spawned_task.poll_result(cx);
                }
            }
            target.poll_result(cx)
        })
        .await
    });
    let future_id = host_futures.insert(task, future);
    StepOutcome::Suspended(TaskState::WaitingHostFuture(future_id))
}

fn select_continuation_tasks(
    cont: &mut VmContinuation,
    branch_count: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
    line: usize,
    col: usize,
) -> StepOutcome {
    if branch_count == 0 {
        return StepOutcome::InstructionError(IonError::runtime(
            "select requires at least one branch",
            line,
            col,
        ));
    }
    if cont.stack.len() < branch_count {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let mut branches = Vec::with_capacity(branch_count);
    for _ in 0..branch_count {
        let value = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        let Value::AsyncTask(task) = value else {
            return StepOutcome::InstructionError(IonError::type_err(
                format!(
                    "select branch requires AsyncTask, got {}",
                    value.type_name()
                ),
                line,
                col,
            ));
        };
        branches.push(task);
    }
    branches.reverse();

    let Some(task) = task else {
        return StepOutcome::InstructionError(IonError::runtime(
            "select requires a runtime task",
            line,
            col,
        ));
    };
    let Some(host_futures) = host_futures else {
        return StepOutcome::InstructionError(IonError::runtime(
            "select requires a host future table",
            line,
            col,
        ));
    };

    cont.spawned_tasks
        .retain(|spawned| !branches.iter().any(|branch| spawned.ptr_eq(branch)));
    let spawned = cont.spawned_tasks.clone();
    let future: BoxIonFuture = Box::pin(async move {
        std::future::poll_fn(|cx| {
            for spawned_task in &spawned {
                if !branches
                    .iter()
                    .any(|branch_task| spawned_task.ptr_eq(branch_task))
                {
                    let _ = spawned_task.poll_result(cx);
                }
            }

            for (idx, branch) in branches.iter().enumerate() {
                match branch.poll_result(cx) {
                    Poll::Ready(Ok(value)) => {
                        return Poll::Ready(Ok(Value::Tuple(vec![Value::Int(idx as i64), value])));
                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => {}
                }
            }
            Poll::Pending
        })
        .await
    });
    let future_id = host_futures.insert(task, future);
    StepOutcome::Suspended(TaskState::WaitingHostFuture(future_id))
}

fn join_spawned_tasks_future(tasks: Vec<AsyncTask>) -> BoxIonFuture {
    Box::pin(async move {
        std::future::poll_fn(|cx| {
            let mut all_ready = true;
            for task in &tasks {
                match task.poll_result(cx) {
                    Poll::Ready(Ok(_)) => {}
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => all_ready = false,
                }
            }
            if all_ready {
                Poll::Ready(Ok(Value::Unit))
            } else {
                Poll::Pending
            }
        })
        .await
    })
}

fn timeout_future(
    arena: &ChunkArena,
    cont: &VmContinuation,
    args: Vec<Value>,
    line: usize,
    col: usize,
) -> Result<BoxIonFuture, IonError> {
    if args.len() != 2 {
        return Err(IonError::runtime(
            "timeout(ms, fn) requires 2 arguments",
            line,
            col,
        ));
    }
    let ms = args[0].as_int().ok_or_else(|| {
        IonError::runtime("timeout: first argument must be int (ms)", line, col)
    })?;
    let callback = args[1].clone();
    let callback_future = callback_as_zero_arg_future(arena, cont, callback, line, col)?;

    Ok(Box::pin(async move {
        match tokio::time::timeout(Duration::from_millis(ms as u64), callback_future).await {
            Ok(Ok(value)) => Ok(Value::Option(Some(Box::new(value)))),
            Ok(Err(err)) => Err(err),
            Err(_) => Ok(Value::Option(None)),
        }
    }))
}

fn callback_as_zero_arg_future(
    arena: &ChunkArena,
    cont: &VmContinuation,
    callback: Value,
    line: usize,
    col: usize,
) -> Result<BoxIonFuture, IonError> {
    match callback {
        Value::Fn(ion_fn) => spawned_ion_function_future(arena, cont, ion_fn, Vec::new(), line, col),
        Value::AsyncBuiltinClosure { func: async_fn, .. } => Ok(async_fn.call(Vec::new())),
        Value::BuiltinFn { func: builtin, .. } => Ok(Box::pin(async move {
            builtin(&[]).map_err(|err| IonError::runtime(err, line, col))
        })),
        Value::BuiltinClosure { func: builtin, .. } => Ok(Box::pin(async move {
            builtin
                .call(&[])
                .map_err(|err| IonError::runtime(err, line, col))
        })),
        other => Err(IonError::type_err(
            format!("timeout callback must be callable, got {}", other.type_name()),
            line,
            col,
        )),
    }
}

fn call_continuation_function_named(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    arg_count: usize,
    named_args: &[(usize, String)],
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    if cont.stack.len() < arg_count + 1 {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let args_start = cont.stack.len() - arg_count;
    let func_idx = args_start - 1;
    let func = cont.stack[func_idx].clone();
    let raw_args: Vec<Value> = cont.stack[args_start..].to_vec();
    cont.stack.truncate(func_idx);

    let Value::Fn(ion_fn) = &func else {
        cont.stack.push(func);
        cont.stack.extend(raw_args);
        return call_continuation_function(
            arena,
            cont,
            arg_count,
            line,
            col,
            task,
            host_futures,
            None,
        );
    };

    let mut ordered = vec![None; ion_fn.params.len()];
    let mut positional = 0usize;
    for (index, value) in raw_args.into_iter().enumerate() {
        if let Some((_, name)) = named_args.iter().find(|(position, _)| *position == index) {
            let Some(param_index) = ion_fn.params.iter().position(|param| &param.name == name)
            else {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!(
                        "unknown parameter '{}' for function '{}'",
                        name, ion_fn.name
                    ),
                    line,
                    col,
                ));
            };
            if ordered[param_index].is_some() {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!("duplicate argument '{}'", name),
                    line,
                    col,
                ));
            }
            ordered[param_index] = Some(value);
        } else {
            while positional < ordered.len() && ordered[positional].is_some() {
                positional += 1;
            }
            if positional >= ordered.len() {
                return StepOutcome::InstructionError(IonError::runtime(
                    format!(
                        "function '{}' expected {} arguments, got {}",
                        ion_fn.name,
                        ion_fn.params.len(),
                        arg_count
                    ),
                    line,
                    col,
                ));
            }
            ordered[positional] = Some(value);
            positional += 1;
        }
    }

    start_continuation_ion_function(cont, ion_fn, ordered, arg_count, line, col, None)
}

fn start_continuation_ion_function(
    cont: &mut VmContinuation,
    ion_fn: &crate::value::IonFn,
    supplied: Vec<Option<Value>>,
    supplied_count: usize,
    line: usize,
    col: usize,
    tail_frame_index: Option<usize>,
) -> StepOutcome {
    if supplied_count > ion_fn.params.len() {
        return StepOutcome::InstructionError(IonError::runtime(
            format!(
                "function '{}' expected {} arguments, got {}",
                ion_fn.name,
                ion_fn.params.len(),
                supplied_count
            ),
            line,
            col,
        ));
    }

    let Some(chunk) = cont.fn_chunks.get(&ion_fn.fn_id).copied() else {
        return StepOutcome::InstructionError(IonError::runtime(
            "function chunk not registered",
            line,
            col,
        ));
    };

    let locals_base = if let Some(frame_index) = tail_frame_index {
        let Some(frame) = cont.frames.get(frame_index).copied() else {
            return StepOutcome::InstructionError(IonError::runtime(
                "missing tail-call frame",
                line,
                col,
            ));
        };
        cleanup_continuation_frame(cont, frame);
        frame.locals_base
    } else {
        cont.locals.len()
    };

    cont.env.push_scope();
    for (name, value) in &ion_fn.captures {
        cont.env.define(name.clone(), value.clone(), false);
    }

    let prepared = match prepare_continuation_function_args(cont, ion_fn, &supplied, line, col) {
        Ok(prepared) => prepared,
        Err(err) => {
            cont.env.pop_scope();
            return StepOutcome::InstructionError(err);
        }
    };

    for value in prepared {
        cont.locals.push(ContinuationLocalSlot {
            value,
            mutable: false,
        });
    }

    let next_frame = CallFrame {
        chunk,
        ip: 0,
        locals_base,
        captured_scope: true,
    };
    if let Some(frame_index) = tail_frame_index {
        cont.frames[frame_index] = next_frame;
    } else {
        cont.frames.push(next_frame);
    }
    StepOutcome::Continue
}

fn prepare_continuation_function_args(
    cont: &mut VmContinuation,
    ion_fn: &crate::value::IonFn,
    supplied: &[Option<Value>],
    line: usize,
    col: usize,
) -> Result<Vec<Value>, IonError> {
    let mut prepared = Vec::with_capacity(ion_fn.params.len());
    for (index, param) in ion_fn.params.iter().enumerate() {
        let value = if let Some(Some(value)) = supplied.get(index) {
            value.clone()
        } else if let Some(default) = &param.default {
            eval_continuation_default_arg(cont, &param.name, default, line, col)?
        } else if supplied.iter().any(Option::is_some) {
            return Err(IonError::runtime(
                format!("missing argument '{}'", param.name),
                line,
                col,
            ));
        } else {
            return Err(IonError::runtime(
                format!(
                    "function '{}' expected {} arguments, got 0",
                    ion_fn.name,
                    ion_fn.params.len()
                ),
                line,
                col,
            ));
        };
        cont.env.define(param.name.clone(), value.clone(), false);
        prepared.push(value);
    }
    Ok(prepared)
}

fn eval_continuation_default_arg(
    cont: &VmContinuation,
    param_name: &str,
    default: &Expr,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    let mut interpreter = crate::interpreter::Interpreter::with_env(cont.env.clone());
    interpreter.types = cont.types.clone();
    interpreter.eval_single_expr(default).map_err(|err| {
        IonError::runtime(
            format!(
                "error evaluating default for '{}': {}",
                param_name, err.message
            ),
            line,
            col,
        )
    })
}

fn build_sequence(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
    build: impl FnOnce(Vec<Value>) -> Value,
) -> StepOutcome {
    let count = match read_u16_operand(chunk, *next_ip, line, col, "truncated sequence operand") {
        Ok(count) => count as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 2;
    if cont.stack.len() < count {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let start = cont.stack.len() - count;
    let items = cont.stack.drain(start..).collect();
    cont.stack.push(build(items));
    StepOutcome::Continue
}

fn build_dict(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let count = match read_u16_operand(chunk, *next_ip, line, col, "truncated dict operand") {
        Ok(count) => count as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 2;
    let value_count = count.saturating_mul(2);
    if cont.stack.len() < value_count {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let start = cont.stack.len() - value_count;
    let items: Vec<Value> = cont.stack.drain(start..).collect();
    let mut map = IndexMap::new();
    for pair in items.chunks(2) {
        let key = match &pair[0] {
            Value::Str(value) => value.clone(),
            other => other.to_string(),
        };
        map.insert(key, pair[1].clone());
    }
    cont.stack.push(Value::Dict(map));
    StepOutcome::Continue
}

fn build_f_string(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let count = match read_u16_operand(chunk, *next_ip, line, col, "truncated f-string operand") {
        Ok(count) => count as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 2;
    if cont.stack.len() < count {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }

    let start = cont.stack.len() - count;
    let parts: Vec<Value> = cont.stack.drain(start..).collect();
    let mut output = String::new();
    for part in parts {
        use std::fmt::Write;
        let _ = write!(output, "{part}");
    }
    cont.stack.push(Value::Str(output));
    StepOutcome::Continue
}

fn define_env_local(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let name_idx = match read_u16_operand(chunk, *next_ip, line, col, "truncated define-local name")
    {
        Ok(idx) => idx as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let mutable = match read_u8_operand(
        chunk,
        *next_ip + 2,
        line,
        col,
        "truncated define-local mutability",
    ) {
        Ok(value) => value != 0,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 3;
    let Some(value) = chunk.constants.get(name_idx) else {
        return StepOutcome::InstructionError(IonError::runtime(
            "local name constant index out of bounds",
            line,
            col,
        ));
    };
    let name = match scaffold_const_as_str(value, line, col) {
        Ok(name) => name,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    match pop_stack(cont, line, col) {
        Ok(value) => {
            let symbol = cont.env.intern(&name);
            cont.env.define_sym(symbol, value, mutable);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(err),
    }
}

fn get_env_local(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let name_idx = match read_u16_operand(chunk, *next_ip, line, col, "truncated local get operand")
    {
        Ok(idx) => idx as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 2;
    let Some(value) = chunk.constants.get(name_idx) else {
        return StepOutcome::InstructionError(IonError::runtime(
            "local name constant index out of bounds",
            line,
            col,
        ));
    };
    let name = match scaffold_const_as_str(value, line, col) {
        Ok(name) => name,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let symbol = cont.env.intern(&name);
    match cont.env.get_sym_or_global(symbol).cloned() {
        Some(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        None => StepOutcome::InstructionError(IonError::name(
            format!("undefined variable: {}", cont.env.resolve(symbol)),
            line,
            col,
        )),
    }
}

fn import_glob(cont: &mut VmContinuation, line: usize, col: usize) -> StepOutcome {
    let module = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    match module {
        Value::Module(table) => {
            for (name_hash, value) in table.items.iter() {
                cont.env.define_h(*name_hash, value.clone());
            }
            StepOutcome::Continue
        }
        Value::Dict(map) => {
            for (name, value) in map {
                let symbol = cont.env.intern(&name);
                cont.env.define_sym(symbol, value, false);
            }
            StepOutcome::Continue
        }
        _ => StepOutcome::InstructionError(IonError::type_err(
            ion_str!("use target is not a module"),
            line,
            col,
        )),
    }
}

fn set_env_local(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let name_idx = match read_u16_operand(chunk, *next_ip, line, col, "truncated local set operand")
    {
        Ok(idx) => idx as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 2;
    let Some(name_value) = chunk.constants.get(name_idx) else {
        return StepOutcome::InstructionError(IonError::runtime(
            "local name constant index out of bounds",
            line,
            col,
        ));
    };
    let name = match scaffold_const_as_str(name_value, line, col) {
        Ok(name) => name,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let value = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let symbol = cont.env.intern(&name);
    match cont.env.set_sym(symbol, value.clone()) {
        Ok(()) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(IonError::runtime(err, line, col)),
    }
}

fn scaffold_const_as_str(value: &Value, line: usize, col: usize) -> Result<String, IonError> {
    match value {
        Value::Str(value) => Ok(value.clone()),
        _ => Err(IonError::runtime("expected string constant", line, col)),
    }
}

fn scaffold_get_field(
    object: Value,
    field: &str,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match &object {
        Value::Dict(map) => Ok(map
            .get(field)
            .cloned()
            .unwrap_or_else(|| Value::Option(None))),
        Value::Module(table) => {
            let fh = crate::hash::h(field);
            table.items.get(&fh).cloned().ok_or_else(|| {
                IonError::runtime(format!("'{}' not found in module", field), line, col)
            })
        }
        Value::HostStruct { fields, .. } => {
            let fh = crate::hash::h(field);
            fields
                .get(&fh)
                .cloned()
                .ok_or_else(|| IonError::runtime(format!("field '{}' not found", field), line, col))
        }
        Value::List(items) if field == "len" => Ok(Value::Int(items.len() as i64)),
        Value::Str(value) if field == "len" => Ok(Value::Int(value.len() as i64)),
        Value::Tuple(items) if field == "len" => Ok(Value::Int(items.len() as i64)),
        Value::List(_) => Err(IonError::runtime(
            format!("list has no field '{}'", field),
            line,
            col,
        )),
        Value::Str(_) => Err(IonError::runtime(
            format!("string has no field '{}'", field),
            line,
            col,
        )),
        Value::Tuple(_) => Err(IonError::runtime(
            format!("tuple has no field '{}'", field),
            line,
            col,
        )),
        _ => Err(IonError::type_err(
            format!("cannot access field '{}' on {}", field, object.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_get_index(
    object: Value,
    index: Value,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match (&object, &index) {
        (Value::List(items), Value::Int(index)) => {
            let idx = if *index < 0 {
                items.len() as i64 + index
            } else {
                *index
            } as usize;
            items.get(idx).cloned().ok_or_else(|| {
                IonError::runtime(format!("index {} out of range", index), line, col)
            })
        }
        (Value::Tuple(items), Value::Int(index)) => {
            let idx = if *index < 0 {
                items.len() as i64 + index
            } else {
                *index
            } as usize;
            items.get(idx).cloned().ok_or_else(|| {
                IonError::runtime(format!("index {} out of range", index), line, col)
            })
        }
        (Value::Dict(map), Value::Str(key)) => {
            Ok(map.get(key).cloned().unwrap_or_else(|| Value::Option(None)))
        }
        (Value::Str(value), Value::Int(index)) => {
            let len = value.chars().count() as i64;
            let idx = if *index < 0 { len + index } else { *index } as usize;
            value
                .chars()
                .nth(idx)
                .map(|value| Value::Str(value.to_string()))
                .ok_or_else(|| {
                    IonError::runtime(format!("index {} out of range", index), line, col)
                })
        }
        (Value::Bytes(bytes), Value::Int(index)) => {
            let idx = if *index < 0 {
                bytes.len() as i64 + index
            } else {
                *index
            } as usize;
            bytes
                .get(idx)
                .map(|value| Value::Int(*value as i64))
                .ok_or_else(|| {
                    IonError::runtime(format!("index {} out of range", index), line, col)
                })
        }
        _ => Err(IonError::type_err(
            format!(
                "cannot index {} with {}",
                object.type_name(),
                index.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_set_field(
    object: Value,
    field: &str,
    value: Value,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match object {
        Value::Dict(mut map) => {
            map.insert(field.to_string(), value);
            Ok(Value::Dict(map))
        }
        Value::HostStruct {
            type_hash,
            mut fields,
        } => {
            let fh = crate::hash::h(field);
            if fields.contains_key(&fh) {
                fields.insert(fh, value);
                Ok(Value::HostStruct { type_hash, fields })
            } else {
                Err(IonError::runtime(
                    format!("field '{}' not found on host struct", field),
                    line,
                    col,
                ))
            }
        }
        _ => Err(IonError::type_err(
            format!("cannot set field on {}", object.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_set_index(
    object: Value,
    index: Value,
    value: Value,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match (object, &index) {
        (Value::List(mut items), Value::Int(index)) => {
            let idx = if *index < 0 {
                items.len() as i64 + index
            } else {
                *index
            } as usize;
            if idx >= items.len() {
                return Err(IonError::runtime(
                    format!("index {} out of range", index),
                    line,
                    col,
                ));
            }
            items[idx] = value;
            Ok(Value::List(items))
        }
        (Value::Dict(mut map), Value::Str(key)) => {
            map.insert(key.clone(), value);
            Ok(Value::Dict(map))
        }
        (object, _) => Err(IonError::type_err(
            format!("cannot set index on {}", object.type_name()),
            line,
            col,
        )),
    }
}

fn start_continuation_method_call(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    receiver: Value,
    method: &str,
    args: Vec<Value>,
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    let func = match args.first() {
        Some(func) => func.clone(),
        None => {
            return StepOutcome::InstructionError(IonError::runtime(
                format!("{} requires a function argument", method),
                line,
                col,
            ))
        }
    };

    let method = match (receiver, method) {
        (Value::List(items), "map") => MethodContinuationKind::ListMap {
            func,
            items,
            index: 0,
            result: Vec::new(),
        },
        (Value::List(items), "filter") => MethodContinuationKind::ListFilter {
            func,
            items,
            index: 0,
            result: Vec::new(),
        },
        (Value::List(items), "any") => MethodContinuationKind::ListAny {
            func,
            items,
            index: 0,
            found: false,
        },
        (Value::List(items), "all") => MethodContinuationKind::ListAll {
            func,
            items,
            index: 0,
            failed: false,
        },
        (Value::List(items), "flat_map") => MethodContinuationKind::ListFlatMap {
            func,
            items,
            index: 0,
            result: Vec::new(),
        },
        (Value::List(items), "fold") => {
            let init = args.first().cloned().unwrap_or(Value::Unit);
            let func = args.get(1).cloned().ok_or_else(|| {
                IonError::runtime("fold requires an initial value and a function", line, col)
            });
            let func = match func {
                Ok(func) => func,
                Err(err) => return StepOutcome::InstructionError(err),
            };
            MethodContinuationKind::ListFold {
                func,
                items,
                index: 0,
                acc: init,
            }
        }
        (Value::List(items), "reduce") => {
            let Some((first, rest)) = items.split_first() else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "reduce on empty list",
                    line,
                    col,
                ));
            };
            MethodContinuationKind::ListFold {
                func,
                items: rest.to_vec(),
                index: 0,
                acc: first.clone(),
            }
        }
        (Value::List(items), "sort_by") => MethodContinuationKind::ListSortBy {
            func,
            items,
            pass: 0,
            index: 0,
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "map",
        ) => MethodContinuationKind::ListMap {
            func,
            items: Value::range_to_list(start, end, inclusive),
            index: 0,
            result: Vec::new(),
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "filter",
        ) => MethodContinuationKind::ListFilter {
            func,
            items: Value::range_to_list(start, end, inclusive),
            index: 0,
            result: Vec::new(),
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "any",
        ) => MethodContinuationKind::ListAny {
            func,
            items: Value::range_to_list(start, end, inclusive),
            index: 0,
            found: false,
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "all",
        ) => MethodContinuationKind::ListAll {
            func,
            items: Value::range_to_list(start, end, inclusive),
            index: 0,
            failed: false,
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "flat_map",
        ) => MethodContinuationKind::ListFlatMap {
            func,
            items: Value::range_to_list(start, end, inclusive),
            index: 0,
            result: Vec::new(),
        },
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "fold",
        ) => {
            let init = args.first().cloned().unwrap_or(Value::Unit);
            let func = args.get(1).cloned().ok_or_else(|| {
                IonError::runtime("fold requires an initial value and a function", line, col)
            });
            let func = match func {
                Ok(func) => func,
                Err(err) => return StepOutcome::InstructionError(err),
            };
            MethodContinuationKind::ListFold {
                func,
                items: Value::range_to_list(start, end, inclusive),
                index: 0,
                acc: init,
            }
        }
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "reduce",
        ) => {
            let items = Value::range_to_list(start, end, inclusive);
            let Some((first, rest)) = items.split_first() else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "reduce on empty list",
                    line,
                    col,
                ));
            };
            MethodContinuationKind::ListFold {
                func,
                items: rest.to_vec(),
                index: 0,
                acc: first.clone(),
            }
        }
        (
            Value::Range {
                start,
                end,
                inclusive,
            },
            "sort_by",
        ) => MethodContinuationKind::ListSortBy {
            func,
            items: Value::range_to_list(start, end, inclusive),
            pass: 0,
            index: 0,
        },
        (Value::Option(Some(value)), "map") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::WrapSome,
        },
        (Value::Option(None), "map" | "and_then") => {
            cont.stack.push(Value::Option(None));
            return StepOutcome::Continue;
        }
        (Value::Option(Some(value)), "and_then") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::Direct,
        },
        (Value::Option(Some(value)), "or_else") => {
            cont.stack.push(Value::Option(Some(value)));
            return StepOutcome::Continue;
        }
        (Value::Option(None), "or_else") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![],
            started: false,
            after: MethodSingleCallbackAfter::Direct,
        },
        (Value::Option(Some(value)), "unwrap_or_else") => {
            cont.stack.push(*value);
            return StepOutcome::Continue;
        }
        (Value::Option(None), "unwrap_or_else") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![],
            started: false,
            after: MethodSingleCallbackAfter::Direct,
        },
        (Value::Result(Ok(value)), "map") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::WrapOk,
        },
        (Value::Result(Err(value)), "map" | "and_then") => {
            cont.stack.push(Value::Result(Err(value)));
            return StepOutcome::Continue;
        }
        (Value::Result(Err(value)), "map_err") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::WrapErr,
        },
        (Value::Result(Ok(value)), "map_err" | "or_else") => {
            cont.stack.push(Value::Result(Ok(value)));
            return StepOutcome::Continue;
        }
        (Value::Result(Ok(value)), "and_then") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::Direct,
        },
        (Value::Result(Err(value)), "or_else") => MethodContinuationKind::SingleCallback {
            func,
            args: vec![*value],
            started: false,
            after: MethodSingleCallbackAfter::Direct,
        },
        (Value::Result(Ok(value)), "unwrap_or_else") => {
            cont.stack.push(*value);
            return StepOutcome::Continue;
        }
        (Value::Result(Err(value)), "unwrap_or_else") => {
            MethodContinuationKind::SingleCallback {
                func,
                args: vec![*value],
                started: false,
                after: MethodSingleCallbackAfter::Direct,
            }
        }
        (Value::Cell(cell), "update") => {
            let current = cell.lock().unwrap().clone();
            MethodContinuationKind::SingleCallback {
                func,
                args: vec![current],
                started: false,
                after: MethodSingleCallbackAfter::CellSet(cell),
            }
        }
        (Value::Dict(map), "map") => MethodContinuationKind::DictMap {
            func,
            entries: map.into_iter().collect(),
            index: 0,
            result: IndexMap::new(),
        },
        (Value::Dict(map), "filter") => MethodContinuationKind::DictFilter {
            func,
            entries: map.into_iter().collect(),
            index: 0,
            result: IndexMap::new(),
        },
        (receiver, method) => {
            return StepOutcome::InstructionError(IonError::runtime(
                format!(
                    "method '{}.{}' requires nested function invocation; async VM continuation dispatch did not route this closure-based method",
                    receiver.type_name(),
                    method
                ),
                line,
                col,
            ))
        }
    };

    cont.pending_method = Some(MethodContinuation {
        parent_frame_depth: cont.frames.len(),
        line,
        col,
        method,
    });
    step_next_method_callback(arena, cont, task, host_futures)
}

fn step_pending_method_continuation(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> Option<StepOutcome> {
    let pending = cont.pending_method.as_ref()?;
    if cont.frames.len() != pending.parent_frame_depth {
        return None;
    }

    let line = pending.line;
    let col = pending.col;
    let callback_result = match pop_stack(cont, line, col) {
        Ok(value) => value,
        Err(err) => return Some(StepOutcome::InstructionError(err)),
    };

    if let Err(err) = apply_method_callback_result(cont, callback_result) {
        return Some(StepOutcome::InstructionError(err));
    }

    Some(step_next_method_callback(arena, cont, task, host_futures))
}

enum MethodStepAction {
    Call { func: Value, args: Vec<Value> },
    Complete(Value),
}

fn apply_method_callback_result(
    cont: &mut VmContinuation,
    callback_result: Value,
) -> Result<(), IonError> {
    let Some(pending) = cont.pending_method.as_mut() else {
        return Ok(());
    };
    match &mut pending.method {
        MethodContinuationKind::Complete(_) => {}
        MethodContinuationKind::SingleCallback { after, .. } => {
            let value = match after {
                MethodSingleCallbackAfter::Direct => callback_result,
                MethodSingleCallbackAfter::WrapSome => {
                    Value::Option(Some(Box::new(callback_result)))
                }
                MethodSingleCallbackAfter::WrapOk => Value::Result(Ok(Box::new(callback_result))),
                MethodSingleCallbackAfter::WrapErr => Value::Result(Err(Box::new(callback_result))),
                MethodSingleCallbackAfter::CellSet(cell) => {
                    *cell.lock().unwrap() = callback_result.clone();
                    callback_result
                }
            };
            pending.method = MethodContinuationKind::Complete(value);
        }
        MethodContinuationKind::ListMap { result, .. } => {
            result.push(callback_result);
        }
        MethodContinuationKind::ListFilter {
            items,
            index,
            result,
            ..
        } => {
            let item_index = index.saturating_sub(1);
            let Some(item) = items.get(item_index).cloned() else {
                return Err(IonError::runtime(
                    "method callback result arrived without a pending item",
                    pending.line,
                    pending.col,
                ));
            };
            if callback_result.is_truthy() {
                result.push(item);
            }
        }
        MethodContinuationKind::ListAny { found, .. } => {
            if callback_result.is_truthy() {
                *found = true;
            }
        }
        MethodContinuationKind::ListAll { failed, .. } => {
            if !callback_result.is_truthy() {
                *failed = true;
            }
        }
        MethodContinuationKind::ListFlatMap { result, .. } => match callback_result {
            Value::List(items) => result.extend(items),
            value => result.push(value),
        },
        MethodContinuationKind::ListFold { acc, .. } => {
            *acc = callback_result;
        }
        MethodContinuationKind::ListSortBy { items, index, .. } => {
            let Some(ordering) = callback_result.as_int() else {
                return Err(IonError::type_err(
                    "sort_by function must return int",
                    pending.line,
                    pending.col,
                ));
            };
            if *index + 1 >= items.len() {
                return Err(IonError::runtime(
                    "method callback result arrived without a pending sort comparison",
                    pending.line,
                    pending.col,
                ));
            }
            if ordering > 0 {
                items.swap(*index, *index + 1);
            }
            *index += 1;
        }
        MethodContinuationKind::DictMap {
            entries,
            index,
            result,
            ..
        } => {
            let item_index = index.saturating_sub(1);
            let Some((key, _)) = entries.get(item_index) else {
                return Err(IonError::runtime(
                    "method callback result arrived without a pending dict entry",
                    pending.line,
                    pending.col,
                ));
            };
            result.insert(key.clone(), callback_result);
        }
        MethodContinuationKind::DictFilter {
            entries,
            index,
            result,
            ..
        } => {
            let item_index = index.saturating_sub(1);
            let Some((key, value)) = entries.get(item_index).cloned() else {
                return Err(IonError::runtime(
                    "method callback result arrived without a pending dict entry",
                    pending.line,
                    pending.col,
                ));
            };
            if callback_result.is_truthy() {
                result.insert(key, value);
            }
        }
    }
    Ok(())
}

fn next_method_step_action(cont: &mut VmContinuation) -> Option<MethodStepAction> {
    let pending = cont.pending_method.as_mut()?;
    match &mut pending.method {
        MethodContinuationKind::Complete(value) => {
            let value = value.clone();
            cont.pending_method = None;
            Some(MethodStepAction::Complete(value))
        }
        MethodContinuationKind::SingleCallback {
            func,
            args,
            started,
            ..
        } => {
            if *started {
                return None;
            }
            *started = true;
            Some(MethodStepAction::Call {
                func: func.clone(),
                args: args.clone(),
            })
        }
        MethodContinuationKind::ListMap {
            func,
            items,
            index,
            result,
        } => {
            if *index >= items.len() {
                let result = std::mem::take(result);
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::List(result)))
            } else {
                let item = items[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![item],
                })
            }
        }
        MethodContinuationKind::ListFilter {
            func,
            items,
            index,
            result,
        } => {
            if *index >= items.len() {
                let result = std::mem::take(result);
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::List(result)))
            } else {
                let item = items[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![item],
                })
            }
        }
        MethodContinuationKind::ListAny {
            func,
            items,
            index,
            found,
        } => {
            if *found || *index >= items.len() {
                let value = *found;
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::Bool(value)))
            } else {
                let item = items[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![item],
                })
            }
        }
        MethodContinuationKind::ListAll {
            func,
            items,
            index,
            failed,
        } => {
            if *failed || *index >= items.len() {
                let value = !*failed;
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::Bool(value)))
            } else {
                let item = items[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![item],
                })
            }
        }
        MethodContinuationKind::ListFlatMap {
            func,
            items,
            index,
            result,
        } => {
            if *index >= items.len() {
                let result = std::mem::take(result);
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::List(result)))
            } else {
                let item = items[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![item],
                })
            }
        }
        MethodContinuationKind::ListFold {
            func,
            items,
            index,
            acc,
        } => {
            if *index >= items.len() {
                let result = acc.clone();
                cont.pending_method = None;
                Some(MethodStepAction::Complete(result))
            } else {
                let item = items[*index].clone();
                let acc = acc.clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![acc, item],
                })
            }
        }
        MethodContinuationKind::ListSortBy {
            func,
            items,
            pass,
            index,
        } => {
            let len = items.len();
            if len < 2 || *pass >= len - 1 {
                let result = std::mem::take(items);
                cont.pending_method = None;
                return Some(MethodStepAction::Complete(Value::List(result)));
            }

            while *index + 1 >= len - *pass {
                *pass += 1;
                *index = 0;
                if *pass >= len - 1 {
                    let result = std::mem::take(items);
                    cont.pending_method = None;
                    return Some(MethodStepAction::Complete(Value::List(result)));
                }
            }

            let left = items[*index].clone();
            let right = items[*index + 1].clone();
            Some(MethodStepAction::Call {
                func: func.clone(),
                args: vec![left, right],
            })
        }
        MethodContinuationKind::DictMap {
            func,
            entries,
            index,
            result,
        } => {
            if *index >= entries.len() {
                let result = std::mem::take(result);
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::Dict(result)))
            } else {
                let (key, value) = entries[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![Value::Str(key), value],
                })
            }
        }
        MethodContinuationKind::DictFilter {
            func,
            entries,
            index,
            result,
        } => {
            if *index >= entries.len() {
                let result = std::mem::take(result);
                cont.pending_method = None;
                Some(MethodStepAction::Complete(Value::Dict(result)))
            } else {
                let (key, value) = entries[*index].clone();
                *index += 1;
                Some(MethodStepAction::Call {
                    func: func.clone(),
                    args: vec![Value::Str(key), value],
                })
            }
        }
    }
}

fn step_next_method_callback(
    arena: &ChunkArena,
    cont: &mut VmContinuation,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    let Some(action) = next_method_step_action(cont) else {
        return StepOutcome::InstructionError(IonError::runtime(
            "missing method continuation",
            0,
            0,
        ));
    };

    match action {
        MethodStepAction::Complete(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        MethodStepAction::Call { func, args } => {
            let (line, col) = cont
                .pending_method
                .as_ref()
                .map(|pending| (pending.line, pending.col))
                .unwrap_or((0, 0));
            cont.stack.push(func);
            let arg_count = args.len();
            cont.stack.extend(args);
            match call_continuation_function(
                arena,
                cont,
                arg_count,
                line,
                col,
                task,
                host_futures,
                None,
            ) {
                StepOutcome::InstructionError(err) => {
                    cont.pending_method = None;
                    StepOutcome::InstructionError(err)
                }
                outcome => outcome,
            }
        }
    }
}

fn scaffold_call_method(
    receiver: Value,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    if method == "to_string" {
        return Ok(Value::Str(receiver.to_string()));
    }

    if scaffold_is_closure_method(&receiver, method) {
        return Err(IonError::runtime(
            format!(
                "method '{}.{}' requires nested function invocation; async VM continuation dispatch did not route this closure-based method",
                receiver.type_name(),
                method
            ),
            line,
            col,
        ));
    }

    match &receiver {
        Value::List(items) => scaffold_list_method(items, method, args, line, col),
        Value::Tuple(items) => scaffold_tuple_method(items, method, args, line, col),
        Value::Str(value) => scaffold_str_method(value, method, args, line, col),
        Value::Dict(map) => scaffold_dict_method(map, method, args, line, col),
        Value::Bytes(bytes) => scaffold_bytes_method(bytes, method, args, line, col),
        Value::Set(items) => scaffold_set_method(items, method, args, line, col),
        Value::Option(_) => scaffold_option_method(&receiver, method, args, line, col),
        Value::Result(_) => scaffold_result_method(&receiver, method, args, line, col),
        Value::Range {
            start,
            end,
            inclusive,
        } => match method {
            "len" => Ok(Value::Int(Value::range_len(*start, *end, *inclusive))),
            "contains" => {
                let value = args
                    .first()
                    .and_then(Value::as_int)
                    .ok_or_else(|| IonError::type_err("range.contains requires int", line, col))?;
                let in_range = if *inclusive {
                    value >= *start && value <= *end
                } else {
                    value >= *start && value < *end
                };
                Ok(Value::Bool(in_range))
            }
            "to_list" => Ok(Value::List(Value::range_to_list(*start, *end, *inclusive))),
            _ => {
                let items = Value::range_to_list(*start, *end, *inclusive);
                scaffold_list_method(&items, method, args, line, col)
            }
        },
        Value::Cell(cell) => match method {
            "get" => Ok(cell.lock().unwrap().clone()),
            "set" => {
                let value = args.first().cloned().ok_or_else(|| {
                    IonError::runtime("cell.set() requires 1 argument", line, col)
                })?;
                *cell.lock().unwrap() = value;
                Ok(Value::Unit)
            }
            _ => Err(IonError::type_err(
                format!("no method '{}' on cell", method),
                line,
                col,
            )),
        },
        _ => Err(IonError::type_err(
            format!("{} has no method '{}'", receiver.type_name(), method),
            line,
            col,
        )),
    }
}

fn call_native_async_channel_method(
    cont: &mut VmContinuation,
    receiver: Value,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> Option<StepOutcome> {
    match receiver {
        Value::AsyncChannelSender(sender) => Some(call_native_async_sender_method(
            cont,
            sender,
            method,
            args,
            line,
            col,
            task,
            host_futures,
        )),
        Value::AsyncChannelReceiver(receiver) => Some(call_native_async_receiver_method(
            cont,
            receiver,
            method,
            args,
            line,
            col,
            task,
            host_futures,
        )),
        _ => None,
    }
}

fn call_native_async_sender_method(
    cont: &mut VmContinuation,
    sender: NativeChannelSender,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    match method {
        "send" => {
            let Some(value) = args.first().cloned() else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "send requires a value",
                    line,
                    col,
                ));
            };
            let Some(sender) = sender.sender() else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "channel is closed",
                    line,
                    col,
                ));
            };
            suspend_native_channel_future(
                task,
                host_futures,
                line,
                col,
                Box::pin(async move {
                    sender.send(value).await.map_err(|_| {
                        IonError::runtime("channel send failed: channel is closed", line, col)
                    })?;
                    Ok(Value::Unit)
                }),
            )
        }
        "close" => {
            sender.close();
            cont.stack.push(Value::Unit);
            StepOutcome::Continue
        }
        _ => StepOutcome::InstructionError(IonError::type_err(
            format!("no method '{}' on AsyncChannelSender", method),
            line,
            col,
        )),
    }
}

fn call_native_async_receiver_method(
    cont: &mut VmContinuation,
    receiver: NativeChannelReceiver,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
) -> StepOutcome {
    match method {
        "recv" => suspend_native_channel_future(
            task,
            host_futures,
            line,
            col,
            Box::pin(async move {
                let mut receiver = receiver.inner.lock().await;
                Ok(Value::Option(
                    receiver.recv().await.map(|value| Box::new(value)),
                ))
            }),
        ),
        "try_recv" => match receiver.inner.try_lock() {
            Ok(mut receiver) => match receiver.try_recv() {
                Ok(value) => {
                    cont.stack.push(Value::Option(Some(Box::new(value))));
                    StepOutcome::Continue
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    cont.stack.push(Value::Option(None));
                    StepOutcome::Continue
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    cont.stack.push(Value::Option(None));
                    StepOutcome::Continue
                }
            },
            Err(_) => StepOutcome::InstructionError(IonError::runtime(
                "channel receiver is already waiting",
                line,
                col,
            )),
        },
        "recv_timeout" => {
            let Some(ms) = args.first().and_then(Value::as_int) else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "recv_timeout requires int (ms)",
                    line,
                    col,
                ));
            };
            suspend_native_channel_future(
                task,
                host_futures,
                line,
                col,
                Box::pin(async move {
                    let recv = async {
                        let mut receiver = receiver.inner.lock().await;
                        receiver.recv().await
                    };
                    match tokio::time::timeout(Duration::from_millis(ms as u64), recv).await {
                        Ok(Some(value)) => Ok(Value::Option(Some(Box::new(value)))),
                        Ok(None) | Err(_) => Ok(Value::Option(None)),
                    }
                }),
            )
        }
        "close" => suspend_native_channel_future(
            task,
            host_futures,
            line,
            col,
            Box::pin(async move {
                let mut receiver = receiver.inner.lock().await;
                receiver.close();
                Ok(Value::Unit)
            }),
        ),
        _ => StepOutcome::InstructionError(IonError::type_err(
            format!("no method '{}' on AsyncChannelReceiver", method),
            line,
            col,
        )),
    }
}

fn suspend_native_channel_future(
    task: Option<TaskId>,
    host_futures: Option<&mut HostFutureTable>,
    line: usize,
    col: usize,
    future: BoxIonFuture,
) -> StepOutcome {
    let Some(task) = task else {
        return StepOutcome::InstructionError(IonError::runtime(
            "async channel method requires a runtime task",
            line,
            col,
        ));
    };
    let Some(host_futures) = host_futures else {
        return StepOutcome::InstructionError(IonError::runtime(
            "async channel method requires a host future table",
            line,
            col,
        ));
    };
    let future_id = host_futures.insert(task, future);
    StepOutcome::Suspended(TaskState::WaitingHostFuture(future_id))
}

fn scaffold_is_closure_method(receiver: &Value, method: &str) -> bool {
    matches!(
        (receiver, method),
        (
            Value::List(_),
            "map" | "filter" | "fold" | "reduce" | "flat_map" | "any" | "all" | "sort_by"
        ) | (
            Value::Range { .. },
            "map" | "filter" | "fold" | "reduce" | "flat_map" | "any" | "all" | "sort_by"
        ) | (Value::Dict(_), "map" | "filter")
            | (
                Value::Option(_),
                "map" | "and_then" | "or_else" | "unwrap_or_else"
            )
            | (
                Value::Result(_),
                "map" | "map_err" | "and_then" | "or_else" | "unwrap_or_else"
            )
            | (Value::Cell(_), "update")
    )
}

fn scaffold_list_method(
    items: &[Value],
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(items.len() as i64)),
        "push" => {
            let mut new = items.to_vec();
            new.extend(args.iter().cloned());
            Ok(Value::List(new))
        }
        "pop" => {
            if items.is_empty() {
                Ok(Value::Tuple(vec![Value::List(vec![]), Value::Option(None)]))
            } else {
                let mut new = items.to_vec();
                let popped = new.pop().unwrap();
                Ok(Value::Tuple(vec![
                    Value::List(new),
                    Value::Option(Some(Box::new(popped))),
                ]))
            }
        }
        "contains" => Ok(Value::Bool(
            args.first().is_some_and(|arg| items.contains(arg)),
        )),
        "is_empty" => Ok(Value::Bool(items.is_empty())),
        "reverse" => {
            let mut new = items.to_vec();
            new.reverse();
            Ok(Value::List(new))
        }
        "join" => {
            let sep = args.first().and_then(Value::as_str).unwrap_or("");
            Ok(Value::Str(
                items
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(sep),
            ))
        }
        "enumerate" => Ok(Value::List(
            items
                .iter()
                .enumerate()
                .map(|(index, value)| Value::Tuple(vec![Value::Int(index as i64), value.clone()]))
                .collect(),
        )),
        "first" => Ok(items
            .first()
            .cloned()
            .map(|value| Value::Option(Some(Box::new(value))))
            .unwrap_or(Value::Option(None))),
        "last" => Ok(items
            .last()
            .cloned()
            .map(|value| Value::Option(Some(Box::new(value))))
            .unwrap_or(Value::Option(None))),
        "sort" => scaffold_sort_list(items, line, col),
        "flatten" => {
            let mut result = Vec::new();
            for item in items {
                if let Value::List(inner) = item {
                    result.extend(inner.iter().cloned());
                } else {
                    result.push(item.clone());
                }
            }
            Ok(Value::List(result))
        }
        "zip" => {
            let Some(Value::List(other)) = args.first() else {
                return Err(IonError::type_err(
                    "zip requires a list argument",
                    line,
                    col,
                ));
            };
            Ok(Value::List(
                items
                    .iter()
                    .zip(other.iter())
                    .map(|(left, right)| Value::Tuple(vec![left.clone(), right.clone()]))
                    .collect(),
            ))
        }
        "index" => {
            let target = args
                .first()
                .ok_or_else(|| IonError::type_err("index requires an argument", line, col))?;
            Ok(match items.iter().position(|value| value == target) {
                Some(index) => Value::Option(Some(Box::new(Value::Int(index as i64)))),
                None => Value::Option(None),
            })
        }
        "count" => {
            let target = args
                .first()
                .ok_or_else(|| IonError::type_err("count requires an argument", line, col))?;
            Ok(Value::Int(
                items.iter().filter(|value| *value == target).count() as i64,
            ))
        }
        "slice" => {
            let start = args.first().and_then(Value::as_int).unwrap_or(0) as usize;
            let end = args
                .get(1)
                .and_then(Value::as_int)
                .map(|value| value as usize)
                .unwrap_or(items.len());
            Ok(Value::List(
                items[start.min(items.len())..end.min(items.len())].to_vec(),
            ))
        }
        "dedup" => {
            let mut result = Vec::new();
            for item in items {
                if result.last() != Some(item) {
                    result.push(item.clone());
                }
            }
            Ok(Value::List(result))
        }
        "unique" => {
            let mut seen = Vec::new();
            let mut result = Vec::new();
            for item in items {
                if !seen.contains(item) {
                    seen.push(item.clone());
                    result.push(item.clone());
                }
            }
            Ok(Value::List(result))
        }
        "min" => scaffold_min_max_list(items, true, line, col),
        "max" => scaffold_min_max_list(items, false, line, col),
        "sum" => scaffold_sum_list(items, line, col),
        "window" => {
            let size = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("window requires int argument", line, col))?
                as usize;
            if size == 0 {
                return Err(IonError::runtime("window size must be > 0", line, col));
            }
            Ok(Value::List(
                items
                    .windows(size)
                    .map(|window| Value::List(window.to_vec()))
                    .collect(),
            ))
        }
        "chunk" => {
            let size = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("chunk requires int argument", line, col))?
                as usize;
            if size == 0 {
                return Err(IonError::type_err("chunk size must be > 0", line, col));
            }
            Ok(Value::List(
                items
                    .chunks(size)
                    .map(|chunk| Value::List(chunk.to_vec()))
                    .collect(),
            ))
        }
        _ => Err(IonError::type_err(
            format!("list has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_tuple_method(
    items: &[Value],
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(items.len() as i64)),
        "contains" => Ok(Value::Bool(
            args.first().is_some_and(|arg| items.contains(arg)),
        )),
        "to_list" => Ok(Value::List(items.to_vec())),
        _ => Err(IonError::type_err(
            format!("tuple has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_str_method(
    value: &str,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(value.len() as i64)),
        "to_upper" => Ok(Value::Str(value.to_uppercase())),
        "to_lower" => Ok(Value::Str(value.to_lowercase())),
        "trim" => Ok(Value::Str(value.trim().to_string())),
        "contains" => match args.first() {
            Some(Value::Str(needle)) => Ok(Value::Bool(value.contains(needle.as_str()))),
            Some(Value::Int(code)) => {
                let ch = char::from_u32(*code as u32)
                    .ok_or_else(|| IonError::type_err("invalid char code", line, col))?;
                Ok(Value::Bool(value.contains(ch)))
            }
            _ => Err(IonError::type_err(
                "contains requires string or int argument",
                line,
                col,
            )),
        },
        "starts_with" => Ok(Value::Bool(
            value.starts_with(args.first().and_then(Value::as_str).unwrap_or("")),
        )),
        "ends_with" => Ok(Value::Bool(
            value.ends_with(args.first().and_then(Value::as_str).unwrap_or("")),
        )),
        "split" => Ok(Value::List(
            value
                .split(args.first().and_then(Value::as_str).unwrap_or(" "))
                .map(|part| Value::Str(part.to_string()))
                .collect(),
        )),
        "replace" => Ok(Value::Str(value.replace(
            args.first().and_then(Value::as_str).unwrap_or(""),
            args.get(1).and_then(Value::as_str).unwrap_or(""),
        ))),
        "chars" => Ok(Value::List(
            value.chars().map(|ch| Value::Str(ch.to_string())).collect(),
        )),
        "char_len" => Ok(Value::Int(value.chars().count() as i64)),
        "is_empty" => Ok(Value::Bool(value.is_empty())),
        "trim_start" => Ok(Value::Str(value.trim_start().to_string())),
        "trim_end" => Ok(Value::Str(value.trim_end().to_string())),
        "repeat" => {
            let count = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("repeat requires int argument", line, col))?;
            Ok(Value::Str(value.repeat(count as usize)))
        }
        "find" => {
            let needle = args.first().and_then(Value::as_str).unwrap_or("");
            Ok(match value.find(needle) {
                Some(byte_idx) => {
                    let char_idx = value[..byte_idx].chars().count();
                    Value::Option(Some(Box::new(Value::Int(char_idx as i64))))
                }
                None => Value::Option(None),
            })
        }
        "to_int" => Ok(match value.trim().parse::<i64>() {
            Ok(number) => Value::Result(Ok(Box::new(Value::Int(number)))),
            Err(err) => Value::Result(Err(Box::new(Value::Str(err.to_string())))),
        }),
        "to_float" => Ok(match value.trim().parse::<f64>() {
            Ok(number) => Value::Result(Ok(Box::new(Value::Float(number)))),
            Err(err) => Value::Result(Err(Box::new(Value::Str(err.to_string())))),
        }),
        "bytes" => Ok(Value::List(
            value.bytes().map(|byte| Value::Int(byte as i64)).collect(),
        )),
        "strip_prefix" => {
            let prefix = args.first().and_then(Value::as_str).unwrap_or("");
            Ok(Value::Str(
                value.strip_prefix(prefix).unwrap_or(value).to_string(),
            ))
        }
        "strip_suffix" => {
            let suffix = args.first().and_then(Value::as_str).unwrap_or("");
            Ok(Value::Str(
                value.strip_suffix(suffix).unwrap_or(value).to_string(),
            ))
        }
        "pad_start" => scaffold_pad_string(value, args, true, line, col),
        "pad_end" => scaffold_pad_string(value, args, false, line, col),
        "reverse" => Ok(Value::Str(value.chars().rev().collect())),
        "slice" => {
            let chars: Vec<char> = value.chars().collect();
            let start = args.first().and_then(Value::as_int).unwrap_or(0) as usize;
            let end = args
                .get(1)
                .and_then(Value::as_int)
                .map(|value| value as usize)
                .unwrap_or(chars.len());
            Ok(Value::Str(
                chars[start.min(chars.len())..end.min(chars.len())]
                    .iter()
                    .collect(),
            ))
        }
        _ => Err(IonError::type_err(
            format!("string has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_dict_method(
    map: &IndexMap<String, Value>,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(map.len() as i64)),
        "keys" => Ok(Value::List(
            map.keys().map(|key| Value::Str(key.clone())).collect(),
        )),
        "values" => Ok(Value::List(map.values().cloned().collect())),
        "contains_key" => Ok(Value::Bool(
            map.contains_key(args.first().and_then(Value::as_str).unwrap_or("")),
        )),
        "get" => {
            let key = args.first().and_then(Value::as_str).unwrap_or("");
            Ok(map
                .get(key)
                .cloned()
                .map(|value| Value::Option(Some(Box::new(value))))
                .unwrap_or(Value::Option(None)))
        }
        "is_empty" => Ok(Value::Bool(map.is_empty())),
        "entries" => Ok(Value::List(
            map.iter()
                .map(|(key, value)| Value::Tuple(vec![Value::Str(key.clone()), value.clone()]))
                .collect(),
        )),
        "insert" => {
            let key = args.first().and_then(Value::as_str).unwrap_or("");
            let value = args.get(1).cloned().unwrap_or(Value::Unit);
            let mut new = map.clone();
            new.insert(key.to_string(), value);
            Ok(Value::Dict(new))
        }
        "remove" => {
            let key = args.first().and_then(Value::as_str).unwrap_or("");
            let mut new = map.clone();
            new.shift_remove(key);
            Ok(Value::Dict(new))
        }
        "merge" | "update" => {
            let Some(Value::Dict(other)) = args.first() else {
                return Err(IonError::type_err(
                    format!("{} requires a dict argument", method),
                    line,
                    col,
                ));
            };
            let mut new = map.clone();
            for (key, value) in other {
                new.insert(key.clone(), value.clone());
            }
            Ok(Value::Dict(new))
        }
        "keys_of" => {
            let target = args
                .first()
                .ok_or_else(|| IonError::type_err("keys_of requires an argument", line, col))?;
            Ok(Value::List(
                map.iter()
                    .filter(|(_, value)| *value == target)
                    .map(|(key, _)| Value::Str(key.clone()))
                    .collect(),
            ))
        }
        "zip" => {
            let Some(Value::Dict(other)) = args.first() else {
                return Err(IonError::type_err(
                    "zip requires a dict argument",
                    line,
                    col,
                ));
            };
            let mut result = IndexMap::new();
            for (key, value) in map {
                if let Some(other_value) = other.get(key) {
                    result.insert(
                        key.clone(),
                        Value::Tuple(vec![value.clone(), other_value.clone()]),
                    );
                }
            }
            Ok(Value::Dict(result))
        }
        _ => Err(IonError::type_err(
            format!("dict has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_bytes_method(
    bytes: &[u8],
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(bytes.len() as i64)),
        "is_empty" => Ok(Value::Bool(bytes.is_empty())),
        "contains" => {
            let byte = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("bytes.contains() requires an int", line, col))?;
            Ok(Value::Bool(bytes.contains(&(byte as u8))))
        }
        "slice" => {
            let start = args.first().and_then(Value::as_int).unwrap_or(0) as usize;
            let end = args
                .get(1)
                .and_then(Value::as_int)
                .map(|value| value as usize)
                .unwrap_or(bytes.len());
            Ok(Value::Bytes(
                bytes[start.min(bytes.len())..end.min(bytes.len())].to_vec(),
            ))
        }
        "to_list" => Ok(Value::List(
            bytes.iter().map(|byte| Value::Int(*byte as i64)).collect(),
        )),
        "to_str" => Ok(match std::str::from_utf8(bytes) {
            Ok(value) => Value::Result(Ok(Box::new(Value::Str(value.to_string())))),
            Err(err) => Value::Result(Err(Box::new(Value::Str(err.to_string())))),
        }),
        "to_hex" => Ok(Value::Str(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        )),
        "find" => {
            let needle = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("bytes.find() requires an int", line, col))?;
            Ok(match bytes.iter().position(|byte| *byte == needle as u8) {
                Some(index) => Value::Option(Some(Box::new(Value::Int(index as i64)))),
                None => Value::Option(None),
            })
        }
        "reverse" => {
            let mut reversed = bytes.to_vec();
            reversed.reverse();
            Ok(Value::Bytes(reversed))
        }
        "push" => {
            let byte = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::type_err("bytes.push() requires an int", line, col))?;
            let mut new = bytes.to_vec();
            new.push(byte as u8);
            Ok(Value::Bytes(new))
        }
        _ => Err(IonError::type_err(
            format!("bytes has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_set_method(
    items: &[Value],
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    match method {
        "len" => Ok(Value::Int(items.len() as i64)),
        "contains" => Ok(Value::Bool(
            args.first().is_some_and(|arg| items.contains(arg)),
        )),
        "is_empty" => Ok(Value::Bool(items.is_empty())),
        "add" => {
            let value = args
                .first()
                .ok_or_else(|| IonError::type_err("set.add requires an argument", line, col))?;
            let mut new = items.to_vec();
            if !new.iter().any(|item| item == value) {
                new.push(value.clone());
            }
            Ok(Value::Set(new))
        }
        "remove" => {
            let value = args
                .first()
                .ok_or_else(|| IonError::type_err("set.remove requires an argument", line, col))?;
            Ok(Value::Set(
                items
                    .iter()
                    .filter(|item| *item != value)
                    .cloned()
                    .collect(),
            ))
        }
        "union" => {
            let Some(Value::Set(other)) = args.first() else {
                return Err(IonError::type_err(
                    "union requires a set argument",
                    line,
                    col,
                ));
            };
            let mut new = items.to_vec();
            for value in other {
                if !new.iter().any(|item| item == value) {
                    new.push(value.clone());
                }
            }
            Ok(Value::Set(new))
        }
        "intersection" => {
            let Some(Value::Set(other)) = args.first() else {
                return Err(IonError::type_err(
                    "intersection requires a set argument",
                    line,
                    col,
                ));
            };
            Ok(Value::Set(
                items
                    .iter()
                    .filter(|value| other.iter().any(|other_value| other_value == *value))
                    .cloned()
                    .collect(),
            ))
        }
        "difference" => {
            let Some(Value::Set(other)) = args.first() else {
                return Err(IonError::type_err(
                    "difference requires a set argument",
                    line,
                    col,
                ));
            };
            Ok(Value::Set(
                items
                    .iter()
                    .filter(|value| !other.iter().any(|other_value| other_value == *value))
                    .cloned()
                    .collect(),
            ))
        }
        "to_list" => Ok(Value::List(items.to_vec())),
        _ => Err(IonError::type_err(
            format!("set has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_option_method(
    value: &Value,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    let Value::Option(option) = value else {
        return Err(IonError::type_err("expected Option", line, col));
    };
    match method {
        "is_some" => Ok(Value::Bool(option.is_some())),
        "is_none" => Ok(Value::Bool(option.is_none())),
        "unwrap" => match option {
            Some(value) => Ok(*value.clone()),
            None => Err(IonError::runtime("called unwrap on None", line, col)),
        },
        "unwrap_or" => Ok(option
            .as_ref()
            .map(|value| *value.clone())
            .unwrap_or_else(|| args.first().cloned().unwrap_or(Value::Unit))),
        "expect" => match option {
            Some(value) => Ok(*value.clone()),
            None => Err(IonError::runtime(
                args.first()
                    .and_then(Value::as_str)
                    .unwrap_or("called expect on None")
                    .to_string(),
                line,
                col,
            )),
        },
        _ => Err(IonError::type_err(
            format!("Option has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_result_method(
    value: &Value,
    method: &str,
    args: &[Value],
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    let Value::Result(result) = value else {
        return Err(IonError::type_err("expected Result", line, col));
    };
    match method {
        "is_ok" => Ok(Value::Bool(result.is_ok())),
        "is_err" => Ok(Value::Bool(result.is_err())),
        "unwrap" => match result {
            Ok(value) => Ok(*value.clone()),
            Err(err) => Err(IonError::runtime(
                format!("called unwrap on Err: {}", err),
                line,
                col,
            )),
        },
        "unwrap_or" => Ok(match result {
            Ok(value) => *value.clone(),
            Err(_) => args.first().cloned().unwrap_or(Value::Unit),
        }),
        "expect" => match result {
            Ok(value) => Ok(*value.clone()),
            Err(err) => Err(IonError::runtime(
                format!(
                    "{}: {}",
                    args.first()
                        .and_then(Value::as_str)
                        .unwrap_or("called expect on Err"),
                    err
                ),
                line,
                col,
            )),
        },
        _ => Err(IonError::type_err(
            format!("Result has no method '{}'", method),
            line,
            col,
        )),
    }
}

fn scaffold_sort_list(items: &[Value], line: usize, col: usize) -> Result<Value, IonError> {
    if !items.is_empty() {
        let first_type = std::mem::discriminant(&items[0]);
        for item in items.iter().skip(1) {
            if std::mem::discriminant(item) != first_type {
                return Err(IonError::type_err(
                    "sort() requires all elements to be the same type",
                    line,
                    col,
                ));
            }
        }
    }
    let mut sorted = items.to_vec();
    sorted.sort_by(|left, right| match (left, right) {
        (Value::Int(left), Value::Int(right)) => left.cmp(right),
        (Value::Float(left), Value::Float(right)) => {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::Str(left), Value::Str(right)) => left.cmp(right),
        _ => std::cmp::Ordering::Equal,
    });
    Ok(Value::List(sorted))
}

fn scaffold_min_max_list(
    items: &[Value],
    min: bool,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    if items.is_empty() {
        return Ok(Value::Option(None));
    }
    let mut best = &items[0];
    for item in items.iter().skip(1) {
        let replace = match (best, item) {
            (Value::Int(left), Value::Int(right)) => {
                if min {
                    right < left
                } else {
                    right > left
                }
            }
            (Value::Float(left), Value::Float(right)) => {
                if min {
                    right < left
                } else {
                    right > left
                }
            }
            (Value::Str(left), Value::Str(right)) => {
                if min {
                    right < left
                } else {
                    right > left
                }
            }
            _ => {
                return Err(IonError::type_err(
                    format!(
                        "{}() requires homogeneous comparable elements",
                        if min { "min" } else { "max" }
                    ),
                    line,
                    col,
                ))
            }
        };
        if replace {
            best = item;
        }
    }
    Ok(Value::Option(Some(Box::new(best.clone()))))
}

fn scaffold_sum_list(items: &[Value], line: usize, col: usize) -> Result<Value, IonError> {
    let mut int_sum = 0i64;
    let mut float_sum = 0.0f64;
    let mut has_float = false;
    for item in items {
        match item {
            Value::Int(value) => int_sum += value,
            Value::Float(value) => {
                has_float = true;
                float_sum += value;
            }
            _ => {
                return Err(IonError::type_err(
                    "sum() requires numeric elements",
                    line,
                    col,
                ))
            }
        }
    }
    if has_float {
        Ok(Value::Float(float_sum + int_sum as f64))
    } else {
        Ok(Value::Int(int_sum))
    }
}

fn scaffold_pad_string(
    value: &str,
    args: &[Value],
    start: bool,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    let method = if start { "pad_start" } else { "pad_end" };
    let width =
        args.first().and_then(Value::as_int).ok_or_else(|| {
            IonError::type_err(format!("{} requires int argument", method), line, col)
        })? as usize;
    let ch = args
        .get(1)
        .and_then(Value::as_str)
        .and_then(|value| value.chars().next())
        .unwrap_or(' ');
    let char_len = value.chars().count();
    if char_len >= width {
        return Ok(Value::Str(value.to_string()));
    }
    let pad: String = std::iter::repeat_n(ch, width - char_len).collect();
    if start {
        Ok(Value::Str(format!("{}{}", pad, value)))
    } else {
        Ok(Value::Str(format!("{}{}", value, pad)))
    }
}

fn scaffold_slice_access(
    object: Value,
    start: Option<Value>,
    end: Option<Value>,
    inclusive: bool,
    line: usize,
    col: usize,
) -> Result<Value, IonError> {
    let get_index = |value: Option<Value>, default: i64| -> Result<i64, IonError> {
        match value {
            Some(Value::Int(value)) => Ok(value),
            None => Ok(default),
            Some(value) => Err(IonError::type_err(
                format!("slice index must be int, got {}", value.type_name()),
                line,
                col,
            )),
        }
    };

    match &object {
        Value::List(items) => {
            let len = items.len() as i64;
            let start = get_index(start, 0)?.max(0).min(len) as usize;
            let end = get_index(end, len)?;
            let end = if inclusive { end + 1 } else { end }.max(0).min(len) as usize;
            Ok(Value::List(items[start..end].to_vec()))
        }
        Value::Str(value) => {
            let chars: Vec<char> = value.chars().collect();
            let len = chars.len() as i64;
            let start = get_index(start, 0)?.max(0).min(len) as usize;
            let end = get_index(end, len)?;
            let end = if inclusive { end + 1 } else { end }.max(0).min(len) as usize;
            Ok(Value::Str(chars[start..end].iter().collect()))
        }
        Value::Bytes(bytes) => {
            let len = bytes.len() as i64;
            let start = get_index(start, 0)?.max(0).min(len) as usize;
            let end = get_index(end, len)?;
            let end = if inclusive { end + 1 } else { end }.max(0).min(len) as usize;
            Ok(Value::Bytes(bytes[start..end].to_vec()))
        }
        _ => Err(IonError::type_err(
            format!("cannot slice {}", object.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_check_type(
    value: &Value,
    type_name: &str,
    line: usize,
    col: usize,
) -> Result<(), IonError> {
    let ok = match type_name {
        "int" => matches!(value, Value::Int(_)),
        "float" => matches!(value, Value::Float(_)),
        "bool" => matches!(value, Value::Bool(_)),
        "string" => matches!(value, Value::Str(_)),
        "bytes" => matches!(value, Value::Bytes(_)),
        "list" => matches!(value, Value::List(_)),
        "dict" => matches!(value, Value::Dict(_)),
        "tuple" => matches!(value, Value::Tuple(_)),
        "set" => matches!(value, Value::Set(_)),
        "fn" => match value {
            Value::Fn(_) | Value::BuiltinFn { .. } | Value::BuiltinClosure { .. } => true,
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { .. } => true,
            _ => false,
        },
        "cell" => matches!(value, Value::Cell(_)),
        "any" => true,
        name if name.starts_with("Option") => matches!(value, Value::Option(_)),
        name if name.starts_with("Result") => matches!(value, Value::Result(_)),
        name if name.starts_with("list<") => matches!(value, Value::List(_)),
        name if name.starts_with("dict<") => matches!(value, Value::Dict(_)),
        _ => true,
    };

    if ok {
        Ok(())
    } else {
        Err(IonError::type_err(
            format!(
                "type mismatch: expected {}, got {}",
                type_name,
                value.type_name()
            ),
            line,
            col,
        ))
    }
}

fn construct_host_struct(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let type_idx =
        match read_u16_operand(chunk, *next_ip, line, col, "truncated struct type operand") {
            Ok(value) => value as usize,
            Err(err) => return StepOutcome::InstructionError(err),
        };
    let raw_count = match read_u16_operand(
        chunk,
        *next_ip + 2,
        line,
        col,
        "truncated struct field-count operand",
    ) {
        Ok(value) => value as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 4;
    let type_name = match chunk.constants.get(type_idx) {
        Some(value) => match scaffold_const_as_str(value, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        },
        None => {
            return StepOutcome::InstructionError(IonError::runtime(
                "struct type constant index out of bounds",
                line,
                col,
            ));
        }
    };

    let has_spread = raw_count & 0x8000 != 0;
    let field_count = raw_count & 0x7fff;
    let mut fields: IndexMap<u64, Value> = IndexMap::new();
    if has_spread {
        let value_count = field_count.saturating_mul(2);
        if cont.stack.len() < value_count + 1 {
            return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
        }
        let start = cont.stack.len() - value_count;
        let overrides: Vec<Value> = cont.stack.drain(start..).collect();
        let spread = match pop_stack(cont, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        };
        let Value::HostStruct {
            fields: spread_fields,
            ..
        } = spread
        else {
            return StepOutcome::InstructionError(IonError::type_err(
                "spread in struct constructor requires a struct",
                line,
                col,
            ));
        };
        fields.extend(spread_fields);
        for pair in overrides.chunks(2) {
            let Value::Str(name) = &pair[0] else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "invalid field name",
                    line,
                    col,
                ));
            };
            fields.insert(crate::hash::h(name), pair[1].clone());
        }
    } else {
        let value_count = field_count.saturating_mul(2);
        if cont.stack.len() < value_count {
            return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
        }
        let start = cont.stack.len() - value_count;
        let values: Vec<Value> = cont.stack.drain(start..).collect();
        for pair in values.chunks(2) {
            let Value::Str(name) = &pair[0] else {
                return StepOutcome::InstructionError(IonError::runtime(
                    "invalid field name",
                    line,
                    col,
                ));
            };
            fields.insert(crate::hash::h(name), pair[1].clone());
        }
    }

    match cont.types.construct_struct(&type_name, fields) {
        Ok(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(IonError::runtime(err, line, col)),
    }
}

fn construct_host_enum(
    chunk: &Chunk,
    cont: &mut VmContinuation,
    next_ip: &mut usize,
    line: usize,
    col: usize,
) -> StepOutcome {
    let enum_idx = match read_u16_operand(chunk, *next_ip, line, col, "truncated enum type operand")
    {
        Ok(value) => value as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let variant_idx = match read_u16_operand(
        chunk,
        *next_ip + 2,
        line,
        col,
        "truncated enum variant operand",
    ) {
        Ok(value) => value as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    let arg_count = match read_u8_operand(
        chunk,
        *next_ip + 4,
        line,
        col,
        "truncated enum argument count operand",
    ) {
        Ok(value) => value as usize,
        Err(err) => return StepOutcome::InstructionError(err),
    };
    *next_ip += 5;
    let enum_name = match chunk.constants.get(enum_idx) {
        Some(value) => match scaffold_const_as_str(value, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        },
        None => {
            return StepOutcome::InstructionError(IonError::runtime(
                "enum type constant index out of bounds",
                line,
                col,
            ));
        }
    };
    let variant = match chunk.constants.get(variant_idx) {
        Some(value) => match scaffold_const_as_str(value, line, col) {
            Ok(value) => value,
            Err(err) => return StepOutcome::InstructionError(err),
        },
        None => {
            return StepOutcome::InstructionError(IonError::runtime(
                "enum variant constant index out of bounds",
                line,
                col,
            ));
        }
    };
    if cont.stack.len() < arg_count {
        return StepOutcome::InstructionError(IonError::runtime("stack underflow", line, col));
    }
    let start = cont.stack.len() - arg_count;
    let args: Vec<Value> = cont.stack.drain(start..).collect();
    match cont.types.construct_enum(&enum_name, &variant, args) {
        Ok(value) => {
            cont.stack.push(value);
            StepOutcome::Continue
        }
        Err(err) => StepOutcome::InstructionError(IonError::runtime(err, line, col)),
    }
}

fn scaffold_list_append(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
) -> Result<(), IonError> {
    let item = pop_stack(cont, line, col)?;
    for value in cont.stack.iter_mut().rev() {
        if let Value::List(items) = value {
            items.push(item);
            return Ok(());
        }
    }
    Err(IonError::runtime("ListAppend: no list on stack", line, col))
}

fn scaffold_list_extend(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
) -> Result<(), IonError> {
    let source = pop_stack(cont, line, col)?;
    let Value::List(source) = source else {
        return Err(IonError::type_err(
            format!("spread requires a list, got {}", source.type_name()),
            line,
            col,
        ));
    };
    for value in cont.stack.iter_mut().rev() {
        if let Value::List(items) = value {
            items.extend(source);
            return Ok(());
        }
    }
    Err(IonError::runtime("ListExtend: no list on stack", line, col))
}

fn scaffold_dict_insert(
    cont: &mut VmContinuation,
    line: usize,
    col: usize,
) -> Result<(), IonError> {
    let value = pop_stack(cont, line, col)?;
    let key = pop_stack(cont, line, col)?;
    let key = match key {
        Value::Str(value) => value,
        value => value.to_string(),
    };
    for value_on_stack in cont.stack.iter_mut().rev() {
        if let Value::Dict(map) = value_on_stack {
            map.insert(key, value);
            return Ok(());
        }
    }
    Err(IonError::runtime("DictInsert: no dict on stack", line, col))
}

fn scaffold_dict_merge(cont: &mut VmContinuation, line: usize, col: usize) -> Result<(), IonError> {
    let source = pop_stack(cont, line, col)?;
    let Value::Dict(source) = source else {
        return Err(IonError::type_err("spread requires a dict", line, col));
    };
    for value in cont.stack.iter_mut().rev() {
        if let Value::Dict(map) = value {
            map.extend(source);
            return Ok(());
        }
    }
    Err(IonError::runtime("DictMerge: no dict on stack", line, col))
}

fn scaffold_value_iterator(
    value: Value,
    line: usize,
    col: usize,
) -> Result<Box<dyn Iterator<Item = Value>>, IonError> {
    match value {
        Value::List(items) | Value::Set(items) | Value::Tuple(items) => {
            Ok(Box::new(items.into_iter()))
        }
        Value::Dict(map) => {
            Ok(Box::new(map.into_iter().map(|(key, value)| {
                Value::Tuple(vec![Value::Str(key), value])
            })))
        }
        Value::Str(value) => Ok(Box::new(
            value
                .chars()
                .map(|value| Value::Str(value.to_string()))
                .collect::<Vec<_>>()
                .into_iter(),
        )),
        Value::Bytes(bytes) => Ok(Box::new(
            bytes
                .into_iter()
                .map(|value| Value::Int(value as i64))
                .collect::<Vec<_>>()
                .into_iter(),
        )),
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            let values = if inclusive {
                (start..=end).map(Value::Int).collect::<Vec<_>>()
            } else if end > start {
                (start..end).map(Value::Int).collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            Ok(Box::new(values.into_iter()))
        }
        value => Err(IonError::type_err(
            format!("cannot iterate over {}", value.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_add(left: Value, right: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match (&left, &right) {
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x + y)),
        (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x + y)),
        (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 + y)),
        (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x + *y as f64)),
        (Value::Str(x), Value::Str(y)) => Ok(Value::Str(format!("{x}{y}"))),
        (Value::List(x), Value::List(y)) => {
            let mut out = x.clone();
            out.extend(y.clone());
            Ok(Value::List(out))
        }
        (Value::Bytes(x), Value::Bytes(y)) => {
            let mut out = x.clone();
            out.extend(y);
            Ok(Value::Bytes(out))
        }
        _ => Err(IonError::type_err(
            format!("cannot add {} and {}", left.type_name(), right.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_sub(left: Value, right: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match (&left, &right) {
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x - y)),
        (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x - y)),
        (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 - y)),
        (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x - *y as f64)),
        _ => Err(IonError::type_err(
            format!(
                "cannot subtract {} from {}",
                right.type_name(),
                left.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_mul(left: Value, right: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match (&left, &right) {
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x * y)),
        (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x * y)),
        (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 * y)),
        (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x * *y as f64)),
        (Value::Str(s), Value::Int(n)) | (Value::Int(n), Value::Str(s)) => {
            Ok(Value::Str(s.repeat(*n as usize)))
        }
        _ => Err(IonError::type_err(
            format!(
                "cannot multiply {} and {}",
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_div(left: Value, right: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match (&left, &right) {
        (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("division by zero", line, col)),
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x / y)),
        (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x / y)),
        (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 / y)),
        (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x / *y as f64)),
        _ => Err(IonError::type_err(
            format!(
                "cannot divide {} by {}",
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_mod(left: Value, right: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match (&left, &right) {
        (Value::Int(_), Value::Int(0)) => Err(IonError::runtime("modulo by zero", line, col)),
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
        (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x % y)),
        (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 % y)),
        (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x % *y as f64)),
        _ => Err(IonError::type_err(
            format!(
                "cannot modulo {} by {}",
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_neg(value: Value, line: usize, col: usize) -> Result<Value, IonError> {
    match value {
        Value::Int(value) => Ok(Value::Int(-value)),
        Value::Float(value) => Ok(Value::Float(-value)),
        other => Err(IonError::type_err(
            format!("cannot negate {}", other.type_name()),
            line,
            col,
        )),
    }
}

fn scaffold_bitwise(
    left: Value,
    right: Value,
    line: usize,
    col: usize,
    op: &str,
    apply: impl FnOnce(i64, i64) -> i64,
) -> Result<Value, IonError> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => Ok(Value::Int(apply(left, right))),
        (left, right) => Err(IonError::type_err(
            format!(
                "'{}' expects int, got {} and {}",
                op,
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_shift(
    left: Value,
    right: Value,
    line: usize,
    col: usize,
    op: &str,
    apply: impl FnOnce(i64, i64) -> i64,
) -> Result<Value, IonError> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) if (0..64).contains(&right) => {
            Ok(Value::Int(apply(left, right)))
        }
        (Value::Int(_), Value::Int(right)) => Err(IonError::runtime(
            format!("shift count {} is out of range 0..64", right),
            line,
            col,
        )),
        (left, right) => Err(IonError::type_err(
            format!(
                "'{}' expects int, got {} and {}",
                op,
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

fn scaffold_compare_lt(
    left: &Value,
    right: &Value,
    line: usize,
    col: usize,
) -> Result<bool, IonError> {
    match (left, right) {
        (Value::Int(x), Value::Int(y)) => Ok(x < y),
        (Value::Float(x), Value::Float(y)) => Ok(x < y),
        (Value::Int(x), Value::Float(y)) => Ok((*x as f64) < *y),
        (Value::Float(x), Value::Int(y)) => Ok(*x < (*y as f64)),
        (Value::Str(x), Value::Str(y)) => Ok(x < y),
        _ => Err(IonError::type_err(
            format!(
                "cannot compare {} and {}",
                left.type_name(),
                right.type_name()
            ),
            line,
            col,
        )),
    }
}

impl Default for IonTask {
    fn default() -> Self {
        Self {
            state: TaskState::Ready,
            cancel_requested: false,
            waiters: Vec::new(),
            result: None,
            resumed_value: None,
            pending_error: None,
        }
    }
}

/// Result of awaiting a runtime-managed Ion task.
#[derive(Debug, Clone)]
pub enum TaskAwait {
    Ready(Result<Value, IonError>),
    Waiting,
    Missing,
}

/// A waiter that should be resumed after a task completes.
#[derive(Debug, Clone)]
pub struct TaskResume {
    pub waiter: TaskId,
    pub result: Result<Value, IonError>,
}

/// Lightweight Ion task storage used by `spawn` / `.await`.
#[derive(Default)]
pub struct TaskTable {
    tasks: HashMap<TaskId, IonTask>,
    next_id: u64,
}

impl TaskTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn_ready(&mut self) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.tasks.insert(id, IonTask::default());
        id
    }

    pub fn get(&self, id: TaskId) -> Option<&IonTask> {
        self.tasks.get(&id)
    }

    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut IonTask> {
        self.tasks.get_mut(&id)
    }

    pub fn await_task(&mut self, waiter: TaskId, target: TaskId) -> TaskAwait {
        let Some(target_task) = self.tasks.get_mut(&target) else {
            return TaskAwait::Missing;
        };

        if let Some(result) = &target_task.result {
            return TaskAwait::Ready(result.clone());
        }

        if !target_task.waiters.contains(&waiter) {
            target_task.waiters.push(waiter);
        }
        if let Some(waiter_task) = self.tasks.get_mut(&waiter) {
            waiter_task.state = TaskState::WaitingTask(target);
        }
        TaskAwait::Waiting
    }

    pub fn finish(&mut self, id: TaskId, result: Result<Value, IonError>) -> Vec<TaskResume> {
        let Some(task) = self.tasks.get_mut(&id) else {
            return Vec::new();
        };

        task.state = TaskState::Done;
        task.result = Some(result.clone());
        let waiters = std::mem::take(&mut task.waiters);

        for waiter in &waiters {
            if let Some(waiter_task) = self.tasks.get_mut(waiter) {
                waiter_task.state = TaskState::Ready;
            }
        }

        waiters
            .into_iter()
            .map(|waiter| TaskResume {
                waiter,
                result: result.clone(),
            })
            .collect()
    }

    pub fn cancel(&mut self, id: TaskId) -> bool {
        let Some(task) = self.tasks.get_mut(&id) else {
            return false;
        };
        task.cancel_requested = true;
        true
    }

    pub fn park_on_host_future(&mut self, id: TaskId, future: FutureId) -> bool {
        let Some(task) = self.tasks.get_mut(&id) else {
            return false;
        };
        task.state = TaskState::WaitingHostFuture(future);
        true
    }

    pub fn resume_from_host_result(&mut self, id: TaskId, result: Result<Value, IonError>) -> bool {
        let Some(task) = self.tasks.get_mut(&id) else {
            return false;
        };
        task.state = TaskState::Ready;
        match result {
            Ok(value) => {
                task.resumed_value = Some(value);
                task.pending_error = None;
            }
            Err(err) => {
                task.pending_error = Some(err);
                task.resumed_value = None;
            }
        }
        true
    }

    pub fn take_resumed_value(&mut self, id: TaskId) -> Option<Value> {
        self.tasks
            .get_mut(&id)
            .and_then(|task| task.resumed_value.take())
    }

    pub fn take_pending_error(&mut self, id: TaskId) -> Option<IonError> {
        self.tasks
            .get_mut(&id)
            .and_then(|task| task.pending_error.take())
    }
}

/// Summary of a budgeted task run.
#[derive(Debug, Clone)]
pub struct TaskRun {
    pub consumed: u32,
    pub outcome: TaskRunOutcome,
}

#[derive(Debug, Clone)]
pub enum TaskRunOutcome {
    BudgetExhausted,
    Yielded,
    Suspended(TaskState),
    InstructionError(IonError),
    Done(Result<Value, IonError>),
    Cancelled,
}

/// Runs a task-like stepper with the same accounting rules the final
/// async VM must obey. Every transition charges at least one budget unit.
pub fn run_budgeted_steps<F>(task: &mut IonTask, budget: u32, mut step: F) -> TaskRun
where
    F: FnMut(&mut IonTask) -> StepOutcome,
{
    let budget = budget.max(1);
    let mut consumed = 0u32;

    loop {
        if task.cancel_requested {
            task.state = TaskState::Done;
            return TaskRun {
                consumed: consumed.max(1),
                outcome: TaskRunOutcome::Cancelled,
            };
        }

        match step(task) {
            StepOutcome::Continue => {
                consumed += 1;
                if consumed >= budget {
                    return TaskRun {
                        consumed,
                        outcome: TaskRunOutcome::BudgetExhausted,
                    };
                }
            }
            StepOutcome::Yield => {
                consumed = (consumed + 1).max(1);
                return TaskRun {
                    consumed,
                    outcome: TaskRunOutcome::Yielded,
                };
            }
            StepOutcome::Suspended(state) => {
                consumed = (consumed + 1).max(1);
                task.state = state.clone();
                return TaskRun {
                    consumed,
                    outcome: TaskRunOutcome::Suspended(state),
                };
            }
            StepOutcome::InstructionError(err) => {
                consumed = (consumed + 1).max(1);
                return TaskRun {
                    consumed,
                    outcome: TaskRunOutcome::InstructionError(err),
                };
            }
            StepOutcome::Done(result) => {
                consumed = (consumed + 1).max(1);
                task.state = TaskState::Done;
                return TaskRun {
                    consumed,
                    outcome: TaskRunOutcome::Done(result),
                };
            }
        }
    }
}

/// Stable identifier for bytecode stored by the async runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkId {
    slot: usize,
    generation: u64,
}

/// Stable identifier for a Tokio timer owned by the async runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId {
    slot: usize,
    generation: u64,
}

/// Stable identifier for an async runtime channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId {
    slot: usize,
    generation: u64,
}

/// Stable identifier for a structured-concurrency nursery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NurseryId {
    slot: usize,
    generation: u64,
}

/// A host future that completed during a poll pass.
pub struct ReadyHostFuture {
    pub id: FutureId,
    pub waiter: TaskId,
    pub result: Result<Value, IonError>,
}

/// A timer that expired during a poll pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyTimer {
    pub id: TimerId,
    pub waiter: TaskId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelSend {
    Sent,
    Delivered { receiver: TaskId, value: Value },
    Blocked,
    Closed(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelRecv {
    Received {
        value: Value,
        unblocked_sender: Option<TaskId>,
    },
    Blocked,
    Closed,
}

#[derive(Debug, Clone)]
pub enum NurseryState {
    Open,
    Failing(IonError),
    Draining,
}

#[derive(Debug, Clone)]
pub struct Nursery {
    pub parent: TaskId,
    pub children: Vec<TaskId>,
    pub state: NurseryState,
}

struct HostFutureEntry {
    generation: u64,
    waiter: TaskId,
    future: BoxIonFuture,
}

/// Cancellable host-future storage.
///
/// This table deliberately supports by-ID removal. That is the key
/// property needed when cancelling an Ion task parked on a host future.
#[derive(Default)]
pub struct HostFutureTable {
    entries: Vec<Option<HostFutureEntry>>,
    free: Vec<usize>,
    next_generation: u64,
}

/// Stable owner for compiled bytecode chunks.
///
/// Resumable VM frames should store `ChunkId` instead of borrowed
/// `&Chunk`, avoiding self-referential runtime lifetimes.
#[derive(Clone, Default)]
pub struct ChunkArena {
    chunks: Vec<Option<ChunkEntry>>,
    free: Vec<usize>,
    next_generation: u64,
}

#[derive(Clone)]
struct ChunkEntry {
    generation: u64,
    chunk: Chunk,
}

struct TimerEntry {
    generation: u64,
    waiter: TaskId,
    sleep: Pin<Box<tokio::time::Sleep>>,
}

/// Cancellable Tokio timer storage.
#[derive(Default)]
pub struct TimerTable {
    timers: Vec<Option<TimerEntry>>,
    free: Vec<usize>,
    next_generation: u64,
}

/// Runtime-managed bounded channel.
#[derive(Debug, Clone)]
pub struct AsyncChannel {
    buffer: std::collections::VecDeque<Value>,
    capacity: usize,
    closed: bool,
    recv_waiters: std::collections::VecDeque<TaskId>,
    send_waiters: std::collections::VecDeque<(TaskId, Value)>,
}

impl AsyncChannel {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: std::collections::VecDeque::new(),
            capacity: capacity.max(1),
            closed: false,
            recv_waiters: std::collections::VecDeque::new(),
            send_waiters: std::collections::VecDeque::new(),
        }
    }

    pub fn send(&mut self, sender: TaskId, value: Value) -> ChannelSend {
        if self.closed {
            return ChannelSend::Closed(value);
        }

        if let Some(receiver) = self.recv_waiters.pop_front() {
            return ChannelSend::Delivered { receiver, value };
        }

        if self.buffer.len() < self.capacity {
            self.buffer.push_back(value);
            return ChannelSend::Sent;
        }

        self.send_waiters.push_back((sender, value));
        ChannelSend::Blocked
    }

    pub fn recv(&mut self, receiver: TaskId) -> ChannelRecv {
        if let Some(value) = self.buffer.pop_front() {
            let unblocked_sender = self.fill_freed_capacity();
            return ChannelRecv::Received {
                value,
                unblocked_sender,
            };
        }

        if let Some((sender, value)) = self.send_waiters.pop_front() {
            return ChannelRecv::Received {
                value,
                unblocked_sender: Some(sender),
            };
        }

        if self.closed {
            return ChannelRecv::Closed;
        }

        self.recv_waiters.push_back(receiver);
        ChannelRecv::Blocked
    }

    pub fn close(&mut self) -> (Vec<TaskId>, Vec<TaskId>) {
        self.closed = true;
        let receivers = self.recv_waiters.drain(..).collect();
        let senders = self
            .send_waiters
            .drain(..)
            .map(|(sender, _)| sender)
            .collect();
        (receivers, senders)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    fn fill_freed_capacity(&mut self) -> Option<TaskId> {
        if self.closed || self.buffer.len() >= self.capacity {
            return None;
        }

        let (sender, value) = self.send_waiters.pop_front()?;
        self.buffer.push_back(value);
        Some(sender)
    }
}

#[derive(Default)]
pub struct ChannelTable {
    channels: Vec<Option<ChannelEntry>>,
    free: Vec<usize>,
    next_generation: u64,
}

struct ChannelEntry {
    generation: u64,
    channel: AsyncChannel,
}

#[derive(Default)]
pub struct NurseryTable {
    nurseries: Vec<Option<NurseryEntry>>,
    free: Vec<usize>,
    next_generation: u64,
}

struct NurseryEntry {
    generation: u64,
    nursery: Nursery,
}

impl ChannelTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, channel: AsyncChannel) -> ChannelId {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let entry = ChannelEntry {
            generation,
            channel,
        };

        if let Some(slot) = self.free.pop() {
            self.channels[slot] = Some(entry);
            ChannelId { slot, generation }
        } else {
            let slot = self.channels.len();
            self.channels.push(Some(entry));
            ChannelId { slot, generation }
        }
    }

    pub fn get(&self, id: ChannelId) -> Option<&AsyncChannel> {
        self.channels
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &entry.channel)
    }

    pub fn get_mut(&mut self, id: ChannelId) -> Option<&mut AsyncChannel> {
        self.channels
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &mut entry.channel)
    }
}

impl NurseryTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, parent: TaskId) -> NurseryId {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let entry = NurseryEntry {
            generation,
            nursery: Nursery {
                parent,
                children: Vec::new(),
                state: NurseryState::Open,
            },
        };

        if let Some(slot) = self.free.pop() {
            self.nurseries[slot] = Some(entry);
            NurseryId { slot, generation }
        } else {
            let slot = self.nurseries.len();
            self.nurseries.push(Some(entry));
            NurseryId { slot, generation }
        }
    }

    pub fn get(&self, id: NurseryId) -> Option<&Nursery> {
        self.nurseries
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &entry.nursery)
    }

    pub fn get_mut(&mut self, id: NurseryId) -> Option<&mut Nursery> {
        self.nurseries
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &mut entry.nursery)
    }

    pub fn add_child(&mut self, id: NurseryId, child: TaskId) -> bool {
        let Some(nursery) = self.get_mut(id) else {
            return false;
        };
        if !nursery.children.contains(&child) {
            nursery.children.push(child);
        }
        true
    }

    pub fn child_finished(&mut self, id: NurseryId, child: TaskId) -> bool {
        let Some(nursery) = self.get_mut(id) else {
            return false;
        };
        nursery.children.retain(|existing| *existing != child);
        nursery.children.is_empty()
    }

    pub fn fail_fast(&mut self, id: NurseryId, err: IonError) -> Vec<TaskId> {
        let Some(nursery) = self.get_mut(id) else {
            return Vec::new();
        };
        nursery.state = NurseryState::Failing(err);
        nursery.children.clone()
    }

    pub fn drain(&mut self, id: NurseryId) -> Option<Nursery> {
        let is_current = self
            .nurseries
            .get(id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.generation == id.generation);
        if !is_current {
            return None;
        }

        let entry = self
            .nurseries
            .get_mut(id.slot)
            .and_then(Option::take)
            .expect("nursery disappeared after generation check");
        self.free.push(id.slot);
        Some(entry.nursery)
    }
}

impl ChunkArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, chunk: Chunk) -> ChunkId {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let entry = ChunkEntry { generation, chunk };

        if let Some(slot) = self.free.pop() {
            self.chunks[slot] = Some(entry);
            ChunkId { slot, generation }
        } else {
            let slot = self.chunks.len();
            self.chunks.push(Some(entry));
            ChunkId { slot, generation }
        }
    }

    pub fn get(&self, id: ChunkId) -> Option<&Chunk> {
        self.chunks
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &entry.chunk)
    }

    pub fn get_mut(&mut self, id: ChunkId) -> Option<&mut Chunk> {
        self.chunks
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.generation == id.generation)
            .map(|entry| &mut entry.chunk)
    }

    pub fn remove(&mut self, id: ChunkId) -> Option<Chunk> {
        let is_current = self
            .chunks
            .get(id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.generation == id.generation);
        if !is_current {
            return None;
        }

        let entry = self
            .chunks
            .get_mut(id.slot)
            .and_then(Option::take)
            .expect("chunk disappeared after generation check");
        self.free.push(id.slot);
        Some(entry.chunk)
    }
}

impl TimerTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_sleep(&mut self, waiter: TaskId, duration: Duration) -> TimerId {
        self.insert_sleep_until(waiter, tokio::time::Instant::now() + duration)
    }

    pub fn insert_sleep_until(
        &mut self,
        waiter: TaskId,
        deadline: tokio::time::Instant,
    ) -> TimerId {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let entry = TimerEntry {
            generation,
            waiter,
            sleep: Box::pin(tokio::time::sleep_until(deadline)),
        };

        if let Some(slot) = self.free.pop() {
            self.timers[slot] = Some(entry);
            TimerId { slot, generation }
        } else {
            let slot = self.timers.len();
            self.timers.push(Some(entry));
            TimerId { slot, generation }
        }
    }

    pub fn contains(&self, id: TimerId) -> bool {
        self.timers
            .get(id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.generation == id.generation)
    }

    pub fn cancel(&mut self, id: TimerId) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.timers[id.slot] = None;
        self.free.push(id.slot);
        true
    }

    pub fn poll_ready(&mut self, cx: &mut Context<'_>) -> Vec<ReadyTimer> {
        let mut ready = Vec::new();

        for slot in 0..self.timers.len() {
            let poll_result = {
                let Some(entry) = self.timers[slot].as_mut() else {
                    continue;
                };
                match entry.sleep.as_mut().poll(cx) {
                    Poll::Ready(()) => Some((entry.generation, entry.waiter)),
                    Poll::Pending => None,
                }
            };

            if let Some((generation, waiter)) = poll_result {
                self.timers[slot] = None;
                self.free.push(slot);
                ready.push(ReadyTimer {
                    id: TimerId { slot, generation },
                    waiter,
                });
            }
        }

        ready
    }
}

impl HostFutureTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, waiter: TaskId, future: BoxIonFuture) -> FutureId {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);

        let entry = HostFutureEntry {
            generation,
            waiter,
            future,
        };

        if let Some(slot) = self.free.pop() {
            self.entries[slot] = Some(entry);
            FutureId { slot, generation }
        } else {
            let slot = self.entries.len();
            self.entries.push(Some(entry));
            FutureId { slot, generation }
        }
    }

    pub fn contains(&self, id: FutureId) -> bool {
        self.entries
            .get(id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.generation == id.generation)
    }

    pub fn cancel_and_drop(&mut self, id: FutureId) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.entries[id.slot] = None;
        self.free.push(id.slot);
        true
    }

    pub fn poll_ready(&mut self, cx: &mut Context<'_>) -> Vec<ReadyHostFuture> {
        let mut ready = Vec::new();

        for slot in 0..self.entries.len() {
            let poll_result = {
                let Some(entry) = self.entries[slot].as_mut() else {
                    continue;
                };
                match entry.future.as_mut().poll(cx) {
                    Poll::Ready(result) => Some((entry.generation, entry.waiter, result)),
                    Poll::Pending => None,
                }
            };

            if let Some((generation, waiter, result)) = poll_result {
                self.entries[slot] = None;
                self.free.push(slot);
                ready.push(ReadyHostFuture {
                    id: FutureId { slot, generation },
                    waiter,
                    result,
                });
            }
        }

        ready
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(Option::is_none)
    }
}

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug)]
enum AsyncSignal {
    Return(Value),
}

#[derive(Debug)]
enum AsyncSignalOrError {
    Signal(AsyncSignal),
    Error(IonError),
}

impl From<IonError> for AsyncSignalOrError {
    fn from(err: IonError) -> Self {
        Self::Error(err)
    }
}

impl From<AsyncSignal> for AsyncSignalOrError {
    fn from(signal: AsyncSignal) -> Self {
        Self::Signal(signal)
    }
}

type AsyncSignalResult = Result<Value, AsyncSignalOrError>;

/// Narrow async tree-walk bridge retained as historical scaffold.
///
/// `eval_async` no longer uses this as its production fallback for
/// async-host programs. Unsupported language forms should be moved into
/// the VM continuation runtime instead of expanding this bridge.
#[allow(dead_code)]
struct AsyncTreeInterpreter<'env> {
    env: &'env mut Env,
    limits: crate::interpreter::Limits,
    call_depth: usize,
    tasks: Rc<RefCell<Vec<AsyncTask>>>,
    nursery_stack: Rc<RefCell<Vec<Vec<AsyncTask>>>>,
}

impl<'env> AsyncTreeInterpreter<'env> {
    fn new(env: &'env mut Env, limits: crate::interpreter::Limits) -> Self {
        Self::new_with_runtime(
            env,
            limits,
            Rc::new(RefCell::new(Vec::new())),
            Rc::new(RefCell::new(Vec::new())),
        )
    }

    fn new_with_runtime(
        env: &'env mut Env,
        limits: crate::interpreter::Limits,
        tasks: Rc<RefCell<Vec<AsyncTask>>>,
        nursery_stack: Rc<RefCell<Vec<Vec<AsyncTask>>>>,
    ) -> Self {
        Self {
            env,
            limits,
            call_depth: 0,
            tasks,
            nursery_stack,
        }
    }

    fn eval_program<'a>(
        &'a mut self,
        program: &'a Program,
    ) -> LocalBoxFuture<'a, Result<Value, IonError>> {
        Box::pin(async move {
            match self.eval_stmts(&program.stmts).await {
                Ok(value) => Ok(value),
                Err(AsyncSignalOrError::Error(err)) if err.kind == ErrorKind::PropagatedErr => {
                    Ok(Value::Result(Err(Box::new(Value::Str(err.message)))))
                }
                Err(AsyncSignalOrError::Error(err)) if err.kind == ErrorKind::PropagatedNone => {
                    Ok(Value::Option(None))
                }
                Err(AsyncSignalOrError::Error(err)) => Err(err),
                Err(AsyncSignalOrError::Signal(AsyncSignal::Return(value))) => Ok(value),
            }
        })
    }

    fn eval_stmts<'a>(&'a mut self, stmts: &'a [Stmt]) -> LocalBoxFuture<'a, AsyncSignalResult> {
        Box::pin(async move {
            let mut last = Value::Unit;
            for (i, stmt) in stmts.iter().enumerate() {
                let is_last = i == stmts.len() - 1;
                match &stmt.kind {
                    StmtKind::ExprStmt { expr, has_semi } => {
                        let value = self.eval_expr(expr).await?;
                        last = if is_last && !has_semi {
                            value
                        } else {
                            Value::Unit
                        };
                    }
                    StmtKind::Let {
                        mutable,
                        pattern,
                        type_ann: _,
                        value,
                    } => {
                        let value = self.eval_expr(value).await?;
                        self.bind_pattern(pattern, value, *mutable, stmt.span)?;
                        last = Value::Unit;
                    }
                    StmtKind::FnDecl { name, params, body } => {
                        let captures = self.env.capture();
                        self.env.define(
                            name.clone(),
                            Value::Fn(crate::value::IonFn::new(
                                name.clone(),
                                params.clone(),
                                body.clone(),
                                captures,
                            )),
                            false,
                        );
                        last = Value::Unit;
                    }
                    StmtKind::Return { value } => {
                        let value = match value {
                            Some(expr) => self.eval_expr(expr).await?,
                            None => Value::Unit,
                        };
                        return Err(AsyncSignal::Return(value).into());
                    }
                    _ => {
                        return Err(IonError::runtime(
                            "statement is not supported by the async host bridge yet",
                            stmt.span.line,
                            stmt.span.col,
                        )
                        .into());
                    }
                }
            }
            Ok(last)
        })
    }

    fn eval_expr<'a>(&'a mut self, expr: &'a Expr) -> LocalBoxFuture<'a, AsyncSignalResult> {
        Box::pin(async move {
            let span = expr.span;
            match &expr.kind {
                ExprKind::Int(value) => Ok(Value::Int(*value)),
                ExprKind::Float(value) => Ok(Value::Float(*value)),
                ExprKind::Bool(value) => Ok(Value::Bool(*value)),
                ExprKind::Str(value) => Ok(Value::Str(value.clone())),
                ExprKind::Bytes(value) => Ok(Value::Bytes(value.clone())),
                ExprKind::None => Ok(Value::Option(None)),
                ExprKind::Unit => Ok(Value::Unit),
                ExprKind::Ident(name) => self.env.get(name).cloned().ok_or_else(|| {
                    IonError::name(format!("undefined variable: {name}"), span.line, span.col)
                        .into()
                }),
                ExprKind::FStr(parts) => {
                    let mut out = String::new();
                    for part in parts {
                        match part {
                            FStrPart::Literal(text) => out.push_str(text),
                            FStrPart::Expr(expr) => {
                                out.push_str(&self.eval_expr(expr).await?.to_string())
                            }
                        }
                    }
                    Ok(Value::Str(out))
                }
                ExprKind::SomeExpr(inner) => {
                    let value = self.eval_expr(inner).await?;
                    Ok(Value::Option(Some(Box::new(value))))
                }
                ExprKind::OkExpr(inner) => {
                    let value = self.eval_expr(inner).await?;
                    Ok(Value::Result(Ok(Box::new(value))))
                }
                ExprKind::ErrExpr(inner) => {
                    let value = self.eval_expr(inner).await?;
                    Ok(Value::Result(Err(Box::new(value))))
                }
                ExprKind::List(items) => {
                    let mut values = Vec::new();
                    for entry in items {
                        match entry {
                            ListEntry::Elem(item) => values.push(self.eval_expr(item).await?),
                            ListEntry::Spread(item) => match self.eval_expr(item).await? {
                                Value::List(items) => values.extend(items),
                                other => {
                                    return Err(IonError::type_err(
                                        format!(
                                            "spread requires a list, got {}",
                                            other.type_name()
                                        ),
                                        span.line,
                                        span.col,
                                    )
                                    .into())
                                }
                            },
                        }
                    }
                    Ok(Value::List(values))
                }
                ExprKind::Tuple(items) => {
                    let mut values = Vec::new();
                    for item in items {
                        values.push(self.eval_expr(item).await?);
                    }
                    Ok(Value::Tuple(values))
                }
                ExprKind::Dict(entries) => {
                    let mut map = indexmap::IndexMap::new();
                    for entry in entries {
                        match entry {
                            DictEntry::KeyValue(key, value) => {
                                let key = self.eval_expr(key).await?;
                                let Value::Str(key) = key else {
                                    return Err(IonError::type_err(
                                        "dict keys must be strings",
                                        span.line,
                                        span.col,
                                    )
                                    .into());
                                };
                                let value = self.eval_expr(value).await?;
                                map.insert(key, value);
                            }
                            DictEntry::Spread(expr) => match self.eval_expr(expr).await? {
                                Value::Dict(other) => map.extend(other),
                                _ => {
                                    return Err(IonError::type_err(
                                        "spread requires a dict",
                                        span.line,
                                        span.col,
                                    )
                                    .into())
                                }
                            },
                        }
                    }
                    Ok(Value::Dict(map))
                }
                ExprKind::BinOp { left, op, right } => {
                    if matches!(op, BinOp::And) {
                        let left = self.eval_expr(left).await?;
                        if !left.is_truthy() {
                            return Ok(Value::Bool(false));
                        }
                        return Ok(Value::Bool(self.eval_expr(right).await?.is_truthy()));
                    }
                    if matches!(op, BinOp::Or) {
                        let left = self.eval_expr(left).await?;
                        if left.is_truthy() {
                            return Ok(Value::Bool(true));
                        }
                        return Ok(Value::Bool(self.eval_expr(right).await?.is_truthy()));
                    }
                    let left = self.eval_expr(left).await?;
                    let right = self.eval_expr(right).await?;
                    self.eval_binop(*op, left, right, span)
                }
                ExprKind::UnaryOp { op, expr } => {
                    let value = self.eval_expr(expr).await?;
                    match op {
                        UnaryOp::Neg => match value {
                            Value::Int(n) => Ok(Value::Int(-n)),
                            Value::Float(n) => Ok(Value::Float(-n)),
                            other => Err(IonError::type_err(
                                format!("cannot negate {}", other.type_name()),
                                span.line,
                                span.col,
                            )
                            .into()),
                        },
                        UnaryOp::Not => Ok(Value::Bool(!value.is_truthy())),
                    }
                }
                ExprKind::Try(inner) => {
                    let value = self.eval_expr(inner).await?;
                    match value {
                        Value::Result(Ok(value)) => Ok(*value),
                        Value::Result(Err(err)) => {
                            Err(
                                IonError::propagated_err(err.to_string(), span.line, span.col)
                                    .into(),
                            )
                        }
                        Value::Option(Some(value)) => Ok(*value),
                        Value::Option(None) => {
                            Err(IonError::propagated_none(span.line, span.col).into())
                        }
                        other => Err(IonError::type_err(
                            format!("? applied to non-Result/Option: {}", other.type_name()),
                            span.line,
                            span.col,
                        )
                        .into()),
                    }
                }
                ExprKind::Call { func, args } => {
                    if args.iter().any(|arg| arg.name.is_some()) {
                        return Err(IonError::runtime(
                            "named arguments are not supported by the async host bridge yet",
                            span.line,
                            span.col,
                        )
                        .into());
                    }
                    let func = self.eval_expr(func).await?;
                    let mut values = Vec::with_capacity(args.len());
                    for arg in args {
                        values.push(self.eval_expr(&arg.value).await?);
                    }
                    self.call_value(func, values, span).await
                }
                ExprKind::SpawnExpr(expr) => {
                    if self.nursery_stack.borrow().is_empty() {
                        return Err(IonError::runtime(
                            "spawn is only allowed inside async {}",
                            span.line,
                            span.col,
                        )
                        .into());
                    }

                    let expr = (**expr).clone();
                    let captures = self.env.capture();
                    let limits = self.limits.clone();
                    let tasks = Rc::clone(&self.tasks);
                    let nursery_stack = Rc::clone(&self.nursery_stack);
                    let task = AsyncTask::new(Box::pin(async move {
                        let mut env = Env::new();
                        for (name, value) in captures {
                            env.define(name, value, false);
                        }
                        let mut child = AsyncTreeInterpreter::new_with_runtime(
                            &mut env,
                            limits,
                            tasks,
                            nursery_stack,
                        );
                        match child.eval_expr(&expr).await {
                            Ok(value) => Ok(value),
                            Err(AsyncSignalOrError::Signal(AsyncSignal::Return(value))) => {
                                Ok(value)
                            }
                            Err(AsyncSignalOrError::Error(err)) => Err(err),
                        }
                    }));
                    self.tasks.borrow_mut().push(task.clone());
                    if let Some(nursery) = self.nursery_stack.borrow_mut().last_mut() {
                        nursery.push(task.clone());
                    }
                    Ok(Value::AsyncTask(task))
                }
                ExprKind::AwaitExpr(expr) => {
                    let value = self.eval_expr(expr).await?;
                    let Value::AsyncTask(task) = value else {
                        return Err(IonError::type_err(
                            format!("cannot await {}", value.type_name()),
                            span.line,
                            span.col,
                        )
                        .into());
                    };
                    self.await_task(task).await.map_err(Into::into)
                }
                ExprKind::Lambda { params, body } => {
                    let captures = self.env.capture();
                    let fn_params = params
                        .iter()
                        .map(|name| crate::ast::Param {
                            name: name.clone(),
                            default: None,
                        })
                        .collect();
                    let body = vec![Stmt {
                        kind: StmtKind::ExprStmt {
                            expr: (**body).clone(),
                            has_semi: false,
                        },
                        span,
                    }];
                    Ok(Value::Fn(crate::value::IonFn::new(
                        "<lambda>".to_string(),
                        fn_params,
                        body,
                        captures,
                    )))
                }
                ExprKind::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    let cond = self.eval_expr(cond).await?;
                    self.env.push_scope();
                    let result = if cond.is_truthy() {
                        self.eval_stmts(then_body).await
                    } else if let Some(else_body) = else_body {
                        self.eval_stmts(else_body).await
                    } else {
                        Ok(Value::Unit)
                    };
                    self.env.pop_scope();
                    result
                }
                ExprKind::Block(stmts) => {
                    self.env.push_scope();
                    let result = self.eval_stmts(stmts).await;
                    self.env.pop_scope();
                    result
                }
                ExprKind::TryCatch { body, var, handler } => {
                    self.env.push_scope();
                    let body_result = self.eval_stmts(body).await;
                    self.env.pop_scope();
                    match body_result {
                        Ok(value) => Ok(value),
                        Err(AsyncSignalOrError::Signal(signal)) => {
                            Err(AsyncSignalOrError::Signal(signal))
                        }
                        Err(AsyncSignalOrError::Error(err)) => {
                            self.env.push_scope();
                            self.env.define(var.clone(), Value::Str(err.message), false);
                            let handler_result = self.eval_stmts(handler).await;
                            self.env.pop_scope();
                            handler_result
                        }
                    }
                }
                ExprKind::AsyncBlock(body) => {
                    self.nursery_stack.borrow_mut().push(Vec::new());
                    self.env.push_scope();
                    let body_result = self.eval_stmts(body).await;
                    self.env.pop_scope();
                    let nursery_tasks = self
                        .nursery_stack
                        .borrow_mut()
                        .pop()
                        .expect("async nursery disappeared");

                    let mut join_error = None;
                    for task in nursery_tasks {
                        if let Err(err) = self.await_task(task).await {
                            join_error.get_or_insert(err);
                        }
                    }

                    match (body_result, join_error) {
                        (Ok(value), None) => Ok(value),
                        (Ok(_), Some(err)) => Err(err.into()),
                        (Err(err), _) => Err(err),
                    }
                }
                ExprKind::SelectExpr(branches) => {
                    if branches.is_empty() {
                        return Err(IonError::runtime(
                            "select requires at least one branch",
                            span.line,
                            span.col,
                        )
                        .into());
                    }

                    let mut branch_tasks = Vec::with_capacity(branches.len());
                    for branch in branches {
                        let expr = branch.future_expr.clone();
                        let captures = self.env.capture();
                        let limits = self.limits.clone();
                        let tasks = Rc::clone(&self.tasks);
                        let nursery_stack = Rc::clone(&self.nursery_stack);
                        let task = AsyncTask::new(Box::pin(async move {
                            let mut env = Env::new();
                            for (name, value) in captures {
                                env.define(name, value, false);
                            }
                            let mut child = AsyncTreeInterpreter::new_with_runtime(
                                &mut env,
                                limits,
                                tasks,
                                nursery_stack,
                            );
                            match child.eval_expr(&expr).await {
                                Ok(Value::AsyncTask(task)) => {
                                    std::future::poll_fn(|cx| task.poll_result(cx)).await
                                }
                                Ok(value) => Ok(value),
                                Err(AsyncSignalOrError::Signal(AsyncSignal::Return(value))) => {
                                    Ok(value)
                                }
                                Err(AsyncSignalOrError::Error(err)) => Err(err),
                            }
                        }));
                        self.tasks.borrow_mut().push(task.clone());
                        branch_tasks.push(task);
                    }

                    let (winner, value) = self.select_ready_branch(&branch_tasks).await?;
                    let branch = &branches[winner];
                    self.env.push_scope();
                    let bind_result = self.bind_pattern(&branch.pattern, value, false, span);
                    let body_result = match bind_result {
                        Ok(()) => self.eval_expr(&branch.body).await,
                        Err(err) => Err(err),
                    };
                    self.env.pop_scope();
                    body_result
                }
                other => Err(IonError::runtime(
                    format!(
                        "expression is not supported by the async host bridge yet: {:?}",
                        std::mem::discriminant(other)
                    ),
                    span.line,
                    span.col,
                )
                .into()),
            }
        })
    }

    async fn await_task(&self, task: AsyncTask) -> Result<Value, IonError> {
        std::future::poll_fn(|cx| {
            let tasks = self.tasks.borrow().clone();
            for spawned in tasks {
                if !spawned.ptr_eq(&task) {
                    let _ = spawned.poll_result(cx);
                }
            }
            task.poll_result(cx)
        })
        .await
    }

    async fn select_ready_branch(
        &self,
        branches: &[AsyncTask],
    ) -> Result<(usize, Value), IonError> {
        std::future::poll_fn(|cx| {
            let tasks = self.tasks.borrow().clone();
            for spawned in tasks {
                if !branches.iter().any(|branch| spawned.ptr_eq(branch)) {
                    let _ = spawned.poll_result(cx);
                }
            }

            for (idx, branch) in branches.iter().enumerate() {
                match branch.poll_result(cx) {
                    Poll::Ready(Ok(value)) => return Poll::Ready(Ok((idx, value))),
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => {}
                }
            }

            Poll::Pending
        })
        .await
    }

    fn call_value<'a>(
        &'a mut self,
        func: Value,
        args: Vec<Value>,
        span: Span,
    ) -> LocalBoxFuture<'a, AsyncSignalResult> {
        Box::pin(async move {
            match func {
                Value::Fn(ion_fn) => {
                    if self.call_depth >= self.limits.max_call_depth {
                        return Err(IonError::runtime(
                            "maximum call depth exceeded",
                            span.line,
                            span.col,
                        )
                        .into());
                    }

                    self.call_depth += 1;
                    self.env.push_scope();
                    for (name, value) in &ion_fn.captures {
                        self.env.define(name.clone(), value.clone(), false);
                    }

                    let mut bind_result = Ok(());
                    for (idx, param) in ion_fn.params.iter().enumerate() {
                        let value = if idx < args.len() {
                            args[idx].clone()
                        } else if let Some(default) = &param.default {
                            match self.eval_expr(default).await {
                                Ok(value) => value,
                                Err(err) => {
                                    bind_result = Err(err);
                                    break;
                                }
                            }
                        } else {
                            bind_result = Err(IonError::runtime(
                                format!(
                                    "function '{}' expected {} arguments, got {}",
                                    ion_fn.name,
                                    ion_fn.params.len(),
                                    args.len()
                                ),
                                span.line,
                                span.col,
                            )
                            .into());
                            break;
                        };
                        self.env.define(param.name.clone(), value, false);
                    }

                    let result = match bind_result {
                        Ok(()) => self.eval_stmts(&ion_fn.body).await,
                        Err(err) => Err(err),
                    };
                    self.env.pop_scope();
                    self.call_depth -= 1;

                    match result {
                        Ok(value) => Ok(value),
                        Err(AsyncSignalOrError::Signal(AsyncSignal::Return(value))) => Ok(value),
                        Err(AsyncSignalOrError::Error(err))
                            if err.kind == ErrorKind::PropagatedErr =>
                        {
                            Ok(Value::Result(Err(Box::new(Value::Str(err.message)))))
                        }
                        Err(AsyncSignalOrError::Error(err))
                            if err.kind == ErrorKind::PropagatedNone =>
                        {
                            Ok(Value::Option(None))
                        }
                        Err(err) => Err(err),
                    }
                }
                Value::BuiltinFn { func, .. } => {
                    func(&args).map_err(|msg| IonError::runtime(msg, span.line, span.col).into())
                }
                Value::BuiltinClosure { func, .. } => func
                    .call(&args)
                    .map_err(|msg| IonError::runtime(msg, span.line, span.col).into()),
                Value::AsyncBuiltinClosure { func, .. } => {
                    func.call(args).await.map_err(Into::into)
                }
                other => Err(IonError::type_err(
                    format!("not callable: {}", other.type_name()),
                    span.line,
                    span.col,
                )
                .into()),
            }
        })
    }

    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        value: Value,
        mutable: bool,
        span: Span,
    ) -> Result<(), AsyncSignalOrError> {
        match pattern {
            Pattern::Ident(name) => {
                self.env.define(name.clone(), value, mutable);
                Ok(())
            }
            Pattern::Wildcard => Ok(()),
            _ => Err(IonError::runtime(
                "pattern is not supported by the async host bridge yet",
                span.line,
                span.col,
            )
            .into()),
        }
    }

    fn eval_binop(&self, op: BinOp, left: Value, right: Value, span: Span) -> AsyncSignalResult {
        match op {
            BinOp::Add => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + b as f64)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
                (a, b) => Err(IonError::type_err(
                    format!("cannot add {} and {}", a.type_name(), b.type_name()),
                    span.line,
                    span.col,
                )
                .into()),
            },
            BinOp::Sub => numeric_binop(left, right, span, "subtract", |a, b| a - b, |a, b| a - b),
            BinOp::Mul => numeric_binop(left, right, span, "multiply", |a, b| a * b, |a, b| a * b),
            BinOp::Div => match (&left, &right) {
                (_, Value::Int(0)) => {
                    Err(IonError::runtime("division by zero", span.line, span.col).into())
                }
                (_, Value::Float(value)) if *value == 0.0 => {
                    Err(IonError::runtime("division by zero", span.line, span.col).into())
                }
                _ => numeric_binop(left, right, span, "divide", |a, b| a / b, |a, b| a / b),
            },
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => compare_binop(op, left, right, span),
            _ => Err(IonError::runtime(
                "operator is not supported by the async host bridge yet",
                span.line,
                span.col,
            )
            .into()),
        }
    }
}

fn numeric_binop(
    left: Value,
    right: Value,
    span: Span,
    verb: &str,
    int_op: impl FnOnce(i64, i64) -> i64,
    float_op: impl FnOnce(f64, f64) -> f64,
) -> AsyncSignalResult {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(int_op(a, b))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(a, b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(a as f64, b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(a, b as f64))),
        (a, b) => Err(IonError::type_err(
            format!("cannot {verb} {} and {}", a.type_name(), b.type_name()),
            span.line,
            span.col,
        )
        .into()),
    }
}

fn compare_binop(op: BinOp, left: Value, right: Value, span: Span) -> AsyncSignalResult {
    let ord = match (left, right) {
        (Value::Int(a), Value::Int(b)) => a.partial_cmp(&b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(&b),
        (Value::Int(a), Value::Float(b)) => (a as f64).partial_cmp(&b),
        (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(b as f64)),
        (Value::Str(a), Value::Str(b)) => a.partial_cmp(&b),
        (a, b) => {
            return Err(IonError::type_err(
                format!("cannot compare {} and {}", a.type_name(), b.type_name()),
                span.line,
                span.col,
            )
            .into())
        }
    };
    let Some(ord) = ord else {
        return Ok(Value::Bool(false));
    };
    let value = match op {
        BinOp::Lt => ord.is_lt(),
        BinOp::Gt => ord.is_gt(),
        BinOp::Le => ord.is_le(),
        BinOp::Ge => ord.is_ge(),
        _ => unreachable!("compare_binop called for non-comparison operator"),
    };
    Ok(Value::Bool(value))
}

fn program_references_async_host(program: &Program, env: &Env) -> bool {
    program
        .stmts
        .iter()
        .any(|stmt| stmt_references_async_host(stmt, env))
}

fn stmt_references_async_host(stmt: &Stmt, env: &Env) -> bool {
    match &stmt.kind {
        StmtKind::Let { value, .. } | StmtKind::ExprStmt { expr: value, .. } => {
            expr_references_async_host(value, env)
        }
        StmtKind::FnDecl { body, .. } | StmtKind::Loop { body, .. } => body
            .iter()
            .any(|stmt| stmt_references_async_host(stmt, env)),
        StmtKind::For { iter, body, .. } => {
            expr_references_async_host(iter, env)
                || body
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
        }
        StmtKind::While { cond, body, .. } => {
            expr_references_async_host(cond, env)
                || body
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
        }
        StmtKind::WhileLet { expr, body, .. } => {
            expr_references_async_host(expr, env)
                || body
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
        }
        StmtKind::Break { value, .. } | StmtKind::Return { value } => value
            .as_ref()
            .is_some_and(|expr| expr_references_async_host(expr, env)),
        StmtKind::Assign { target, value, .. } => {
            assign_target_references_async_host(target, env)
                || expr_references_async_host(value, env)
        }
        StmtKind::Continue { .. } | StmtKind::Use { .. } => false,
    }
}

fn assign_target_references_async_host(target: &crate::ast::AssignTarget, env: &Env) -> bool {
    match target {
        crate::ast::AssignTarget::Ident(_) => false,
        crate::ast::AssignTarget::Index(expr, index) => {
            expr_references_async_host(expr, env) || expr_references_async_host(index, env)
        }
        crate::ast::AssignTarget::Field(expr, _) => expr_references_async_host(expr, env),
    }
}

fn expr_references_async_host(expr: &Expr, env: &Env) -> bool {
    match &expr.kind {
        ExprKind::Ident(name) => env.get(name).is_some_and(value_is_async_host),
        ExprKind::ModulePath(segments) => {
            module_path_value(segments, env).is_some_and(value_is_async_host)
        }
        ExprKind::SomeExpr(expr)
        | ExprKind::OkExpr(expr)
        | ExprKind::ErrExpr(expr)
        | ExprKind::UnaryOp { expr, .. }
        | ExprKind::Try(expr)
        | ExprKind::FieldAccess { expr, .. }
        | ExprKind::AwaitExpr(expr)
        | ExprKind::SpawnExpr(expr) => expr_references_async_host(expr, env),
        ExprKind::List(items) => items.iter().any(|item| match item {
            ListEntry::Elem(expr) | ListEntry::Spread(expr) => {
                expr_references_async_host(expr, env)
            }
        }),
        ExprKind::Dict(entries) => entries.iter().any(|entry| match entry {
            DictEntry::KeyValue(key, value) => {
                expr_references_async_host(key, env) || expr_references_async_host(value, env)
            }
            DictEntry::Spread(expr) => expr_references_async_host(expr, env),
        }),
        ExprKind::Tuple(items) => items
            .iter()
            .any(|expr| expr_references_async_host(expr, env)),
        ExprKind::ListComp {
            expr, iter, cond, ..
        } => {
            expr_references_async_host(expr, env)
                || expr_references_async_host(iter, env)
                || cond
                    .as_ref()
                    .is_some_and(|expr| expr_references_async_host(expr, env))
        }
        ExprKind::DictComp {
            key,
            value,
            iter,
            cond,
            ..
        } => {
            expr_references_async_host(key, env)
                || expr_references_async_host(value, env)
                || expr_references_async_host(iter, env)
                || cond
                    .as_ref()
                    .is_some_and(|expr| expr_references_async_host(expr, env))
        }
        ExprKind::BinOp { left, right, .. } | ExprKind::PipeOp { left, right } => {
            expr_references_async_host(left, env) || expr_references_async_host(right, env)
        }
        ExprKind::Index { expr, index } => {
            expr_references_async_host(expr, env) || expr_references_async_host(index, env)
        }
        ExprKind::Slice {
            expr, start, end, ..
        } => {
            expr_references_async_host(expr, env)
                || start
                    .as_ref()
                    .is_some_and(|expr| expr_references_async_host(expr, env))
                || end
                    .as_ref()
                    .is_some_and(|expr| expr_references_async_host(expr, env))
        }
        ExprKind::MethodCall { expr, args, .. } => {
            expr_references_async_host(expr, env)
                || args
                    .iter()
                    .any(|arg| expr_references_async_host(&arg.value, env))
        }
        ExprKind::Call { func, args } => {
            expr_references_async_host(func, env)
                || args
                    .iter()
                    .any(|arg| expr_references_async_host(&arg.value, env))
        }
        ExprKind::Lambda { body, .. } => expr_references_async_host(body, env),
        ExprKind::If {
            cond,
            then_body,
            else_body,
        } => {
            expr_references_async_host(cond, env)
                || then_body
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
                || else_body.as_ref().is_some_and(|body| {
                    body.iter()
                        .any(|stmt| stmt_references_async_host(stmt, env))
                })
        }
        ExprKind::IfLet {
            expr,
            then_body,
            else_body,
            ..
        } => {
            expr_references_async_host(expr, env)
                || then_body
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
                || else_body.as_ref().is_some_and(|body| {
                    body.iter()
                        .any(|stmt| stmt_references_async_host(stmt, env))
                })
        }
        ExprKind::Match { expr, arms } => {
            expr_references_async_host(expr, env)
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|guard| expr_references_async_host(guard, env))
                        || expr_references_async_host(&arm.body, env)
                })
        }
        ExprKind::Block(body) | ExprKind::LoopExpr(body) | ExprKind::AsyncBlock(body) => body
            .iter()
            .any(|stmt| stmt_references_async_host(stmt, env)),
        ExprKind::TryCatch { body, handler, .. } => {
            body.iter()
                .any(|stmt| stmt_references_async_host(stmt, env))
                || handler
                    .iter()
                    .any(|stmt| stmt_references_async_host(stmt, env))
        }
        ExprKind::StructConstruct { fields, spread, .. } => {
            fields
                .iter()
                .any(|(_, expr)| expr_references_async_host(expr, env))
                || spread
                    .as_ref()
                    .is_some_and(|expr| expr_references_async_host(expr, env))
        }
        ExprKind::EnumVariantCall { args, .. } => args
            .iter()
            .any(|expr| expr_references_async_host(expr, env)),
        ExprKind::Range { start, end, .. } => {
            expr_references_async_host(start, env) || expr_references_async_host(end, env)
        }
        ExprKind::SelectExpr(branches) => branches.iter().any(|branch| {
            expr_references_async_host(&branch.future_expr, env)
                || expr_references_async_host(&branch.body, env)
        }),
        ExprKind::FStr(parts) => parts.iter().any(|part| match part {
            FStrPart::Literal(_) => false,
            FStrPart::Expr(expr) => expr_references_async_host(expr, env),
        }),
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Bytes(_)
        | ExprKind::None
        | ExprKind::Unit
        | ExprKind::EnumVariant { .. } => false,
    }
}

fn value_is_async_host(value: &Value) -> bool {
    matches!(value, Value::AsyncBuiltinClosure { .. })
}

fn module_path_value<'a>(segments: &[String], env: &'a Env) -> Option<&'a Value> {
    let mut current = env.get(segments.first()?)?;
    for segment in &segments[1..] {
        match current {
            Value::Module(table) => current = table.items.get(&crate::hash::h(segment))?,
            Value::Dict(map) => current = map.get(segment)?,
            _ => return None,
        }
    }
    Some(current)
}

fn install_async_runtime_builtins(env: &mut Env) {
    fn install_async(env: &mut Env, name_hash: u64, func: crate::value::AsyncBuiltinClosureFn) {
        if env.get_h(name_hash).is_none() {
            env.define_h(
                name_hash,
                Value::AsyncBuiltinClosure {
                    qualified_hash: name_hash,
                    func,
                },
            );
        }
    }

    install_async(
        env,
        crate::h!("sleep"),
        crate::value::AsyncBuiltinClosureFn::new(|args| async move {
            let ms = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::runtime("sleep requires int (ms)", 0, 0))?;
            tokio::time::sleep(Duration::from_millis(ms as u64)).await;
            Ok(Value::Unit)
        }),
    );

    install_async(
        env,
        crate::h!("channel"),
        crate::value::AsyncBuiltinClosureFn::new(|args| async move {
            let capacity = args
                .first()
                .and_then(Value::as_int)
                .ok_or_else(|| IonError::runtime("channel requires int capacity", 0, 0))?;
            let capacity = usize::try_from(capacity).unwrap_or(0).max(1);
            let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
            Ok(Value::Tuple(vec![
                Value::AsyncChannelSender(NativeChannelSender::new(sender)),
                Value::AsyncChannelReceiver(NativeChannelReceiver::new(receiver)),
            ]))
        }),
    );

    install_async(
        env,
        crate::h!("timeout"),
        crate::value::AsyncBuiltinClosureFn::new(|_args| async move {
            Err(IonError::runtime(
                "timeout requires the async VM runtime",
                0,
                0,
            ))
        }),
    );
}

#[allow(dead_code)]
async fn eval_async_bridge(engine: &mut Engine, source: &str) -> Result<Value, IonError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program()?;
    if !program_references_async_host(&program, &engine.interpreter().env) {
        return engine.eval_sync_internal(source);
    }

    let interpreter = engine.interpreter_mut();
    AsyncTreeInterpreter::new(&mut interpreter.env, interpreter.limits.clone())
        .eval_program(&program)
        .await
}

async fn eval_async_entry(engine: &mut Engine, source: &str) -> Result<Value, IonError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program()?;

    let mut async_env = engine.interpreter().env.clone();
    install_async_runtime_builtins(&mut async_env);
    if !program_references_async_host(&program, &async_env) {
        return engine.eval_sync_internal(source);
    }

    let (chunk, fn_chunks) = Compiler::new().compile_program(&program)?;
    let mut runtime = IonRuntime::from_compiled(engine, chunk, fn_chunks);
    std::future::poll_fn(move |cx| runtime.poll(cx)).await
}

/// Future returned by `Engine::eval_async`.
///
/// This uses the pollable bytecode continuation runtime for compilable
/// async-host programs. Programs without async host references continue to
/// use the existing synchronous evaluator.
pub struct IonEvalFuture<'a> {
    inner: LocalBoxFuture<'a, Result<Value, IonError>>,
}

impl<'a> IonEvalFuture<'a> {
    pub(crate) fn new(engine: &'a mut Engine, source: &'a str) -> Self {
        Self {
            inner: Box::pin(eval_async_entry(engine, source)),
        }
    }
}

impl Future for IonEvalFuture<'_> {
    type Output = Result<Value, IonError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(cx)
    }
}

/// Async runtime owner for compiled `eval_async` programs.
///
/// This drives a resumable bytecode continuation plus runtime tables for
/// host futures, timers, channels, nurseries, and external requests.
#[allow(dead_code)]
pub struct IonRuntime<'a> {
    engine: Option<&'a mut Engine>,
    source: &'a str,
    root_task: TaskId,
    waiting: Option<TaskState>,
    final_result: Option<Result<Value, IonError>>,
    continuation: Option<VmContinuation>,
    tasks: TaskTable,
    external_queue: ExternalQueue,
    chunks: ChunkArena,
    host_futures: HostFutureTable,
    timers: TimerTable,
    channels: ChannelTable,
    nurseries: NurseryTable,
    external_calls: Vec<ExternalCallTask>,
    waker: Option<Waker>,
}

enum RuntimeResume {
    None,
    Resumed,
    Done(Result<Value, IonError>),
}

#[allow(dead_code)]
impl<'a> IonRuntime<'a> {
    fn new(engine: &'a mut Engine, source: &'a str) -> Self {
        let external_queue = engine.external_queue();
        Self {
            engine: Some(engine),
            source,
            root_task: TaskId(0),
            waiting: None,
            final_result: None,
            continuation: None,
            tasks: TaskTable::new(),
            external_queue,
            chunks: ChunkArena::new(),
            host_futures: HostFutureTable::new(),
            timers: TimerTable::new(),
            channels: ChannelTable::new(),
            nurseries: NurseryTable::new(),
            external_calls: Vec::new(),
            waker: None,
        }
    }

    fn from_compiled(engine: &'a mut Engine, chunk: Chunk, fn_chunks: FnChunkCache) -> Self {
        let external_queue = engine.external_queue();
        let output = engine.output_handler();
        let types = engine.interpreter().types.clone();
        let env = std::mem::take(&mut engine.interpreter_mut().env);
        let mut chunks = ChunkArena::new();
        let root = chunks.insert(chunk);
        let mut continuation = VmContinuation::with_env(root, env);
        install_async_runtime_builtins(&mut continuation.env);
        continuation.set_output_handler(output);
        *continuation.types_mut() = types;
        for (fn_id, chunk) in fn_chunks {
            let chunk_id = chunks.insert(chunk);
            continuation.register_fn_chunk(fn_id, chunk_id);
        }

        Self {
            engine: Some(engine),
            source: "",
            root_task: TaskId(0),
            waiting: None,
            final_result: None,
            continuation: Some(continuation),
            tasks: TaskTable::new(),
            external_queue,
            chunks,
            host_futures: HostFutureTable::new(),
            timers: TimerTable::new(),
            channels: ChannelTable::new(),
            nurseries: NurseryTable::new(),
            external_calls: Vec::new(),
            waker: None,
        }
    }

    pub fn chunks(&mut self) -> &mut ChunkArena {
        &mut self.chunks
    }

    pub fn tasks(&mut self) -> &mut TaskTable {
        &mut self.tasks
    }

    pub fn host_futures(&mut self) -> &mut HostFutureTable {
        &mut self.host_futures
    }

    pub fn timers(&mut self) -> &mut TimerTable {
        &mut self.timers
    }

    pub fn channels(&mut self) -> &mut ChannelTable {
        &mut self.channels
    }

    pub fn nurseries(&mut self) -> &mut NurseryTable {
        &mut self.nurseries
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<Value, IonError>> {
        self.waker = Some(cx.waker().clone());
        self.external_queue.register_waker(cx.waker());

        let external_requests = self.external_queue.drain();
        self.enqueue_external_requests(external_requests);
        self.poll_external_calls(cx);

        if self.continuation.is_some() {
            return self.poll_continuation(cx);
        }

        let ready = self.host_futures.poll_ready(cx);
        debug_assert!(ready.is_empty(), "unexpected host future without VM task");
        let ready_timers = self.timers.poll_ready(cx);
        debug_assert!(
            ready_timers.is_empty(),
            "unexpected timer without VM task"
        );

        let source = self.source;
        let engine = self
            .engine
            .take()
            .expect("IonEvalFuture polled after completion");
        Poll::Ready(engine.eval_sync_internal(source))
    }

    fn enqueue_external_requests(&mut self, requests: Vec<ExternalRequest>) {
        for request in requests {
            match request {
                ExternalRequest::Call {
                    fn_name,
                    args,
                    result_tx,
                } => match self.external_call_future(&fn_name, args) {
                    Ok(future) => self.external_calls.push(ExternalCallTask {
                        future,
                        result_tx: Some(result_tx),
                    }),
                    Err(err) => {
                        let _ = result_tx.send(Err(err));
                    }
                },
            }
        }
    }

    fn external_call_future(
        &self,
        fn_name: &str,
        args: Vec<Value>,
    ) -> Result<BoxIonFuture, IonError> {
        let Some(cont) = self.continuation.as_ref() else {
            return Err(IonError::runtime(
                "external Ion call requires the async VM runtime",
                0,
                0,
            ));
        };
        let Some(func) = cont.env.get(fn_name).cloned() else {
            return Err(IonError::runtime(
                format!("external Ion call target '{fn_name}' not found"),
                0,
                0,
            ));
        };

        match func {
            Value::Fn(ion_fn) => spawned_ion_function_future(&self.chunks, cont, ion_fn, args, 0, 0),
            Value::AsyncBuiltinClosure { func: async_fn, .. } => Ok(async_fn.call(args)),
            Value::BuiltinFn { func: builtin, .. } => Ok(Box::pin(async move {
                builtin(&args).map_err(|err| IonError::runtime(err, 0, 0))
            })),
            Value::BuiltinClosure { func: builtin, .. } => Ok(Box::pin(async move {
                builtin
                    .call(&args)
                    .map_err(|err| IonError::runtime(err, 0, 0))
            })),
            other => Err(IonError::type_err(
                format!(
                    "external Ion call target '{fn_name}' is {}, not callable",
                    other.type_name()
                ),
                0,
                0,
            )),
        }
    }

    fn poll_external_calls(&mut self, cx: &mut Context<'_>) {
        let mut index = 0;
        while index < self.external_calls.len() {
            let poll_result = {
                let task = &mut self.external_calls[index];
                task.future.as_mut().poll(cx)
            };
            match poll_result {
                Poll::Ready(result) => {
                    let mut task = self.external_calls.swap_remove(index);
                    if let Some(result_tx) = task.result_tx.take() {
                        let _ = result_tx.send(result);
                    }
                }
                Poll::Pending => {
                    index += 1;
                }
            }
        }
    }

    fn poll_continuation(&mut self, cx: &mut Context<'_>) -> Poll<Result<Value, IonError>> {
        const STEP_BUDGET: usize = 1024;

        for _ in 0..STEP_BUDGET {
            if self.waiting.is_some() {
                match self.resume_ready_host_future(cx) {
                    RuntimeResume::None => return Poll::Pending,
                    RuntimeResume::Resumed => {}
                    RuntimeResume::Done(result) => return Poll::Ready(result),
                }
            }

            let cont = self
                .continuation
                .as_mut()
                .expect("compiled runtime missing continuation");
            match step_task_with_host_futures(
                &self.chunks,
                cont,
                self.root_task,
                &mut self.host_futures,
            ) {
                StepOutcome::Continue => {}
                StepOutcome::Yield => {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                StepOutcome::Suspended(state) => {
                    self.waiting = Some(state);
                    match self.resume_ready_host_future(cx) {
                        RuntimeResume::None => return Poll::Pending,
                        RuntimeResume::Resumed => {}
                        RuntimeResume::Done(result) => return Poll::Ready(result),
                    }
                }
                StepOutcome::InstructionError(err) => {
                    return Poll::Ready(self.finish_continuation(Err(err)));
                }
                StepOutcome::Done(result) => match self.schedule_final_join(result, cx) {
                    RuntimeResume::None => return Poll::Pending,
                    RuntimeResume::Resumed => {}
                    RuntimeResume::Done(result) => return Poll::Ready(result),
                },
            }
        }

        cx.waker().wake_by_ref();
        Poll::Pending
    }

    fn resume_ready_host_future(&mut self, cx: &mut Context<'_>) -> RuntimeResume {
        if let Some(cont) = self.continuation.as_ref() {
            for spawned in &cont.spawned_tasks {
                let _ = spawned.poll_result(cx);
            }
        }

        let ready = self.host_futures.poll_ready(cx);
        let mut resumed = false;
        for item in ready {
            if item.waiter != self.root_task {
                continue;
            }
            if let Some(final_result) = self.final_result.take() {
                self.waiting = None;
                let result = match item.result {
                    Ok(_) => final_result,
                    Err(err) => Err(err),
                };
                return RuntimeResume::Done(self.finish_continuation(result));
            }
            let cont = self
                .continuation
                .as_mut()
                .expect("compiled runtime missing continuation");
            match cont.resume_host_result(item.result) {
                StepOutcome::Continue => {
                    self.waiting = None;
                    resumed = true;
                }
                StepOutcome::InstructionError(err) => {
                    return RuntimeResume::Done(self.finish_continuation(Err(err)));
                }
                StepOutcome::Done(result) => {
                    return RuntimeResume::Done(self.finish_continuation(result));
                }
                StepOutcome::Yield | StepOutcome::Suspended(_) => {
                    resumed = true;
                }
            }
        }
        if resumed {
            RuntimeResume::Resumed
        } else {
            RuntimeResume::None
        }
    }

    fn schedule_final_join(
        &mut self,
        result: Result<Value, IonError>,
        cx: &mut Context<'_>,
    ) -> RuntimeResume {
        let spawned = self
            .continuation
            .as_ref()
            .map(|cont| cont.spawned_tasks.clone())
            .unwrap_or_default();
        if spawned.is_empty() {
            return RuntimeResume::Done(self.finish_continuation(result));
        }

        self.final_result = Some(result);
        let future_id = self
            .host_futures
            .insert(self.root_task, join_spawned_tasks_future(spawned));
        self.waiting = Some(TaskState::WaitingHostFuture(future_id));
        self.resume_ready_host_future(cx)
    }

    fn finish_continuation(&mut self, result: Result<Value, IonError>) -> Result<Value, IonError> {
        if let Some(mut cont) = self.continuation.take() {
            if let Some(engine) = self.engine.as_deref_mut() {
                engine.interpreter_mut().env = cont.take_env();
            }
        }
        result
    }
}
