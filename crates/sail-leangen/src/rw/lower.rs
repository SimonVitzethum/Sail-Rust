#![forbid(unsafe_code)]
//! `remove_blocks`, `exp_lift_assign`, `remove_e_assign` (`rewrite_var_updates`),
//! `internal_lets`, `remove_superfluous_letbinds/returns`.

use std::collections::BTreeMap;

use sail_ast::Interner as _;
use sail_ast::{
    Annotation, Exp, ExpKind, FExp, KOpt, Kind, LExp, LExpKind, Lit, LitKind, Loop, Name, Node, Order, PExp, PExpKind,
    Pat, PatKind, Typ, TypArg, TypArgKind, TypKind, TypPatKind,
};

use super::typs::{add_e_typ, add_p_typ, add_typs_let, is_unit_typ, tuple_typ, with_typ};
use super::util::{fold_exp, map_children, mk_exp, mk_pat, set_typ, typ_of, unit_exp, Rw};
use crate::analysis::{effectful, has_early_return};

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

fn mk(kind: ExpKind, typ: Option<Typ>) -> Exp {
    with_typ(mk_exp(kind), typ)
}

fn mkp(kind: PatKind, typ: Option<Typ>) -> Pat {
    with_typ(mk_pat(kind), typ)
}

fn typ_clone(e: &Exp) -> Option<Typ> {
    typ_of(e).cloned()
}

fn unit_lit(rw: &mut Rw<'_>) -> Exp {
    let u = rw.unit_typ();
    mk(unit_exp().kind, Some(u))
}

fn shell(e: &Exp) -> Exp {
    Node { id: e.id, span: e.span, kind: ExpKind::Undef, annotation: e.annotation.clone() }
}

fn rebuild(like: &Exp, kind: ExpKind, typ: Option<Typ>) -> Exp {
    let mut e = Node { id: like.id, span: like.span, kind, annotation: like.annotation.clone() };
    set_typ(&mut e, typ);
    e
}

fn is_intro(e: &Exp) -> bool {
    e.annotation.as_ref().is_some_and(|a: &Box<Annotation>| a.attributes.iter().any(|x| x.name == "intro"))
}

fn lexp_ids<'a>(l: &'a LExp, out: &mut Vec<&'a Name>) {
    match &l.kind {
        LExpKind::Id(n) | LExpKind::Typ { id: n, .. } => out.push(n),
        LExpKind::Deref(_) => {}
        LExpKind::Tuple(xs) | LExpKind::VectorConcat(xs) => xs.iter().for_each(|x| lexp_ids(x, out)),
        LExpKind::Vector { lexp, .. } | LExpKind::VectorRange { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_ids(lexp, out);
        }
    }
}

/// `lexp_is_local`: an assignment target that is not a register or reference.
fn lexp_is_local(rw: &Rw<'_>, l: &LExp) -> bool {
    match &l.kind {
        LExpKind::Deref(_) => false,
        LExpKind::Id(n) | LExpKind::Typ { id: n, .. } => !rw.oracle.is_register(&rw.text(n)),
        LExpKind::Tuple(xs) | LExpKind::VectorConcat(xs) => xs.iter().all(|x| lexp_is_local(rw, x)),
        LExpKind::Vector { lexp, .. } | LExpKind::VectorRange { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_is_local(rw, lexp)
        }
    }
}

/// Member of the overloads `vector_access`/`vector_update` for a value of
/// type `typ` (the typechecker's resolution of the `#` primitives).
fn vector_member(rw: &Rw<'_>, bitvector: &str, plain: &str, typ: Option<&Typ>) -> String {
    let is_bits = typ
        .and_then(|t| rw.oracle.typ_head(t, &rw.it))
        .is_some_and(|h| h == "bits" || h == "bitvector");
    if is_bits { bitvector } else { plain }.to_owned()
}

fn lexp_to_exp(rw: &mut Rw<'_>, l: &LExp) -> Exp {
    let typ = typ_of(l).cloned();
    match &l.kind {
        LExpKind::Id(n) => mk(ExpKind::Id(n.clone()), typ),
        LExpKind::Typ { typ: t, id } => {
            let inner = mk(ExpKind::Id(id.clone()), Some(t.clone()));
            mk(ExpKind::Typ { typ: t.clone(), exp: Box::new(inner) }, Some(t.clone()))
        }
        LExpKind::Tuple(xs) => {
            let es: Vec<Exp> = xs.iter().map(|x| lexp_to_exp(rw, x)).collect();
            let t = tuple_typ(es.iter().map(typ_clone).collect());
            mk(ExpKind::Tuple(es), t)
        }
        LExpKind::Vector { lexp, index } => {
            let base = lexp_to_exp(rw, lexp);
            let member = vector_member(rw, "bitvector_access", "plain_vector_access", typ_of(&base));
            let func = rw.name(&member);
            mk(ExpKind::App { func, args: vec![base, (**index).clone()] }, typ)
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            let base = lexp_to_exp(rw, lexp);
            let func = rw.name("subrange_bits");
            mk(ExpKind::App { func, args: vec![base, (**lo).clone(), (**hi).clone()] }, typ)
        }
        LExpKind::Field { lexp, field } => {
            let base = lexp_to_exp(rw, lexp);
            mk(ExpKind::Field { record: Box::new(base), field: field.clone() }, typ)
        }
        LExpKind::Deref(e) => {
            let func = rw.name("__deref");
            mk(ExpKind::App { func, args: vec![(**e).clone()] }, typ)
        }
        LExpKind::VectorConcat(xs) => {
            let es: Vec<Exp> = xs.iter().map(|x| lexp_to_exp(rw, x)).collect();
            es.into_iter()
                .reduce(|a, b| mk(ExpKind::VectorAppend(Box::new(a), Box::new(b)), typ.clone()))
                .unwrap_or_else(|| mk(ExpKind::Vector(Vec::new()), typ))
        }
    }
}

type Rhs = Box<dyn FnOnce(Exp) -> Exp>;

