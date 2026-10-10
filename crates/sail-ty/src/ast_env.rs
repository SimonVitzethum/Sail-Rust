#![forbid(unsafe_code)]
//! Canonical views over the local environments (Lane 72, Lane-61-Reste #23/#24).
//!
//! ## Decision: `KindEnv` / `TypeEnv` stay local to `sail-ty`
//!
//! `sail-ast` is pure data by contract ("no parser / typechecker / printer",
//! `sail-ast/src/lib.rs`); OCaml `Env.t` (`env.ml`) is typechecker *state*
//! (val specs, registers, scattered defs, kind scopes, constraints,
//! locations), not an AST node. Moving the envs there would drag checker
//! behaviour — kind checking, quantified schemes, the Lane-68 `Arc`-shared
//! snapshot diet, the Lane-65 `serde` stability promise — into the data
//! crate (or force an `sail-ast -> checker` dependency cycle).
//!
//! What canonically belongs to `sail-ast` is already wired in
//! [`crate::conv`]: [`crate::Kind`] erases onto `sail_ast::Kind`
//! (`KNat -> Int`, never silently recovered) and [`crate::TypQuant`] lowers
//! onto `sail_ast::TypSchm`. This module adds the two missing views so no
//! caller needs a second, hand-rolled lowering:
//!
//! - [`kenv_to_kopts`] / [`kenv_from_kopts`]: `KindEnv` as a sorted
//!   `KOpt` telescope (the `quant_item` shape `TypSchm` expects).
//! - [`TypeEnv::val_scheme_ast`] / [`TypeEnv::register_scheme_ast`]:
//!   stored schemes as canonical `TypSchm` nodes.
//!
//! Scattered-freeze (Lane 111, see `ENV_COVERAGE.md`): no scattered views
//! here — open/`impl`/`end` linkage stays in `sail-resolve`; the checker
//! only lowers *closed* schemes through these two methods.

use sail_ast::{Interner, KOpt, TypSchm};

use crate::conv::{kind_erase, kind_from_ast, typquant_to_ast};
use crate::env::TypeEnv;
use crate::kind::{Kind, KindEnv};

/// Lower a kind environment onto a canonical `KOpt` telescope, sorted by
/// variable name so output is deterministic across hash orders.
///
/// `KNat` erases to `Int` (same documented rule as [`kind_erase`]); the
/// refinement is checker-local and never guessed back.
pub fn kenv_to_kopts(env: &KindEnv, it: &mut impl Interner) -> Vec<KOpt> {
    let mut bindings: Vec<(&str, Kind)> = env.iter().collect();
    bindings.sort_by(|a, b| a.0.cmp(b.0));
    bindings
        .into_iter()
        .map(|(var, kind)| KOpt {
            kind: kind_erase(kind),
            kid: it.intern(var),
        })
        .collect()
}

/// Lift a `KOpt` telescope back onto a kind environment.
///
/// `Int` yields `KInt` (a `Nat` refinement must be re-established by Lane
/// 04/10 logic, never guessed here — same rule as [`kind_from_ast`]).
/// Returns `None` when a binder no longer resolves in `it`.
pub fn kenv_from_kopts(kopts: &[KOpt], it: &impl Interner) -> Option<KindEnv> {
    let mut env = KindEnv::new();
    for kopt in kopts {
        let var = it.resolve(kopt.kid)?.to_owned();
        env.bind(var, kind_from_ast(kopt.kind));
    }
    Some(env)
}

impl TypeEnv {
    /// Stored `val` scheme for `name` as a canonical `TypSchm` node.
    /// `None` for unknown names.
    pub fn val_scheme_ast(&self, name: &str, it: &mut impl Interner) -> Option<TypSchm> {
        self.lookup_val(name).map(|scheme| typquant_to_ast(scheme, it))
    }

    /// Stored `register` scheme for `name` as a canonical `TypSchm` node.
    /// `None` for unknown names.
    pub fn register_scheme_ast(&self, name: &str, it: &mut impl Interner) -> Option<TypSchm> {
        self.lookup_register(name)
            .map(|scheme| typquant_to_ast(scheme, it))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nexp::{MiniTyp, NConstraint, Nexp, TypQuant};
    use sail_ast::SimpleInterner;

    fn quantified_bits() -> TypQuant {
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::Ge(
                Box::new(Nexp::Var("n".into())),
                Box::new(Nexp::Const(0)),
            ),
            typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
        }
    }

    #[test]
    fn kenv_telescope_is_sorted_and_total() {
        let mut it = SimpleInterner::new();
        let mut env = KindEnv::new();
        env.bind("n", Kind::KInt);
        env.bind("b", Kind::KBool);
        let kopts = kenv_to_kopts(&env, &mut it);
        let names: Vec<String> = kopts
            .iter()
            .map(|k| it.resolve(k.kid).expect("resolves").to_owned())
            .collect();
        assert_eq!(names, vec!["b".to_string(), "n".to_string()]);
    }

    #[test]
    fn kenv_roundtrip_erases_knat_without_recovery() {
        let mut it = SimpleInterner::new();
        let mut env = KindEnv::new();
        env.bind("m", Kind::KNat);
        env.bind("t", Kind::KType);
        let back = kenv_from_kopts(&kenv_to_kopts(&env, &mut it), &it).expect("roundtrips");
        // `KNat` erases to `K_int` and is NEVER silently recovered.
        assert_eq!(back.lookup("m"), Some(Kind::KInt));
        assert_eq!(back.lookup("t"), Some(Kind::KType));
    }

    #[test]
    fn kenv_from_foreign_interner_yields_none() {
        let mut it = SimpleInterner::new();
        let mut env = KindEnv::new();
        env.bind("n", Kind::KInt);
        let kopts = kenv_to_kopts(&env, &mut it);
        let foreign = SimpleInterner::new();
        assert_eq!(kenv_from_kopts(&kopts, &foreign), None);
    }

    #[test]
    fn type_env_scheme_views_lower_onto_typschm() {
        let mut it = SimpleInterner::new();
        let mut env = TypeEnv::new();
        env.add_val("mem_read", quantified_bits());
        let scheme = env.val_scheme_ast("mem_read", &mut it).expect("known val");
        assert_eq!(scheme.quant.len(), 2, "one binder + one constraint");
        assert_eq!(env.val_scheme_ast("missing", &mut it), None);
        assert_eq!(env.register_scheme_ast("mem_read", &mut it), None);
    }
}
