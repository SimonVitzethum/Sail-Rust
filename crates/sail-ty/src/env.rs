#![forbid(unsafe_code)]
//! Type environment: value specs + registers.
//!
//! Scattered-freeze (Lane 111): this env intentionally tracks NO scattered
//! open/`impl`/`end` state. Open/close linkage lives in `sail-resolve`
//! (`scattered_functions/_mappings/_variants/_enums` + `insert_scattered` /
//! `lookup_scattered`); the checker only ever sees *closed* schemes via
//! [`TypeEnv::add_val`] after resolution. See `WORK_STATE.md` (root).

use crate::nexp::{MiniTyp, TypQuant};
use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// Retained (ARCHITECTURE.md #24, Lane 72 decision: kept local, see
/// [`crate::ast_env`]): `sail-ast` has no `Env.t`; the val/register/local
/// tables stay here until that lane lands. Schemes lower onto
/// `sail_ast::TypSchm` (see [`crate::conv`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Number of stored `val` specs.
    #[must_use]
    pub fn val_count(&self) -> usize {
        self.shared.vals.len()
    }

    /// Number of stored register schemes.
    #[must_use]
    pub fn register_count(&self) -> usize {
        self.shared.registers.len()
    }

    /// Number of local bindings.
    #[must_use]
    pub fn local_count(&self) -> usize {
        self.locals.len()
    }

    /// Whether any `val` spec is stored.
    #[must_use]
    pub fn contains_val(&self, name: &str) -> bool {
        self.shared.vals.contains_key(name)
    }

    /// Whether any register scheme is stored.
    #[must_use]
    pub fn contains_register(&self, name: &str) -> bool {
        self.shared.registers.contains_key(name)
    }

    /// Iterate `(name, scheme)` over `val` specs (hash order; sort for
    /// deterministic output — see [`crate::ast_env::kenv_to_kopts`]).
    pub fn iter_vals(&self) -> impl Iterator<Item = (&str, &TypQuant)> + '_ {
        self.shared.vals.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate `(name, scheme)` over register schemes (hash order).
    pub fn iter_registers(&self) -> impl Iterator<Item = (&str, &TypQuant)> + '_ {
        self.shared.registers.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Sorted `val` names (deterministic snapshot / oracle order).
    #[must_use]
    pub fn val_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.shared.vals.keys().cloned().collect();
        names.sort();
        names
    }

    /// Sorted register names (deterministic snapshot / oracle order).
    #[must_use]
    pub fn register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.shared.registers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Remove a `val` spec (re-declaration / shadowing final state).
    /// Returns the old scheme when present. Copy-on-write: a cloned env
    /// never sees the removal (same guarantee as [`TypeEnv::add_val`]).
    pub fn remove_val(&mut self, name: &str) -> Option<TypQuant> {
        Arc::make_mut(&mut self.shared).vals.remove(name)
    }

    /// Remove a register scheme. Copy-on-write like [`TypeEnv::remove_val`].
    pub fn remove_register(&mut self, name: &str) -> Option<TypQuant> {
        Arc::make_mut(&mut self.shared).registers.remove(name)
    }

    /// Drop all local bindings, keep program tables (`let`-scope pop).
    pub fn clear_locals(&mut self) {
        self.locals.clear();
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

    #[test]
    fn shared_tables_eq_and_serde_roundtrip() {
        let mut env = TypeEnv::new();
        env.add_val("f", mono_int());
        let fork = env.clone();
        // `PartialEq` compares through the shared tables.
        assert_eq!(env, fork);
        let text = serde_json::to_string(&env).expect("env serializes");
        let back: TypeEnv = serde_json::from_str(&text).expect("env deserializes");
        assert_eq!(env, back);
    }

    #[test]
    fn val_register_namespaces_stay_separate() {
        let mut env = TypeEnv::new();
        env.add_val("r", mono_int());
        env.add_register("r", mono_int());
        assert!(env.contains_val("r"));
        assert!(env.contains_register("r"));
        assert_eq!(env.val_count(), 1);
        assert_eq!(env.register_count(), 1);
        assert_eq!(env.val_names(), vec!["r".to_string()]);
        assert_eq!(env.register_names(), vec!["r".to_string()]);
        // Same name, independent schemes: removing one keeps the other.
        assert!(env.remove_val("r").is_some());
        assert!(!env.contains_val("r"));
        assert!(env.contains_register("r"));
    }

    #[test]
    fn overwrite_is_last_wins_and_removal_is_cow() {
        let mut env = TypeEnv::new();
        env.add_val("f", mono_int());
        env.add_val("f", mono_int());
        assert_eq!(env.val_count(), 1);
        assert!(env.remove_register("missing").is_none());
        let mut fork = env.clone();
        fork.add_local("x", MiniTyp::Bool);
        assert_eq!(fork.local_count(), 1);
        fork.clear_locals();
        assert_eq!(fork.local_count(), 0);
        assert!(fork.lookup_local("x").is_none());
        // Program tables survive the locals pop; removal detaches (CoW).
        assert!(fork.remove_val("f").is_some());
        assert!(env.contains_val("f"));
    }

    #[test]
    fn iter_covers_all_entries() {
        let mut env = TypeEnv::new();
        env.add_val("b", mono_int());
        env.add_val("a", mono_int());
        env.add_register("z", mono_int());
        let mut vals: Vec<String> =
            env.iter_vals().map(|(n, _)| n.to_owned()).collect();
        vals.sort();
        assert_eq!(vals, vec!["a".to_string(), "b".to_string()]);
        let regs: Vec<String> =
            env.iter_registers().map(|(n, _)| n.to_owned()).collect();
        assert_eq!(regs, vec!["z".to_string()]);
    }
}