/// `rewrite_lexp_to_rhs`: the plain target and a function building the new
/// value of the whole target from the assigned value.
fn lexp_to_rhs(rw: &mut Rw<'_>, le: LExp) -> Result<(LExp, Rhs), String> {
    match le.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } | LExpKind::Tuple(_) | LExpKind::Deref(_) => Ok((le, Box::new(|e| e))),
        LExpKind::Vector { lexp, index } => {
            let base_exp = lexp_to_exp(rw, &lexp);
            let typ = typ_of(&*lexp).cloned();
            let member = vector_member(rw, "bitvector_update", "plain_vector_update", typ.as_ref());
            let func = rw.name(&member);
            let (lhs, rhs) = lexp_to_rhs(rw, *lexp)?;
            Ok((
                lhs,
                Box::new(move |value| rhs(mk(ExpKind::App { func, args: vec![base_exp, *index, value] }, typ))),
            ))
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            let base_exp = lexp_to_exp(rw, &lexp);
            let typ = typ_of(&*lexp).cloned();
            let func = rw.name("update_subrange_bits");
            let (lhs, rhs) = lexp_to_rhs(rw, *lexp)?;
            Ok((
                lhs,
                Box::new(move |value| rhs(mk(ExpKind::App { func, args: vec![base_exp, *lo, *hi, value] }, typ))),
            ))
        }
        LExpKind::Field { lexp, field } => {
            let base_exp = lexp_to_exp(rw, &lexp);
            let typ = typ_of(&*lexp).cloned();
            let (lhs, rhs) = lexp_to_rhs(rw, *lexp)?;
            Ok((
                lhs,
                Box::new(move |value| {
                    let fe = FExp { id: sail_ast::NodeId::DUMMY, span: sail_ast::Span::DUMMY, field, value };
                    rhs(mk(ExpKind::StructUpdate { record: Box::new(base_exp), fields: vec![fe] }, typ))
                }),
            ))
        }
        LExpKind::VectorConcat(_) => Err("vector concatenation assignment survived the rewrites".into()),
    }
}

fn pat_of_local_lexp(rw: &Rw<'_>, l: &LExp) -> Result<Pat, String> {
    let typ = typ_of(l).cloned();
    match &l.kind {
        LExpKind::Id(n) => Ok(mkp(PatKind::Id(n.clone()), typ)),
        LExpKind::Typ { typ: t, id } => {
            let p = mkp(PatKind::Id(id.clone()), typ);
            Ok(add_p_typ(rw, Some(t), p))
        }
        LExpKind::Tuple(xs) => {
            let ps: Result<Vec<Pat>, String> = xs.iter().map(|x| pat_of_local_lexp(rw, x)).collect();
            Ok(mkp(PatKind::Tuple(ps?), typ))
        }
        _ => Err("unexpected local lexp".into()),
    }
}

// ---------------------------------------------------------------------------
// remove_blocks
// ---------------------------------------------------------------------------

fn blocks_to_lets(rw: &mut Rw<'_>, mut es: Vec<Exp>) -> Exp {
    match es.len() {
        0 => unit_lit(rw),
        1 => es.pop().expect("one"),
        _ => {
            let first = es.remove(0);
            let rest = blocks_to_lets(rw, es);
            let ftyp = typ_clone(&first);
            let wild = mkp(PatKind::Wild, ftyp.clone());
            let body_typ = typ_clone(&rest);
            let like = shell(&first);
            let e = rebuild(&like, ExpKind::Let { pat: wild, bind: Box::new(first), body: Box::new(rest) }, body_typ);
            add_typs_let(rw, ftyp.as_ref(), e)
        }
    }
}

pub fn remove_blocks(rw: &mut Rw<'_>, e: Exp) -> Exp {
    fold_exp(e, &mut |x| match x.kind {
        ExpKind::Block(es) => {
            let out = blocks_to_lets(rw, es);
            Exp { id: x.id, span: x.span, ..out }
        }
        _ => x,
    })
}

// ---------------------------------------------------------------------------
// exp_lift_assign
// ---------------------------------------------------------------------------

pub fn exp_lift_assign(rw: &mut Rw<'_>, e: Exp) -> Result<Exp, String> {
    let like = shell(&e);
    match e.kind {
        ExpKind::Block(es) => {
            let walked = walk_block(rw, es)?;
            Ok(rebuild(&like, ExpKind::Block(walked), typ_clone(&like)))
        }
        ExpKind::Assign { lexp, value } if is_intro(&like) && lexp_is_local(rw, &lexp) => {
            let value = exp_lift_assign(rw, *value)?;
            let (le, rhs) = lexp_to_rhs(rw, *lexp)?;
            let init = rhs(value);
            let u = rw.unit_typ();
            let body = mk(ExpKind::Block(Vec::new()), Some(u));
            Ok(rebuild(
                &like,
                ExpKind::Var { lexp: Box::new(le), init: Box::new(init), body: Box::new(body) },
                typ_clone(&like),
            ))
        }
        kind => {
            let e = Exp { kind, ..like };
            let mut err = None;
            let out = map_children(e, &mut |c| match exp_lift_assign(rw, c.clone()) {
                Ok(x) => x,
                Err(m) => {
                    err = Some(m);
                    c
                }
            });
            match err {
                Some(m) => Err(m),
                None => Ok(out),
            }
        }
    }
}

