use indexmap::IndexMap;
use serde_json;
use std::collections::{HashMap, HashSet};
use std::fmt;
#[cfg(feature = "async-runtime")]
use std::future::Future;
#[cfg(feature = "async-runtime")]
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::ast::{Param, ParamKind, Stmt};
#[cfg(feature = "vm")]
use crate::bytecode::Chunk;
#[cfg(feature = "async-runtime")]
use crate::error::IonError;

static NEXT_FN_ID: AtomicU64 = AtomicU64::new(1);

/// Runtime value representation.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(IndexMap<String, Value>),
    Tuple(Vec<Value>),
    Option(Option<Box<Value>>),
    Result(Result<Box<Value>, Box<Value>>),
    Fn(IonFn),
    /// Host-registered builtin function. `qualified_hash` is the precomputed
    /// `mix(module_hash, fn_name_hash)` so dispatch is one integer compare.
    /// The function's identifier is intentionally NOT stored as a string —
    /// names live only in the optional sidecar described in docs/hide-names.md.
    BuiltinFn {
        qualified_hash: u64,
        func: BuiltinFn,
        signature: Option<Arc<HostSignature>>,
    },
    /// Closure-backed builtin (captures host-side state like a
    /// `tokio::runtime::Handle`, DB pool, etc.). Same shape as `BuiltinFn`.
    BuiltinClosure {
        qualified_hash: u64,
        func: BuiltinClosureFn,
        signature: Option<Arc<HostSignature>>,
    },
    /// Async closure-backed builtin — only executable by the async runtime.
    #[cfg(feature = "async-runtime")]
    AsyncBuiltinClosure {
        qualified_hash: u64,
        func: AsyncBuiltinClosureFn,
        signature: Option<Arc<HostSignature>>,
    },
    /// Host-registered module. Items (functions, constants, submodules) are
    /// keyed by name hash; lookup is `IndexMap::get(&hash)` — no string
    /// compare. See `module::ModuleTable`.
    Module(Arc<crate::module::ModuleTable>),
    /// Async-runtime task handle used by the native async scaffold.
    #[cfg(feature = "async-runtime")]
    AsyncTask(crate::async_runtime::AsyncTask),
    /// Native async-runtime channel sender.
    #[cfg(feature = "async-runtime")]
    AsyncChannelSender(crate::async_runtime::NativeChannelSender),
    /// Native async-runtime channel receiver.
    #[cfg(feature = "async-runtime")]
    AsyncChannelReceiver(crate::async_runtime::NativeChannelReceiver),
    /// Ordered set of unique values
    Set(Vec<Value>),
    /// Host-injected struct: `TypeName { field: val, ... }`.
    /// `type_hash` is `h("TypeName")` computed at macro-expansion / parse time.
    /// `fields` keys are `h("field_name")`. Field names from script source
    /// never appear in the host binary's `.rodata` — see docs/hide-names.md.
    HostStruct {
        type_hash: u64,
        fields: IndexMap<u64, Value>,
    },
    /// Host-injected enum variant: `EnumName::Variant` or `EnumName::Variant(data)`.
    /// Both `enum_hash` and `variant_hash` are FNV-1a 64-bit hashes computed
    /// at macro-expansion / parse time. `data` is `Vec<Value>` because
    /// `SmallVec<[Value; N]>` would make `Value` recursive without indirection,
    /// and `SmallVec<[Box<Value>; N]>` adds a per-element allocation that
    /// outweighs the inline-storage win.
    HostEnum {
        enum_hash: u64,
        variant_hash: u64,
        data: Vec<Value>,
    },
    /// Async task handle (legacy-threaded-concurrency feature)
    #[cfg(all(
        feature = "legacy-threaded-concurrency",
        not(feature = "async-runtime")
    ))]
    Task(std::sync::Arc<dyn crate::async_rt::TaskHandle>),
    /// Channel sender/receiver pair
    #[cfg(all(
        feature = "legacy-threaded-concurrency",
        not(feature = "async-runtime")
    ))]
    Channel(crate::async_rt::ChannelEnd),
    /// Shared mutable reference cell for closure state
    Cell(Arc<Mutex<Value>>),
    /// Lazy integer range (start..end or start..=end)
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },
    Unit,
}

