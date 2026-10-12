//! Re-elaborate every definition, matching `recheck_defs` in OCaml
//! `sail/src/lib/rewrites.ml` (`all_rewriters`):
//!
//! ```ocaml
//! ("recheck_defs",
//!   checking_rewriter (fun _ ast -> Type_error.check initial_env
//!     ~internal:"During rewriting" (strip_ast ast)))
//! ```
//!
//! Contract: strip all type annotations, then re-run the full typechecker.
//! On success the returned AST has the same shape (this pass is
//! shape-preserving); on failure the checker raises.
//!
//! Checker-free status: the string-based [`Shard`](crate::ast::Shard) surface
//! carries no type annotations and `sail-rewrite` has no dependency on a
//! checker (`sail-ty` works on its own mini-AST and exposes no
//! whole-shard `check` entry; see the lane report for the missing-API spec).
//! Until that binding exists there is nothing semantic to verify without
//! guessing, so `run` is the identity — exactly the success-case behaviour.
//! It stays shard-local, deterministic, and idempotent, satisfying the
//! [`Pass`](crate::pass::Pass) contract.

#![forbid(unsafe_code)]

use crate::ast::Shard;
use crate::pass::Pass;

/// Re-typecheck every definition in the shard.
///
/// Currently the identity: without a checker binding only the
/// success-case shape preservation can be honoured. See the module docs
/// and the lane report for the missing checker API.
#[derive(Clone, Copy, Debug, Default)]
pub struct RecheckDefs;

impl Pass for RecheckDefs {
    fn name(&self) -> &'static str {
        "recheck_defs"
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind, Stmt};

    fn sample() -> Shard {
        Shard::new(
            3,
            vec![
                Def::new(
                    "f",
                    DefKind::Function,
                    vec![
                        Stmt::Assign {
                            lhs: "x".into(),
                            rhs: "x".into(),
                        },
                        Stmt::Return("x".into()),
                    ],
                ),
                Def::new("sig", DefKind::Valspec, vec![]),
                Def::new("r", DefKind::Register, vec![Stmt::Expr("0".into())]),
                Def::new("t", DefKind::Type, vec![]),
                Def::new("p", DefKind::Other, vec![]),
            ],
        )
    }

    #[test]
    fn preserves_every_def_kind_verbatim() {
        let shard = sample();
        assert_eq!(RecheckDefs.run(shard.clone()), shard);
    }

    #[test]
    fn preserves_empty_shards_and_nested_scopes() {
        assert_eq!(
            RecheckDefs.run(Shard::new(0, vec![])),
            Shard::new(0, vec![])
        );
        let nested = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Undefined {
                        target: "u".into(),
                        ty: "int".into(),
                    },
                    Stmt::LetTuple {
                        bindings: vec![],
                        rhs: "e".into(),
                        body: vec![Stmt::Expr("inner".into())],
                    },
                ],
            )],
        );
        assert_eq!(RecheckDefs.run(nested.clone()), nested);
    }

    #[test]
    fn is_idempotent_and_deterministic() {
        let shard = sample();
        let once = RecheckDefs.run(shard.clone());
        assert_eq!(RecheckDefs.run(once.clone()), once);
        assert_eq!(RecheckDefs.run(shard.clone()), once);
        assert_eq!(RecheckDefs.name(), "recheck_defs");
    }
}