fn walk_block(rw: &mut Rw<'_>, mut es: Vec<Exp>) -> Result<Vec<Exp>, String> {
    if es.is_empty() {
        return Ok(es);
    }
    let first = es.remove(0);
    let intro = matches!(&first.kind, ExpKind::Assign { lexp, .. } if lexp_is_local(rw, lexp)) && is_intro(&first);
    if intro {
        let like = shell(&first);
        let ExpKind::Assign { lexp, value } = first.kind else { unreachable!() };
        let (le, rhs) = lexp_to_rhs(rw, *lexp)?;
        let value = exp_lift_assign(rw, *value)?;
        let init = rhs(value);
        let rest = walk_block(rw, es)?;
        let block_typ = rest.last().and_then(typ_clone).or_else(|| Some(rw.unit_typ()));
        let block = mk(ExpKind::Block(rest), block_typ.clone());
        Ok(vec![rebuild(
            &like,
            ExpKind::Var { lexp: Box::new(le), init: Box::new(init), body: Box::new(block) },
            block_typ,
        )])
    } else {
        let mut out = vec![exp_lift_assign(rw, first)?];
        out.extend(walk_block(rw, es)?);
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// internal_lets
// ---------------------------------------------------------------------------

pub fn internal_lets(rw: &mut Rw<'_>, e: Exp) -> Exp {
    fold_exp(e, &mut |x| {
        let like = shell(&x);
        match x.kind {
            ExpKind::Let { pat, bind, body } => {
                let wild = matches!(pat.kind, PatKind::Wild)
                    || matches!(&pat.kind, PatKind::Typ { pat: p, .. } if matches!(p.kind, PatKind::Wild));
                let local_assign = wild
                    && matches!(&bind.kind, ExpKind::Assign { lexp, .. } if lexp_is_local(rw, lexp));
                if local_assign {
                    let bind_like = shell(&bind);
                    let ExpKind::Assign { lexp, value } = bind.kind.clone() else { unreachable!() };
                    if let Ok((lhs, rhs)) = lexp_to_rhs(rw, *lexp) {
                        if let Ok(new_pat) = pat_of_local_lexp(rw, &lhs) {
                            let ltyp = typ_of(&lhs).cloned();
                            let rhs_exp = add_e_typ(rw, ltyp.as_ref(), rhs(*value));
                            return rebuild(
                                &like,
                                ExpKind::Let { pat: new_pat, bind: Box::new(rhs_exp), body },
                                typ_clone(&like),
                            );
                        }
                    }
                    let _ = bind_like;
                }
                plet_or_let(rw, like, pat, *bind, body)
            }
            ExpKind::Var { lexp, init, body } => {
                let (pat, ok) = match &lexp.kind {
                    LExpKind::Id(n) => (mkp(PatKind::Id(n.clone()), typ_of(&*lexp).cloned()), true),
                    LExpKind::Typ { typ, id } => {
                        let p = mkp(PatKind::Id(id.clone()), typ_of(&*lexp).cloned());
                        (add_p_typ(rw, Some(typ), p), true)
                    }
                    _ => (mk_pat(PatKind::Wild), false),
                };
                if !ok {
                    return Exp { kind: ExpKind::Var { lexp, init, body }, ..like };
                }
                plet_or_let(rw, like, pat, *init, body)
            }
            kind => Exp { kind, ..like },
        }
    })
}

fn plet_or_let(rw: &mut Rw<'_>, like: Exp, pat: Pat, bind: Exp, body: Box<Exp>) -> Exp {
    let t = typ_clone(&like);
    let monadic = effectful(&bind, &rw.it, rw.oracle, true) || has_early_return(&bind, &rw.it);
    if monadic {
        rebuild(&like, ExpKind::InternalPlet { pat, bind: Box::new(bind), body }, t)
    } else {
        rebuild(&like, ExpKind::Let { pat, bind: Box::new(bind), body }, t)
    }
}

// ---------------------------------------------------------------------------
// remove_superfluous_letbinds / returns
// ---------------------------------------------------------------------------

fn untyp_pat(p: &Pat) -> (&Pat, Option<&Typ>) {
    match &p.kind {
        PatKind::Typ { pat, typ } => (pat, Some(typ)),
        _ => (p, None),
    }
}

fn uncast_exp(e: &Exp) -> (&Exp, Option<&Typ>) {
    match &e.kind {
        ExpKind::Typ { exp, typ } => (exp, Some(typ)),
        _ => (e, None),
    }
}

fn small(e: &Exp) -> bool {
    match &e.kind {
        ExpKind::Id(_) | ExpKind::Lit(_) | ExpKind::Sizeof(_) => true,
        ExpKind::Typ { exp, .. } => small(exp),
        ExpKind::List(es) => es.iter().all(small),
        ExpKind::Cons(a, b) => small(a) && small(b),
        _ => false,
    }
}

fn same_name(rw: &Rw<'_>, a: &Name, b: &Name) -> bool {
    rw.text(a) == rw.text(b)
}

pub fn remove_superfluous_letbinds(rw: &mut Rw<'_>, e: Exp) -> Exp {
    fold_exp(e, &mut |x| {
        let (pat, bind, body) = match &x.kind {
            ExpKind::Let { pat, bind, body } | ExpKind::InternalPlet { pat, bind, body } => (pat, bind, body),
            _ => return x,
        };
        let (p, _) = untyp_pat(pat);
        let (e1, _) = uncast_exp(bind);
        let (e2, _) = uncast_exp(body);
        // `let x = E in x` => E
        if let (PatKind::Id(id), ExpKind::Id(id2)) = (&p.kind, &e2.kind) {
            if same_name(rw, id, id2) {
                return (**bind).clone();
            }
        }
        // `let _ = () in e` => e
        if matches!(p.kind, PatKind::Wild) && matches!(&e1.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit)) {
            return (**body).clone();
        }
        // `let x = E in return x` => `return E` for small pure E
        if let (PatKind::Id(id), ExpKind::InternalReturn(inner)) = (&p.kind, &e2.kind) {
            if let ExpKind::Id(id2) = &inner.kind {
                if same_name(rw, id, id2) && small(bind) && !effectful(bind, &rw.it, rw.oracle, true) {
                    let t = typ_clone(bind);
                    return rebuild(bind, ExpKind::InternalReturn(bind.clone()), t);
                }
            }
        }
        // `let _ = throw e in ...` => throw e
        if let ExpKind::Throw(_) = &e1.kind {
            return e1.clone();
        }
        x
    })
}

pub fn remove_superfluous_returns(rw: &mut Rw<'_>, e: Exp) -> Exp {
    fold_exp(e, &mut |x| {
        let (pat, bind, body) = match &x.kind {
            ExpKind::Let { pat, bind, body } | ExpKind::InternalPlet { pat, bind, body } => (pat, bind, body),
            _ => return x,
        };
        if !(effectful(bind, &rw.it, rw.oracle, true) || has_early_return(bind, &rw.it)) {
            return x;
        }
        let (p, ptyp) = untyp_pat(pat);
        let (b, etyp) = uncast_exp(body);
        let matched = match (&p.kind, &b.kind) {
            (PatKind::Lit(l), ExpKind::InternalReturn(r)) => {
                matches!(&r.kind, ExpKind::Lit(l2) if l2.kind == l.kind)
            }
            (PatKind::Wild, ExpKind::InternalReturn(r)) => {
                matches!(&r.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit))
                    && typ_of(bind).is_some_and(|t| is_unit_typ(rw, t))
            }
            (PatKind::Wild, ExpKind::Lit(l)) => {
                matches!(l.kind, LitKind::Unit) && typ_of(bind).is_some_and(|t| is_unit_typ(rw, t))
            }
            (PatKind::Id(id), ExpKind::InternalReturn(r)) => {
                matches!(&r.kind, ExpKind::Id(id2) if same_name(rw, id, id2))
            }
            (PatKind::Tuple(ps), ExpKind::InternalReturn(r)) => match &r.kind {
                ExpKind::Tuple(es) if ps.len() == es.len() => ps.iter().zip(es).all(|(p, e)| {
                    let (p, _) = untyp_pat(p);
                    let (e, _) = uncast_exp(e);
                    matches!((&p.kind, &e.kind), (PatKind::Id(a), ExpKind::Id(b)) if same_name(rw, a, b))
                }),
                _ => false,
            },
            _ => false,
        };
        if matched {
            let typ = ptyp.or(etyp).cloned();
            add_e_typ(rw, typ.as_ref(), (**bind).clone())
        } else {
            x
        }
    })
}

