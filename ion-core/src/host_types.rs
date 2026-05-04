//! Host type registration: enums and structs that scripts can construct
//! and pattern-match.
//!
//! Names are FNV-1a 64-bit hashes computed at macro-expansion / parse time.
//! No identifier strings end up in the host binary's `.rodata`. See
//! `docs/hide-names.md` for the overview.

use indexmap::IndexMap;
use std::collections::HashMap;

use crate::hash::h;
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

/// Describes a host-injected struct type. Field order is preserved
/// (positional `fields[i]` corresponds to the i-th declared field).
#[derive(Debug, Clone)]
pub struct HostStructDef {
    pub name_hash: u64,
    /// Field-name hashes in declaration order.
    pub fields: Vec<u64>,
}

/// Describes a single enum variant.
#[derive(Debug, Clone)]
pub struct HostVariantDef {
    pub name_hash: u64,
    /// Number of positional data fields (0 = unit variant).
    pub arity: usize,
}

/// Describes a host-injected enum type.
#[derive(Debug, Clone)]
pub struct HostEnumDef {
    pub name_hash: u64,
    pub variants: Vec<HostVariantDef>,
}

/// True iff two variant lists are identical in name hashes and arities.
/// Used by `TypeRegistry::register_enum` to distinguish a benign re-register
/// (same enum, same variants) from a hash collision (same name hash but
/// different variant set).
fn shape_matches(a: &[HostVariantDef], b: &[HostVariantDef]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .all(|(x, y)| x.name_hash == y.name_hash && x.arity == y.arity)
}

#[cfg(debug_assertions)]
fn panic_struct_collision(name_hash: u64) -> ! {
    panic!(
        "internal: struct hash collision at #{name_hash:016x}: registered shape differs from incoming definition"
    );
}

#[cfg(not(debug_assertions))]
fn panic_struct_collision(_name_hash: u64) -> ! {
    panic!("{}", ion_str!("type collision"));
}

#[cfg(debug_assertions)]
fn panic_enum_collision(name_hash: u64) -> ! {
    panic!(
        "internal: enum hash collision at #{name_hash:016x}: registered variants differ from incoming definition"
    );
}

#[cfg(not(debug_assertions))]
fn panic_enum_collision(_name_hash: u64) -> ! {
    panic!("{}", ion_str!("type collision"));
}

/// Registry of host-provided types available to scripts. Lookup by hash;
/// no string keys live in the registry at runtime.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    pub structs: HashMap<u64, HostStructDef>,
    pub enums: HashMap<u64, HostEnumDef>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a struct definition. Panics on hash collision with an
    /// already-registered struct (see docs/hide-names.md). Re-registering
    /// the same `T` (same shape) is permitted and replaces silently — common
    /// when an embedder calls `Engine::register_type::<T>()` more than once.
    pub fn register_struct(&mut self, def: HostStructDef) {
        if let Some(existing) = self.structs.get(&def.name_hash) {
            if existing.fields != def.fields {
                panic_struct_collision(def.name_hash);
            }
        }
        self.structs.insert(def.name_hash, def);
    }

    /// Register an enum definition. Panics on hash collision with an
    /// already-registered enum that has a different variant set.
    pub fn register_enum(&mut self, def: HostEnumDef) {
        if let Some(existing) = self.enums.get(&def.name_hash) {
            if !shape_matches(&existing.variants, &def.variants) {
                panic_enum_collision(def.name_hash);
            }
        }
        self.enums.insert(def.name_hash, def);
    }

    /// Construct a host struct value from pre-hashed field-name → value
    /// pairs. Caller is expected to hash script-source field names; the
    /// spread path (`Config { ...base, debug: true }`) supplies already
    /// hashed fields from the base struct directly. `name` is used only
    /// for `type_hash` derivation and error messages.
    pub fn construct_struct(
        &self,
        name: &str,
        fields: IndexMap<u64, Value>,
    ) -> Result<Value, String> {
        let type_hash = h(name);
        let def = self
            .structs
            .get(&type_hash)
            .ok_or_else(|| ion_format!("unknown type '{}'", name))?;

        for fhash in fields.keys() {
            if !def.fields.contains(fhash) {
                return Err(ion_format!("unknown field in {}", name));
            }
        }
        for expected in &def.fields {
            if !fields.contains_key(expected) {
                return Err(ion_format!("missing field in {}", name));
            }
        }
        Ok(Value::HostStruct { type_hash, fields })
    }

    /// Construct a host enum variant from script-side enum/variant names
    /// and positional data.
    pub fn construct_enum(
        &self,
        enum_name: &str,
        variant: &str,
        data: Vec<Value>,
    ) -> Result<Value, String> {
        let enum_hash = h(enum_name);
        let variant_hash = h(variant);
        let def = self
            .enums
            .get(&enum_hash)
            .ok_or_else(|| ion_format!("unknown enum '{}'", enum_name))?;
        let variant_def = def
            .variants
            .iter()
            .find(|v| v.name_hash == variant_hash)
            .ok_or_else(|| ion_format!("unknown variant '{}' in {}", variant, enum_name))?;
        if data.len() != variant_def.arity {
            return Err(ion_format!(
                "{}::{} expects {} arguments, got {}",
                enum_name,
                variant,
                variant_def.arity,
                data.len()
            ));
        }
        Ok(Value::HostEnum {
            enum_hash,
            variant_hash,
            data,
        })
    }

    /// Register a type via the IonType trait.
    pub fn register_ion_type<T: IonType>(&mut self) {
        match T::ion_type_def() {
            IonTypeDef::Struct(def) => self.register_struct(def),
            IonTypeDef::Enum(def) => self.register_enum(def),
        }
    }

    /// Look up a struct field by script-source field name. Hashes once.
    pub fn get_field(
        &self,
        type_name: &str,
        val: &Value,
        field: &str,
    ) -> Result<Option<Value>, String> {
        if let Value::HostStruct {
            type_hash: vt,
            fields,
        } = val
        {
            let want = h(type_name);
            if *vt == want || type_name.is_empty() {
                let field_hash = h(field);
                return Ok(fields.get(&field_hash).cloned());
            }
        }
        Err(ion_format!(
            "cannot access field '{}' on {}",
            field,
            val.type_name()
        ))
    }
}

// --- IonType impls for primitive types ---

impl IonType for i64 {
    fn to_ion(&self) -> Value {
        Value::Int(*self)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for i32 {
    fn to_ion(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .map(|n| n as i32)
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for u16 {
    fn to_ion(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .map(|n| n as u16)
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for u32 {
    fn to_ion(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .map(|n| n as u32)
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for u64 {
    fn to_ion(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .map(|n| n as u64)
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for usize {
    fn to_ion(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_int()
            .map(|n| n as usize)
            .ok_or_else(|| format!("expected int, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for f64 {
    fn to_ion(&self) -> Value {
        Value::Float(*self)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_float()
            .ok_or_else(|| format!("expected float, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for f32 {
    fn to_ion(&self) -> Value {
        Value::Float(*self as f64)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_float()
            .map(|n| n as f32)
            .ok_or_else(|| format!("expected float, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for bool {
    fn to_ion(&self) -> Value {
        Value::Bool(*self)
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_bool()
            .ok_or_else(|| format!("expected bool, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}

impl IonType for String {
    fn to_ion(&self) -> Value {
        Value::Str(self.clone())
    }
    fn from_ion(val: &Value) -> Result<Self, String> {
        val.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| format!("expected string, got {}", val.type_name()))
    }
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
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
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
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
    fn ion_type_def() -> IonTypeDef {
        unreachable!("primitives are not registered")
    }
}
