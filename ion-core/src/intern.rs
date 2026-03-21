//! String interner for fast variable name lookups.
//!
//! Symbols are u32 indices into a global pool. Equality is O(1).

use std::collections::HashMap;

/// An interned string identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(pub u32);

/// A string interner that maps strings to unique `Symbol` IDs.
#[derive(Debug, Clone)]
pub struct StringPool {
    map: HashMap<String, Symbol>,
    strings: Vec<String>,
}

impl StringPool {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            strings: Vec::new(),
        }
    }

    /// Intern a string, returning its symbol. If already interned, returns the existing symbol.
    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let sym = Symbol(self.strings.len() as u32);
        self.strings.push(s.to_string());
        self.map.insert(s.to_string(), sym);
        sym
    }

    /// Resolve a symbol back to its string.
    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.strings[sym.0 as usize]
    }

    /// Look up a string without interning it. Returns None if not yet interned.
    pub fn map_get(&self, s: &str) -> Option<Symbol> {
        self.map.get(s).copied()
    }
}