// ---------------------------------------------------------------------------
// remove_e_assign: rewrite_var_updates
// ---------------------------------------------------------------------------

type Vars = Vec<(String, Name, Option<Typ>)>;

fn collect_intros(rw: &Rw<'_>, e: &Exp, out: &mut Vec<String>) {
    crate::analysis::exp_exists(e, &mut |x| {
        match &x.kind {
            ExpKind::Var { lexp, .. } => {
                let mut ids = Vec::new();
                lexp_ids(lexp, &mut ids);
                out.extend(ids.iter().map(|n| rw.text(n)));
            }
            ExpKind::Assign { lexp, .. } if is_intro(x) => {
                let mut ids = Vec::new();
                lexp_ids(lexp, &mut ids);
                out.extend(ids.iter().map(|n| rw.text(n)));
            }
            _ => {}
        }
        false
    });
}

fn lexp_root_types(rw: &Rw<'_>, l: &LExp, found: &mut Vec<(String, Name, Option<Typ>)>) {
    match &l.kind {
        LExpKind::Id(n) | LExpKind::Typ { id: n, .. } => {
            let text = rw.text(n);
            if !rw.oracle.is_register(&text) {
                found.push((text, n.clone(), typ_of(l).cloned()));
            }
        }
        LExpKind::Deref(_) => {}
        LExpKind::Tuple(xs) | LExpKind::VectorConcat(xs) => xs.iter().for_each(|x| lexp_root_types(rw, x, found)),
        LExpKind::Vector { lexp, .. } | LExpKind::VectorRange { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_root_types(rw, lexp, found);
        }
    }
}

/// `find_updated_vars`: local variables assigned (not introduced) in `e`.
fn find_updated_vars(rw: &Rw<'_>, e: &Exp) -> BTreeMap<String, (Name, Option<Typ>)> {
    let mut intros = Vec::new();
    collect_intros(rw, e, &mut intros);
    let mut out: BTreeMap<String, (Name, Option<Typ>)> = BTreeMap::new();
    crate::analysis::exp_exists(e, &mut |x| {
        if let ExpKind::Assign { lexp, .. } = &x.kind {
            let mut found = Vec::new();
            lexp_root_types(rw, lexp, &mut found);
            for (text, name, typ) in found {
                if !intros.contains(&text) {
                    let entry = out.entry(text).or_insert((name, None));
                    if entry.1.is_none() {
                        entry.1 = typ;
                    }
                }
            }
        }
        false
    });
    out
}

fn find_used_vars(rw: &Rw<'_>, e: &Exp) -> Vec<String> {
    let mut out = Vec::new();
    crate::analysis::exp_exists(e, &mut |x| {
        if let ExpKind::Id(n) = &x.kind {
            out.push(rw.text(n));
        }
        false
    });
    out
}

fn updates_vars(rw: &Rw<'_>, e: &Exp) -> bool {
    crate::analysis::exp_exists(e, &mut |x| match &x.kind {
        ExpKind::Assign { lexp, .. } => lexp_is_local(rw, lexp),
        _ => false,
    })
}

enum Updated {
    Added(Exp, Pat),
    Same(Exp),
}

pub struct VarUpdates<'a, 'r> {
    pub rw: &'a mut Rw<'r>,
}

fn is_wild_pat(pat: &Pat) -> bool {
    matches!(pat.kind, PatKind::Wild) || matches!(&pat.kind, PatKind::Typ { pat: p, .. } if matches!(p.kind, PatKind::Wild))
}

impl VarUpdates<'_, '_> {
    fn tuple_exp(&mut self, mut es: Vec<Exp>, like: &Exp) -> Exp {
        match es.len() {
            0 => unit_lit(self.rw),
            1 => es.pop().expect("one"),
            _ => {
                let t = tuple_typ(es.iter().map(typ_clone).collect());
                rebuild(like, ExpKind::Tuple(es), t)
            }
        }
    }

    fn tuple_pat(&mut self, mut ps: Vec<Pat>) -> Pat {
        match ps.len() {
            0 => {
                let u = self.rw.unit_typ();
                mkp(PatKind::Wild, Some(u))
            }
            1 => {
                let p = ps.pop().expect("one");
                let t = typ_of(&p).cloned();
                add_p_typ(self.rw, t.as_ref(), p)
            }
            _ => {
                let t = tuple_typ(ps.iter().map(|p| typ_of(p).cloned()).collect());
                let p = mkp(PatKind::Tuple(ps), t.clone());
                add_p_typ(self.rw, t.as_ref(), p)
            }
        }
    }

    fn var_exps_pats(&mut self, vars: &Vars) -> (Vec<Exp>, Vec<Pat>) {
        let mut es = Vec::new();
        let mut ps = Vec::new();
        for (_, name, typ) in vars {
            es.push(mk(ExpKind::Id(name.clone()), typ.clone()));
            ps.push(mkp(PatKind::Id(name.clone()), typ.clone()));
        }
        (es, ps)
    }

    fn vars_for(&self, updated: BTreeMap<String, (Name, Option<Typ>)>, used: &[String]) -> Vars {
        updated.into_iter().filter(|(k, _)| used.contains(k)).map(|(k, (n, t))| (k, n, t)).collect()
    }

