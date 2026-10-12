//! Effect inference, matching `infer_effects` in OCaml
//! `sail/src/lib/rewrites.ml` (`all_rewriters`):
//!
//! ```ocaml
//! ("infer_effects",
//!   Bool_rewriter (fun asserts_termination ->
//!     base_rewriter (fun side_effects env ast ->
//!       (ast, Effects.infer_side_effects asserts_termination ast, env))))
//! ```
//!
//! Key fidelity point: the OCaml pass returns the AST **unchanged** — it only
//! refreshes the threaded `side_effect_info` state. `run` below is therefore
//! the identity on [`Shard`](crate::ast::Shard), exactly as in OCaml.
//!
//! What is implemented without a checker binding:
//!
//! - [`SideEffect`]: the full OCaml `side_effect` taxonomy from
//!   `sail/src/lib/effects.ml` (`Throw`, `Exit`, `IncompleteMatch`,
//!   `Register`, `External`, `Undefined`, `Scattered`, `NonExec`,
//!   `Outcome`), with [`SideEffect`] display strings mirroring
//!   `string_of_side_effect`.
//! - [`EffectSet`] with the OCaml predicates [`is_pure`] (`pure =
//!   `EffectSet.is_empty`), [`is_effectful`], [`throws`], and
//!   [`is_pure_or_terminal`] (`pure_or_terminal`: subset of
//!   `{Exit, IncompleteMatch}`).
//! - [`direct_effects_of_stmt`]: the only syntactically certain mapping on
//!   the string statement surface — `Stmt::Undefined` carries
//!   [`SideEffect::Undefined`], mirroring `E_undef` in
//!   `infer_def_direct_effects`. Every other statement yields the empty set,
//!   which means "no *certain* direct effect", **not** "pure": register
//!   accesses, throws, exits, calls, incomplete matches and outcomes need a
//!   type environment and a whole-program callgraph (see the lane report).
//!   No `is_pure_stmt` helper is offered on purpose — purity is unprovable
//!   without the checker, and a guessing helper would silently misclassify.
//! - [`summarize_shard`]: deterministic per-definition union of certain
//!   direct effects, in shard order.
//!
//! The transitive closure over functions/letbinds/mappings, the
//! `asserts_termination` flag plumbing, and the pipeline-level effect store
//! all need the checker binding and are specified in the lane report.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

/// A single side effect, mirroring OCaml `Effects.side_effect`
/// (`sail/src/lib/effects.ml`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SideEffect {
    /// `Throw`: `throw` statement.
    Throw,
    /// `Exit`: `exit` / failing `assert` (also termination-measure asserts).
    Exit,
    /// `IncompleteMatch`: incomplete pattern match.
    IncompleteMatch,
    /// `Register`: register (or `__deref`) access.
    Register,
    /// `External`: call to an impure external function.
    External,
    /// `Undefined`: `undefined` literal.
    Undefined,
    /// `Scattered`: scattered definition.
    Scattered,
    /// `NonExec`: `P_string_append` pattern (not executable).
    NonExec,
    /// `Outcome id`: concurrency-interface outcome invocation.
    Outcome(String),
}

impl std::fmt::Display for SideEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Throw => write!(f, "throws exception"),
            Self::Exit => write!(f, "exit statement"),
            Self::IncompleteMatch => write!(f, "incomplete pattern match"),
            Self::Register => write!(f, "register access"),
            Self::External => write!(f, "calls external function not marked pure"),
            Self::Undefined => write!(f, "contains undefined literal"),
            Self::Scattered => write!(f, "scattered function"),
            Self::NonExec => write!(f, "not executable"),
            Self::Outcome(id) => write!(f, "outcome {id}"),
        }
    }
}

/// A set of side effects, mirroring OCaml `Effects.EffectSet`.
pub type EffectSet = BTreeSet<SideEffect>;

/// Mirror of OCaml `Effects.pure`: no effects at all.
#[must_use]
pub fn is_pure(set: &EffectSet) -> bool {
    set.is_empty()
}

/// Mirror of OCaml `Effects.effectful`.
#[must_use]
pub fn is_effectful(set: &EffectSet) -> bool {
    !is_pure(set)
}

/// Mirror of OCaml `Effects.throws`.
#[must_use]
pub fn throws(set: &EffectSet) -> bool {
    set.contains(&SideEffect::Throw)
}

/// Mirror of OCaml `pure_or_terminal`: no effects, or only effects that
/// cause immediate termination (`Exit`, `IncompleteMatch`).
#[must_use]
pub fn is_pure_or_terminal(set: &EffectSet) -> bool {
    set.iter()
        .all(|e| matches!(e, SideEffect::Exit | SideEffect::IncompleteMatch))
}

/// Certain direct effects of one statement.
///
/// Only `Stmt::Undefined` maps with certainty (OCaml `E_undef` adds
/// `Undefined` in `infer_def_direct_effects`). Any other statement yields
/// the empty set: "no syntactically certain effect", not a purity verdict.
#[must_use]
pub fn direct_effects_of_stmt(stmt: &Stmt) -> EffectSet {
    match stmt {
        Stmt::Undefined { .. } => BTreeSet::from([SideEffect::Undefined]),
        Stmt::Assign { .. }
        | Stmt::TupleAssign { .. }
        | Stmt::LetTuple { .. }
        | Stmt::Expr(_)
        | Stmt::Return(_) => BTreeSet::new(),
    }
}

