use indexmap::IndexMap;
use serde_json;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ast::{Param, Stmt};
#[cfg(feature = "vm")]
use crate::bytecode::Chunk;

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
    BuiltinFn(String, BuiltinFn),
    /// Host-injected struct: `TypeName { field: val, ... }`
    HostStruct {
        type_name: String,
        fields: IndexMap<String, Value>,
    },
    /// Host-injected enum variant: `EnumName::Variant` or `EnumName::Variant(data)`
    HostEnum {
        enum_name: String,
        variant: String,
        data: Vec<Value>,
    },
    /// Async task handle (concurrency feature)
    #[cfg(feature = "concurrency")]
    Task(std::sync::Arc<crate::async_rt::TaskHandle>),
    /// Channel sender/receiver pair
    #[cfg(feature = "concurrency")]
    Channel(crate::async_rt::ChannelEnd),
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
            Value::Option(_) => ion_static_str!("Option"),
            Value::Result(_) => ion_static_str!("Result"),
            Value::Fn(_) => ion_static_str!("fn"),
            Value::BuiltinFn(_, _) => ion_static_str!("builtin_fn"),
            Value::HostStruct { .. } => ion_static_str!("struct"),
            Value::HostEnum { .. } => ion_static_str!("enum"),
            #[cfg(feature = "concurrency")]
            Value::Task(_) => ion_static_str!("Task"),
            #[cfg(feature = "concurrency")]
            Value::Channel(_) => ion_static_str!("Channel"),
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
            Value::Option(opt) => match opt {
                Some(v) => write!(f, "Some({})", v),
                None => write!(f, "None"),
            },
            Value::Result(res) => match res {
                Ok(v) => write!(f, "Ok({})", v),
                Err(e) => write!(f, "Err({})", e),
            },
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::BuiltinFn(name, _) => write!(f, "<builtin {}>", name),
            #[cfg(feature = "concurrency")]
            Value::Task(_) => write!(f, "<Task>"),
            #[cfg(feature = "concurrency")]
            Value::Channel(ch) => match ch {
                crate::async_rt::ChannelEnd::Sender(_) => write!(f, "<ChannelTx>"),
                crate::async_rt::ChannelEnd::Receiver(_) => write!(f, "<ChannelRx>"),
            },
            Value::HostStruct { type_name, fields } => {
                write!(f, "{} {{ ", type_name)?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, " }}")
            }
            Value::HostEnum {
                enum_name,
                variant,
                data,
            } => {
                write!(f, "{}::{}", enum_name, variant)?;
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
            (Value::Option(a), Value::Option(b)) => a == b,
            (Value::Result(Ok(a)), Value::Result(Ok(b))) => a == b,
            (Value::Result(Err(a)), Value::Result(Err(b))) => a == b,
            (
                Value::HostStruct {
                    type_name: a_name,
                    fields: a_fields,
                },
                Value::HostStruct {
                    type_name: b_name,
                    fields: b_fields,
                },
            ) => a_name == b_name && a_fields == b_fields,
            (
                Value::HostEnum {
                    enum_name: a_en,
                    variant: a_v,
                    data: a_d,
                },
                Value::HostEnum {
                    enum_name: b_en,
                    variant: b_v,
                    data: b_d,
                },
            ) => a_en == b_en && a_v == b_v && a_d == b_d,
            (Value::Unit, Value::Unit) => true,
            (Value::Option(None), Value::Unit) => false,
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
            Value::Option(Some(v)) => v.to_json(),
            Value::Option(None) | Value::Unit => serde_json::Value::Null,
            Value::Result(Ok(v)) => v.to_json(),
            Value::Result(Err(v)) => {
                let mut map = serde_json::Map::new();
                map.insert("error".to_string(), v.to_json());
                serde_json::Value::Object(map)
            }
            Value::HostStruct { fields, .. } => {
                let obj: serde_json::Map<String, serde_json::Value> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
                serde_json::Value::Object(obj)
            }
            Value::HostEnum {
                enum_name,
                variant,
                data,
            } => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "_type".to_string(),
                    serde_json::Value::String(format!("{}::{}", enum_name, variant)),
                );
                if !data.is_empty() {
                    map.insert(
                        "data".to_string(),
                        serde_json::Value::Array(data.iter().map(|v| v.to_json()).collect()),
                    );
                }
                serde_json::Value::Object(map)
            }
            #[cfg(feature = "concurrency")]
            Value::Task(_) | Value::Channel(_) => serde_json::Value::Null,
            Value::Bytes(b) => {
                let hex: String = b.iter().map(|byte| format!("{:02x}", byte)).collect();
                serde_json::Value::String(hex)
            }
            Value::Fn(_) | Value::BuiltinFn(_, _) => serde_json::Value::Null,
        }
    }

    /// Encode an Ion Value to MessagePack bytes.
    #[cfg(feature = "msgpack")]
    pub fn to_msgpack(&self) -> Result<Vec<u8>, String> {
        let mp = self.to_msgpack_value();
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &mp)
            .map_err(|e| format!("{}{}", ion_str!("msgpack_encode error: "), e))?;
        Ok(buf)
    }

    /// Decode MessagePack bytes to an Ion Value.
    #[cfg(feature = "msgpack")]
    pub fn from_msgpack(data: &[u8]) -> Result<Value, String> {
        let mut cursor = std::io::Cursor::new(data);
        let mp = rmpv::decode::read_value(&mut cursor)
            .map_err(|e| format!("{}{}", ion_str!("msgpack_decode error: "), e))?;
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
            Value::Option(Some(v)) => v.to_msgpack_value(),
            Value::Option(None) | Value::Unit => rmpv::Value::Nil,
            Value::Result(Ok(v)) => v.to_msgpack_value(),
            Value::Result(Err(v)) => {
                let pairs = vec![(rmpv::Value::String("error".into()), v.to_msgpack_value())];
                rmpv::Value::Map(pairs)
            }
            Value::HostStruct { fields, .. } => {
                let pairs: Vec<(rmpv::Value, rmpv::Value)> = fields
                    .iter()
                    .map(|(k, v)| (rmpv::Value::String(k.clone().into()), v.to_msgpack_value()))
                    .collect();
                rmpv::Value::Map(pairs)
            }
            Value::HostEnum {
                enum_name,
                variant,
                data,
            } => {
                let mut pairs = vec![(
                    rmpv::Value::String("_type".into()),
                    rmpv::Value::String(format!("{}::{}", enum_name, variant).into()),
                )];
                if !data.is_empty() {
                    pairs.push((
                        rmpv::Value::String("data".into()),
                        rmpv::Value::Array(data.iter().map(|v| v.to_msgpack_value()).collect()),
                    ));
                }
                rmpv::Value::Map(pairs)
            }
            #[cfg(feature = "concurrency")]
            Value::Task(_) | Value::Channel(_) => rmpv::Value::Nil,
            Value::Fn(_) | Value::BuiltinFn(_, _) => rmpv::Value::Nil,
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
