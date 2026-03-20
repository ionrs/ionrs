use std::collections::HashMap;
use std::fmt;
use indexmap::IndexMap;
use serde_json;

use crate::ast::{Param, Stmt};

/// Runtime value representation.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Value>),
    Dict(IndexMap<String, Value>),
    Tuple(Vec<Value>),
    Option(Option<Box<Value>>),
    Result(Result<Box<Value>, Box<Value>>),
    Fn(IonFn),
    BuiltinFn(String, BuiltinFn),
    /// Host-injected struct: `TypeName { field: val, ... }`
    HostStruct { type_name: String, fields: IndexMap<String, Value> },
    /// Host-injected enum variant: `EnumName::Variant` or `EnumName::Variant(data)`
    HostEnum { enum_name: String, variant: String, data: Vec<Value> },
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
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// Captured environment for closures
    pub captures: HashMap<String, Value>,
}

/// A built-in function: Rust-side callback.
pub type BuiltinFn = fn(&[Value]) -> Result<Value, String>;

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => ion_static_str!("int"),
            Value::Float(_) => ion_static_str!("float"),
            Value::Bool(_) => ion_static_str!("bool"),
            Value::Str(_) => ion_static_str!("string"),
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
            Value::Option(None) => false,
            Value::Unit => false,
            _ => true,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self { Value::Int(n) => Some(*n), _ => None }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(n) => Some(*n),
            Value::Int(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self { Value::Str(s) => Some(s), _ => None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self { Value::Bool(b) => Some(*b), _ => None }
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
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Dict(map) => {
                write!(f, "#{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                if items.len() == 1 { write!(f, ",")?; }
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
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, " }}")
            }
            Value::HostEnum { enum_name, variant, data } => {
                write!(f, "{}::{}", enum_name, variant)?;
                if !data.is_empty() {
                    write!(f, "(")?;
                    for (i, v) in data.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
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
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Option(a), Value::Option(b)) => a == b,
            (Value::Result(Ok(a)), Value::Result(Ok(b))) => a == b,
            (Value::Result(Err(a)), Value::Result(Err(b))) => a == b,
            (Value::HostStruct { type_name: a_name, fields: a_fields },
             Value::HostStruct { type_name: b_name, fields: b_fields }) =>
                a_name == b_name && a_fields == b_fields,
            (Value::HostEnum { enum_name: a_en, variant: a_v, data: a_d },
             Value::HostEnum { enum_name: b_en, variant: b_v, data: b_d }) =>
                a_en == b_en && a_v == b_v && a_d == b_d,
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
            Value::List(items) => serde_json::Value::Array(
                items.iter().map(|v| v.to_json()).collect()
            ),
            Value::Dict(map) => {
                let obj: serde_json::Map<String, serde_json::Value> = map.iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
                serde_json::Value::Object(obj)
            }
            Value::Tuple(items) => serde_json::Value::Array(
                items.iter().map(|v| v.to_json()).collect()
            ),
            Value::Option(Some(v)) => v.to_json(),
            Value::Option(None) | Value::Unit => serde_json::Value::Null,
            Value::Result(Ok(v)) => v.to_json(),
            Value::Result(Err(v)) => {
                let mut map = serde_json::Map::new();
                map.insert("error".to_string(), v.to_json());
                serde_json::Value::Object(map)
            }
            Value::HostStruct { fields, .. } => {
                let obj: serde_json::Map<String, serde_json::Value> = fields.iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
                serde_json::Value::Object(obj)
            }
            Value::HostEnum { enum_name, variant, data } => {
                let mut map = serde_json::Map::new();
                map.insert("_type".to_string(), serde_json::Value::String(format!("{}::{}", enum_name, variant)));
                if !data.is_empty() {
                    map.insert("data".to_string(), serde_json::Value::Array(data.iter().map(|v| v.to_json()).collect()));
                }
                serde_json::Value::Object(map)
            }
            #[cfg(feature = "concurrency")]
            Value::Task(_) | Value::Channel(_) => serde_json::Value::Null,
            Value::Fn(_) | Value::BuiltinFn(_, _) => serde_json::Value::Null,
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
                let dict: IndexMap<String, Value> = map.into_iter()
                    .map(|(k, v)| (k, Value::from_json(v)))
                    .collect();
                Value::Dict(dict)
            }
        }
    }
}
