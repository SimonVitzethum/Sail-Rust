//! Preserve simple assignments as OCaml `rewrite_simple_assignments` does.
//!
//! OCaml lowers complex lvalues to updates; it does not eliminate `x = x`.
//! Such writes may target registers or effectful lvalues. The text-based
//! statement model has no typed lvalue information, so retain assignments
//! until complex lvalues can be lowered through the unified AST.

#![forbid(unsafe_code)]

use crate::ast::Shard;
use crate::pass::{Pass, TypedPass};

#[derive(Clone, Copy, Debug, Default)]
pub struct SimpleAssignments;

impl Pass for SimpleAssignments {
    fn name(&self) -> &'static str { "simple_assignments" }
    fn run(&self, shard: Shard) -> Shard { shard }
}

impl TypedPass for SimpleAssignments {
    fn name(&self) -> &'static str { "simple_assignments" }

    fn run(&self, _ast: &mut sail_ast::Ast, _interner: &mut dyn sail_ast::Interner) {
        // Typed-path preserve: OCaml `rewrite_simple_assignments` keeps
        // `LE_id`/`LE_typ` targets and lowers every other lvalue through
        // `rewrite_lexp_to_rhs` (vector/field/range updates need the type
        // environment and `check_exp`). Without those, rewriting a complex
        // lvalue would change register/effect semantics, so all assignments
        // are preserved — the same conservative choice as the string path.
    }
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
        assert_eq!(crate::pass::Pass::run(&SimpleAssignments, shard.clone()), shard);
    }

    #[test]
    fn typed_preserves_simple_and_complex_lvalues() {
        use sail_ast::{Ast, DefKind, ExpKind, Interner, LExpKind, PatKind, SimpleInterner};

        use crate::passes::typed::{mk_exp, mk_lexp, mk_name, mk_pat};

        let mut it = SimpleInterner::new();
        let x = mk_name(it.intern("x"));
        let r = mk_exp(ExpKind::Id(x.clone()));
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp: mk_exp(ExpKind::Block(vec![
                mk_exp(ExpKind::Assign {
                    lexp: Box::new(mk_lexp(LExpKind::Id(x))),
                    value: Box::new(r),
                }),
                mk_exp(ExpKind::Assign {
                    lexp: Box::new(mk_lexp(LExpKind::Field {
                        lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("rec"))))),
                        field: mk_name(it.intern("f")),
                    })),
                    value: Box::new(mk_exp(ExpKind::Undef)),
                }),
            ])),
        })]);
        let snapshot = ast.clone();
        crate::pass::TypedPass::run(&SimpleAssignments, &mut ast, &mut it);
        assert_eq!(ast, snapshot, "assignments must survive without type info");
    }
}
