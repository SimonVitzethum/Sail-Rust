//! Tuple assignment lowering, matching `rewrite_tuple_assignments` in
//! `sail/src/lib/rewrites.ml`: bind every RHS component before writing any
//! target. Repeated projections of the original RHS duplicate effects and
//! break swaps when earlier assignments mutate later components.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use crate::ast::{Shard, Stmt};
use crate::pass::{Pass, TypedPass};

use super::typed::{Fresh, collect_used, mk_exp, mk_lexp, mk_name, mk_pat, visit_all};

#[derive(Clone, Copy, Debug, Default)]
pub struct TupleAssignments;

impl Pass for TupleAssignments {
    fn name(&self) -> &'static str { "tuple_assignments" }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            let mut used = BTreeSet::new();
            collect_identifiers(&def.body, &mut used);
            let mut next = 0;
            rewrite_body(&mut def.body, &mut used, &mut next);
        }
        shard
    }
}

impl TypedPass for TupleAssignments {
    fn name(&self) -> &'static str { "tuple_assignments" }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        // OCaml `rewrite_tuple_assignments` loops `rewrite_pass` until no
        // `E_assign (LE_tuple, _)` remains (`go`). The bottom-up walk already
        // rewrites nested tuples inside-out; the fixpoint covers tuples that
        // only appear after an outer rewrite.
        let mut fresh = Fresh::new(collect_used(ast, interner));
        loop {
            let mut changed = false;
            visit_all(
                ast,
                &mut |exp| {
                    if rewrite_tuple_exp(exp, &mut fresh, &mut *interner) {
                        changed = true;
                    }
                },
                &mut |_| {},
            );
            if !changed {
                break;
            }
        }
    }
}

/// Rewrite one `E_assign (LE_tuple lexps, rhs)` into
/// `E_let (P_tuple (P_id tup__n ...), rhs, E_block [assign ...])`.
///
/// Returns whether the node was rewritten. Binds the RHS once before writing
/// any target, exactly as OCaml does: repeated projections of the original
/// RHS would duplicate effects and break swaps.
fn rewrite_tuple_exp(
    exp: &mut sail_ast::Exp,
    fresh: &mut Fresh,
    interner: &mut dyn sail_ast::Interner,
) -> bool {
    use sail_ast::{ExpKind, LExpKind, Name, PatKind, Symbol};
    let ExpKind::Assign { lexp, .. } = &mut exp.kind else {
        return false;
    };
    if !matches!(lexp.kind, LExpKind::Tuple(_)) {
        return false;
    }
    let LExpKind::Tuple(targets) =
        std::mem::replace(&mut lexp.kind, LExpKind::Id(Name::dummy(Symbol::DUMMY)))
    else {
        return false;
    };
    let ExpKind::Assign { lexp: _, value } = std::mem::replace(&mut exp.kind, ExpKind::Undef) else {
        return false;
    };
    let ids: Vec<Name> = targets
        .iter()
        .map(|_| mk_name(interner.intern(&fresh.fresh("tup__"))))
        .collect();
    let assigns: Vec<sail_ast::Exp> = targets
        .into_iter()
        .zip(ids.iter().cloned())
        .map(|(target, id)| {
            // Fresh `LExp` nodes drop source ids/spans, the untyped-AST
            // equivalent of OCaml `strip_lexp` (no type annotations exist
            // here to preserve).
            mk_exp(ExpKind::Assign {
                lexp: Box::new(mk_lexp(target.kind)),
                value: Box::new(mk_exp(ExpKind::Id(id))),
            })
        })
        .collect();
    let pat_ids: Vec<sail_ast::Pat> = ids
        .into_iter()
        .map(|id| mk_pat(PatKind::Id(id)))
        .collect();
    exp.kind = ExpKind::Let {
        pat: mk_pat(PatKind::Tuple(pat_ids)),
        bind: value,
        body: Box::new(mk_exp(ExpKind::Block(assigns))),
    };
    true
}

fn collect_identifiers(body: &[Stmt], used: &mut BTreeSet<String>) {
    fn text(src: &str, used: &mut BTreeSet<String>) {
        used.extend(src.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty()).map(str::to_owned));
    }
    for stmt in body {
        match stmt {
            Stmt::Assign { lhs, rhs } => { text(lhs, used); text(rhs, used); }
            Stmt::TupleAssign { targets, rhs } => {
                for target in targets { text(target, used); }
                text(rhs, used);
            }
            Stmt::LetTuple { bindings, rhs, body } => {
                for binding in bindings { text(binding, used); }
                text(rhs, used);
                collect_identifiers(body, used);
            }
            Stmt::Undefined { target, ty } => { text(target, used); text(ty, used); }
            Stmt::Expr(src) | Stmt::Return(src) => text(src, used),
        }
    }
}

