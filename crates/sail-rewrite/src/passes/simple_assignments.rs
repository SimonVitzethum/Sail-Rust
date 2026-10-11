//! Preserve simple assignments as OCaml `rewrite_simple_assignments` does.
//!
//! OCaml lowers complex lvalues to updates; it does not eliminate `x = x`.
//! Such writes may target registers or effectful lvalues. The text-based
//! statement model has no typed lvalue information, so retain assignments
//! until complex lvalues can be lowered through the unified AST.

#![forbid(unsafe_code)]

use crate::ast::Shard;
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct SimpleAssignments;

impl Pass for SimpleAssignments {
    fn name(&self) -> &'static str { "simple_assignments" }
    fn run(&self, shard: Shard) -> Shard { shard }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind, Stmt};

    #[test]
    fn preserves_register_writes_and_effectful_lvalues() {
        let shard = Shard::new(0, vec![Def::new("f", DefKind::Function, vec![
            Stmt::Assign { lhs: "R".into(), rhs: "R".into() },
            Stmt::Assign { lhs: "a[next()]".into(), rhs: "a[next()]".into() },
            Stmt::Assign { lhs: "x".into(), rhs: "y".into() },
        ])]);
        assert_eq!(SimpleAssignments.run(shard.clone()), shard);
    }
}
