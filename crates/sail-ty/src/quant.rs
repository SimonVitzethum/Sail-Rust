#![forbid(unsafe_code)]
//! Type quantifier (`TypQuant`) management.
//!
//! Tracks `forall 'n 'm. nc => ...` binders with kinds and lexical scopes.
//! Used to discharge scoping before kind checking / proving.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::kind::Kind;
use std::collections::{HashMap, hash_map::Entry};

/// A single quantified variable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuantVar {
    pub name: String,
    pub kind: Kind,
}

impl QuantVar {
    pub fn new(name: impl Into<String>, kind: Kind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }
}

/// Lexically scoped quantifier stack.
#[derive(Clone, Debug, Default)]
pub struct QuantScope {
    scopes: Vec<HashMap<String, Kind>>,
}

impl QuantScope {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a new scope, run `f`, then leave it (RAII-free explicit form).
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Bind `var` in the innermost scope. Errors on duplicate in same scope.
    ///
    /// RAM diet (Lane 68): single lookup via the `entry` API instead of
    /// `contains_key` + `insert` (two hashes).
    pub fn bind(&mut self, var: QuantVar, span: Span) -> TypeResult<()> {
        if self.scopes.is_empty() {
            self.push_scope();
        }
        let top = self.scopes.last_mut().expect("scope just pushed");
        match top.entry(var.name) {
            Entry::Occupied(o) => Err(TypeError {
                span,
                kind: TypeErrorKind::QuantError(format!(
                    "duplicate quantifier '{}'",
                    o.key()
                )),
            }),
            Entry::Vacant(v) => {
                v.insert(var.kind);
                Ok(())
            }
        }
    }

    /// Bind a whole `forall` telescope at once.
    pub fn bind_all(&mut self, vars: &[QuantVar], span: Span) -> TypeResult<()> {
        self.push_scope();
        for v in vars {
            self.bind(v.clone(), span)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<Kind> {
        for scope in self.scopes.iter().rev() {
            if let Some(k) = scope.get(name) {
                return Some(*k);
            }
        }
        None
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Instantiate all bound vars with fresh names `base{i}` (capture-avoiding
    /// skeleton used by later monomorphisation in Lane 09).
    #[must_use]
    pub fn freshen(&self, base: &str) -> HashMap<String, String> {
        let total: usize = self.scopes.iter().map(HashMap::len).sum();
        let mut out = HashMap::with_capacity(total);
        let mut i = 0usize;
        for scope in &self.scopes {
            for name in scope.keys() {
                out.insert(name.clone(), format!("{base}{i}"));
                i += 1;
            }
        }
        out
    }
}
