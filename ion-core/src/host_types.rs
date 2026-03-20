use std::collections::HashMap;
use indexmap::IndexMap;

use crate::value::Value;

/// Trait for Rust types that can be used in Ion scripts.
/// Implemented manually or via `#[derive(IonType)]`.
pub trait IonType: Sized {
    /// Convert this Rust value to an Ion Value.
    fn to_ion(&self) -> Value;
    /// Convert an Ion Value back to this Rust type.
    fn from_ion(val: &Value) -> Result<Self, String>;
    /// Get the type definition for registration.
    fn ion_type_def() -> IonTypeDef;
}

/// Describes what kind of type this is (struct or enum) for registration.
pub enum IonTypeDef {
    Struct(HostStructDef),
    Enum(HostEnumDef),
}

/// Describes a host-injected struct type.
#[derive(Debug, Clone)]
pub struct HostStructDef {
    pub name: String,
    pub fields: Vec<String>,
}

/// Describes a single enum variant.
#[derive(Debug, Clone)]
pub struct HostVariantDef {
    pub name: String,
    /// Number of positional data fields (0 = unit variant)
    pub arity: usize,
}

/// Describes a host-injected enum type.
#[derive(Debug, Clone)]
pub struct HostEnumDef {
    pub name: String,
    pub variants: Vec<HostVariantDef>,
}

/// Registry of host-provided types available to scripts.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    pub structs: HashMap<String, HostStructDef>,
    pub enums: HashMap<String, HostEnumDef>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_struct(&mut self, def: HostStructDef) {
        self.structs.insert(def.name.clone(), def);
    }

    pub fn register_enum(&mut self, def: HostEnumDef) {
        self.enums.insert(def.name.clone(), def);
    }

    /// Validate and construct a host struct value.
    pub fn construct_struct(&self, name: &str, fields: IndexMap<String, Value>) -> Result<Value, String> {
        let def = self.structs.get(name).ok_or_else(|| format!("unknown type '{}'", name))?;
        // Verify all required fields are present
        for field_name in &def.fields {
            if !fields.contains_key(field_name) {
                return Err(format!("missing field '{}' in {}", field_name, name));
            }
        }
        // Verify no extra fields
        for key in fields.keys() {
            if !def.fields.contains(key) {
                return Err(format!("unknown field '{}' in {}", key, name));
            }
        }
        Ok(Value::HostStruct { type_name: name.to_string(), fields })
    }

    /// Validate and construct a host enum variant.
    pub fn construct_enum(&self, enum_name: &str, variant: &str, data: Vec<Value>) -> Result<Value, String> {
        let def = self.enums.get(enum_name).ok_or_else(|| format!("unknown enum '{}'", enum_name))?;
        let variant_def = def.variants.iter().find(|v| v.name == variant)
            .ok_or_else(|| format!("unknown variant '{}' in {}", variant, enum_name))?;
        if data.len() != variant_def.arity {
            return Err(format!(
                "{}::{} expects {} arguments, got {}",
                enum_name, variant, variant_def.arity, data.len()
            ));
        }
        Ok(Value::HostEnum { enum_name: enum_name.to_string(), variant: variant.to_string(), data })
    }

    /// Register a type via the IonType trait.
    pub fn register_ion_type<T: IonType>(&mut self) {
        match T::ion_type_def() {
            IonTypeDef::Struct(def) => self.register_struct(def),
            IonTypeDef::Enum(def) => self.register_enum(def),
        }
    }

    pub fn get_field(&self, type_name: &str, val: &Value, field: &str) -> Result<Option<Value>, String> {
        if let Value::HostStruct { type_name: vt, fields } = val {
            if vt == type_name || type_name.is_empty() {
                return Ok(fields.get(field).cloned());
            }
        }
        Err(format!("cannot access field '{}' on {}", field, val.type_name()))
    }
}

// --- IonType impls for primitive types ---

impl IonType for i64 {
    fn to_ion(&self) -> Value { Value::Int(*self) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for i32 {
    fn to_ion(&self) -> Value { Value::Int(*self as i64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().map(|n| n as i32).ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for u16 {
    fn to_ion(&self) -> Value { Value::Int(*self as i64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().map(|n| n as u16).ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for u32 {
    fn to_ion(&self) -> Value { Value::Int(*self as i64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().map(|n| n as u32).ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for u64 {
    fn to_ion(&self) -> Value { Value::Int(*self as i64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().map(|n| n as u64).ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for usize {
    fn to_ion(&self) -> Value { Value::Int(*self as i64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int().map(|n| n as usize).ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for f64 {
    fn to_ion(&self) -> Value { Value::Float(*self) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_float().ok_or_else(|| format!("expected float, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for f32 {
    fn to_ion(&self) -> Value { Value::Float(*self as f64) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_float().map(|n| n as f32).ok_or_else(|| format!("expected float, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for bool {
    fn to_ion(&self) -> Value { Value::Bool(*self) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_bool().ok_or_else(|| format!("expected bool, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl IonType for String {
    fn to_ion(&self) -> Value { Value::Str(self.clone()) }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_str().map(|s| s.to_string()).ok_or_else(|| format!("expected string, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl<T: IonType> IonType for Vec<T> {
    fn to_ion(&self) -> Value {
        Value::List(self.iter().map(|v| v.to_ion()).collect())
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        match val {
            Value::List(items) => items.iter().map(T::from_ion).collect(),
            _ => Err(format!("expected list, got {}", val.type_name())),
        }
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}

impl<T: IonType> IonType for Option<T> {
    fn to_ion(&self) -> Value {
        match self {
            Some(v) => Value::Option(Some(Box::new(v.to_ion()))),
            None => Value::Option(None),
        }
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        match val {
            Value::Option(Some(v)) => Ok(Some(T::from_ion(v)?)),
            Value::Option(None) => Ok(None),
            _ => Err(format!("expected Option, got {}", val.type_name())),
        }
    }
    fn ion_type_def() -> IonTypeDef { unreachable!("primitives are not registered") }
}
