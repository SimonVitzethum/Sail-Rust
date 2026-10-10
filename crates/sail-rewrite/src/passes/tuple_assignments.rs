//! `tuple_assignments` pass: desugar tuple destructuring assignments.
//!
//! OCaml equivalent: `("tuple_assignments", [])` in `lean_rewrites`
//! (`sail_plugin_lean.ml`). The Lean backend cannot emit a multi-target
//! assignment directly, so `(a, b) = rhs` must become separate assignments.
//!
//! Lowering: `(t0, t1, ..) = rhs` becomes
//! `t0 = rhs.0; t1 = rhs.1; ..` (tuple field projection), preserving order.
//! A single-element tuple degrades to a plain assignment; an empty tuple
//! becomes a bare effect statement. Shard-local, no cross-def state.

#![forbid(unsafe_code)]

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

/// Desugar [`Stmt::TupleAssign`] into per-target [`Stmt::Assign`]s.
#[derive(Clone, Copy, Debug, Default)]
pub struct TupleAssignments;

impl Pass for TupleAssignments {
    fn name(&self) -> &'static str {
        "tuple_assignments"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            def.body = rewrite_body(std::mem::take(&mut def.body));
        }
        shard
    }
}

/// Rewrite one statement list, expanding tuple assignments in place.
fn rewrite_body(body: Vec<Stmt>) -> Vec<Stmt> {
    // Tuple expansion grows the list; reserve a little headroom.
    let mut out = Vec::with_capacity(body.len() + 2);
    for stmt in body {
        match stmt {
            Stmt::TupleAssign { targets, rhs } => {
                expand_tuple(&mut out, targets, rhs);
            }
            other => out.push(other),
        }
    }
    out
}

/// Push the expansion of one tuple assignment onto `out`.
fn expand_tuple(out: &mut Vec<Stmt>, targets: Vec<String>, rhs: String) {
    match targets.len() {
        0 => {
            // `() = rhs` keeps only the effect of evaluating rhs.
            out.push(Stmt::Expr(rhs));
        }
        1 => {
            let lhs = targets.into_iter().next().unwrap_or_default();
            out.push(Stmt::Assign { lhs, rhs });
        }
        _ => {
            for (i, target) in targets.into_iter().enumerate() {
                out.push(Stmt::Assign {
                    lhs: target,
                    rhs: format!("{rhs}.{i}"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn run_body(body: Vec<Stmt>) -> Vec<Stmt> {
        let p = TupleAssignments;
        let shard = Shard::new(0, vec![Def::new("f", DefKind::Function, body)]);
        p.run(shard).defs.swap_remove(0).body
    }

    #[test]
    fn expands_pair_into_projections() {
        let out = run_body(vec![Stmt::TupleAssign {
            targets: vec!["a".to_string(), "b".to_string()],
            rhs: "call()".to_string(),
        }]);
        assert_eq!(
            out,
            vec![
                Stmt::Assign {
                    lhs: "a".to_string(),
                    rhs: "call().0".to_string()
                },
                Stmt::Assign {
                    lhs: "b".to_string(),
                    rhs: "call().1".to_string()
                },
            ]
        );
    }

    #[test]
    fn single_target_becomes_plain_assign() {
        let out = run_body(vec![Stmt::TupleAssign {
            targets: vec!["a".to_string()],
            rhs: "e".to_string(),
        }]);
        assert_eq!(
            out,
            vec![Stmt::Assign {
                lhs: "a".to_string(),
                rhs: "e".to_string()
            }]
        );
    }

    #[test]
    fn empty_tuple_keeps_effect_only() {
        let out = run_body(vec![Stmt::TupleAssign {
            targets: vec![],
            rhs: "e".to_string(),
        }]);
        assert_eq!(out, vec![Stmt::Expr("e".to_string())]);
    }

    #[test]
    fn preserves_statement_order_and_others() {
        let out = run_body(vec![
            Stmt::Expr("first".to_string()),
            Stmt::TupleAssign {
                targets: vec!["x".to_string(), "y".to_string()],
                rhs: "t".to_string(),
            },
            Stmt::Return("x".to_string()),
        ]);
        assert_eq!(
            out,
            vec![
                Stmt::Expr("first".to_string()),
                Stmt::Assign {
                    lhs: "x".to_string(),
                    rhs: "t.0".to_string()
                },
                Stmt::Assign {
                    lhs: "y".to_string(),
                    rhs: "t.1".to_string()
                },
                Stmt::Return("x".to_string()),
            ]
        );
    }
}
