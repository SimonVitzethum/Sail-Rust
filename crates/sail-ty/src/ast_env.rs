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
//! Scattered-freeze (Lane 111, see `WORK_STATE.md`): no scattered views
//! here — open/`impl`/`end` linkage stays in `sail-resolve`; the checker
//! only lowers *closed* schemes through these two methods.
//!
//! Lane-08 adds the `Program` record/union views over the same idea:
//! [`Program::record_fields_ast`] / [`Program::variant_ctors_ast`] /
//! [`Program::accessor_scheme_ast`] borrow the preserved field payloads
//! (OCaml `Env.records` / `Env.unions` / `Env.accessors`) at a source
//! position. Payloads are already canonical `sail-ast` nodes, so these
//! views borrow instead of lowering.

use sail_ast::{Interner, KOpt, Span, Typ, TypSchm};

use crate::conv::{kind_erase, kind_from_ast, typquant_to_ast};
use crate::env::TypeEnv;
use crate::kind::{Kind, KindEnv};
use crate::program::Program;

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

impl Program {
    /// Borrowed `(field, type, span)` payloads for `struct name` as
    /// declared at `position`. `None` for unknown structs.
    ///
    /// The payloads are the canonical field types (already `sail-ast`
    /// nodes, preserved by `declare`), so this view borrows instead of
    /// lowering. Resolution of binder symbols uses the declaration's
    /// own interner (`record(...).interner`).
    pub fn record_fields_ast(&self, name: &str, position: usize) -> Option<Vec<(&str, &Typ, Span)>> {
        self.record(name, position).map(|record| {
            record
                .fields
                .iter()
                .map(|field| (field.name.as_str(), &field.typ, field.span))
                .collect()
        })
    }

    /// Borrowed `(constructor, payload, span)` entries for
    /// `union name` as declared at `position`. `None` for unknown
    /// unions. Each payload is the constructor argument type; the
    /// matching `Fn(payload) -> union` value scheme stays reachable
    /// via [`Program::value`].
    pub fn variant_ctors_ast(
        &self,
        name: &str,
        position: usize,
    ) -> Option<Vec<(&str, &Typ, Span)>> {
        self.variant(name, position).map(|union| {
            union
                .ctors
                .iter()
                .map(|ctor| (ctor.name.as_str(), &ctor.typ, ctor.span))
                .collect()
        })
    }

    /// Borrowed accessor arrow `Fn(record) -> field` for
    /// `record.field` as declared at `position`. `None` for unknown
    /// pairs. The scheme is quantified over the record telescope.
    pub fn accessor_scheme_ast(
        &self,
        record: &str,
        field: &str,
        position: usize,
    ) -> Option<&TypSchm> {
        self.accessor(record, field, position)
            .map(|scheme| &scheme.typschm)
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
