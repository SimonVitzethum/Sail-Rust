//! Block flattening, matching `rewrite_ast_remove_blocks` in
//! `sail/src/lib/rewrites.ml`: `E_block []` becomes unit, `E_block [e]`
//! becomes `e`, and longer blocks become nested `E_let (wild, e, rest)`.
//!
//! The OCaml version annotates the synthesized `wild` pattern and `let` with
//! the statement types; the untyped AST carries no types, so dummy
//! patterns/nodes are used instead. Semantics are unchanged: every statement
//! still evaluates in order and non-unit values are discarded.

#![forbid(unsafe_code)]

use sail_ast::{ExpKind, LitKind, Node, PatKind};

use crate::pass::TypedPass;

use super::typed::{mk_exp, mk_pat, visit_all};

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveBlocks;

impl TypedPass for RemoveBlocks {
    fn name(&self) -> &'static str { "remove_blocks" }

    fn run(&self, ast: &mut sail_ast::Ast, _interner: &mut dyn sail_ast::Interner) {
        visit_all(
            ast,
            &mut |exp| {
                if matches!(exp.kind, ExpKind::Block(_)) {
                    remove_block(exp);
                }
            },
            &mut |_| {},
        );
    }
}

fn remove_block(exp: &mut sail_ast::Exp) {
    let ExpKind::Block(stmts) = std::mem::replace(&mut exp.kind, ExpKind::Undef) else {
        return;
    };
    match stmts.len() {
        0 => {
            exp.kind = ExpKind::Lit(Node::dummy(LitKind::Unit));
        }
        1 => {
            let mut stmts = stmts;
            *exp = stmts.swap_remove(0);
        }
        _ => {
            // `[a, b, c]` becomes `let _ = a in (let _ = b in c)`: seed with
            // the last statement, wrap the rest back to front.
            let mut stmts = stmts;
            let last = stmts.pop().expect("len >= 2");
            *exp = stmts.into_iter().rev().fold(last, |body, stmt| {
                mk_exp(ExpKind::Let {
                    pat: mk_pat(PatKind::Wild),
                    bind: Box::new(stmt),
                    body: Box::new(body),
                })
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, DefKind, SimpleInterner};

    fn run(exp: sail_ast::Exp) -> sail_ast::Exp {
        let mut it = SimpleInterner::new();
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp,
        })]);
        RemoveBlocks.run(&mut ast, &mut it);
        let DefKind::Let { exp, .. } = ast.defs.swap_remove(0).kind else {
            panic!("def shape")
        };
        exp
    }

    #[test]
    fn empty_block_becomes_unit() {
        let out = run(mk_exp(ExpKind::Block(vec![])));
        assert!(
            matches!(&out.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit)),
            "got {:?}",
            out.kind
        );
    }

    #[test]
    fn singleton_block_unwraps() {
        let out = run(mk_exp(ExpKind::Block(vec![mk_exp(ExpKind::Undef)])));
        assert!(matches!(out.kind, ExpKind::Undef));
    }

    #[test]
    fn longer_block_nests_lets_in_order() {
        let out = run(mk_exp(ExpKind::Block(vec![
            mk_exp(ExpKind::Undef),
            mk_exp(ExpKind::Lit(Node::dummy(LitKind::Unit))),
            mk_exp(ExpKind::Undef),
        ])));
        // let _ = Undef in (let _ = () in Undef)
        let ExpKind::Let { bind, body, .. } = &out.kind else {
            panic!("expected let, got {:?}", out.kind)
        };
        assert!(matches!(bind.kind, ExpKind::Undef));
        let ExpKind::Let { bind, body, .. } = &body.kind else {
            panic!("expected nested let, got {:?}", body.kind)
        };
        assert!(matches!(
            &bind.kind,
            ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit)
        ));
        assert!(matches!(body.kind, ExpKind::Undef));
        // Idempotent: no blocks remain.
        let again = run(out);
        assert!(matches!(&again.kind, ExpKind::Let { .. }));
    }
}
