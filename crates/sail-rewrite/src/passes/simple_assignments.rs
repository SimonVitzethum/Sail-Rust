//! `simple_assignments` pass: normalise trivially redundant assignments.
//!
//! OCaml equivalent: `("simple_assignments", [])` in `lean_rewrites`
//! (`sail_plugin_lean.ml`). After passes like `tuple_assignments` the body
//! contains self-assignments (`x = x`) or duplicate no-op statements that
//! would clutter the Lean output; this pass removes them.
//!
//! Rule: drop `x = x` (lhs text equals rhs text). Everything else passes
//! through unchanged, preserving order. Shard-local, no cross-def state.

#![forbid(unsafe_code)]

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

/// Remove self-assignments (`x = x`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SimpleAssignments;

impl Pass for SimpleAssignments {
    fn name(&self) -> &'static str {
        "simple_assignments"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            def.body = rewrite_body(std::mem::take(&mut def.body));
        }
        shard
    }
}

/// Rewrite one statement list, dropping self-assignments.
fn rewrite_body(body: Vec<Stmt>) -> Vec<Stmt> {
    body.into_iter()
        .filter(|stmt| !matches!(stmt, Stmt::Assign { lhs, rhs } if lhs.trim() == rhs.trim()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn run_body(body: Vec<Stmt>) -> Vec<Stmt> {
        let p = SimpleAssignments;
        let shard = Shard::new(0, vec![Def::new("f", DefKind::Function, body)]);
        p.run(shard).defs.swap_remove(0).body
    }

    #[test]
    fn drops_self_assignments() {
        let out = run_body(vec![
            Stmt::Assign {
                lhs: "x".to_string(),
                rhs: "x".to_string(),
            },
            Stmt::Assign {
                lhs: "y".to_string(),
                rhs: "z".to_string(),
            },
        ]);
        assert_eq!(
            out,
            vec![Stmt::Assign {
                lhs: "y".to_string(),
                rhs: "z".to_string()
            }]
        );
    }

    #[test]
    fn keeps_everything_else_in_order() {
        let body = vec![
            Stmt::TupleAssign {
                targets: vec!["a".to_string()],
                rhs: "e".to_string(),
            },
            Stmt::Expr("skip".to_string()),
            Stmt::Return("a".to_string()),
        ];
        assert_eq!(run_body(body.clone()), body);
    }
}
