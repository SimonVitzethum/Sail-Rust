#![forbid(unsafe_code)]
//! Bridges between the local minimal types and the canonical `sail-ast`
//! nodes (Lane 61 unification, MIGRATION.md §6 steps 2–3).
//!
//! Mapping (MIGRATION.md #17/#18/#22):
//! - `Nexp::{Const,Var,Add,Sub,Mul,Neg,Exp,If}` map 1:1 onto
//!   `NExpKind::{Constant,Var,Sum,Minus,Times,Neg,Exp,If}` (`Const` is
//!   `i128` here vs decimal `String` there; `Var` is `String` here vs
//!   interned `Symbol` there).
//! - `Nexp::{Div,Mod,Abs}` have no dedicated `NExpKind` variant and
//!   become `NExp_app` nodes (`div`/`mod`/`abs`); `NExpKind::{Id,App}`
//!   with any other operator maps back to `None`.
//! - `NConstraint::{True,False,Ge,Gt,Le,Lt,And,Or,Not}` map 1:1;
//!   `Eq/Neq` wrap both sides in `Typ_arg` (`ConstraintKind::Equal`
//!   compares type arguments, not raw nexps).
//! - `NConstraint::{Set,App,Id,Var}` (Lane 107, Sail `NC_set`/`NC_app`/
//!   `NC_id`/`NC_var`) map onto `ConstraintKind::{Set,App,Id,Var}`;
//!   only `A_nexp` arguments are modelled — `App` with non-`NExp`
//!   `Typ_arg`s and unparseable `Set` members lift back to `None`
//!   (residual `X-NcSetApp`).
//! - `Kind::{KInt,KBool,KType}` erase onto `sail_ast::Kind::{Int,Bool,Type}`;
//!   `KNat` (Lean-`Nat` refinement, Lane 04/10) erases to `Int` and is
//!   NEVER silently recovered: [`kind_from_ast`] yields `KInt`.
//!
//! All synthesized nodes carry `NodeId::DUMMY`/`Span::DUMMY`; names are
//! interned through the caller's interner. Behaviour of the local types
//! (folding, kind checking, solving) is untouched.

use sail_ast::{
    Constraint, ConstraintKind, Exp, ExpKind, Interner, Kind as AstKind, KOpt, Lit, LitKind,
    Name, NExp, NExpKind, Node, Pat, PatKind, QuantItem, Typ, TypArg, TypArgKind, TypKind,
    TypSchm,
};

use crate::kind::Kind;
use crate::nexp::{MiniExp, MiniTyp, NConstraint, Nexp, TypQuant};

fn mk_nexp(kind: NExpKind) -> NExp {
    Node::dummy(kind)
}

fn mk_constraint(kind: ConstraintKind) -> Constraint {
    Node::dummy(kind)
}

fn mk_nexp_arg(n: NExp) -> TypArg {
    Node::dummy(TypArgKind::NExp(n))
}

fn op_name(it: &mut impl Interner, op: &str) -> Name {
    Name::dummy(it.intern(op))
}

/// Erase a local kind onto the canonical `sail-ast` kind.
/// `KNat` erases to `Int` (documented, never silent: see module docs).
#[must_use]
pub fn kind_erase(kind: Kind) -> AstKind {
    match kind {
        Kind::KInt | Kind::KNat => AstKind::Int,
        Kind::KBool => AstKind::Bool,
        Kind::KType => AstKind::Type,
    }
}

/// Recover a local kind from a canonical one. `Int` yields `KInt`
/// (a `Nat` refinement must be re-established by Lane 04/10 logic,
/// never guessed here).
#[must_use]
pub fn kind_from_ast(kind: AstKind) -> Kind {
    match kind {
        AstKind::Int => Kind::KInt,
        AstKind::Bool => Kind::KBool,
        AstKind::Type => Kind::KType,
    }
}

