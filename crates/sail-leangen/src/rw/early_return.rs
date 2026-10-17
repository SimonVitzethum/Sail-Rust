#![forbid(unsafe_code)]
//! `rewrite_ast_early_return`: pull `return`s out of blocks, conditionals,
//! matches and lets as far as possible and turn the rest into
//! `early_return(..)` applications.

use sail_ast::{Exp, ExpKind, PExp, PExpKind};

use super::util::{exp_like, fold_exp, is_unit_lit, mk_exp, set_typ, typ_of, Rw};
use sail_ast::{Node, TypKind};

fn is_return(e: &Exp) -> bool {
    match &e.kind {
        ExpKind::Return(_) => true,
        ExpKind::Typ { exp, .. } => is_return(exp),
        _ => false,
    }
}

fn get_return(e: Exp) -> Exp {
    match e.kind {
        ExpKind::Return(inner) => *inner,
        ExpKind::Typ { typ, exp } => {
            let annotation = e.annotation;
            Node {
                id: e.id,
                span: e.span,
                kind: ExpKind::Typ { typ, exp: Box::new(get_return(*exp)) },
                annotation,
            }
        }
        _ => e,
    }
}

fn contains_return(e: &Exp) -> bool {
    crate::analysis::exp_exists(e, &mut |x| matches!(x.kind, ExpKind::Return(_)))
}

fn e_block(mut es: Vec<Exp>) -> Exp {
    // Fold the rest of a block into the branch of an `if` that does not return.
    let mut block: Vec<Exp> = Vec::new();
    while let Some(exp) = es.pop() {
        match exp.kind {
            ExpKind::If { cond, then_branch, else_branch } if is_return(&then_branch) => {
                let annot_src = block.last().unwrap_or(&else_branch).clone();
                let mut inner = block;
                if !is_unit_lit(&else_branch) {
                    inner.insert(0, *else_branch);
                }
                let e_prime = exp_like(&annot_src, ExpKind::Block(inner));
                block = vec![e_if(*cond, *then_branch, e_prime, &annot_src)];
            }
            ExpKind::If { cond, then_branch, else_branch } if is_return(&else_branch) => {
                let annot_src = block.last().unwrap_or(&then_branch).clone();
                let mut inner = block;
                if !is_unit_lit(&then_branch) {
                    inner.insert(0, *then_branch);
                }
                let t_prime = exp_like(&annot_src, ExpKind::Block(inner));
                block = vec![e_if(*cond, t_prime, *else_branch, &annot_src)];
            }
            kind => {
                let exp = Exp { kind, ..exp };
                let mut next = vec![exp];
                next.append(&mut block);
                block = next;
            }
        }
    }
    let es = block;
    if es.len() == 1 {
        return es.into_iter().next().expect("one element");
    }
    if es.last().is_some_and(is_return) {
        let mut es = es;
        let last = es.pop().expect("nonempty");
        let like = last.clone();
        let last_inner = get_return(last);
        es.push(last_inner);
        let block = exp_like(&like, ExpKind::Block(es));
        return exp_like(&like, ExpKind::Return(Box::new(block)));
    }
    mk_exp(ExpKind::Block(es))
}

fn e_if(cond: Exp, then_branch: Exp, else_branch: Exp, like: &Exp) -> Exp {
    if is_return(&then_branch) && is_return(&else_branch) {
        let t = get_return(then_branch);
        let e = get_return(else_branch);
        let annot_src = t.clone();
        let inner = exp_like(&annot_src, ExpKind::If {
            cond: Box::new(cond),
            then_branch: Box::new(t),
            else_branch: Box::new(e),
        });
        return exp_like(like, ExpKind::Return(Box::new(inner)));
    }
    exp_like(like, ExpKind::If {
        cond: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: Box::new(else_branch),
    })
}

fn pexp_is_return(p: &PExp) -> bool {
    match &p.kind {
        PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. } => is_return(body),
    }
}

fn pexp_get_return(p: PExp) -> PExp {
    let kind = match p.kind {
        PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat, body: get_return(body) },
        PExpKind::PatWhen { pat, guard, body } => PExpKind::PatWhen { pat, guard, body: get_return(body) },
    };
    PExp { kind, ..p }
}

/// The pull-out step applied to every rebuilt node (bottom-up).
fn pull(e: Exp) -> Exp {
    match e.kind {
        ExpKind::Block(es) if !es.is_empty() => {
            let like = e.annotation.clone();
            let mut out = e_block(es);
            out.annotation = like;
            out
        }
        ExpKind::If { cond, then_branch, else_branch } => {
            let like = Exp { kind: ExpKind::Undef, id: e.id, span: e.span, annotation: e.annotation };
            e_if(*cond, *then_branch, *else_branch, &like)
        }
        ExpKind::Match { scrutinee, arms } => {
            if !arms.is_empty() && arms.iter().all(pexp_is_return) {
                let arms: Vec<PExp> = arms.into_iter().map(pexp_get_return).collect();
                let annot_src = match &arms[0].kind {
                    PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. } => body.clone(),
                };
                let inner = exp_like(&annot_src, ExpKind::Match { scrutinee, arms });
                Exp {
                    kind: ExpKind::Return(Box::new(inner)),
                    id: e.id,
                    span: e.span,
                    annotation: e.annotation,
                }
            } else {
                Exp { kind: ExpKind::Match { scrutinee, arms }, ..Exp { kind: ExpKind::Undef, id: e.id, span: e.span, annotation: e.annotation } }
            }
        }
        ExpKind::Let { pat, bind, body } => {
            if is_return(&body) {
                let ret = get_return(*body);
                let annot_src = ret.clone();
                let inner = exp_like(&annot_src, ExpKind::Let { pat, bind, body: Box::new(ret) });
                Exp { kind: ExpKind::Return(Box::new(inner)), id: e.id, span: e.span, annotation: e.annotation }
            } else {
                Exp { kind: ExpKind::Let { pat, bind, body }, id: e.id, span: e.span, annotation: e.annotation }
            }
        }
        ExpKind::Var { lexp, init, body } => {
            if is_return(&body) {
                let ret = get_return(*body);
                let annot_src = ret.clone();
                let inner = exp_like(&annot_src, ExpKind::Var { lexp, init, body: Box::new(ret) });
                Exp { kind: ExpKind::Return(Box::new(inner)), id: e.id, span: e.span, annotation: e.annotation }
            } else {
                Exp { kind: ExpKind::Var { lexp, init, body }, id: e.id, span: e.span, annotation: e.annotation }
            }
        }
        ExpKind::App { func, mut args } => {
            if let Some(index) = args.iter().position(is_return) {
                let arg = args.swap_remove(index);
                let inner = get_return(arg);
                Exp { kind: ExpKind::Return(Box::new(inner)), id: e.id, span: e.span, annotation: e.annotation }
            } else {
                Exp { kind: ExpKind::App { func, args }, id: e.id, span: e.span, annotation: e.annotation }
            }
        }
        kind => Exp { kind, ..e },
    }
}

