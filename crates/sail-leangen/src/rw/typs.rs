#![forbid(unsafe_code)]
//! Type helpers for the rewrites: `is_src_typ`, `add_p_typ`, `add_e_typ`,
//! `add_typs_let`, tuple types and `unit` tests.

use sail_ast::{
    ConstraintKind, Exp, ExpKind, Node, NExp, NExpKind, Pat, PatKind, Typ, TypArg, TypArgKind, TypKind,
};

use super::util::{set_typ, Rw};

fn generated(name: &str) -> bool {
    name.contains('#') || name.starts_with("__")
}

fn nexp_has_generated(rw: &Rw<'_>, n: &NExp, bound: &mut Vec<String>) -> bool {
    match &n.kind {
        NExpKind::Var(s) => {
            let text = sail_ast::Interner::resolve(&rw.it, *s).unwrap_or("?");
            !bound.iter().any(|b| b == text) && generated(text)
        }
        NExpKind::Id(_) | NExpKind::Constant(_) => false,
        NExpKind::App { args, .. } => args.iter().any(|a| nexp_has_generated(rw, a, bound)),
        NExpKind::If { cond, then_branch, else_branch } => {
            constraint_has_generated(rw, cond, bound)
                || nexp_has_generated(rw, then_branch, bound)
                || nexp_has_generated(rw, else_branch, bound)
        }
        NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
            nexp_has_generated(rw, a, bound) || nexp_has_generated(rw, b, bound)
        }
        NExpKind::Exp(a) | NExpKind::Neg(a) => nexp_has_generated(rw, a, bound),
    }
}

fn arg_has_generated(rw: &Rw<'_>, a: &TypArg, bound: &mut Vec<String>) -> bool {
    match &a.kind {
        TypArgKind::NExp(n) => nexp_has_generated(rw, n, bound),
        TypArgKind::Typ(t) => typ_has_generated_in(rw, t, bound),
        TypArgKind::Bool(c) => constraint_has_generated(rw, c, bound),
    }
}

fn constraint_has_generated(rw: &Rw<'_>, c: &sail_ast::Constraint, bound: &mut Vec<String>) -> bool {
    match &c.kind {
        ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
            arg_has_generated(rw, a, bound) || arg_has_generated(rw, b, bound)
        }
        ConstraintKind::Ge(a, b)
        | ConstraintKind::Gt(a, b)
        | ConstraintKind::Le(a, b)
        | ConstraintKind::Lt(a, b) => nexp_has_generated(rw, a, bound) || nexp_has_generated(rw, b, bound),
        ConstraintKind::Set { nexp, .. } => nexp_has_generated(rw, nexp, bound),
        ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
            constraint_has_generated(rw, a, bound) || constraint_has_generated(rw, b, bound)
        }
        ConstraintKind::Not(a) => constraint_has_generated(rw, a, bound),
        ConstraintKind::App { args, .. } => args.iter().any(|a| arg_has_generated(rw, a, bound)),
        ConstraintKind::Var(s) => {
            let text = sail_ast::Interner::resolve(&rw.it, *s).unwrap_or("?");
            !bound.iter().any(|b| b == text) && generated(text)
        }
        ConstraintKind::Id(_) | ConstraintKind::True | ConstraintKind::False => false,
    }
}

fn typ_has_generated_in(rw: &Rw<'_>, t: &Typ, bound: &mut Vec<String>) -> bool {
    match &t.kind {
        TypKind::Unknown | TypKind::Id(_) => false,
        TypKind::Var(s) => {
            let text = sail_ast::Interner::resolve(&rw.it, *s).unwrap_or("?");
            !bound.iter().any(|b| b == text) && generated(text)
        }
        TypKind::Fn { args, ret } => {
            args.iter().any(|a| typ_has_generated_in(rw, a, bound)) || typ_has_generated_in(rw, ret, bound)
        }
        TypKind::Bidir { lhs, rhs } => typ_has_generated_in(rw, lhs, bound) || typ_has_generated_in(rw, rhs, bound),
        TypKind::Tuple(items) => items.iter().any(|a| typ_has_generated_in(rw, a, bound)),
        TypKind::App { args, .. } => args.iter().any(|a| arg_has_generated(rw, a, bound)),
        TypKind::Exist { binders, constraint, typ } => {
            let mark = bound.len();
            for b in binders {
                bound.push(sail_ast::Interner::resolve(&rw.it, b.kid).unwrap_or("?").to_owned());
            }
            let r = constraint_has_generated(rw, constraint, bound) || typ_has_generated_in(rw, typ, bound);
            bound.truncate(mark);
            r
        }
    }
}

