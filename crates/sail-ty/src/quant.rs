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

    /// Instantiate all bound vars with globally fresh names
    /// ([`crate::nexp::gensym`], capture-avoiding skeleton used by later
    /// monomorphisation in Lane 09; OCaml `Type_env.fresh_kid` gensym rather
    /// than predictable names, which could collide with user binders).
    #[must_use]
    pub fn freshen(&self, base: &str) -> HashMap<String, String> {
        let total: usize = self.scopes.iter().map(HashMap::len).sum();
        let mut out = HashMap::with_capacity(total);
        for scope in &self.scopes {
            for name in scope.keys() {
                out.insert(name.clone(), crate::nexp::gensym(base));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Lane 10: simple-equation instantiation for omitted implicit arguments.
// ---------------------------------------------------------------------------
//
// Reference: OCaml `type_check.ml` `instantiate_simple_equations`
// (`find_eqs`/`inst_from_eq`): a telescoped quantifier solved by a
// top-level `NC_equal('a, nexp)` conjunct (with occurs check) in the
// scheme constraint is instantiated away instead of requiring an
// explicit argument. This is the `Q-Inst`-adjacent step that lets calls
// omit implicit arguments the constraint already determines.

/// Collect top-level `Var == nexp` equations from a constraint
/// conjunction (OCaml `find_eqs`): each `Eq(Var(v), n)` / `Eq(n, Var(v))`
/// with `v` not free in `n` solves one omitted implicit. `And`-nested
/// conjuncts are scanned; anything else contributes nothing. Total.
#[must_use]
pub fn simple_equations(c: &crate::nexp::NConstraint) -> Vec<(String, crate::nexp::Nexp)> {
    use crate::nexp::{NConstraint, Nexp};
    fn free_in(var: &str, n: &Nexp) -> bool {
        let mut out = Vec::new();
        n.free_vars(&mut out);
        out.iter().any(|v| v == var)
    }
    fn go(c: &NConstraint, out: &mut Vec<(String, Nexp)>) {
        match c {
            NConstraint::And(a, b) => {
                go(a, out);
                go(b, out);
            }
            NConstraint::Eq(a, b) => match (&**a, &**b) {
                (Nexp::Var(v), n) if !free_in(v, n) => out.push((v.clone(), n.clone())),
                (n, Nexp::Var(v)) if !free_in(v, n) => out.push((v.clone(), n.clone())),
                _ => {}
            },
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(c, &mut out);
    out
}