/// A function value.
#[derive(Debug, Clone)]
pub struct IonFn {
    pub fn_id: u64,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// Captured environment for closures
    pub captures: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct HostSignature {
    pub params: Vec<HostParam>,
    pub has_var_args: bool,
    pub has_var_kwargs: bool,
}

#[derive(Debug, Clone)]
pub struct HostParam {
    pub name_hash: u64,
    pub kind: ParamKind,
    pub default: Option<Value>,
}

impl HostSignature {
    pub fn builder() -> HostSignatureBuilder {
        HostSignatureBuilder { params: Vec::new() }
    }
}

#[derive(Debug, Default)]
pub struct HostSignatureBuilder {
    params: Vec<HostParam>,
}

impl HostSignatureBuilder {
    pub fn pos_required(mut self, name_hash: u64) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::Positional,
            default: None,
        });
        self
    }

    pub fn pos(mut self, name_hash: u64, default: Value) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::Positional,
            default: Some(default),
        });
        self
    }

    pub fn pos_only_required(mut self, name_hash: u64) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::PositionalOnly,
            default: None,
        });
        self
    }

    pub fn pos_only(mut self, name_hash: u64, default: Value) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::PositionalOnly,
            default: Some(default),
        });
        self
    }

    pub fn kw_only_required(mut self, name_hash: u64) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::KeywordOnly,
            default: None,
        });
        self
    }

    pub fn kw_only(mut self, name_hash: u64, default: Value) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::KeywordOnly,
            default: Some(default),
        });
        self
    }

    pub fn var_args(mut self, name_hash: u64) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::VarArgs,
            default: None,
        });
        self
    }

    pub fn var_kwargs(mut self, name_hash: u64) -> Self {
        self.params.push(HostParam {
            name_hash,
            kind: ParamKind::VarKwargs,
            default: None,
        });
        self
    }

    pub fn build(self) -> HostSignature {
        let mut seen = HashSet::new();
        let mut has_var_args = false;
        let mut has_var_kwargs = false;
        for param in &self.params {
            if !seen.insert(param.name_hash) {
                panic!("{}", ion_str!("duplicate host parameter"));
            }
            match param.kind {
                ParamKind::VarArgs => {
                    if has_var_args {
                        panic!("{}", ion_str!("duplicate *args parameter"));
                    }
                    has_var_args = true;
                }
                ParamKind::VarKwargs => {
                    if has_var_kwargs {
                        panic!("{}", ion_str!("duplicate **kwargs parameter"));
                    }
                    has_var_kwargs = true;
                }
                _ => {}
            }
        }
        HostSignature {
            params: self.params,
            has_var_args,
            has_var_kwargs,
        }
    }
}

pub struct HostArgs<'a> {
    values: &'a [Value],
    signature: &'a HostSignature,
}

impl<'a> HostArgs<'a> {
    pub fn new(values: &'a [Value], signature: &'a HostSignature) -> Self {
        Self { values, signature }
    }

    pub fn get(&self, name_hash: u64) -> Option<&'a Value> {
        self.signature
            .params
            .iter()
            .position(|param| param.name_hash == name_hash)
            .and_then(|idx| self.values.get(idx))
    }

    pub fn get_str(&self, name_hash: u64) -> Result<&'a str, String> {
        self.get(name_hash)
            .and_then(Value::as_str)
            .ok_or_else(|| ion_str!("expected string argument"))
    }
}