fn rewrite_body(body: &mut [Stmt], used: &mut BTreeSet<String>, next: &mut usize) {
    for stmt in body {
        match stmt {
            Stmt::TupleAssign { targets, rhs } => {
                let bindings: Vec<String> = targets.iter().map(|_| loop {
                    let name = format!("__sail_tuple_{}", *next);
                    *next += 1;
                    if used.insert(name.clone()) { break name; }
                }).collect();
                let assignments = std::mem::take(targets).into_iter()
                    .zip(bindings.iter().cloned())
                    .map(|(lhs, rhs)| Stmt::Assign { lhs, rhs }).collect();
                *stmt = Stmt::LetTuple {
                    bindings, rhs: std::mem::take(rhs), body: assignments,
                };
            }
            Stmt::LetTuple { body, .. } => rewrite_body(body, used, next),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn run_body(body: Vec<Stmt>) -> Vec<Stmt> {
        crate::pass::Pass::run(&TupleAssignments, Shard::new(0,
            vec![Def::new("f", DefKind::Function, body)])).defs.swap_remove(0).body
    }

    #[test]
    fn binds_effectful_rhs_once_before_assigning_any_target() {
        let out = run_body(vec![Stmt::TupleAssign {
            targets: vec!["a".into(), "b".into()], rhs: "call()".into(),
        }]);
        assert_eq!(out, vec![Stmt::LetTuple {
            bindings: vec!["__sail_tuple_0".into(), "__sail_tuple_1".into()],
            rhs: "call()".into(),
            body: vec![
                Stmt::Assign { lhs: "a".into(), rhs: "__sail_tuple_0".into() },
                Stmt::Assign { lhs: "b".into(), rhs: "__sail_tuple_1".into() },
            ],
        }]);
    }

    #[test]
    fn zero_and_one_component_still_use_tuple_binding() {
        for targets in [vec![], vec!["x".into()]] {
            let out = run_body(vec![Stmt::TupleAssign {
                targets: targets.clone(), rhs: "effect()".into(),
            }]);
            let Stmt::LetTuple { bindings, rhs, body } = &out[0] else { panic!("missing binding") };
            assert_eq!(bindings.len(), targets.len());
            assert_eq!(body.len(), targets.len());
            assert_eq!(rhs, "effect()");
        }
    }

    #[test]
    fn avoids_capture_and_preserves_surrounding_statements() {
        let first = Stmt::Expr("__sail_tuple_0".into());
        let last = Stmt::Return("a".into());
        let out = run_body(vec![first.clone(), Stmt::TupleAssign {
            targets: vec!["a".into(), "__sail_tuple_1".into()],
            rhs: "(__sail_tuple_1, a)".into(),
        }, last.clone()]);
        assert_eq!(out[0], first);
        assert_eq!(out[2], last);
        let Stmt::LetTuple { bindings, .. } = &out[1] else { panic!("missing binding") };
        assert_eq!(bindings, &["__sail_tuple_2", "__sail_tuple_3"]);
        assert_eq!(run_body(out.clone()), out, "pass must be idempotent");
    }

    mod typed {
        use sail_ast::{
            Ast, DefKind, ExpKind, Interner, LExpKind, PatKind, SimpleInterner,
        };

        use crate::passes::typed::{mk_exp, mk_lexp, mk_name, mk_pat};

        use super::TupleAssignments;

        fn assign_ast(targets: Vec<&str>, rhs: &str) -> (Ast, SimpleInterner) {
            let mut it = SimpleInterner::new();
            let lexps = targets
                .iter()
                .map(|t| mk_lexp(LExpKind::Id(mk_name(it.intern(t)))))
                .collect();
            let rhs_exp = mk_exp(ExpKind::Id(mk_name(it.intern(rhs))));
            let assign = mk_exp(ExpKind::Assign {
                lexp: Box::new(mk_lexp(LExpKind::Tuple(lexps))),
                value: Box::new(rhs_exp),
            });
            let ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
                pat: mk_pat(PatKind::Wild),
                exp: assign,
            })]);
            (ast, it)
        }

        fn single_exp(ast: &Ast) -> &sail_ast::Exp {
            let DefKind::Let { exp, .. } = &ast.defs[0].kind else {
                panic!("def shape")
            };
            exp
        }

        #[test]
        fn typed_binds_rhs_once_with_fresh_ids() {
            let (mut ast, mut it) = assign_ast(vec!["a", "b"], "t");
            crate::pass::TypedPass::run(&TupleAssignments, &mut ast, &mut it);
            let ExpKind::Let { pat, bind, body } = &single_exp(&ast).kind else {
                panic!("expected let, got {:?}", single_exp(&ast).kind)
            };
            assert!(matches!(bind.kind, ExpKind::Id(_)));
            let PatKind::Tuple(ids) = &pat.kind else {
                panic!("expected tuple pat")
            };
            assert_eq!(ids.len(), 2);
            let ExpKind::Block(assigns) = &body.kind else {
                panic!("expected block")
            };
            assert_eq!(assigns.len(), 2);
            for a in assigns {
                assert!(matches!(a.kind, ExpKind::Assign { .. }));
            }
            // Fresh names follow OCaml (`tup__0`, `tup__1`).
            let got: Vec<_> = ids
                .iter()
                .map(|p| match &p.kind {
                    PatKind::Id(n) => it.resolve(n.symbol).unwrap().to_owned(),
                    other => panic!("expected id pat, got {other:?}"),
                })
                .collect();
            assert_eq!(got, vec!["tup__0", "tup__1"]);
        }

        #[test]
        fn typed_avoids_capture_and_is_idempotent() {
            let (mut ast, mut it) = assign_ast(vec!["a", "tup__0"], "t");
            crate::pass::TypedPass::run(&TupleAssignments, &mut ast, &mut it);
            let ExpKind::Let { pat, .. } = &single_exp(&ast).kind else {
                panic!("expected let")
            };
            let PatKind::Tuple(ids) = &pat.kind else {
                panic!("expected tuple pat")
            };
            let names: Vec<_> = ids
                .iter()
                .map(|p| match &p.kind {
                    PatKind::Id(n) => it.resolve(n.symbol).unwrap().to_owned(),
                    other => panic!("expected id pat, got {other:?}"),
                })
                .collect();
            assert_eq!(names, vec!["tup__1", "tup__2"]);
            let snapshot = ast.clone();
            crate::pass::TypedPass::run(&TupleAssignments, &mut ast, &mut it);
            assert_eq!(ast, snapshot, "pass must be idempotent");
        }
    }
}
