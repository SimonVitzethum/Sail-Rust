#![forbid(unsafe_code)]
//! Type environment: value specs + registers.

use crate::nexp::{MiniTyp, TypQuant};
use std::collections::HashMap;
use std::sync::Arc;

/// Program-wide tables shared across checker snapshots.
///
/// RAM diet (Lane 68): `val`/`register` tables used to be cloned on every
/// `let` scope (`env.clone()` in `check.rs`). They now live behind an
/// [`Arc`]: cloning a [`TypeEnv`] only bumps a refcount plus clones the
/// small locals map. Mutation uses [`Arc::make_mut`] (copy-on-write), so
/// observable behaviour is unchanged — a cloned env never sees later
/// inserts into the original.
#[derive(Clone, Debug, Default)]
struct SharedTables {
    /// `val` specs: function name -> quantified scheme.
    vals: HashMap<String, TypQuant>,
    /// Registers: name -> quantified scheme (usually monomorphic or with a
    /// single length quantifier, e.g. `bits('n)`).
    registers: HashMap<String, TypQuant>,
}

/// Global typing environment (immutable snapshots are `Clone`d per worker
/// thread; no shared interior mutability, so no lock contention).
///
/// Cloning is cheap: the program-wide tables are [`Arc`]-shared, only the
/// locals map is copied.
///
/// Retained (MIGRATION.md #24): `sail-ast` has no `Env.t` yet; the
/// val/register/local tables stay here until that lane lands. Schemes
/// lower onto `sail_ast::TypSchm` (see [`crate::conv`]).
#[derive(Clone, Debug, Default)]
pub struct TypeEnv {
    shared: Arc<SharedTables>,
    /// Local variables: name -> monomorphic type.
    locals: HashMap<String, MiniTyp>,
}

impl TypeEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_val(&mut self, name: impl Into<String>, scheme: TypQuant) {
        Arc::make_mut(&mut self.shared).vals.insert(name.into(), scheme);
    }

    pub fn add_local(&mut self, name: impl Into<String>, typ: MiniTyp) {
        self.locals.insert(name.into(), typ);
    }

    pub fn add_register(&mut self, name: impl Into<String>, scheme: TypQuant) {
        Arc::make_mut(&mut self.shared)
            .registers
            .insert(name.into(), scheme);
    }

    #[must_use]
    pub fn lookup_val(&self, name: &str) -> Option<&TypQuant> {
        self.shared.vals.get(name)
    }

    #[must_use]
    pub fn lookup_local(&self, name: &str) -> Option<&MiniTyp> {
        self.locals.get(name).or_else(|| {
            // Locals shadow globals; fall back to projecting monomorphic
            // val/register schemes with empty quants for convenience.
            self.shared
                .vals
                .get(name)
                .filter(|q| q.quants.is_empty())
                .map(|q| &q.typ)
        })
    }

    #[must_use]
    pub fn lookup_register(&self, name: &str) -> Option<&TypQuant> {
        self.shared.registers.get(name)
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<MiniTyp> {
        if let Some(t) = self.locals.get(name) {
            return Some(t.clone());
        }
        if let Some(q) = self.shared.vals.get(name) {
            if q.quants.is_empty() {
                return Some(q.typ.clone());
            }
        }
        self.shared
            .registers
            .get(name)
            .filter(|q| q.quants.is_empty())
            .map(|q| q.typ.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;

    fn mono_int() -> TypQuant {
        TypQuant::monomorphic(MiniTyp::Int)
    }

    #[test]
    fn clone_shares_program_tables() {
        let mut env = TypeEnv::new();
        env.add_val("f", mono_int());
        let fork = env.clone();
        // Both views see the same tables without copying them.
        assert!(fork.lookup_val("f").is_some());
        assert_eq!(Arc::strong_count(&env.shared), 2);
    }

    #[test]
    fn mutation_after_clone_does_not_leak_into_fork() {
        let mut env = TypeEnv::new();
        env.add_val("f", mono_int());
        let mut fork = env.clone();
        // Copy-on-write: the fork detaches, the original is untouched.
        fork.add_val("g", mono_int());
        fork.add_local("x", MiniTyp::Bool);
        assert!(env.lookup_val("g").is_none());
        assert!(env.lookup_local("x").is_none());
        assert!(fork.lookup_val("g").is_some());
        assert_eq!(fork.lookup_local("x"), Some(&MiniTyp::Bool));
        // Locals still shadow globals through the shared tables.
        assert_eq!(fork.lookup("f"), Some(MiniTyp::Int));
    }

    #[test]
    fn shared_tables_are_send_sync_for_workers() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TypeEnv>();
    }

    #[test]
    fn registers_visible_through_clone() {
        let mut env = TypeEnv::new();
        env.add_register(
            "r",
            TypQuant {
                quants: vec![("n".to_string(), Kind::KInt)],
                constraint: crate::nexp::NConstraint::True,
                typ: MiniTyp::Int,
            },
        );
        let fork = env.clone();
        assert!(fork.lookup_register("r").is_some());
        // Polymorphic schemes do not project into `lookup`.
        assert!(fork.lookup("r").is_none());
    }
}