/// Type variables (free) of a type, by name.
fn typ_vars(rw: &Rw<'_>, t: &Typ, bound: &mut Vec<String>, out: &mut Vec<String>) {
    struct V<'a, 'b> {
        rw: &'a Rw<'b>,
    }
    fn nexp(v: &V<'_, '_>, n: &NExp, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match &n.kind {
            NExpKind::Var(s) => {
                let text = sail_ast::Interner::resolve(&v.rw.it, *s).unwrap_or("?").to_owned();
                if !bound.contains(&text) {
                    out.push(text);
                }
            }
            NExpKind::Id(_) | NExpKind::Constant(_) => {}
            NExpKind::App { args, .. } => args.iter().for_each(|a| nexp(v, a, bound, out)),
            NExpKind::If { cond, then_branch, else_branch } => {
                constraint(v, cond, bound, out);
                nexp(v, then_branch, bound, out);
                nexp(v, else_branch, bound, out);
            }
            NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
                nexp(v, a, bound, out);
                nexp(v, b, bound, out);
            }
            NExpKind::Exp(a) | NExpKind::Neg(a) => nexp(v, a, bound, out),
        }
    }
    fn arg(v: &V<'_, '_>, a: &TypArg, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match &a.kind {
            TypArgKind::NExp(n) => nexp(v, n, bound, out),
            TypArgKind::Typ(t) => typ(v, t, bound, out),
            TypArgKind::Bool(c) => constraint(v, c, bound, out),
        }
    }
    fn constraint(v: &V<'_, '_>, c: &sail_ast::Constraint, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match &c.kind {
            ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
                arg(v, a, bound, out);
                arg(v, b, bound, out);
            }
            ConstraintKind::Ge(a, b)
            | ConstraintKind::Gt(a, b)
            | ConstraintKind::Le(a, b)
            | ConstraintKind::Lt(a, b) => {
                nexp(v, a, bound, out);
                nexp(v, b, bound, out);
            }
            ConstraintKind::Set { nexp: n, .. } => nexp(v, n, bound, out),
            ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
                constraint(v, a, bound, out);
                constraint(v, b, bound, out);
            }
            ConstraintKind::Not(a) => constraint(v, a, bound, out),
            ConstraintKind::App { args, .. } => args.iter().for_each(|a| arg(v, a, bound, out)),
            ConstraintKind::Var(s) => {
                let text = sail_ast::Interner::resolve(&v.rw.it, *s).unwrap_or("?").to_owned();
                if !bound.contains(&text) {
                    out.push(text);
                }
            }
            ConstraintKind::Id(_) | ConstraintKind::True | ConstraintKind::False => {}
        }
    }
    fn typ(v: &V<'_, '_>, t: &Typ, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match &t.kind {
            TypKind::Unknown | TypKind::Id(_) => {}
            TypKind::Var(s) => {
                let text = sail_ast::Interner::resolve(&v.rw.it, *s).unwrap_or("?").to_owned();
                if !bound.contains(&text) {
                    out.push(text);
                }
            }
            TypKind::Fn { args, ret } => {
                args.iter().for_each(|a| typ(v, a, bound, out));
                typ(v, ret, bound, out);
            }
            TypKind::Bidir { lhs, rhs } => {
                typ(v, lhs, bound, out);
                typ(v, rhs, bound, out);
            }
            TypKind::Tuple(items) => items.iter().for_each(|a| typ(v, a, bound, out)),
            TypKind::App { args, .. } => args.iter().for_each(|a| arg(v, a, bound, out)),
            TypKind::Exist { binders, constraint: c, typ: inner } => {
                let mark = bound.len();
                for b in binders {
                    bound.push(sail_ast::Interner::resolve(&v.rw.it, b.kid).unwrap_or("?").to_owned());
                }
                constraint(v, c, bound, out);
                typ(v, inner, bound, out);
                bound.truncate(mark);
            }
        }
    }
    typ(&V { rw }, t, bound, out);
}

