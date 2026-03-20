use std::collections::HashMap;
use std::fmt;
use indexmap::IndexMap;

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
            (Value::Unit, Value::Unit) => true,
            (Value::Option(None), Value::Unit) => false,
            _ => false,
        }
    }
}
