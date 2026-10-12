//! String-append pattern removal, matching `rewrite_ast_pat_string_append`
//! in `sail/src/lib/rewrites.ml`.
//!
//! Any match arm whose pattern contains `P_string_append` cannot be compiled
//! by backends without string-pattern support, so the whole arm becomes
//! `wildcard -> exit ()` (OCaml builds exactly this via `construct_pexp`).
//! Guards on such arms are dropped with the pattern, as in OCaml.

#![forbid(unsafe_code)]

use sail_ast::{ExpKind, LitKind, Node, PatKind, PExpKind};

use crate::pass::TypedPass;

use super::typed::{mk_exp, mk_pat, pat_has_string_append, visit_all};

#[derive(Clone, Copy, Debug, Default)]
pub struct PatStringAppend;

impl TypedPass for PatStringAppend {
    fn name(&self) -> &'static str { "pat_string_append" }

    fn run(&self, ast: &mut sail_ast::Ast, _interner: &mut dyn sail_ast::Interner) {
        visit_all(
            ast,
            &mut |_| {},
            &mut |arm| {
                let has = match &arm.kind {
                    PExpKind::PatExp { pat, .. } => pat_has_string_append(pat),
                    PExpKind::PatWhen { pat, .. } => pat_has_string_append(pat),
                };
                if has {
                    arm.kind = PExpKind::PatExp {
                        pat: mk_pat(PatKind::Wild),
                        body: mk_exp(ExpKind::Exit(Box::new(mk_exp(ExpKind::Lit(Node::dummy(
                            LitKind::Unit,
                        )))))),
                    };
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, DefKind, PExp, SimpleInterner};

    use crate::passes::typed::mk_arm;

    fn ast_with_arm(arm: PExp) -> (Ast, SimpleInterner) {
        let it = SimpleInterner::new();
        let ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp: mk_exp(ExpKind::Match {
                scrutinee: Box::new(mk_exp(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
                arms: vec![arm],
            }),
        })]);
        (ast, it)
    }

    #[test]
    fn string_append_arm_becomes_wildcard_exit() {
        let arm = mk_arm(
            mk_pat(PatKind::StringAppend(vec![mk_pat(PatKind::Wild)])),
            mk_exp(ExpKind::Undef),
        );
        let (mut ast, mut it) = ast_with_arm(arm);
        PatStringAppend.run(&mut ast, &mut it);
        let DefKind::Let { exp, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        let ExpKind::Match { arms, .. } = &exp.kind else {
            panic!("exp shape")
        };
        let PExpKind::PatExp { pat, body } = &arms[0].kind else {
            panic!("arm shape")
        };
        assert!(matches!(pat.kind, PatKind::Wild));
        assert!(matches!(body.kind, ExpKind::Exit(_)));
        // Idempotent.
        let snapshot = ast.clone();
        PatStringAppend.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn plain_arm_survives() {
        let arm = mk_arm(mk_pat(PatKind::Wild), mk_exp(ExpKind::Undef));
        let (mut ast, mut it) = ast_with_arm(arm);
        let snapshot = ast.clone();
        PatStringAppend.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
