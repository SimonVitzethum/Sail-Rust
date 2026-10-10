#![forbid(unsafe_code)]
//! String interning: trait plus a small `std`-only implementation.
//! Later lanes may swap in `lasso` behind the same [`Interner`] trait.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Interned string handle. `Copy + Send + Sync`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(pub u32);

impl Symbol {
    pub const DUMMY: Symbol = Symbol(u32::MAX);

    #[must_use]
    pub fn is_dummy(self) -> bool {
        self == Symbol::DUMMY
    }
}

/// Interface shared by all interner backends.
pub trait Interner: Send + Sync {
    /// Intern `text`, returning its stable handle.
    fn intern(&mut self, text: &str) -> Symbol;
    /// Resolve a handle, or `None` for unknown symbols.
    fn resolve(&self, symbol: Symbol) -> Option<&str>;
    /// Number of interned strings.
    fn len(&self) -> usize;
    /// Whether no string has been interned yet.
    fn is_empty(&self) -> bool;
}

/// Minimal `HashMap`-backed interner. Single-threaded use via `&mut`;
/// cheap to clone per worker for parallel lanes (merge stays a later lane).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleInterner {
    map: HashMap<String, Symbol>,
    vec: Vec<String>,
}

impl SimpleInterner {
    #[must_use]
    pub fn new() -> Self {
        SimpleInterner {
            map: HashMap::new(),
            vec: Vec::new(),
        }
    }
}

impl Interner for SimpleInterner {
    fn intern(&mut self, text: &str) -> Symbol {
        if let Some(&sym) = self.map.get(text) {
            return sym;
        }
        let sym = Symbol(self.vec.len() as u32);
        self.vec.push(text.to_owned());
        self.map.insert(text.to_owned(), sym);
        sym
    }

    fn resolve(&self, symbol: Symbol) -> Option<&str> {
        self.vec.get(symbol.0 as usize).map(String::as_str)
    }

    fn len(&self) -> usize {
        self.vec.len()
    }

    fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_string_same_symbol() {
        let mut interner = SimpleInterner::new();
        assert!(interner.is_empty());
        let a = interner.intern("foo");
        let b = interner.intern("foo");
        let c = interner.intern("bar");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(interner.resolve(a), Some("foo"));
        assert_eq!(interner.resolve(c), Some("bar"));
        assert_eq!(interner.resolve(Symbol(999)), None);
        assert_eq!(interner.len(), 2);
    }

    fn assert_send_sync<T: Send + Sync>() {}
    #[test]
    fn interner_is_send_sync() {
        assert_send_sync::<Symbol>();
        assert_send_sync::<SimpleInterner>();
    }
}