/// Union of certain direct effects over a statement list, visiting nested
/// `LetTuple` scopes like the other passes do.
fn direct_effects_of_body(body: &[Stmt]) -> EffectSet {
    let mut out = EffectSet::new();
    for stmt in body {
        out.extend(direct_effects_of_stmt(stmt));
        if let Stmt::LetTuple { body, .. } = stmt {
            out.extend(direct_effects_of_body(body));
        }
    }
    out
}

/// Per-definition summary of certain direct effects, in shard order.
///
/// Each entry is `(def name, certain direct effects)`. Transitive callee
/// effects and environment-dependent effects (register/throw/exit/calls)
/// are absent by construction — they need the checker binding.
#[must_use]
pub fn summarize_shard(shard: &Shard) -> Vec<(String, EffectSet)> {
    shard
        .defs
        .iter()
        .map(|def| (def.name.clone(), direct_effects_of_body(&def.body)))
        .collect()
}

/// Infer side effects for the shard.
///
/// Currently the identity on the shard: like OCaml, inference only
/// refreshes out-of-band effect state, and that state (whole-program
/// callgraph + type environments) needs the checker binding. The
/// checker-independent taxonomy and direct-effect summary above are
/// available to the future pipeline-level effect store.
#[derive(Clone, Copy, Debug, Default)]
pub struct InferEffects;

impl Pass for InferEffects {
    fn name(&self) -> &'static str {
        "infer_effects"
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    #[test]
    fn display_strings_match_ocaml_string_of_side_effect() {
        let cases = [
            (SideEffect::Throw, "throws exception"),
            (SideEffect::Exit, "exit statement"),
            (SideEffect::IncompleteMatch, "incomplete pattern match"),
            (SideEffect::Register, "register access"),
            (
                SideEffect::External,
                "calls external function not marked pure",
            ),
            (SideEffect::Undefined, "contains undefined literal"),
            (SideEffect::Scattered, "scattered function"),
            (SideEffect::NonExec, "not executable"),
        ];
        for (effect, text) in cases {
            assert_eq!(effect.to_string(), text);
        }
        assert_eq!(
            SideEffect::Outcome("sail_mem_write_request".into()).to_string(),
            "outcome sail_mem_write_request"
        );
    }

    #[test]
    fn purity_predicates_match_ocaml_semantics() {
        let empty = EffectSet::new();
        assert!(is_pure(&empty));
        assert!(!is_effectful(&empty));
        assert!(!throws(&empty));
        assert!(is_pure_or_terminal(&empty));

        let terminal: EffectSet = BTreeSet::from([SideEffect::Exit, SideEffect::IncompleteMatch]);
        assert!(!is_pure(&terminal));
        assert!(is_effectful(&terminal));
        assert!(is_pure_or_terminal(&terminal));

        let throwing = BTreeSet::from([SideEffect::Throw]);
        assert!(throws(&throwing));
        assert!(!is_pure_or_terminal(&throwing));

        let undefined = BTreeSet::from([SideEffect::Undefined]);
        assert!(!is_pure_or_terminal(&undefined));
    }

    #[test]
    fn only_undefined_maps_with_certainty() {
        assert_eq!(
            direct_effects_of_stmt(&Stmt::Undefined {
                target: "u".into(),
                ty: "int".into(),
            }),
            BTreeSet::from([SideEffect::Undefined])
        );
        // Everything else: no *certain* effect (not a purity claim).
        for stmt in [
            Stmt::Assign {
                lhs: "x".into(),
                rhs: "y".into(),
            },
            Stmt::TupleAssign {
                targets: vec!["a".into()],
                rhs: "t".into(),
            },
            Stmt::Expr("call()".into()),
            Stmt::Return("x".into()),
            Stmt::LetTuple {
                bindings: vec![],
                rhs: "e".into(),
                body: vec![],
            },
        ] {
            assert!(direct_effects_of_stmt(&stmt).is_empty());
        }
    }

    #[test]
    fn summary_visits_nested_scopes_in_shard_order() {
        let shard = Shard::new(
            0,
            vec![
                Def::new("pure_fn", DefKind::Function, vec![Stmt::Expr("e".into())]),
                Def::new(
                    "undef_fn",
                    DefKind::Function,
                    vec![
                        Stmt::Assign {
                            lhs: "x".into(),
                            rhs: "1".into(),
                        },
                        Stmt::LetTuple {
                            bindings: vec!["b".into()],
                            rhs: "t".into(),
                            body: vec![Stmt::Undefined {
                                target: "u".into(),
                                ty: "bool".into(),
                            }],
                        },
                    ],
                ),
            ],
        );
        let summary = summarize_shard(&shard);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].0, "pure_fn");
        assert!(summary[0].1.is_empty());
        assert_eq!(summary[1].0, "undef_fn");
        assert_eq!(summary[1].1, BTreeSet::from([SideEffect::Undefined]));
    }

    #[test]
    fn pass_is_identity_idempotent_and_named() {
        let shard = Shard::new(
            1,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Undefined {
                    target: "u".into(),
                    ty: "int".into(),
                }],
            )],
        );
        let once = InferEffects.run(shard.clone());
        assert_eq!(once, shard);
        assert_eq!(InferEffects.run(once.clone()), once);
        assert_eq!(InferEffects.name(), "infer_effects");
    }
}