/// `is_src_typ`: no generated kids, and every existential binder is used.
#[must_use]
pub fn is_src_typ(rw: &Rw<'_>, t: &Typ) -> bool {
    match &t.kind {
        TypKind::Tuple(items) => items.iter().all(|x| is_src_typ(rw, x)),
        TypKind::Exist { binders, typ, .. } => {
            let mut used = Vec::new();
            // The binders are free in the body: collect with no bound names.
            typ_vars(rw, typ, &mut Vec::new(), &mut used);
            let unused = binders.iter().any(|b| {
                let text = sail_ast::Interner::resolve(&rw.it, b.kid).unwrap_or("?");
                !used.iter().any(|u| u == text)
            });
            !unused && !typ_has_generated_in(rw, t, &mut Vec::new())
        }
        _ => !typ_has_generated_in(rw, t, &mut Vec::new()),
    }
}

pub fn remove_p_typ(p: Pat) -> Pat {
    match p.kind {
        PatKind::Typ { pat, .. } => remove_p_typ(*pat),
        _ => p,
    }
}

pub fn remove_e_typ(e: Exp) -> Exp {
    match e.kind {
        ExpKind::Typ { exp, .. } => remove_e_typ(*exp),
        _ => e,
    }
}

/// `add_p_typ env typ pat`.
#[must_use]
pub fn add_p_typ(rw: &Rw<'_>, typ: Option<&Typ>, pat: Pat) -> Pat {
    match typ {
        Some(t) if is_src_typ(rw, t) => {
            let annotation = pat.annotation.clone();
            let (id, span) = (pat.id, pat.span);
            Node {
                id,
                span,
                kind: PatKind::Typ { typ: t.clone(), pat: Box::new(remove_p_typ(pat)) },
                annotation,
            }
        }
        _ => pat,
    }
}

/// `add_e_typ env typ exp`.
#[must_use]
pub fn add_e_typ(rw: &Rw<'_>, typ: Option<&Typ>, exp: Exp) -> Exp {
    match typ {
        Some(t) if is_src_typ(rw, t) => {
            let annotation = exp.annotation.clone();
            let (id, span) = (exp.id, exp.span);
            Node {
                id,
                span,
                kind: ExpKind::Typ { typ: t.clone(), exp: Box::new(remove_e_typ(exp)) },
                annotation,
            }
        }
        _ => exp,
    }
}

/// `add_typs_let env ltyp rtyp exp`: type the pattern of a let.
#[must_use]
pub fn add_typs_let(rw: &Rw<'_>, ltyp: Option<&Typ>, exp: Exp) -> Exp {
    match exp.kind {
        ExpKind::Let { pat, bind, body } => Exp {
            kind: ExpKind::Let { pat: add_p_typ(rw, ltyp, pat), bind, body },
            ..Node { id: exp.id, span: exp.span, kind: ExpKind::Undef, annotation: exp.annotation }
        },
        ExpKind::InternalPlet { pat, bind, body } => Exp {
            kind: ExpKind::InternalPlet { pat: add_p_typ(rw, ltyp, pat), bind, body },
            ..Node { id: exp.id, span: exp.span, kind: ExpKind::Undef, annotation: exp.annotation }
        },
        _ => exp,
    }
}

/// `unit` as a type in the definition's interner.
#[must_use]
pub fn is_unit_typ(rw: &Rw<'_>, t: &Typ) -> bool {
    match &t.kind {
        TypKind::Id(n) => rw.text(n) == "unit",
        _ => false,
    }
}

/// `tuple_typ`; `None` when a component type is unknown.
#[must_use]
pub fn tuple_typ(items: Vec<Option<Typ>>) -> Option<Typ> {
    let items: Option<Vec<Typ>> = items.into_iter().collect();
    items.map(|v| Node::dummy(TypKind::Tuple(v)))
}

pub fn with_typ<K>(mut node: Node<K>, typ: Option<Typ>) -> Node<K> {
    set_typ(&mut node, typ);
    node
}