impl IonFn {
    pub fn new(
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        captures: HashMap<String, Value>,
    ) -> Self {
        Self {
            fn_id: NEXT_FN_ID.fetch_add(1, Ordering::Relaxed),
            name,
            params,
            body,
            captures,
        }
    }
}

/// Precompiled function chunk, keyed by fn_id.
#[cfg(feature = "vm")]
pub type FnChunkCache = HashMap<u64, Chunk>;

/// A built-in function: Rust-side callback.
pub type BuiltinFn = fn(&[Value]) -> Result<Value, String>;
pub type BuiltinClosure = dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync;
#[cfg(feature = "async-runtime")]
pub type BoxIonFuture = Pin<Box<dyn Future<Output = Result<Value, IonError>> + 'static>>;
#[cfg(feature = "async-runtime")]
pub type AsyncBuiltinClosure = dyn Fn(Vec<Value>) -> BoxIonFuture + 'static;
#[cfg(feature = "async-runtime")]
pub type AsyncHostFn = dyn Fn(Vec<Value>) -> HostCallResult + 'static;

/// Result of invoking a host function in the future pollable async VM.
#[cfg(feature = "async-runtime")]
pub enum HostCallResult {
    Ready(Result<Value, IonError>),
    Pending(BoxIonFuture),
}

/// Wrapper around a closure-backed builtin so `Value` can still derive
/// `Debug`. `dyn Fn` doesn't implement `Debug`; we print a placeholder.
#[derive(Clone)]
pub struct BuiltinClosureFn(pub Arc<BuiltinClosure>);

impl fmt::Debug for BuiltinClosureFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ion_obf_string!("<closure>").as_str())
    }
}

impl BuiltinClosureFn {
    pub fn new<F>(func: F) -> Self
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        Self(Arc::new(func))
    }

    pub fn call(&self, args: &[Value]) -> Result<Value, String> {
        (self.0)(args)
    }
}

/// Wrapper around an async closure-backed builtin so `Value` can still
/// derive `Debug`.
#[cfg(feature = "async-runtime")]
#[derive(Clone)]
pub struct AsyncBuiltinClosureFn(pub Arc<AsyncBuiltinClosure>);

#[cfg(feature = "async-runtime")]
impl fmt::Debug for AsyncBuiltinClosureFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ion_obf_string!("<async closure>").as_str())
    }
}

