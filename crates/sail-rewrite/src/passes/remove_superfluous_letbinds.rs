//! Redundant-let removal, matching `rewrite_ast_remove_superfluous_letbinds`
//! in `sail/src/lib/rewrites.ml`.
//!
//! Implemented rules (type/effect-free, hence exact):
//! - `let x = e in x` (and `plet`) becomes `e`;
//! - `let _ = () in e` becomes `e`;
//! - `let pat = throw e in ...` becomes `throw e`.
//!
//! Skipped until a typed path exists, as documented inline:
//! - `let x = e in return x` → `return e` needs `small` + purity analysis;
//! - the `assert(false)` → `exit` rule needs the prover (`prove ... nc_not`).

#![forbid(unsafe_code)]

use sail_ast::{Exp, ExpKind, LitKind, PatKind};

use crate::pass::TypedPass;

use super::typed::visit_all;

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveSuperfluousLetbinds;

impl TypedPass for RemoveSuperfluousLetbinds {
    fn name(&self) -> &'static str { "remove_superfluous_letbinds" }

    fn run(&self, ast: &mut sail_ast::Ast, _interner: &mut dyn sail_ast::Interner) {
        visit_all(
            ast,
            &mut |exp| {
                match &mut exp.kind {
                    ExpKind::Let { .. } | ExpKind::InternalPlet { .. } => {
                        if let Some(replacement) = simpler(exp) {
                            *exp = replacement;
                        }
                    }
                    _ => {}
                }
            },
            &mut |_| {},
        );
    }
}

/// Peel `E_typ` coercions, the untyped-AST equivalent of OCaml `uncast_exp`.
fn uncast(exp: &Exp) -> &Exp {
    let mut cur = exp;
    while let ExpKind::Typ { exp: inner, .. } = &cur.kind {
        cur = inner;
    }
    cur
}

/// Peel `P_typ` annotations, the equivalent of OCaml `untyp_pat`.
fn untyp(pat: &sail_ast::Pat) -> &sail_ast::Pat {
    let mut cur = pat;
    while let PatKind::Typ { pat: inner, .. } = &cur.kind {
        cur = inner;
    }
    cur
}

fn is_unit_lit(exp: &Exp) -> bool {
    matches!(&uncast(exp).kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit))
}

/// Compute the simplified replacement for a let/plet node, if any rule fires.
fn simpler(exp: &Exp) -> Option<Exp> {
    let (pat, bind, body) = match &exp.kind {
        ExpKind::Let { pat, bind, body } => (pat, bind, body),
        ExpKind::InternalPlet { pat, bind, body } => (pat, bind, body),
        _ => return None,
    };
    // `let x = e in x` becomes `e`.
    if let PatKind::Id(id) = &untyp(pat).kind {
        if let ExpKind::Id(id2) = &uncast(body).kind {
            if id.symbol == id2.symbol {
                return Some((**bind).clone());
            }
        }
    }
    // `let _ = () in e` becomes `e`.
    if matches!(untyp(pat).kind, PatKind::Wild) && is_unit_lit(bind) {
        return Some((**body).clone());
    }
    // `let pat = throw e in ...` becomes `throw e`.
    if matches!(&uncast(bind).kind, ExpKind::Throw(_)) {
        return Some((**bind).clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, DefKind, Interner, SimpleInterner};

    use crate::passes::typed::{mk_exp, mk_name, mk_pat};

    fn run(exp: Exp) -> Exp {
        let mut it = SimpleInterner::new();
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp,
        })]);
        RemoveSuperfluousLetbinds.run(&mut ast, &mut it);
        let DefKind::Let { exp, .. } = ast.defs.swap_remove(0).kind else {
            panic!("def shape")
        };
        exp
    }

    fn ident(it: &mut SimpleInterner, name: &str) -> Exp {
        mk_exp(ExpKind::Id(mk_name(it.intern(name))))
    }

    #[test]
    fn let_x_in_x_collapses() {
        let mut it = SimpleInterner::new();
        let pat = mk_pat(PatKind::Id(mk_name(it.intern("x"))));
        let out = run(mk_exp(ExpKind::Let {
            pat,
            bind: Box::new(ident(&mut it, "e")),
            body: Box::new(ident(&mut it, "x")),
        }));
        assert!(matches!(out.kind, ExpKind::Id(_)));
    }

    #[test]
    fn let_wild_unit_collapses_and_throw_wins() {
        let mut it = SimpleInterner::new();
        let unit = mk_exp(ExpKind::Lit(sail_ast::Node::dummy(LitKind::Unit)));
        let out = run(mk_exp(ExpKind::Let {
            pat: mk_pat(PatKind::Wild),
            bind: Box::new(unit),
            body: Box::new(ident(&mut it, "e")),
        }));
        assert!(matches!(out.kind, ExpKind::Id(_)));

        let out = run(mk_exp(ExpKind::Let {
            pat: mk_pat(PatKind::Id(mk_name(it.intern("y")))),
            bind: Box::new(mk_exp(ExpKind::Throw(Box::new(ident(&mut it, "e"))))),
            body: Box::new(ident(&mut it, "z")),
        }));
        assert!(matches!(out.kind, ExpKind::Throw(_)));
    }

    #[test]
    fn unrelated_lets_survive_and_pass_is_idempotent() {
        let mut it = SimpleInterner::new();
        let exp = mk_exp(ExpKind::Let {
            pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
            bind: Box::new(ident(&mut it, "e")),
            body: Box::new(ident(&mut it, "y")),
        });
        let out = run(exp);
        assert!(matches!(&out.kind, ExpKind::Let { .. }));
        let again = run(out.clone());
        assert_eq!(again, out);
    }
}
