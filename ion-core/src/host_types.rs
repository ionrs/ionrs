use std::collections::HashMap;
use indexmap::IndexMap;

use crate::value::Value;

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

    pub fn get_field(&self, type_name: &str, val: &Value, field: &str) -> Result<Option<Value>, String> {
        if let Value::HostStruct { type_name: vt, fields } = val {
            if vt == type_name || type_name.is_empty() {
                return Ok(fields.get(field).cloned());
            }
        }
        Err(format!("cannot access field '{}' on {}", field, val.type_name()))
    }
}
