//! Clause merging, matching `merge_funcls` in `sail/src/lib/rewrites.ml`.
//!
//! A multi-clause function becomes a single clause matching on a fresh
//! variable: `f pat1 = e1 | f pat2 = e2` turns into
//! `f merge#var = match merge#var with pat1 => e1 | pat2 => e2`. OCaml uses
//! the literal name `merge#var`; `#` is unwritable in Sail source, so no
//! capture is possible and the same fixed name is reused here. Single-clause
//! (or empty) definitions are untouched, as in OCaml.

#![forbid(unsafe_code)]

use sail_ast::{DefKind, ExpKind, Funcl, PExp, PatKind};

use crate::pass::TypedPass;

use super::typed::{mk_arm, mk_exp, mk_name, mk_pat};

#[derive(Clone, Copy, Debug, Default)]
pub struct MergeFunctionClauses;

impl TypedPass for MergeFunctionClauses {
    fn name(&self) -> &'static str { "merge_function_clauses" }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        let var = mk_name(interner.intern("merge#var"));
        for def in &mut ast.defs {
            match &mut def.kind {
                DefKind::Fundef { funcls, .. } => merge_funcls(funcls, &var),
                DefKind::InternalMutrec(sets) => {
                    for set in sets {
                        merge_funcls(&mut set.funcls, &var);
                    }
                }
                _ => {}
            }
        }
    }
}

fn merge_funcls(funcls: &mut Vec<Funcl>, var: &sail_ast::Name) {
    if funcls.len() < 2 {
        return;
    }
    let mut old = std::mem::take(funcls);
    let first = &old[0];
    let (id, span, name) = (first.id, first.span, first.name.clone());
    let arms = old
        .drain(..)
        .map(|f| {
            // Each clause arm is already a match arm (`PExp`); moving it under
            // the synthesized `match` preserves guards (`PatWhen`).
            let (pid, pspan, kind) = (f.arm.id, f.arm.span, f.arm.kind);
            PExp { id: pid, span: pspan, kind }
        })
        .collect();
    funcls.push(Funcl {
        id,
        span,
        name,
        arm: mk_arm(
            mk_pat(PatKind::Id(var.clone())),
            mk_exp(ExpKind::Match {
                scrutinee: Box::new(mk_exp(ExpKind::Id(var.clone()))),
                arms,
            }),
        ),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, Interner, NodeId, PExpKind, RecOpt, SimpleInterner, Span, TAnnotOpt};

    use crate::passes::typed::mk_pat;

    fn funcl(it: &mut SimpleInterner, body: sail_ast::Exp) -> Funcl {
        Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: mk_name(it.intern("f")),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: mk_pat(PatKind::Wild),
                    body,
                },
            },
        }
    }

    #[test]
    fn merges_multiple_clauses_into_one_match() {
        let mut it = SimpleInterner::new();
        let body = mk_exp(ExpKind::Undef);
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl(&mut it, body.clone()), funcl(&mut it, body)],
        })]);
        MergeFunctionClauses.run(&mut ast, &mut it);
        let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        assert_eq!(funcls.len(), 1);
        let PExpKind::PatExp { pat, body } = &funcls[0].arm.kind else {
            panic!("arm shape")
        };
        assert!(matches!(&pat.kind, PatKind::Id(n) if it.resolve(n.symbol) == Some("merge#var")));
        let ExpKind::Match { arms, .. } = &body.kind else {
            panic!("expected match, got {:?}", body.kind)
        };
        assert_eq!(arms.len(), 2);
        // Idempotent: a single clause is left alone.
        let snapshot = ast.clone();
        MergeFunctionClauses.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn single_clause_is_untouched() {
        let mut it = SimpleInterner::new();
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl(&mut it, mk_exp(ExpKind::Undef))],
        })]);
        let snapshot = ast.clone();
        MergeFunctionClauses.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