/// Lower a local nexp onto a canonical node.
pub fn nexp_to_ast(n: &Nexp, it: &mut impl Interner) -> NExp {
    match n {
        Nexp::Const(c) => mk_nexp(NExpKind::Constant(c.to_string())),
        Nexp::Var(v) => mk_nexp(NExpKind::Var(it.intern(v))),
        Nexp::Add(a, b) => mk_nexp(NExpKind::Sum(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        Nexp::Sub(a, b) => mk_nexp(NExpKind::Minus(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        Nexp::Mul(a, b) => mk_nexp(NExpKind::Times(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        Nexp::Neg(a) => mk_nexp(NExpKind::Neg(Box::new(nexp_to_ast(a, it)))),
        Nexp::Exp(a) => mk_nexp(NExpKind::Exp(Box::new(nexp_to_ast(a, it)))),
        Nexp::Div(a, b) => mk_nexp(NExpKind::App {
            func: op_name(it, "div"),
            args: vec![nexp_to_ast(a, it), nexp_to_ast(b, it)],
        }),
        Nexp::Mod(a, b) => mk_nexp(NExpKind::App {
            func: op_name(it, "mod"),
            args: vec![nexp_to_ast(a, it), nexp_to_ast(b, it)],
        }),
        Nexp::Abs(a) => mk_nexp(NExpKind::App {
            func: op_name(it, "abs"),
            args: vec![nexp_to_ast(a, it)],
        }),
        Nexp::If(c, a, b) => mk_nexp(NExpKind::If {
            cond: Box::new(constraint_to_ast(c, it)),
            then_branch: Box::new(nexp_to_ast(a, it)),
            else_branch: Box::new(nexp_to_ast(b, it)),
        }),
    }
}

fn app_op(func: &Name, it: &impl Interner) -> Option<String> {
    it.resolve(func.symbol).map(str::to_owned)
}

/// Lift a canonical node back onto a local nexp. `None` for shapes with
/// no local counterpart (`Id`, foreign `App` operators, `NC_set`, …).
pub fn nexp_from_ast(n: &NExp, it: &impl Interner) -> Option<Nexp> {
    match &n.kind {
        NExpKind::Constant(s) => s.parse::<i128>().ok().map(Nexp::Const),
        NExpKind::Var(sym) => it.resolve(*sym).map(|s| Nexp::Var(s.to_owned())),
        NExpKind::Sum(a, b) => Some(Nexp::Add(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Minus(a, b) => Some(Nexp::Sub(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Times(a, b) => Some(Nexp::Mul(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Neg(a) => Some(Nexp::Neg(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::Exp(a) => Some(Nexp::Exp(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::If { cond, then_branch, else_branch } => Some(Nexp::If(
            Box::new(constraint_from_ast(cond, it)?),
            Box::new(nexp_from_ast(then_branch, it)?),
            Box::new(nexp_from_ast(else_branch, it)?),
        )),
        NExpKind::App { func, args } => match (app_op(func, it).as_deref(), args.as_slice()) {
            (Some("div"), [a, b]) => Some(Nexp::Div(
                Box::new(nexp_from_ast(a, it)?),
                Box::new(nexp_from_ast(b, it)?),
            )),
            (Some("mod"), [a, b]) => Some(Nexp::Mod(
                Box::new(nexp_from_ast(a, it)?),
                Box::new(nexp_from_ast(b, it)?),
            )),
            (Some("abs"), [a]) => Some(Nexp::Abs(Box::new(nexp_from_ast(a, it)?))),
            _ => None,
        },
        NExpKind::Id(_) => None,
    }
}

/// Lower a local constraint onto a canonical node.
pub fn constraint_to_ast(c: &NConstraint, it: &mut impl Interner) -> Constraint {
    match c {
        NConstraint::True => mk_constraint(ConstraintKind::True),
        NConstraint::False => mk_constraint(ConstraintKind::False),
        NConstraint::Eq(a, b) => mk_constraint(ConstraintKind::Equal(
            Box::new(mk_nexp_arg(nexp_to_ast(a, it))),
            Box::new(mk_nexp_arg(nexp_to_ast(b, it))),
        )),
        NConstraint::Neq(a, b) => mk_constraint(ConstraintKind::NotEqual(
            Box::new(mk_nexp_arg(nexp_to_ast(a, it))),
            Box::new(mk_nexp_arg(nexp_to_ast(b, it))),
        )),
        NConstraint::Ge(a, b) => mk_constraint(ConstraintKind::Ge(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Gt(a, b) => mk_constraint(ConstraintKind::Gt(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Le(a, b) => mk_constraint(ConstraintKind::Le(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Lt(a, b) => mk_constraint(ConstraintKind::Lt(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::And(a, b) => mk_constraint(ConstraintKind::And(
            Box::new(constraint_to_ast(a, it)),
            Box::new(constraint_to_ast(b, it)),
        )),
        NConstraint::Or(a, b) => mk_constraint(ConstraintKind::Or(
            Box::new(constraint_to_ast(a, it)),
            Box::new(constraint_to_ast(b, it)),
        )),
        NConstraint::Not(a) => mk_constraint(ConstraintKind::Not(Box::new(constraint_to_ast(
            a, it,
        )))),
        NConstraint::Set(n, vs) => mk_constraint(ConstraintKind::Set {
            nexp: nexp_to_ast(n, it),
            members: vs.iter().map(|v| v.to_string()).collect(),
        }),
        NConstraint::App(func, args) => mk_constraint(ConstraintKind::App {
            func: ast_name(it, func),
            args: args.iter().map(|a| mk_nexp_arg(nexp_to_ast(a, it))).collect(),
        }),
        NConstraint::Id(name) => mk_constraint(ConstraintKind::Id(ast_name(it, name))),
        NConstraint::Var(v) => mk_constraint(ConstraintKind::Var(it.intern(v))),
    }
}

fn typarg_nexp(arg: &TypArg, it: &impl Interner) -> Option<Nexp> {
    match &arg.kind {
        TypArgKind::NExp(n) => nexp_from_ast(n, it),
        TypArgKind::Typ(_) | TypArgKind::Bool(_) => None,
    }
}

/// Lift a canonical node back onto a local constraint. `None` for
/// shapes with no local counterpart.
pub fn constraint_from_ast(c: &Constraint, it: &impl Interner) -> Option<NConstraint> {
    match &c.kind {
        ConstraintKind::True => Some(NConstraint::True),
        ConstraintKind::False => Some(NConstraint::False),
        ConstraintKind::Equal(a, b) => Some(NConstraint::Eq(
            Box::new(typarg_nexp(a, it)?),
            Box::new(typarg_nexp(b, it)?),
        )),
        ConstraintKind::NotEqual(a, b) => Some(NConstraint::Neq(
            Box::new(typarg_nexp(a, it)?),
            Box::new(typarg_nexp(b, it)?),
        )),
        ConstraintKind::Ge(a, b) => Some(NConstraint::Ge(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        ConstraintKind::Gt(a, b) => Some(NConstraint::Gt(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        ConstraintKind::Le(a, b) => Some(NConstraint::Le(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        ConstraintKind::Lt(a, b) => Some(NConstraint::Lt(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        ConstraintKind::And(a, b) => Some(NConstraint::And(
            Box::new(constraint_from_ast(a, it)?),
            Box::new(constraint_from_ast(b, it)?),
        )),
        ConstraintKind::Or(a, b) => Some(NConstraint::Or(
            Box::new(constraint_from_ast(a, it)?),
            Box::new(constraint_from_ast(b, it)?),
        )),
        ConstraintKind::Not(x) => Some(NConstraint::Not(Box::new(constraint_from_ast(x, it)?))),
        ConstraintKind::Set { nexp, members } => {
            let mut vs = Vec::with_capacity(members.len());
            for m in members {
                vs.push(m.parse::<i128>().ok()?);
            }
            Some(NConstraint::Set(
                Box::new(nexp_from_ast(nexp, it)?),
                vs,
            ))
        }
        ConstraintKind::App { func, args } => {
            let name = resolve_name(func, it)?;
            let mut nexps = Vec::with_capacity(args.len());
            for a in args {
                // Only `A_nexp` arguments are modelled; `Typ`/`Bool`
                // arguments stay abstracted (`None`, residual `X-NcSetApp`).
                nexps.push(nexp_from_ast(&typarg_nexp_only(a, it)?, it)?);
            }
            Some(NConstraint::App(name, nexps))
        }
        ConstraintKind::Id(name) => Some(NConstraint::Id(resolve_name(name, it)?)),
        ConstraintKind::Var(sym) => Some(NConstraint::Var(it.resolve(*sym)?.to_owned())),
    }
}

// ---------------------------------------------------------------------------
// Types, schemes, expressions (MIGRATION.md #19-#21)
// ---------------------------------------------------------------------------
// Lowerings for `MiniTyp`/`TypQuant`/`MiniExp` onto the `Typ`/`TypSchm`/
// `Exp` families. Structural cases are exact; multi-argument application
// (currying) and existentials stay `None` — full elaboration is Lane 02
// (expression/type grammars) and Lane 09 (existentials).

fn mk_typ(kind: TypKind) -> Typ {
    Node::dummy(kind)
}

fn mk_exp(kind: ExpKind) -> Exp {
    Node::dummy(kind)
}

fn mk_pat(kind: PatKind) -> Pat {
    Node::dummy(kind)
}

fn mk_lit(kind: LitKind) -> Lit {
    Node::dummy(kind)
}

fn ast_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn resolve_name(name: &Name, it: &impl Interner) -> Option<String> {
    it.resolve(name.symbol).map(str::to_owned)
}

/// Lower a local type onto a canonical node.
/// `Range` keeps both bounds as a `range` application (reversible);
/// full atom-bounded-int elaboration is Lane 09.
pub fn typ_to_ast(t: &MiniTyp, it: &mut impl Interner) -> Typ {
    match t {
        MiniTyp::Int => mk_typ(TypKind::Id(ast_name(it, "int"))),
        MiniTyp::Bool => mk_typ(TypKind::Id(ast_name(it, "bool"))),
        MiniTyp::Bit => mk_typ(TypKind::Id(ast_name(it, "bit"))),
        MiniTyp::Bits(n) => mk_typ(TypKind::App {
            name: ast_name(it, "bits"),
            args: vec![mk_nexp_arg(nexp_to_ast(n, it))],
        }),
        MiniTyp::Range(lo, hi) => mk_typ(TypKind::App {
            name: ast_name(it, "range"),
            args: vec![
                mk_nexp_arg(nexp_to_ast(lo, it)),
                mk_nexp_arg(nexp_to_ast(hi, it)),
            ],
        }),
        MiniTyp::Named { name, args } => {
            if args.is_empty() {
                mk_typ(TypKind::Id(ast_name(it, name)))
            } else {
                mk_typ(TypKind::App {
                    name: ast_name(it, name),
                    args: args.iter().map(|a| mk_nexp_arg(nexp_to_ast(a, it))).collect(),
                })
            }
        }
        MiniTyp::Fun(a, b) => mk_typ(TypKind::Fn {
            args: vec![typ_to_ast(a, it)],
            ret: Box::new(typ_to_ast(b, it)),
        }),
        MiniTyp::Tuple(ts) => mk_typ(TypKind::Tuple(
            ts.iter().map(|e| typ_to_ast(e, it)).collect(),
        )),
    }
}

fn typarg_nexp_only(arg: &TypArg, it: &impl Interner) -> Option<NExp> {
    match &arg.kind {
        TypArgKind::NExp(n) => Some(n.clone()),
        TypArgKind::Typ(_) | TypArgKind::Bool(_) => None,
    }
}

/// Lift a canonical node back onto a local type. `None` for shapes with
/// no local counterpart (`Var`, `Bidir`, `Exist`, non-unary `Fn`,
/// non-nexp arguments, unknown primitives).
pub fn typ_from_ast(t: &Typ, it: &impl Interner) -> Option<MiniTyp> {
    match &t.kind {
        TypKind::Id(name) => match resolve_name(name, it).as_deref() {
            Some("int") => Some(MiniTyp::Int),
            Some("bool") => Some(MiniTyp::Bool),
            Some("bit") => Some(MiniTyp::Bit),
            Some(other) => Some(MiniTyp::Named {
                name: other.to_owned(),
                args: Vec::new(),
            }),
            None => None,
        },
        TypKind::App { name, args } => {
            let text = resolve_name(name, it)?;
            let mut nexps = Vec::with_capacity(args.len());
            for a in args {
                nexps.push(nexp_from_ast(&typarg_nexp_only(a, it)?, it)?);
            }
            match (text.as_str(), nexps.as_slice()) {
                ("bits", [n]) => Some(MiniTyp::Bits(Box::new(n.clone()))),
                ("range", [lo, hi]) => Some(MiniTyp::Range(
                    Box::new(lo.clone()),
                    Box::new(hi.clone()),
                )),
                _ => Some(MiniTyp::Named { name: text, args: nexps }),
            }
        }
        TypKind::Fn { args, ret } => match args.as_slice() {
            [a] => Some(MiniTyp::Fun(
                Box::new(typ_from_ast(a, it)?),
                Box::new(typ_from_ast(ret, it)?),
            )),
            _ => None,
        },
        TypKind::Tuple(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for e in ts {
                out.push(typ_from_ast(e, it)?);
            }
            Some(MiniTyp::Tuple(out))
        }
        TypKind::Unknown | TypKind::Var(_) | TypKind::Bidir { .. } | TypKind::Exist { .. } => None,
    }
}

/// Lower a local scheme onto a canonical one. Quantifier telescopes map
/// onto `QuantItem::Id` (kind-erased) plus one `QuantItem::Constraint`.
pub fn typquant_to_ast(q: &TypQuant, it: &mut impl Interner) -> TypSchm {
    let mut quant: Vec<QuantItem> = Vec::with_capacity(q.quants.len() + 1);
    for (var, kind) in &q.quants {
        quant.push(QuantItem::Id(KOpt {
            kind: kind_erase(*kind),
            kid: it.intern(var),
        }));
    }
    quant.push(QuantItem::Constraint(constraint_to_ast(&q.constraint, it)));
    TypSchm { quant, typ: typ_to_ast(&q.typ, it) }
}

fn var_pat(it: &mut impl Interner, var: &str, annot: Option<&MiniTyp>) -> Option<Pat> {
    let id = mk_pat(PatKind::Id(ast_name(it, var)));
    match annot {
        None => Some(id),
        Some(t) => Some(mk_pat(PatKind::Typ {
            typ: typ_to_ast(t, it),
            pat: Box::new(id),
        })),
    }
}

/// Lower a local expression onto a canonical node. `None` for applied
/// non-variables (currying needs Lane 02 elaboration).
pub fn miniexp_to_ast(e: &MiniExp, it: &mut impl Interner) -> Option<Exp> {
    match e {
        MiniExp::IntLit(i) => {
            let sym = it.intern(&i.to_string());
            Some(mk_exp(ExpKind::Lit(mk_lit(LitKind::Num(sym)))))
        }
        MiniExp::BoolLit(true) => Some(mk_exp(ExpKind::Lit(mk_lit(LitKind::True)))),
        MiniExp::BoolLit(false) => Some(mk_exp(ExpKind::Lit(mk_lit(LitKind::False)))),
        MiniExp::Var(v) => Some(mk_exp(ExpKind::Id(ast_name(it, v)))),
        MiniExp::Let { var, annot, bound, body } => Some(mk_exp(ExpKind::Let {
            pat: var_pat(it, var, annot.as_ref())?,
            bind: Box::new(miniexp_to_ast(bound, it)?),
            body: Box::new(miniexp_to_ast(body, it)?),
        })),
        MiniExp::App(f, x) => match &**f {
            MiniExp::Var(v) => Some(mk_exp(ExpKind::App {
                func: ast_name(it, v),
                args: vec![miniexp_to_ast(x, it)?],
            })),
            _ => None,
        },
        MiniExp::Tuple(es) => {
            let mut out = Vec::with_capacity(es.len());
            for e in es {
                out.push(miniexp_to_ast(e, it)?);
            }
            Some(mk_exp(ExpKind::Tuple(out)))
        }
        MiniExp::Annot(e, t) => Some(mk_exp(ExpKind::Typ {
            typ: typ_to_ast(t, it),
            exp: Box::new(miniexp_to_ast(e, it)?),
        })),
        MiniExp::If(c, a, b) => Some(mk_exp(ExpKind::If {
            cond: Box::new(miniexp_to_ast(c, it)?),
            then_branch: Box::new(miniexp_to_ast(a, it)?),
            else_branch: Box::new(miniexp_to_ast(b, it)?),
        })),
    }
}

fn let_pat(p: &Pat, it: &impl Interner) -> Option<(String, Option<MiniTyp>)> {
    match &p.kind {
        PatKind::Id(name) => Some((resolve_name(name, it)?, None)),
        PatKind::Typ { typ, pat } => match &pat.kind {
            PatKind::Id(name) => Some((resolve_name(name, it)?, Some(typ_from_ast(typ, it)?))),
            _ => None,
        },
        _ => None,
    }
}

/// Lift a canonical node back onto a local expression. `None` for
/// shapes outside the `Lit/Id/Let/App/Tuple/Typ/If` subset.
pub fn miniexp_from_ast(e: &Exp, it: &impl Interner) -> Option<MiniExp> {
    match &e.kind {
        ExpKind::Lit(l) => match &l.kind {
            LitKind::Num(s) => it.resolve(*s)?.parse::<i128>().ok().map(MiniExp::IntLit),
            LitKind::True => Some(MiniExp::BoolLit(true)),
            LitKind::False => Some(MiniExp::BoolLit(false)),
            _ => None,
        },
        ExpKind::Id(name) => resolve_name(name, it).map(MiniExp::Var),
        ExpKind::Let { pat, bind, body } => {
            let (var, annot) = let_pat(pat, it)?;
            Some(MiniExp::Let {
                var,
                annot,
                bound: Box::new(miniexp_from_ast(bind, it)?),
                body: Box::new(miniexp_from_ast(body, it)?),
            })
        }
        ExpKind::App { func, args } => match args.as_slice() {
            [x] => Some(MiniExp::App(
                Box::new(MiniExp::Var(resolve_name(func, it)?)),
                Box::new(miniexp_from_ast(x, it)?),
            )),
            _ => None,
        },
        ExpKind::Tuple(es) => {
            let mut out = Vec::with_capacity(es.len());
            for e in es {
                out.push(miniexp_from_ast(e, it)?);
            }
            Some(MiniExp::Tuple(out))
        }
        ExpKind::Typ { typ, exp } => Some(MiniExp::Annot(
            Box::new(miniexp_from_ast(exp, it)?),
            typ_from_ast(typ, it)?,
        )),
        ExpKind::If { cond, then_branch, else_branch } => Some(MiniExp::If(
            Box::new(miniexp_from_ast(cond, it)?),
            Box::new(miniexp_from_ast(then_branch, it)?),
            Box::new(miniexp_from_ast(else_branch, it)?),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::SimpleInterner;

    #[test]
    fn not_constraint_roundtrips() {
        let mut it = SimpleInterner::new();
        let local = NConstraint::Not(Box::new(NConstraint::True));
        let ast = constraint_to_ast(&local, &mut it);
        assert!(matches!(ast.kind, ConstraintKind::Not(_)));
        assert_eq!(constraint_from_ast(&ast, &it), Some(local));
    }

    #[test]
    fn not_roundtrips_through_ast() {
        let mut it = SimpleInterner::new();
        let c = NConstraint::Not(Box::new(NConstraint::True));
        let ast = constraint_to_ast(&c, &mut it);
        assert!(matches!(ast.kind, ConstraintKind::Not(_)));
        assert_eq!(constraint_from_ast(&ast, &it), Some(c));
    }
}