    /// `add_vars`: make the result of `e` also return `vars`.
    fn add_vars(&mut self, overwrite: bool, e: Exp, vars: &Vars) -> Exp {
        let like = shell(&e);
        match e.kind {
            ExpKind::Let { pat, bind, body } => {
                let body = self.add_vars(overwrite, *body, vars);
                let t = typ_clone(&body);
                rebuild(&like, ExpKind::Let { pat, bind, body: Box::new(body) }, t)
            }
            ExpKind::Var { lexp, init, body } => {
                let body = self.add_vars(overwrite, *body, vars);
                let t = typ_clone(&body);
                rebuild(&like, ExpKind::Var { lexp, init, body: Box::new(body) }, t)
            }
            ExpKind::InternalPlet { pat, bind, body } => {
                let body = self.add_vars(overwrite, *body, vars);
                let t = typ_clone(&body);
                rebuild(&like, ExpKind::InternalPlet { pat, bind, body: Box::new(body) }, t)
            }
            ExpKind::InternalReturn(inner) => {
                let inner = self.add_vars(overwrite, *inner, vars);
                let t = typ_clone(&inner);
                rebuild(&like, ExpKind::InternalReturn(Box::new(inner)), t)
            }
            ExpKind::Typ { exp, .. } => {
                let inner = self.add_vars(overwrite, *exp, vars);
                let t = typ_clone(&inner);
                add_e_typ(self.rw, t.as_ref(), inner)
            }
            ExpKind::App { ref func, .. } if self.rw.text(func) == "early_return" => Exp { kind: e.kind, ..like },
            ExpKind::InternalAssume { constraint, exp } => {
                let inner = self.add_vars(overwrite, *exp, vars);
                let t = typ_clone(&inner);
                rebuild(&like, ExpKind::InternalAssume { constraint, exp: Box::new(inner) }, t)
            }
            kind => {
                let exp = Exp { kind, ..like.clone() };
                let (var_es, _) = self.var_exps_pats(vars);
                if overwrite {
                    let tup = self.tuple_exp(var_es, &exp);
                    let t = typ_clone(&tup);
                    let wild = mkp(PatKind::Wild, typ_clone(&exp));
                    let bind_t = typ_clone(&exp);
                    let e2 = rebuild(&like, ExpKind::Let { pat: wild, bind: Box::new(exp), body: Box::new(tup) }, t);
                    add_typs_let(self.rw, bind_t.as_ref(), e2)
                } else {
                    let mut es = vec![exp];
                    es.extend(var_es);
                    self.tuple_exp(es, &like)
                }
            }
        }
    }

    pub fn rewrite(&mut self, exp: Exp) -> Result<Exp, String> {
        let like = shell(&exp);
        let typ = typ_clone(&exp);
        match exp.kind {
            ExpKind::Let { pat, bind, body } => {
                let body = self.rewrite(*body)?;
                let used = find_used_vars(self.rw, &body);
                let (pat, bind) = match self.rewrite_bound(&used, *bind, pat.clone())? {
                    Updated::Added(v, p) => (p, v),
                    Updated::Same(v) => (pat, v),
                };
                let t = typ_clone(&body);
                Ok(rebuild(&like, ExpKind::Let { pat, bind: Box::new(bind), body: Box::new(body) }, t.or(typ)))
            }
            ExpKind::Var { lexp, init, body } => {
                let pat = self.var_pat(&lexp, &init)?;
                let t = typ_clone(&body);
                let e = rebuild(&like, ExpKind::Let { pat, bind: init, body }, t);
                self.rewrite(e)
            }
            kind @ (ExpKind::For { .. } | ExpKind::Loop { .. } | ExpKind::Assign { .. }) => {
                let unit = unit_lit(self.rw);
                let inner = Exp { kind, ..like.clone() };
                let wild = mkp(PatKind::Wild, typ_clone(&inner));
                let e = rebuild(&like, ExpKind::Let { pat: wild, bind: Box::new(inner), body: Box::new(unit) }, typ);
                self.rewrite(e)
            }
            kind @ (ExpKind::If { .. } | ExpKind::Match { .. } | ExpKind::Try { .. }) => {
                let name = self.rw.fresh("u__");
                let inner = Exp { kind, ..like.clone() };
                let pat = mkp(PatKind::Id(name.clone()), typ.clone());
                let var = mk(ExpKind::Id(name), typ.clone());
                let e = rebuild(&like, ExpKind::Let { pat, bind: Box::new(inner), body: Box::new(var) }, typ);
                self.rewrite(e)
            }
            ExpKind::InternalPlet { .. } => Err("rewrite_var_updates: E_internal_plet shouldn't be introduced yet".into()),
            ExpKind::Typ { typ: t, exp: inner } => {
                let inner = self.rewrite(*inner)?;
                Ok(rebuild(&like, ExpKind::Typ { typ: t, exp: Box::new(inner) }, typ))
            }
            ExpKind::InternalAssume { constraint, exp: inner } => {
                let inner = self.rewrite(*inner)?;
                Ok(rebuild(&like, ExpKind::InternalAssume { constraint, exp: Box::new(inner) }, typ))
            }
            ExpKind::Tuple(es) => {
                let whole = Exp { kind: ExpKind::Tuple(es.clone()), ..like.clone() };
                if find_updated_vars(self.rw, &whole).is_empty() {
                    return Ok(whole);
                }
                let typs: Vec<Option<Typ>> = match typ.as_ref().map(|t| &t.kind) {
                    Some(TypKind::Tuple(ts)) if ts.len() == es.len() => ts.iter().cloned().map(Some).collect(),
                    _ => es.iter().map(typ_clone).collect(),
                };
                let bindings: Vec<(Name, Option<Typ>, Exp)> =
                    typs.into_iter().zip(es).map(|(t, e)| (self.rw.fresh("t__"), t, e)).collect();
                let trivial: Vec<Exp> = bindings.iter().map(|(id, t, e)| self.trivial_of(id, t, e)).collect();
                let tup = rebuild(&like, ExpKind::Tuple(trivial), typ);
                let folded = self.bind_all(bindings, tup, &like);
                self.rewrite(folded)
            }
            ExpKind::App { func, args } => {
                let whole = Exp { kind: ExpKind::App { func: func.clone(), args: args.clone() }, ..like.clone() };
                if find_updated_vars(self.rw, &whole).is_empty() {
                    return Ok(whole);
                }
                let bindings: Vec<(Name, Option<Typ>, Exp)> =
                    args.into_iter().map(|a| (self.rw.fresh("a__"), typ_clone(&a), a)).collect();
                let trivial: Vec<Exp> = bindings.iter().map(|(id, t, e)| self.trivial_of(id, t, e)).collect();
                let app = rebuild(&like, ExpKind::App { func, args: trivial }, typ);
                let folded = self.bind_all(bindings, app, &like);
                self.rewrite(folded)
            }
            kind => Ok(Exp { kind, ..like }),
        }
    }

