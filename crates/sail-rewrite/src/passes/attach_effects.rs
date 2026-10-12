//! Attach inferred effects to the AST, matching `attach_effects` in OCaml
//! `sail/src/lib/rewrites.ml` (`all_rewriters`):
//!
//! ```ocaml
//! ("attach_effects",
//!   base_rewriter (fun effect_info env ast ->
//!     (Effects.rewrite_attach_effects effect_info ast, effect_info, env)))
//! ```
//!
//! OCaml `Effects.rewrite_attach_effects` (`sail/src/lib/effects.ml`) folds
//! every expression/lexp and annotates its `tannot` with the effect:
//! `E_app` looks up the callee in `effect_info.functions`, `E_undef`/
//! `E_throw`/`E_exit`/`E_assert` and register-typed `E_id`/`LE_id`/`LE_typ`
//! are monadic, `LE_deref` is monadic, everything else inherits the child
//! effect. Per `effects.mli`, "the effect info is simplified in its
//! annotated form — it just becomes a boolean representing
//! effectful/non-effectful".
//!
//! Checker-free status: the string-based [`Stmt`](crate::ast::Stmt) surface
//! has no annotation slot, and the `effect_info` table plus the per-node
//! type environments both need the checker binding (see the lane report).
//! Until they exist there is no honest place to write effects to, so `run`
//! is the identity. What ships without the checker:
//!
//! - [`EffectAnnot`]: the simplified boolean as a typed enum (`Pure` vs
//!   `Monadic`), the exact annotation payload the future slot will carry.
//! - [`annotation_points`]: the number of statement nodes (visiting nested
//!   `LetTuple` scopes) that will each need one annotation once the slot
//!   exists — the structural coverage the OCaml `fold_exp` provides.

#![forbid(unsafe_code)]

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

/// Simplified per-node effect annotation, mirroring the boolean OCaml
/// `rewrite_attach_effects` writes into each `tannot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectAnnot {
    /// Pure node: no side effects.
    Pure,
    /// Effectful node: must run in the Sail monad.
    Monadic,
}

impl EffectAnnot {
    /// Whether this node must run in the Sail monad.
    #[must_use]
    pub fn is_monadic(self) -> bool {
        matches!(self, Self::Monadic)
    }

    /// Whether this node is pure.
    #[must_use]
    pub fn is_pure(self) -> bool {
        matches!(self, Self::Pure)
    }

    /// The OCaml simplified form: `true` = effectful.
    #[must_use]
    pub fn as_bool(self) -> bool {
        self.is_monadic()
    }
}

/// Number of statement nodes in the shard, visiting nested `LetTuple`
/// scopes.
///
/// Each counted node will need one [`EffectAnnot`] once the annotation
/// slot exists; the count pins the traversal coverage (no dropped scopes)
/// independent of the checker binding.
#[must_use]
pub fn annotation_points(shard: &Shard) -> usize {
    shard.defs.iter().map(|def| count_body(&def.body)).sum()
}

fn count_body(body: &[Stmt]) -> usize {
    body.iter()
        .map(|stmt| {
            1 + match stmt {
                Stmt::LetTuple { body, .. } => count_body(body),
                Stmt::Assign { .. }
                | Stmt::TupleAssign { .. }
                | Stmt::Undefined { .. }
                | Stmt::Expr(_)
                | Stmt::Return(_) => 0,
            }
        })
        .sum()
}

/// Attach inferred effects to every node in the shard.
///
/// Currently the identity: without an annotation slot on [`Stmt`] and
/// without the `effect_info` table from `infer_effects` there is nowhere
/// honest to write effects to. See the module docs and the lane report
/// for the missing API.
#[derive(Clone, Copy, Debug, Default)]
pub struct AttachEffects;

impl Pass for AttachEffects {
    fn name(&self) -> &'static str {
        "attach_effects"
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
    fn annotation_boolean_matches_ocaml_simplified_form() {
        assert!(EffectAnnot::Monadic.is_monadic());
        assert!(!EffectAnnot::Monadic.is_pure());
        assert!(EffectAnnot::Monadic.as_bool());
        assert!(EffectAnnot::Pure.is_pure());
        assert!(!EffectAnnot::Pure.is_monadic());
        assert!(!EffectAnnot::Pure.as_bool());
    }

    #[test]
    fn counts_every_node_including_nested_scopes() {
        let shard = Shard::new(
            0,
            vec![
                Def::new(
                    "f",
                    DefKind::Function,
                    vec![
                        Stmt::Assign {
                            lhs: "x".into(),
                            rhs: "1".into(),
                        },
                        Stmt::LetTuple {
                            bindings: vec!["b".into()],
                            rhs: "t".into(),
                            body: vec![
                                Stmt::Expr("inner".into()),
                                Stmt::LetTuple {
                                    bindings: vec![],
                                    rhs: "e".into(),
                                    body: vec![Stmt::Return("b".into())],
                                },
                            ],
                        },
                    ],
                ),
                Def::new("g", DefKind::Function, vec![]),
            ],
        );
        // f: Assign(1) + outer LetTuple(1) + Expr(1) + inner LetTuple(1) +
        // Return(1) = 5; g: 0.
        assert_eq!(annotation_points(&shard), 5);
        assert_eq!(annotation_points(&Shard::new(0, vec![])), 0);
    }

    #[test]
    fn pass_is_identity_idempotent_and_named() {
        let shard = Shard::new(
            2,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Undefined {
                        target: "u".into(),
                        ty: "int".into(),
                    },
                    Stmt::Return("u".into()),
                ],
            )],
        );
        let once = AttachEffects.run(shard.clone());
        assert_eq!(once, shard);
        assert_eq!(AttachEffects.run(once.clone()), once);
        assert_eq!(AttachEffects.name(), "attach_effects");
    }
}