/// Ensure every final leaf is a `return` so the pull-out is uniform.
fn add_final_return(nested: bool, e: Exp) -> Exp {
    let Exp { id, span, kind, annotation } = e;
    let rewrap = |kind: ExpKind, annotation: Option<Box<sail_ast::Annotation>>| Exp { id, span, kind, annotation };
    match kind {
        ExpKind::Return(_) => rewrap(kind, annotation),
        ExpKind::Typ { typ, exp } => {
            let inner = add_final_return(nested, *exp);
            match inner.kind {
                ExpKind::Return(x) => {
                    let cast = rewrap(ExpKind::Typ { typ, exp: x }, annotation.clone());
                    rewrap(ExpKind::Return(Box::new(cast)), annotation)
                }
                other => {
                    let inner = Exp { kind: other, ..inner };
                    rewrap(ExpKind::Typ { typ, exp: Box::new(inner) }, annotation)
                }
            }
        }
        ExpKind::Block(mut es) if !es.is_empty() => {
            let last = es.pop().expect("nonempty");
            es.push(add_final_return(true, last));
            rewrap(ExpKind::Block(es), annotation)
        }
        ExpKind::If { cond, then_branch, else_branch } => rewrap(
            ExpKind::If {
                cond,
                then_branch: Box::new(add_final_return(true, *then_branch)),
                else_branch: Box::new(add_final_return(true, *else_branch)),
            },
            annotation,
        ),
        ExpKind::Match { scrutinee, arms } => {
            let arms = arms
                .into_iter()
                .map(|p| {
                    let kind = match p.kind {
                        PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat, body: add_final_return(true, body) },
                        PExpKind::PatWhen { pat, guard, body } => {
                            PExpKind::PatWhen { pat, guard, body: add_final_return(true, body) }
                        }
                    };
                    PExp { kind, ..p }
                })
                .collect();
            rewrap(ExpKind::Match { scrutinee, arms }, annotation)
        }
        ExpKind::Let { pat, bind, body } => rewrap(
            ExpKind::Let { pat, bind, body: Box::new(add_final_return(true, *body)) },
            annotation,
        ),
        ExpKind::Var { lexp, init, body } => rewrap(
            ExpKind::Var { lexp, init, body: Box::new(add_final_return(true, *body)) },
            annotation,
        ),
        other => {
            let exp = rewrap(other, annotation.clone());
            if nested && !contains_return(&exp) {
                rewrap(ExpKind::Return(Box::new(exp)), annotation)
            } else {
                exp
            }
        }
    }
}

/// Remaining `return e` become `early_return((e : ret_typ))`.
fn finalize(rw: &mut Rw<'_>, ret_typ: Option<&sail_ast::Typ>, e: Exp) -> Exp {
    match e.kind {
        ExpKind::Return(inner) => {
            let annotation = e.annotation;
            let cast = match ret_typ {
                Some(typ) => {
                    let mut cast = Node::dummy(ExpKind::Typ { typ: typ.clone(), exp: inner });
                    set_typ(&mut cast, Some(typ.clone()));
                    cast
                }
                None => *inner,
            };
            let func = rw.name("early_return");
            Exp {
                id: e.id,
                span: e.span,
                kind: ExpKind::App { func, args: vec![cast] },
                annotation,
            }
        }
        kind => Exp { kind, ..e },
    }
}

/// Rewrite one function body.
pub fn early_return_body(rw: &mut Rw<'_>, ret_typ: Option<&sail_ast::Typ>, exp: Exp) -> Exp {
    if !contains_return(&exp) {
        return exp;
    }
    let original = exp.clone();
    let pulled = fold_exp(add_final_return(false, exp), &mut pull);
    let chosen = if is_return(&pulled) { get_return(pulled) } else { original };
    fold_exp(chosen, &mut |e| finalize(rw, ret_typ, e))
}

/// Return type of a function type, in the definition's interner.
#[must_use]
pub fn ret_of(typ: &sail_ast::Typ) -> Option<&sail_ast::Typ> {
    match &typ.kind {
        TypKind::Fn { ret, .. } => Some(ret),
        _ => None,
    }
}

#[allow(dead_code)]
fn unused() {
    let _ = typ_of::<ExpKind>;
}
