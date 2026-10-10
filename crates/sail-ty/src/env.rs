#![forbid(unsafe_code)]
//! Type environment: value specs + registers.

use crate::nexp::{MiniTyp, TypQuant};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Global typing environment (immutable snapshots are `Clone`d per worker
/// thread; no shared interior mutability, so no lock contention).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeEnv {
    /// `val` specs: function name -> quantified scheme.
    vals: HashMap<String, TypQuant>,
    /// Local variables: name -> monomorphic type.
    locals: HashMap<String, MiniTyp>,
    /// Registers: name -> quantified scheme (usually monomorphic or with a
    /// single length quantifier, e.g. `bits('n)`).
    registers: HashMap<String, TypQuant>,
}

impl TypeEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_val(&mut self, name: impl Into<String>, scheme: TypQuant) {
        self.vals.insert(name.into(), scheme);
    }

    pub fn add_local(&mut self, name: impl Into<String>, typ: MiniTyp) {
        self.locals.insert(name.into(), typ);
    }

    pub fn add_register(&mut self, name: impl Into<String>, scheme: TypQuant) {
        self.registers.insert(name.into(), scheme);
    }

    #[must_use]
    pub fn lookup_val(&self, name: &str) -> Option<&TypQuant> {
        self.vals.get(name)
    }

    #[must_use]
    pub fn lookup_local(&self, name: &str) -> Option<&MiniTyp> {
        self.locals.get(name).or_else(|| {
            // Locals shadow globals; fall back to projecting monomorphic
            // val/register schemes with empty quants for convenience.
            self.vals
                .get(name)
                .filter(|q| q.quants.is_empty())
                .map(|q| &q.typ)
        })
    }

    #[must_use]
    pub fn lookup_register(&self, name: &str) -> Option<&TypQuant> {
        self.registers.get(name)
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<MiniTyp> {
        if let Some(t) = self.locals.get(name) {
            return Some(t.clone());
        }
        if let Some(q) = self.vals.get(name) {
            if q.quants.is_empty() {
                return Some(q.typ.clone());
            }
        }
        self.registers
            .get(name)
            .filter(|q| q.quants.is_empty())
            .map(|q| q.typ.clone())
    }
}