#[cfg(feature = "async-runtime")]
impl AsyncBuiltinClosureFn {
    pub fn new<F, Fut>(func: F) -> Self
    where
        F: Fn(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Value, IonError>> + 'static,
    {
        Self(Arc::new(move |args| Box::pin(func(args))))
    }

    pub fn call(&self, args: Vec<Value>) -> BoxIonFuture {
        (self.0)(args)
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => ion_static_str!("int"),
            Value::Float(_) => ion_static_str!("float"),
            Value::Bool(_) => ion_static_str!("bool"),
            Value::Str(_) => ion_static_str!("string"),
            Value::Bytes(_) => ion_static_str!("bytes"),
            Value::List(_) => ion_static_str!("list"),
            Value::Dict(_) => ion_static_str!("dict"),
            Value::Tuple(_) => ion_static_str!("tuple"),
            Value::Set(_) => ion_static_str!("set"),
            Value::Option(_) => ion_static_str!("Option"),
            Value::Result(_) => ion_static_str!("Result"),
            Value::Fn(_) => ion_static_str!("fn"),
            Value::BuiltinFn { .. } => ion_static_str!("builtin_fn"),
            Value::BuiltinClosure { .. } => ion_static_str!("builtin_fn"),
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { .. } => ion_static_str!("async_builtin_fn"),
            Value::Module(_) => ion_static_str!("module"),
            #[cfg(feature = "async-runtime")]
            Value::AsyncTask(_) => ion_static_str!("AsyncTask"),
            #[cfg(feature = "async-runtime")]
            Value::AsyncChannelSender(_) => ion_static_str!("AsyncChannelSender"),
            #[cfg(feature = "async-runtime")]
            Value::AsyncChannelReceiver(_) => ion_static_str!("AsyncChannelReceiver"),
            Value::HostStruct { .. } => ion_static_str!("struct"),
            Value::HostEnum { .. } => ion_static_str!("enum"),
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Task(_) => ion_static_str!("Task"),
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Channel(_) => ion_static_str!("Channel"),
            Value::Cell(_) => ion_static_str!("cell"),
            Value::Range { .. } => ion_static_str!("range"),
            Value::Unit => ion_static_str!("()"),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Bytes(b) => !b.is_empty(),
            Value::Option(None) => false,
            Value::Unit => false,
            _ => true,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(n) => Some(*n),
            Value::Int(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Materialize a range into a list of ints.
    pub fn range_to_list(start: i64, end: i64, inclusive: bool) -> Vec<Value> {
        if inclusive {
            (start..=end).map(Value::Int).collect()
        } else {
            (start..end).map(Value::Int).collect()
        }
    }

    /// Length of a range without materializing.
    pub fn range_len(start: i64, end: i64, inclusive: bool) -> i64 {
        if inclusive {
            (end - start + 1).max(0)
        } else {
            (end - start).max(0)
        }
    }
}

#[cfg(not(debug_assertions))]
fn fmt_opaque_value(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(ion_obf_string!("<value>").as_str())
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => {
                if *n == n.floor() && n.is_finite() {
                    write!(f, "{:.1}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bytes(bytes) => {
                write!(f, "b\"")?;
                for &b in bytes {
                    match b {
                        b'\\' => write!(f, "\\\\")?,
                        b'"' => write!(f, "\\\"")?,
                        b'\n' => write!(f, "\\n")?,
                        b'\t' => write!(f, "\\t")?,
                        b'\r' => write!(f, "\\r")?,
                        0x20..=0x7e => write!(f, "{}", b as char)?,
                        _ => write!(f, "\\x{:02x}", b)?,
                    }
                }
                write!(f, "\"")
            }
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Dict(map) => {
                write!(f, "#{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                if items.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
            Value::Set(items) => {
                write!(f, "{}{{", ion_obf_string!("set"))?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "}}")
            }
            Value::Option(opt) => match opt {
                Some(v) => write!(f, "{}{})", ion_obf_string!("Some("), v),
                None => f.write_str(ion_obf_string!("None").as_str()),
            },
            Value::Result(res) => match res {
                Ok(v) => write!(f, "{}{})", ion_obf_string!("Ok("), v),
                Err(e) => write!(f, "{}{})", ion_obf_string!("Err("), e),
            },
            Value::Fn(func) => {
                #[cfg(debug_assertions)]
                {
                    write!(f, "<fn {}>", func.name)
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = func;
                    fmt_opaque_value(f)
                }
            }
            Value::BuiltinFn { qualified_hash, .. } => {
                #[cfg(debug_assertions)]
                {
                    match crate::names::lookup(*qualified_hash) {
                        Some(name) => write!(f, "<builtin {}>", name),
                        None => write!(f, "<builtin #{:016x}>", qualified_hash),
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = qualified_hash;
                    fmt_opaque_value(f)
                }
            }
            Value::BuiltinClosure { qualified_hash, .. } => {
                #[cfg(debug_assertions)]
                {
                    match crate::names::lookup(*qualified_hash) {
                        Some(name) => write!(f, "<builtin {}>", name),
                        None => write!(f, "<builtin #{:016x}>", qualified_hash),
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = qualified_hash;
                    fmt_opaque_value(f)
                }
            }
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { qualified_hash, .. } => {
                #[cfg(debug_assertions)]
                {
                    match crate::names::lookup(*qualified_hash) {
                        Some(name) => write!(f, "<async builtin {}>", name),
                        None => write!(f, "<async builtin #{:016x}>", qualified_hash),
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = qualified_hash;
                    fmt_opaque_value(f)
                }
            }
            Value::Module(table) => {
                #[cfg(debug_assertions)]
                {
                    match crate::names::lookup(table.name_hash) {
                        Some(name) => write!(f, "<module {}>", name),
                        None => write!(f, "<module #{:016x}>", table.name_hash),
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = table;
                    fmt_opaque_value(f)
                }
            }
            #[cfg(feature = "async-runtime")]
            Value::AsyncTask(_) => {
                #[cfg(debug_assertions)]
                {
                    write!(f, "<AsyncTask>")
                }
                #[cfg(not(debug_assertions))]
                {
                    fmt_opaque_value(f)
                }
            }
            #[cfg(feature = "async-runtime")]
            Value::AsyncChannelSender(_) => {
                #[cfg(debug_assertions)]
                {
                    write!(f, "<AsyncChannelTx>")
                }
                #[cfg(not(debug_assertions))]
                {
                    fmt_opaque_value(f)
                }
            }
            #[cfg(feature = "async-runtime")]
            Value::AsyncChannelReceiver(_) => {
                #[cfg(debug_assertions)]
                {
                    write!(f, "<AsyncChannelRx>")
                }
                #[cfg(not(debug_assertions))]
                {
                    fmt_opaque_value(f)
                }
            }
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Task(_) => {
                #[cfg(debug_assertions)]
                {
                    write!(f, "<Task>")
                }
                #[cfg(not(debug_assertions))]
                {
                    fmt_opaque_value(f)
                }
            }
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Channel(ch) => {
                #[cfg(debug_assertions)]
                {
                    match ch {
                        crate::async_rt::ChannelEnd::Sender(_) => write!(f, "<ChannelTx>"),
                        crate::async_rt::ChannelEnd::Receiver(_) => write!(f, "<ChannelRx>"),
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = ch;
                    fmt_opaque_value(f)
                }
            }
            Value::HostStruct { type_hash, fields } => {
                #[cfg(not(debug_assertions))]
                {
                    let _ = (type_hash, fields);
                    fmt_opaque_value(f)
                }
                #[cfg(debug_assertions)]
                {
                    // Names live only in the optional `names` registry; debug
                    // builds auto-populate it from h!() sites, release builds
                    // can load a sidecar — see docs/hide-names.md.
                    match crate::names::lookup(*type_hash) {
                        Some(name) => write!(f, "{} {{ ", name)?,
                        None => write!(f, "<struct#{:016x}> {{ ", type_hash)?,
                    }
                    for (i, (k, v)) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        match crate::names::lookup(*k) {
                            Some(name) => write!(f, "{}: {}", name, v)?,
                            None => write!(f, "#{:016x}: {}", k, v)?,
                        }
                    }
                    write!(f, " }}")
                }
            }
            Value::HostEnum {
                enum_hash,
                variant_hash,
                data,
            } => {
                #[cfg(not(debug_assertions))]
                {
                    let _ = (enum_hash, variant_hash, data);
                    fmt_opaque_value(f)
                }
                #[cfg(debug_assertions)]
                {
                    match crate::names::lookup(*enum_hash) {
                        Some(name) => write!(f, "{}::", name)?,
                        None => write!(f, "<enum#{:016x}>::", enum_hash)?,
                    }
                    match crate::names::lookup(*variant_hash) {
                        Some(name) => write!(f, "{}", name)?,
                        None => write!(f, "<v#{:016x}>", variant_hash)?,
                    }
                    if !data.is_empty() {
                        write!(f, "(")?;
                        for (i, v) in data.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", v)?;
                        }
                        write!(f, ")")?;
                    }
                    Ok(())
                }
            }
            Value::Cell(cell) => {
                #[cfg(debug_assertions)]
                {
                    let inner = cell.lock().unwrap();
                    write!(f, "cell({})", *inner)
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = cell;
                    fmt_opaque_value(f)
                }
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                if *inclusive {
                    write!(f, "{}..={}", start, end)
                } else {
                    write!(f, "{}..{}", start, end)
                }
            }
            Value::Unit => write!(f, "()"),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Dict(a), Value::Dict(b)) => a == b,
            (Value::Set(a), Value::Set(b)) => a.len() == b.len() && a.iter().all(|v| b.contains(v)),
            (Value::Option(a), Value::Option(b)) => a == b,
            (Value::Result(Ok(a)), Value::Result(Ok(b))) => a == b,
            (Value::Result(Err(a)), Value::Result(Err(b))) => a == b,
            (
                Value::HostStruct {
                    type_hash: a_h,
                    fields: a_fields,
                },
                Value::HostStruct {
                    type_hash: b_h,
                    fields: b_fields,
                },
            ) => a_h == b_h && a_fields == b_fields,
            (
                Value::HostEnum {
                    enum_hash: a_en,
                    variant_hash: a_v,
                    data: a_d,
                },
                Value::HostEnum {
                    enum_hash: b_en,
                    variant_hash: b_v,
                    data: b_d,
                },
            ) => a_en == b_en && a_v == b_v && a_d == b_d,
            (Value::Cell(a), Value::Cell(b)) => Arc::ptr_eq(a, b),
            (
                Value::Range {
                    start: s1,
                    end: e1,
                    inclusive: i1,
                },
                Value::Range {
                    start: s2,
                    end: e2,
                    inclusive: i2,
                },
            ) => s1 == s2 && e1 == e2 && i1 == i2,
            (Value::Unit, Value::Unit) => true,
            (Value::Option(None), Value::Unit) => false,
            #[cfg(feature = "async-runtime")]
            (Value::AsyncTask(a), Value::AsyncTask(b)) => a.ptr_eq(b),
            #[cfg(feature = "async-runtime")]
            (Value::AsyncChannelSender(a), Value::AsyncChannelSender(b)) => a.ptr_eq(b),
            #[cfg(feature = "async-runtime")]
            (Value::AsyncChannelReceiver(a), Value::AsyncChannelReceiver(b)) => a.ptr_eq(b),
            // Task and Channel are not comparable
            _ => false,
        }
    }
}

// ---- Serde JSON conversions ----

impl Value {
    /// Convert an Ion Value to a serde_json::Value.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Int(n) => serde_json::Value::Number((*n).into()),
            Value::Float(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::List(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            Value::Dict(map) => {
                let obj: serde_json::Map<String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                serde_json::Value::Object(obj)
            }
            Value::Tuple(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            Value::Set(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            Value::Option(Some(v)) => v.to_json(),
            Value::Option(None) | Value::Unit => serde_json::Value::Null,
            Value::Result(Ok(v)) => v.to_json(),
            Value::Result(Err(v)) => {
                let mut map = serde_json::Map::new();
                map.insert("error".to_string(), v.to_json());
                serde_json::Value::Object(map)
            }
            Value::HostStruct { fields, .. } => {
                // Use registered names when available; fall back to hex
                // hash so the encoding stays lossless either way.
                let obj: serde_json::Map<String, serde_json::Value> = fields
                    .iter()
                    .map(|(k, v)| {
                        let key = match crate::names::lookup(*k) {
                            Some(name) => name.to_string(),
                            None => format!("#{:016x}", k),
                        };
                        (key, v.to_json())
                    })
                    .collect();
                serde_json::Value::Object(obj)
            }
            Value::HostEnum {
                enum_hash,
                variant_hash,
                data,
            } => {
                let enum_str = match crate::names::lookup(*enum_hash) {
                    Some(name) => name.to_string(),
                    None => format!("#{:016x}", enum_hash),
                };
                let variant_str = match crate::names::lookup(*variant_hash) {
                    Some(name) => name.to_string(),
                    None => format!("#{:016x}", variant_hash),
                };
                let mut map = serde_json::Map::new();
                map.insert(
                    "_type".to_string(),
                    serde_json::Value::String(format!("{}::{}", enum_str, variant_str)),
                );
                if !data.is_empty() {
                    map.insert(
                        "data".to_string(),
                        serde_json::Value::Array(data.iter().map(|v| v.to_json()).collect()),
                    );
                }
                serde_json::Value::Object(map)
            }
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Task(_) | Value::Channel(_) => serde_json::Value::Null,
            Value::Cell(cell) => cell.lock().unwrap().to_json(),
            Value::Bytes(b) => {
                let hex: String = b.iter().map(|byte| format!("{:02x}", byte)).collect();
                serde_json::Value::String(hex)
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => serde_json::Value::Array(
                Value::range_to_list(*start, *end, *inclusive)
                    .iter()
                    .map(|v| v.to_json())
                    .collect(),
            ),
            Value::Fn(_)
            | Value::BuiltinFn { .. }
            | Value::BuiltinClosure { .. }
            | Value::Module(_) => serde_json::Value::Null,
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { .. }
            | Value::AsyncTask(_)
            | Value::AsyncChannelSender(_)
            | Value::AsyncChannelReceiver(_) => serde_json::Value::Null,
        }
    }

    /// Encode an Ion Value to MessagePack bytes.
    #[cfg(feature = "msgpack")]
    pub fn to_msgpack(&self) -> Result<Vec<u8>, String> {
        let mp = self.to_msgpack_value();
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &mp)
            .map_err(|e| ion_format!("{}{}", ion_str!("msgpack_encode error: "), e))?;
        Ok(buf)
    }

    /// Decode MessagePack bytes to an Ion Value.
    #[cfg(feature = "msgpack")]
    pub fn from_msgpack(data: &[u8]) -> Result<Value, String> {
        let mut cursor = std::io::Cursor::new(data);
        let mp = rmpv::decode::read_value(&mut cursor)
            .map_err(|e| ion_format!("{}{}", ion_str!("msgpack_decode error: "), e))?;
        Ok(Self::from_msgpack_value(mp))
    }

    #[cfg(feature = "msgpack")]
    fn to_msgpack_value(&self) -> rmpv::Value {
        match self {
            Value::Int(n) => rmpv::Value::Integer((*n).into()),
            Value::Float(n) => rmpv::Value::F64(*n),
            Value::Bool(b) => rmpv::Value::Boolean(*b),
            Value::Str(s) => rmpv::Value::String(s.clone().into()),
            Value::Bytes(b) => rmpv::Value::Binary(b.clone()),
            Value::List(items) => {
                rmpv::Value::Array(items.iter().map(|v| v.to_msgpack_value()).collect())
            }
            Value::Dict(map) => {
                let pairs: Vec<(rmpv::Value, rmpv::Value)> = map
                    .iter()
                    .map(|(k, v)| (rmpv::Value::String(k.clone().into()), v.to_msgpack_value()))
                    .collect();
                rmpv::Value::Map(pairs)
            }
            Value::Tuple(items) => {
                rmpv::Value::Array(items.iter().map(|v| v.to_msgpack_value()).collect())
            }
            Value::Set(items) => {
                rmpv::Value::Array(items.iter().map(|v| v.to_msgpack_value()).collect())
            }
            Value::Option(Some(v)) => v.to_msgpack_value(),
            Value::Option(None) | Value::Unit => rmpv::Value::Nil,
            Value::Result(Ok(v)) => v.to_msgpack_value(),
            Value::Result(Err(v)) => {
                let pairs = vec![(rmpv::Value::String("error".into()), v.to_msgpack_value())];
                rmpv::Value::Map(pairs)
            }
            Value::HostStruct { fields, .. } => {
                // Field-name keys hashed; encoded as u64. Compact and lossless
                // for re-decoding within the same registry.
                let pairs: Vec<(rmpv::Value, rmpv::Value)> = fields
                    .iter()
                    .map(|(k, v)| (rmpv::Value::Integer((*k).into()), v.to_msgpack_value()))
                    .collect();
                rmpv::Value::Map(pairs)
            }
            Value::HostEnum {
                enum_hash,
                variant_hash,
                data,
            } => {
                // Encode as [enum_hash, variant_hash, data...] — ordered array,
                // not a name-keyed map. Wire format change vs. pre-Phase 2.
                let mut arr: Vec<rmpv::Value> = Vec::with_capacity(2 + data.len());
                arr.push(rmpv::Value::Integer((*enum_hash).into()));
                arr.push(rmpv::Value::Integer((*variant_hash).into()));
                for v in data {
                    arr.push(v.to_msgpack_value());
                }
                rmpv::Value::Array(arr)
            }
            #[cfg(all(
                feature = "legacy-threaded-concurrency",
                not(feature = "async-runtime")
            ))]
            Value::Task(_) | Value::Channel(_) => rmpv::Value::Nil,
            Value::Cell(cell) => cell.lock().unwrap().to_msgpack_value(),
            Value::Range {
                start,
                end,
                inclusive,
            } => rmpv::Value::Array(
                Value::range_to_list(*start, *end, *inclusive)
                    .iter()
                    .map(|v| v.to_msgpack_value())
                    .collect(),
            ),
            Value::Fn(_)
            | Value::BuiltinFn { .. }
            | Value::BuiltinClosure { .. }
            | Value::Module(_) => rmpv::Value::Nil,
            #[cfg(feature = "async-runtime")]
            Value::AsyncBuiltinClosure { .. }
            | Value::AsyncTask(_)
            | Value::AsyncChannelSender(_)
            | Value::AsyncChannelReceiver(_) => rmpv::Value::Nil,
        }
    }

    #[cfg(feature = "msgpack")]
    fn from_msgpack_value(mp: rmpv::Value) -> Value {
        match mp {
            rmpv::Value::Nil => Value::Option(None),
            rmpv::Value::Boolean(b) => Value::Bool(b),
            rmpv::Value::Integer(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::Int(u as i64)
                } else {
                    Value::Int(0)
                }
            }
            rmpv::Value::F32(f) => Value::Float(f as f64),
            rmpv::Value::F64(f) => Value::Float(f),
            rmpv::Value::String(s) => Value::Str(s.into_str().unwrap_or_default().to_string()),
            rmpv::Value::Binary(b) => Value::Bytes(b),
            rmpv::Value::Array(arr) => {
                Value::List(arr.into_iter().map(Self::from_msgpack_value).collect())
            }
            rmpv::Value::Map(pairs) => {
                let dict: IndexMap<String, Value> = pairs
                    .into_iter()
                    .filter_map(|(k, v)| {
                        let key = match k {
                            rmpv::Value::String(s) => s.into_str().map(|s| s.to_string()),
                            _ => None,
                        };
                        key.map(|k| (k, Self::from_msgpack_value(v)))
                    })
                    .collect();
                Value::Dict(dict)
            }
            rmpv::Value::Ext(_, data) => Value::Bytes(data),
        }
    }

    /// Convert a serde_json::Value to an Ion Value.
    pub fn from_json(json: serde_json::Value) -> Value {
        match json {
            serde_json::Value::Null => Value::Option(None),
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f)
                } else {
                    Value::Int(0)
                }
            }
            serde_json::Value::String(s) => Value::Str(s),
            serde_json::Value::Array(arr) => {
                Value::List(arr.into_iter().map(Value::from_json).collect())
            }
            serde_json::Value::Object(map) => {
                let dict: IndexMap<String, Value> = map
                    .into_iter()
                    .map(|(k, v)| (k, Value::from_json(v)))
                    .collect();
                Value::Dict(dict)
            }
        }
    }
}