    fn trivial_of(&mut self, id: &Name, t: &Option<Typ>, e: &Exp) -> Exp {
        if matches!(e.kind, ExpKind::Id(_) | ExpKind::Lit(_)) {
            e.clone()
        } else if t.as_ref().is_some_and(|t| is_unit_typ(self.rw, t)) {
            unit_lit(self.rw)
        } else {
            mk(ExpKind::Id(id.clone()), t.clone())
        }
    }

    fn bind_all(&mut self, bindings: Vec<(Name, Option<Typ>, Exp)>, inner: Exp, like: &Exp) -> Exp {
        let mut acc = inner;
        for (id, t, e) in bindings.into_iter().rev() {
            if matches!(e.kind, ExpKind::Id(_) | ExpKind::Lit(_)) {
                continue;
            }
            let pat = if t.as_ref().is_some_and(|t| is_unit_typ(self.rw, t)) {
                mkp(PatKind::Wild, t.clone())
            } else {
                let p = mkp(PatKind::Id(id), t.clone());
                add_p_typ(self.rw, t.as_ref(), p)
            };
            let rt = typ_clone(&acc);
            acc = rebuild(like, ExpKind::Let { pat, bind: Box::new(e), body: Box::new(acc) }, rt);
        }
        acc
    }

    fn var_pat(&mut self, lexp: &LExp, init: &Exp) -> Result<Pat, String> {
        match &lexp.kind {
            LExpKind::Id(n) => Ok(mkp(PatKind::Id(n.clone()), typ_clone(init))),
            LExpKind::Typ { typ, id } => {
                let p = mkp(PatKind::Id(id.clone()), typ_clone(init));
                Ok(add_p_typ(self.rw, Some(typ), p))
            }
            LExpKind::Tuple(xs) => {
                let mut pats = Vec::new();
                for x in xs {
                    pats.push(self.var_pat(x, init)?);
                }
                let t = tuple_typ(pats.iter().map(|p| typ_of(p).cloned()).collect());
                Ok(mkp(PatKind::Tuple(pats), t))
            }
            _ => Err("E_var with a lexp that is not a variable".into()),
        }
    }

    fn with_extra(&mut self, overwrite: bool, pat: Pat, var_pats: Vec<Pat>) -> Pat {
        let mut pats = Vec::new();
        if !overwrite {
            pats.push(pat);
        }
        pats.extend(var_pats);
        self.tuple_pat(pats)
    }

    fn rewrite_bound(&mut self, used: &[String], full: Exp, pat: Pat) -> Result<Updated, String> {
        let like = shell(&full);
        let typ = typ_clone(&full);
        let overwrite = is_wild_pat(&pat);
        match full.kind {
            ExpKind::For { var, from, to, step, order, body } => {
                self.foreach(used, &like, overwrite, pat, var, *from, *to, *step, order, *body)
            }
            ExpKind::Loop { loop_kind, measure, cond, body } => {
                let whole = Exp {
                    kind: ExpKind::Loop { loop_kind, measure: measure.clone(), cond: cond.clone(), body: body.clone() },
                    ..like.clone()
                };
                let mut uses = used.to_vec();
                uses.extend(find_used_vars(self.rw, &whole));
                let vars = self.vars_for(find_updated_vars(self.rw, &body), &uses);
                let (var_es, var_pats) = self.var_exps_pats(&vars);
                let body = self.add_vars(overwrite, *body, &vars);
                let body = self.rewrite(body)?;
                let bt = typ_clone(&body);
                let body = add_e_typ(self.rw, bt.as_ref(), body);
                let (fname, measure_arg): (&str, Vec<Exp>) = match (loop_kind, measure) {
                    (Loop::While, None) => ("while#", Vec::new()),
                    (Loop::Until, None) => ("until#", Vec::new()),
                    (Loop::While, Some(m)) => ("while#t", vec![*m]),
                    (Loop::Until, Some(m)) => ("until#t", vec![*m]),
                };
                let func = self.rw.name(fname);
                let tuple = self.tuple_exp(var_es, &like);
                let body_t = typ_clone(&body);
                let mut args = vec![*cond, tuple, body];
                args.extend(measure_arg);
                let v = rebuild(&like, ExpKind::App { func, args }, body_t);
                Ok(Updated::Added(v, self.with_extra(overwrite, pat, var_pats)))
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                let mut both = find_updated_vars(self.rw, &then_branch);
                for (k, v) in find_updated_vars(self.rw, &else_branch) {
                    both.entry(k).or_insert(v);
                }
                let vars = self.vars_for(both, used);
                if vars.is_empty() {
                    let t = self.rewrite(*then_branch)?;
                    let e = self.rewrite(*else_branch)?;
                    return Ok(Updated::Same(rebuild(
                        &like,
                        ExpKind::If { cond, then_branch: Box::new(t), else_branch: Box::new(e) },
                        typ,
                    )));
                }
                let (_, var_pats) = self.var_exps_pats(&vars);
                let t1 = self.add_vars(overwrite, *then_branch, &vars);
                let t1 = self.rewrite(t1)?;
                let e1 = self.add_vars(overwrite, *else_branch, &vars);
                let e1 = self.rewrite(e1)?;
                let rt = typ_clone(&t1);
                let v = rebuild(&like, ExpKind::If { cond, then_branch: Box::new(t1), else_branch: Box::new(e1) }, rt);
                Ok(Updated::Added(v, self.with_extra(overwrite, pat, var_pats)))
            }
            ExpKind::Match { scrutinee, arms } => self.arms(used, &like, typ, overwrite, pat, *scrutinee, arms, true),
            ExpKind::Try { exp, arms } => self.arms(used, &like, typ, overwrite, pat, *exp, arms, false),
            ExpKind::Assign { lexp, value } => {
                let whole = Exp { kind: ExpKind::Assign { lexp: lexp.clone(), value: value.clone() }, ..like.clone() };
                if effectful(&whole, &self.rw.it, self.rw.oracle, true) {
                    return Ok(Updated::Same(whole));
                }
                match &lexp.kind {
                    LExpKind::Id(id) => {
                        let t = typ_of(&*lexp).cloned();
                        let p = mkp(PatKind::Id(id.clone()), t.clone());
                        let p = add_p_typ(self.rw, t.as_ref(), p);
                        Ok(Updated::Added(*value, p))
                    }
                    LExpKind::Typ { typ, id } => {
                        let p = mkp(PatKind::Id(id.clone()), typ_clone(&value));
                        let p = add_p_typ(self.rw, Some(typ), p);
                        Ok(Updated::Added(*value, p))
                    }
                    LExpKind::Vector { lexp: b, .. } | LExpKind::VectorRange { lexp: b, .. }
                        if matches!(b.kind, LExpKind::Id(_)) =>
                    {
                        let LExpKind::Id(id) = &b.kind else { unreachable!() };
                        let id = id.clone();
                        let (_, rhs) = lexp_to_rhs(self.rw, *lexp)?;
                        let newv = rhs(*value);
                        let p = mkp(PatKind::Id(id), typ_clone(&newv));
                        Ok(Updated::Added(newv, p))
                    }
                    _ => Ok(Updated::Same(whole)),
                }
            }
            ExpKind::Typ { typ: t, exp: inner } => match self.rewrite_bound(used, *inner, pat)? {
                Updated::Added(e2, p2) => {
                    let et = typ_clone(&e2);
                    Ok(Updated::Added(add_e_typ(self.rw, et.as_ref(), e2), p2))
                }
                Updated::Same(e2) => Ok(Updated::Same(rebuild(&like, ExpKind::Typ { typ: t, exp: Box::new(e2) }, typ))),
            },
            kind => {
                let e = Exp { kind, ..like };
                if updates_vars(self.rw, &e) {
                    Ok(Updated::Same(self.rewrite(e)?))
                } else {
                    Ok(Updated::Same(e))
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn arms(
        &mut self,
        used: &[String],
        like: &Exp,
        typ: Option<Typ>,
        overwrite: bool,
        pat: Pat,
        first: Exp,
        arms: Vec<PExp>,
        is_case: bool,
    ) -> Result<Updated, String> {
        let mut sets = BTreeMap::new();
        if !is_case {
            for (k, v) in find_updated_vars(self.rw, &first) {
                sets.entry(k).or_insert(v);
            }
        }
        for a in &arms {
            let (PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. }) = &a.kind;
            for (k, v) in find_updated_vars(self.rw, body) {
                sets.entry(k).or_insert(v);
            }
        }
        let vars = self.vars_for(sets, used);
        let first = if is_case {
            first
        } else {
            let f = self.add_vars(overwrite, first, &vars);
            self.rewrite(f)?
        };
        let (_, var_pats) = self.var_exps_pats(&vars);
        let mut new_arms = Vec::new();
        for a in arms {
            let kind = match a.kind {
                PExpKind::PatExp { pat: p, body } => {
                    let b = if vars.is_empty() { body } else { self.add_vars(overwrite, body, &vars) };
                    PExpKind::PatExp { pat: p, body: self.rewrite(b)? }
                }
                PExpKind::PatWhen { pat: p, guard, body } => {
                    if !vars.is_empty() {
                        return Err("Guarded patterns should have been rewritten already".into());
                    }
                    PExpKind::PatWhen { pat: p, guard, body: self.rewrite(body)? }
                }
            };
            new_arms.push(PExp { kind, ..a });
        }
        let new_typ = match new_arms.first().map(|a| &a.kind) {
            Some(PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. }) if !vars.is_empty() => typ_clone(body),
            _ => typ,
        };
        let kind = if is_case {
            ExpKind::Match { scrutinee: Box::new(first), arms: new_arms }
        } else {
            ExpKind::Try { exp: Box::new(first), arms: new_arms }
        };
        let e = rebuild(like, kind, new_typ);
        if vars.is_empty() {
            return Ok(Updated::Same(e));
        }
        Ok(Updated::Added(e, self.with_extra(overwrite, pat, var_pats)))
    }

    #[allow(clippy::too_many_arguments)]
    fn foreach(
        &mut self,
        used: &[String],
        like: &Exp,
        overwrite: bool,
        pat: Pat,
        var: Name,
        from: Exp,
        to: Exp,
        step: Exp,
        order: Order,
        body: Exp,
    ) -> Result<Updated, String> {
        let whole = Exp {
            kind: ExpKind::For {
                var: var.clone(),
                from: Box::new(from.clone()),
                to: Box::new(to.clone()),
                step: Box::new(step.clone()),
                order,
                body: Box::new(body.clone()),
            },
            ..like.clone()
        };
        let mut uses = used.to_vec();
        uses.extend(find_used_vars(self.rw, &whole));
        let vars = self.vars_for(find_updated_vars(self.rw, &body), &uses);
        let (var_es, var_pats) = self.var_exps_pats(&vars);
        let body_hint = typ_clone(&body);
        let body = self.add_vars(overwrite, body, &vars);
        let body = self.rewrite(body)?;
        let body_t = typ_clone(&body).or(body_hint);

        let var_text = self.rw.text(&var);
        let lower_text = format!("loop_{var_text}_lower");
        let upper_text = format!("loop_{var_text}_upper");
        let lower_id = self.rw.name(&lower_text);
        let upper_id = self.rw.name(&upper_text);
        let lvar_sym = self.rw.it.intern(&format!("loop_{var_text}"));
        let lower_sym = self.rw.it.intern(&lower_text);
        let upper_sym = self.rw.it.intern(&upper_text);

        let atom = |rw: &mut Rw<'_>, sym: sail_ast::Symbol| -> Typ {
            let name = rw.name("atom");
            Node::dummy(TypKind::App {
                name,
                args: vec![TypArg::dummy(TypArgKind::NExp(Node::dummy(sail_ast::NExpKind::Var(sym))))],
            })
        };
        let lower_typ = atom(self.rw, lower_sym);
        let upper_typ = atom(self.rw, upper_sym);
        let lvar_atom = atom(self.rw, lvar_sym);
        let bool_t = self.rw.bool_typ();

        let lower_id_exp = mk(ExpKind::Id(lower_id.clone()), Some(lower_typ));
        let upper_id_exp = mk(ExpKind::Id(upper_id.clone()), Some(upper_typ.clone()));
        let (ord_value, lower_exp, upper_exp, exp1, exp2) = if order == Order::Inc {
            (true, from, to, lower_id_exp, upper_id_exp)
        } else {
            (false, to, from, upper_id_exp, lower_id_exp)
        };
        let ord_exp = mk(
            ExpKind::Lit(Lit::dummy(if ord_value { LitKind::True } else { LitKind::False })),
            Some(bool_t.clone()),
        );

        let var_nexp = |s| Node::dummy(sail_ast::NExpKind::Var(s));
        let lvar_nc = Node::dummy(sail_ast::ConstraintKind::And(
            Box::new(Node::dummy(sail_ast::ConstraintKind::Le(var_nexp(lower_sym), var_nexp(lvar_sym)))),
            Box::new(Node::dummy(sail_ast::ConstraintKind::Le(var_nexp(lvar_sym), var_nexp(upper_sym)))),
        ));
        let lvar_typ = Node::dummy(TypKind::Exist {
            binders: vec![KOpt { kind: Kind::Int, kid: lvar_sym }],
            constraint: Box::new(lvar_nc),
            typ: Box::new(lvar_atom.clone()),
        });
        let lvar_pat = mkp(
            PatKind::Var {
                pat: Box::new(mkp(PatKind::Id(var), Some(lvar_atom))),
                typ_pat: Node::dummy(TypPatKind::Var(lvar_sym)),
            },
            Some(lvar_typ.clone()),
        );
        let let_body = add_typs_let(
            self.rw,
            Some(&lvar_typ),
            mk(ExpKind::Let { pat: lvar_pat, bind: Box::new(exp1.clone()), body: Box::new(body) }, body_t.clone()),
        );

        let guard_nc = Node::dummy(sail_ast::ConstraintKind::Le(var_nexp(lower_sym), var_nexp(upper_sym)));
        let guard = mk(ExpKind::Constraint(guard_nc), Some(bool_t));
        let unit = unit_lit(self.rw);
        let skip_exps = if overwrite {
            var_es.clone()
        } else {
            let mut v = vec![unit];
            v.extend(var_es.clone());
            v
        };
        let skip_val = self.tuple_exp(skip_exps, like);
        let guarded = mk(
            ExpKind::If { cond: Box::new(guard), then_branch: Box::new(let_body), else_branch: Box::new(skip_val) },
            body_t.clone(),
        );
        let tuple = self.tuple_exp(var_es, like);
        let foreach_name = self.rw.name("foreach#");
        let app = mk(
            ExpKind::App { func: foreach_name, args: vec![exp1, exp2, step, ord_exp, tuple, guarded] },
            body_t.clone(),
        );

        let atom_pat = |rw: &mut Rw<'_>, id: Name, sym: sail_ast::Symbol, typ: Option<Typ>| -> Pat {
            let name = rw.name("atom");
            mkp(
                PatKind::Var {
                    pat: Box::new(mkp(PatKind::Id(id), typ.clone())),
                    typ_pat: Node::dummy(TypPatKind::App { name, args: vec![Node::dummy(TypPatKind::Var(sym))] }),
                },
                typ,
            )
        };
        let upper_pat = atom_pat(self.rw, upper_id, upper_sym, typ_clone(&upper_exp));
        let lower_pat = atom_pat(self.rw, lower_id, lower_sym, typ_clone(&lower_exp));
        let inner = rebuild(
            like,
            ExpKind::Let { pat: upper_pat, bind: Box::new(upper_exp), body: Box::new(app) },
            body_t.clone(),
        );
        let v = rebuild(like, ExpKind::Let { pat: lower_pat, bind: Box::new(lower_exp), body: Box::new(inner) }, body_t);
        Ok(Updated::Added(v, self.with_extra(overwrite, pat, var_pats)))
    }
}

/// `remove_e_assign` on one expression.
pub fn remove_e_assign(rw: &mut Rw<'_>, e: Exp) -> Result<Exp, String> {
    VarUpdates { rw }.rewrite(e)
}

// ---------------------------------------------------------------------------
// bit_lists_to_lits
// ---------------------------------------------------------------------------

fn bit_of_exp(rw: &Rw<'_>, e: &Exp) -> Option<bool> {
    match &e.kind {
        ExpKind::Lit(l) => match l.kind {
            LitKind::Bin(s) => match sail_ast::Interner::resolve(&rw.it, s)? {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// `rewrite_bit_lists_to_lits`: `[bit, ...]` of literal bits becomes one
/// bitvector literal.
pub fn bit_lists_to_lits(rw: &mut Rw<'_>, e: Exp) -> Exp {
    fold_exp(e, &mut |x| {
        let is_bits = typ_of(&x).and_then(|t| rw.oracle.typ_head(t, &rw.it)).is_some_and(|h| h == "bits" || h == "bitvector");
        match &x.kind {
            ExpKind::Vector(es) if is_bits => {
                let bits: Option<Vec<bool>> = es.iter().map(|e| bit_of_exp(rw, e)).collect();
                let Some(bits) = bits else { return x };
                let digits: String = bits.iter().map(|b| if *b { '1' } else { '0' }).collect();
                let kind = if !bits.is_empty() && bits.len() % 4 == 0 {
                    let hex: String = bits
                        .chunks(4)
                        .map(|c| {
                            let v = c.iter().fold(0u32, |a, b| a * 2 + u32::from(*b));
                            std::char::from_digit(v, 16).expect("nibble").to_ascii_uppercase()
                        })
                        .collect();
                    LitKind::Hex(rw.it.intern(&hex))
                } else {
                    LitKind::Bin(rw.it.intern(&digits))
                };
                let typ = typ_clone(&x);
                let lit = mk(ExpKind::Lit(Lit::dummy(kind)), typ.clone());
                let Some(t) = typ else { return x };
                rebuild(&x, ExpKind::Typ { typ: t, exp: Box::new(lit) }, typ_clone(&x))
            }
            ExpKind::Typ { typ, exp } => match &exp.kind {
                ExpKind::Typ { exp: inner, .. } => {
                    rebuild(&x, ExpKind::Typ { typ: typ.clone(), exp: inner.clone() }, typ_clone(&x))
                }
                _ => x,
            },
            _ => x,
        }
    })
}
