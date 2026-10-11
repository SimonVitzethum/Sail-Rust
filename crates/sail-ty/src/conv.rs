#![forbid(unsafe_code)]
//! Bridges between the local minimal types and the canonical `sail-ast`
//! nodes (Lane 61 unification, ARCHITECTURE.md §6 steps 2–3).
//!
//! Mapping (ARCHITECTURE.md #17/#18/#22):
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
//!
//! Span hygiene: `Span::DUMMY` is the `FileId(u32::MAX)` sentinel, never
//! file 0 — synthesized nodes can never be mistaken for `file0:0-0`.
//! Conversely `Span::default()` (file 0, `0..0`) is a REAL span, not a
//! placeholder; it must never stand in for "no location" (use
//! `Span::DUMMY`, tested by `synthesized_spans_are_dummy_never_file_zero`).

use sail_ast::{
    Constraint, ConstraintKind, Exp, ExpKind, FExp, Interner, Kind as AstKind, KOpt, LExp,
    LExpKind, Lit, LitKind, Loop as AstLoop, Name, NExp, NExpKind, Node, NodeId, Order as AstOrder,
    Pat, PatKind, PExp, PExpKind, QuantItem, Span, StructName, Typ, TypArg, TypArgKind, TypKind,
    TypSchm,
};

use crate::kind::Kind;
use crate::nexp::{MiniExp, MiniFExp, MiniLexp, MiniLoop, MiniOrder, MiniPExp, MiniTyp, NConstraint, Nexp, TypQuant};
use crate::pat::MiniPat;

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
// Types, schemes, expressions (ARCHITECTURE.md #19-#21)
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

fn mk_lexp(kind: LExpKind) -> LExp {
    Node::dummy(kind)
}

fn mk_pexp(kind: PExpKind) -> PExp {
    PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind,
    }
}

fn mk_fexp(field: Name, value: Exp) -> FExp {
    FExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        field,
        value,
    }
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

/// Lower a local pattern onto a canonical node. `None` is unreachable
/// inside `MiniPat` (every constructor maps); the `Option` keeps the
/// combinator shape uniform with the expression converters.
pub fn minipat_to_ast(p: &MiniPat, it: &mut impl Interner) -> Option<Pat> {
    match p {
        MiniPat::Wild => Some(mk_pat(PatKind::Wild)),
        MiniPat::Var(v) => Some(mk_pat(PatKind::Id(ast_name(it, v)))),
        MiniPat::IntLit(i) => {
            let sym = it.intern(&i.to_string());
            Some(mk_pat(PatKind::Lit(mk_lit(LitKind::Num(sym)))))
        }
        MiniPat::BoolLit(true) => Some(mk_pat(PatKind::Lit(mk_lit(LitKind::True)))),
        MiniPat::BoolLit(false) => Some(mk_pat(PatKind::Lit(mk_lit(LitKind::False)))),
        MiniPat::Tuple(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for e in ps {
                out.push(minipat_to_ast(e, it)?);
            }
            Some(mk_pat(PatKind::Tuple(out)))
        }
        MiniPat::Annot(inner, t) => Some(mk_pat(PatKind::Typ {
            typ: typ_to_ast(t, it),
            pat: Box::new(minipat_to_ast(inner, it)?),
        })),
    }
}

/// Lift a canonical node back onto a local pattern. `None` for shapes with
/// no local counterpart (`Or`/`Not`/`As`/`App`/vector/struct/list/…).
pub fn minipat_from_ast(p: &Pat, it: &impl Interner) -> Option<MiniPat> {
    match &p.kind {
        PatKind::Wild => Some(MiniPat::Wild),
        PatKind::Id(name) => resolve_name(name, it).map(MiniPat::Var),
        PatKind::Lit(l) => match &l.kind {
            LitKind::Num(s) => it.resolve(*s)?.parse::<i128>().ok().map(MiniPat::IntLit),
            LitKind::True => Some(MiniPat::BoolLit(true)),
            LitKind::False => Some(MiniPat::BoolLit(false)),
            _ => None,
        },
        PatKind::Tuple(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for e in ps {
                out.push(minipat_from_ast(e, it)?);
            }
            Some(MiniPat::Tuple(out))
        }
        PatKind::Typ { typ, pat } => Some(MiniPat::Annot(
            Box::new(minipat_from_ast(pat, it)?),
            typ_from_ast(typ, it)?,
        )),
        _ => None,
    }
}

fn miniloop_to_ast(kind: MiniLoop) -> AstLoop {
    match kind {
        MiniLoop::While => AstLoop::While,
        MiniLoop::Until => AstLoop::Until,
    }
}

fn miniloop_from_ast(kind: AstLoop) -> MiniLoop {
    match kind {
        AstLoop::While => MiniLoop::While,
        AstLoop::Until => MiniLoop::Until,
    }
}

fn miniorder_to_ast(order: MiniOrder) -> AstOrder {
    match order {
        MiniOrder::Inc => AstOrder::Inc,
        MiniOrder::Dec => AstOrder::Dec,
    }
}

fn miniorder_from_ast(order: AstOrder) -> MiniOrder {
    match order {
        AstOrder::Inc => MiniOrder::Inc,
        AstOrder::Dec => MiniOrder::Dec,
    }
}

/// Lower a local l-value onto a canonical node.
pub fn minilexp_to_ast(l: &MiniLexp, it: &mut impl Interner) -> Option<LExp> {
    match l {
        MiniLexp::Id(v) => Some(mk_lexp(LExpKind::Id(ast_name(it, v)))),
        MiniLexp::Deref(e) => Some(mk_lexp(LExpKind::Deref(Box::new(miniexp_to_ast(e, it)?)))),
        MiniLexp::Typed { typ, id } => Some(mk_lexp(LExpKind::Typ {
            typ: typ_to_ast(typ, it),
            id: ast_name(it, id),
        })),
        MiniLexp::Tuple(ls) => {
            let mut out = Vec::with_capacity(ls.len());
            for e in ls {
                out.push(minilexp_to_ast(e, it)?);
            }
            Some(mk_lexp(LExpKind::Tuple(out)))
        }
        MiniLexp::VectorConcat(ls) => {
            let mut out = Vec::with_capacity(ls.len());
            for e in ls {
                out.push(minilexp_to_ast(e, it)?);
            }
            Some(mk_lexp(LExpKind::VectorConcat(out)))
        }
        MiniLexp::Vector { lexp, index } => Some(mk_lexp(LExpKind::Vector {
            lexp: Box::new(minilexp_to_ast(lexp, it)?),
            index: Box::new(miniexp_to_ast(index, it)?),
        })),
        MiniLexp::VectorRange { lexp, lo, hi } => Some(mk_lexp(LExpKind::VectorRange {
            lexp: Box::new(minilexp_to_ast(lexp, it)?),
            lo: Box::new(miniexp_to_ast(lo, it)?),
            hi: Box::new(miniexp_to_ast(hi, it)?),
        })),
        MiniLexp::Field { lexp, field } => Some(mk_lexp(LExpKind::Field {
            lexp: Box::new(minilexp_to_ast(lexp, it)?),
            field: ast_name(it, field),
        })),
    }
}

/// Lift a canonical l-value back onto a local one.
pub fn minilexp_from_ast(l: &LExp, it: &impl Interner) -> Option<MiniLexp> {
    match &l.kind {
        LExpKind::Id(name) => resolve_name(name, it).map(MiniLexp::Id),
        LExpKind::Deref(e) => Some(MiniLexp::Deref(Box::new(miniexp_from_ast(e, it)?))),
        LExpKind::Typ { typ, id } => Some(MiniLexp::Typed {
            typ: typ_from_ast(typ, it)?,
            id: resolve_name(id, it)?,
        }),
        LExpKind::Tuple(ls) => {
            let mut out = Vec::with_capacity(ls.len());
            for e in ls {
                out.push(minilexp_from_ast(e, it)?);
            }
            Some(MiniLexp::Tuple(out))
        }
        LExpKind::VectorConcat(ls) => {
            let mut out = Vec::with_capacity(ls.len());
            for e in ls {
                out.push(minilexp_from_ast(e, it)?);
            }
            Some(MiniLexp::VectorConcat(out))
        }
        LExpKind::Vector { lexp, index } => Some(MiniLexp::Vector {
            lexp: Box::new(minilexp_from_ast(lexp, it)?),
            index: Box::new(miniexp_from_ast(index, it)?),
        }),
        LExpKind::VectorRange { lexp, lo, hi } => Some(MiniLexp::VectorRange {
            lexp: Box::new(minilexp_from_ast(lexp, it)?),
            lo: Box::new(miniexp_from_ast(lo, it)?),
            hi: Box::new(miniexp_from_ast(hi, it)?),
        }),
        LExpKind::Field { lexp, field } => Some(MiniLexp::Field {
            lexp: Box::new(minilexp_from_ast(lexp, it)?),
            field: resolve_name(field, it)?,
        }),
    }
}

fn minifexps_to_ast(fs: &[MiniFExp], it: &mut impl Interner) -> Option<Vec<FExp>> {
    let mut out = Vec::with_capacity(fs.len());
    for f in fs {
        out.push(mk_fexp(ast_name(it, &f.field), miniexp_to_ast(&f.value, it)?));
    }
    Some(out)
}

fn minifexps_from_ast(fs: &[FExp], it: &impl Interner) -> Option<Vec<MiniFExp>> {
    let mut out = Vec::with_capacity(fs.len());
    for f in fs {
        out.push(MiniFExp {
            field: resolve_name(&f.field, it)?,
            value: miniexp_from_ast(&f.value, it)?,
        });
    }
    Some(out)
}

fn minipexps_to_ast(arms: &[MiniPExp], it: &mut impl Interner) -> Option<Vec<PExp>> {
    let mut out = Vec::with_capacity(arms.len());
    for a in arms {
        let pat = minipat_to_ast(&a.pat, it)?;
        let body = miniexp_to_ast(&a.body, it)?;
        match &a.guard {
            None => out.push(mk_pexp(PExpKind::PatExp { pat, body })),
            Some(g) => out.push(mk_pexp(PExpKind::PatWhen {
                pat,
                guard: miniexp_to_ast(g, it)?,
                body,
            })),
        }
    }
    Some(out)
}

fn minipexps_from_ast(arms: &[PExp], it: &impl Interner) -> Option<Vec<MiniPExp>> {
    let mut out = Vec::with_capacity(arms.len());
    for a in arms {
        match &a.kind {
            PExpKind::PatExp { pat, body } => out.push(MiniPExp {
                pat: minipat_from_ast(pat, it)?,
                guard: None,
                body: miniexp_from_ast(body, it)?,
            }),
            PExpKind::PatWhen { pat, guard, body } => out.push(MiniPExp {
                pat: minipat_from_ast(pat, it)?,
                guard: Some(miniexp_from_ast(guard, it)?),
                body: miniexp_from_ast(body, it)?,
            }),
        }
    }
    Some(out)
}

fn miniexps_to_ast(es: &[MiniExp], it: &mut impl Interner) -> Option<Vec<Exp>> {
    let mut out = Vec::with_capacity(es.len());
    for e in es {
        out.push(miniexp_to_ast(e, it)?);
    }
    Some(out)
}

fn miniexps_from_ast(es: &[Exp], it: &impl Interner) -> Option<Vec<MiniExp>> {
    let mut out = Vec::with_capacity(es.len());
    for e in es {
        out.push(miniexp_from_ast(e, it)?);
    }
    Some(out)
}

/// Lower a local expression onto a canonical node. `None` for applied
/// non-variables (currying needs Lane 02 elaboration) and for literal
/// payloads outside the `Num`/`True`/`False` subset.
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
        MiniExp::Block(es) => Some(mk_exp(ExpKind::Block(miniexps_to_ast(es, it)?))),
        MiniExp::Loop { kind, measure, cond, body } => Some(mk_exp(ExpKind::Loop {
            loop_kind: miniloop_to_ast(*kind),
            measure: match measure {
                None => None,
                Some(m) => Some(Box::new(miniexp_to_ast(m, it)?)),
            },
            cond: Box::new(miniexp_to_ast(cond, it)?),
            body: Box::new(miniexp_to_ast(body, it)?),
        })),
        MiniExp::For { var, from, to, step, order, body } => Some(mk_exp(ExpKind::For {
            var: ast_name(it, var),
            from: Box::new(miniexp_to_ast(from, it)?),
            to: Box::new(miniexp_to_ast(to, it)?),
            step: Box::new(miniexp_to_ast(step, it)?),
            order: miniorder_to_ast(*order),
            body: Box::new(miniexp_to_ast(body, it)?),
        })),
        MiniExp::Vector(es) => Some(mk_exp(ExpKind::Vector(miniexps_to_ast(es, it)?))),
        MiniExp::VectorAppend(a, b) => Some(mk_exp(ExpKind::VectorAppend(
            Box::new(miniexp_to_ast(a, it)?),
            Box::new(miniexp_to_ast(b, it)?),
        ))),
        MiniExp::List(es) => Some(mk_exp(ExpKind::List(miniexps_to_ast(es, it)?))),
        MiniExp::Cons(h, t) => Some(mk_exp(ExpKind::Cons(
            Box::new(miniexp_to_ast(h, it)?),
            Box::new(miniexp_to_ast(t, it)?),
        ))),
        MiniExp::Struct { name, fields } => Some(mk_exp(ExpKind::Struct {
            name: match name {
                None => StructName::Anon,
                Some(n) => StructName::Id(ast_name(it, n)),
            },
            fields: minifexps_to_ast(fields, it)?,
        })),
        MiniExp::StructUpdate { record, fields } => Some(mk_exp(ExpKind::StructUpdate {
            record: Box::new(miniexp_to_ast(record, it)?),
            fields: minifexps_to_ast(fields, it)?,
        })),
        MiniExp::Field { record, field } => Some(mk_exp(ExpKind::Field {
            record: Box::new(miniexp_to_ast(record, it)?),
            field: ast_name(it, field),
        })),
        MiniExp::Match { scrutinee, arms } => Some(mk_exp(ExpKind::Match {
            scrutinee: Box::new(miniexp_to_ast(scrutinee, it)?),
            arms: minipexps_to_ast(arms, it)?,
        })),
        MiniExp::Assign { lexp, value } => Some(mk_exp(ExpKind::Assign {
            lexp: Box::new(minilexp_to_ast(lexp, it)?),
            value: Box::new(miniexp_to_ast(value, it)?),
        })),
        MiniExp::Sizeof(n) => Some(mk_exp(ExpKind::Sizeof(nexp_to_ast(n, it)))),
        MiniExp::Return(e) => Some(mk_exp(ExpKind::Return(Box::new(miniexp_to_ast(e, it)?)))),
        MiniExp::Exit(e) => Some(mk_exp(ExpKind::Exit(Box::new(miniexp_to_ast(e, it)?)))),
        MiniExp::Config(keys) => Some(mk_exp(ExpKind::Config(keys.clone()))),
        MiniExp::Ref(v) => Some(mk_exp(ExpKind::Ref(ast_name(it, v)))),
        MiniExp::Throw(e) => Some(mk_exp(ExpKind::Throw(Box::new(miniexp_to_ast(e, it)?)))),
        MiniExp::Try { exp, arms } => Some(mk_exp(ExpKind::Try {
            exp: Box::new(miniexp_to_ast(exp, it)?),
            arms: minipexps_to_ast(arms, it)?,
        })),
        MiniExp::Assert { cond, msg } => Some(mk_exp(ExpKind::Assert {
            cond: Box::new(miniexp_to_ast(cond, it)?),
            msg: Box::new(miniexp_to_ast(msg, it)?),
        })),
        MiniExp::VarDecl { lexp, init, body } => Some(mk_exp(ExpKind::Var {
            lexp: Box::new(minilexp_to_ast(lexp, it)?),
            init: Box::new(miniexp_to_ast(init, it)?),
            body: Box::new(miniexp_to_ast(body, it)?),
        })),
        MiniExp::Undef => Some(mk_exp(ExpKind::Undef)),
        MiniExp::Constraint(c) => Some(mk_exp(ExpKind::Constraint(constraint_to_ast(c, it)))),
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

/// Lift a canonical node back onto a local expression. `None` for the
/// internal `E_internal_*`/`E_internal_value` shapes, multi-argument
/// applications, literal payloads outside `Num`/`True`/`False`, and
/// patterns/l-values outside the local subsets.
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
        ExpKind::Block(es) => Some(MiniExp::Block(miniexps_from_ast(es, it)?)),
        ExpKind::Loop { loop_kind, measure, cond, body } => Some(MiniExp::Loop {
            kind: miniloop_from_ast(*loop_kind),
            measure: match measure {
                None => None,
                Some(m) => Some(Box::new(miniexp_from_ast(m, it)?)),
            },
            cond: Box::new(miniexp_from_ast(cond, it)?),
            body: Box::new(miniexp_from_ast(body, it)?),
        }),
        ExpKind::For { var, from, to, step, order, body } => Some(MiniExp::For {
            var: resolve_name(var, it)?,
            from: Box::new(miniexp_from_ast(from, it)?),
            to: Box::new(miniexp_from_ast(to, it)?),
            step: Box::new(miniexp_from_ast(step, it)?),
            order: miniorder_from_ast(*order),
            body: Box::new(miniexp_from_ast(body, it)?),
        }),
        ExpKind::Vector(es) => Some(MiniExp::Vector(miniexps_from_ast(es, it)?)),
        ExpKind::VectorAppend(a, b) => Some(MiniExp::VectorAppend(
            Box::new(miniexp_from_ast(a, it)?),
            Box::new(miniexp_from_ast(b, it)?),
        )),
        ExpKind::List(es) => Some(MiniExp::List(miniexps_from_ast(es, it)?)),
        ExpKind::Cons(h, t) => Some(MiniExp::Cons(
            Box::new(miniexp_from_ast(h, it)?),
            Box::new(miniexp_from_ast(t, it)?),
        )),
        ExpKind::Struct { name, fields } => Some(MiniExp::Struct {
            name: match name {
                StructName::Anon => None,
                StructName::Id(n) => Some(resolve_name(n, it)?),
            },
            fields: minifexps_from_ast(fields, it)?,
        }),
        ExpKind::StructUpdate { record, fields } => Some(MiniExp::StructUpdate {
            record: Box::new(miniexp_from_ast(record, it)?),
            fields: minifexps_from_ast(fields, it)?,
        }),
        ExpKind::Field { record, field } => Some(MiniExp::Field {
            record: Box::new(miniexp_from_ast(record, it)?),
            field: resolve_name(field, it)?,
        }),
        ExpKind::Match { scrutinee, arms } => Some(MiniExp::Match {
            scrutinee: Box::new(miniexp_from_ast(scrutinee, it)?),
            arms: minipexps_from_ast(arms, it)?,
        }),
        ExpKind::Assign { lexp, value } => Some(MiniExp::Assign {
            lexp: Box::new(minilexp_from_ast(lexp, it)?),
            value: Box::new(miniexp_from_ast(value, it)?),
        }),
        ExpKind::Sizeof(n) => Some(MiniExp::Sizeof(nexp_from_ast(n, it)?)),
        ExpKind::Return(e) => Some(MiniExp::Return(Box::new(miniexp_from_ast(e, it)?))),
        ExpKind::Exit(e) => Some(MiniExp::Exit(Box::new(miniexp_from_ast(e, it)?))),
        ExpKind::Config(keys) => Some(MiniExp::Config(keys.clone())),
        ExpKind::Ref(name) => resolve_name(name, it).map(MiniExp::Ref),
        ExpKind::Throw(e) => Some(MiniExp::Throw(Box::new(miniexp_from_ast(e, it)?))),
        ExpKind::Try { exp, arms } => Some(MiniExp::Try {
            exp: Box::new(miniexp_from_ast(exp, it)?),
            arms: minipexps_from_ast(arms, it)?,
        }),
        ExpKind::Assert { cond, msg } => Some(MiniExp::Assert {
            cond: Box::new(miniexp_from_ast(cond, it)?),
            msg: Box::new(miniexp_from_ast(msg, it)?),
        }),
        ExpKind::Var { lexp, init, body } => Some(MiniExp::VarDecl {
            lexp: Box::new(minilexp_from_ast(lexp, it)?),
            init: Box::new(miniexp_from_ast(init, it)?),
            body: Box::new(miniexp_from_ast(body, it)?),
        }),
        ExpKind::Undef => Some(MiniExp::Undef),
        ExpKind::Constraint(c) => Some(MiniExp::Constraint(constraint_from_ast(c, it)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::SimpleInterner;

    #[test]
    fn synthesized_spans_are_dummy_never_file_zero() {
        // DUMMY-span hygiene: lowered nodes carry the `FileId(u32::MAX)`
        // sentinel, never a file-0 span that could alias a real location.
        let mut it = SimpleInterner::new();
        let ast = typ_to_ast(&MiniTyp::Bits(Box::new(Nexp::Const(8))), &mut it);
        assert!(ast.span.is_dummy());
        assert_ne!(ast.span.file, sail_ast::FileId(0));
        // `Span::default()` (file 0, `0..0`) is a REAL span, not a
        // placeholder — it must never be treated as "no location".
        assert!(!Span::default().is_dummy());
    }

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

    #[test]
    fn var_union_keeps_id_and_evar_distinct() {
        // Lane 120 union: tuple `Var` is the `E_id` reference, `VarDecl`
        // is the `E_var` declaration form — they must lower to distinct
        // `ExpKind`s and roundtrip separately.
        let mut it = SimpleInterner::new();
        let id = MiniExp::Var("x".to_owned());
        let ast = miniexp_to_ast(&id, &mut it).expect("id lowering must succeed");
        assert!(matches!(ast.kind, ExpKind::Id(_)));
        assert_eq!(miniexp_from_ast(&ast, &it), Some(id));
        let decl = MiniExp::VarDecl {
            lexp: Box::new(MiniLexp::Id("tmp".to_owned())),
            init: Box::new(MiniExp::IntLit(0)),
            body: Box::new(MiniExp::Var("tmp".to_owned())),
        };
        let ast = miniexp_to_ast(&decl, &mut it).expect("evar lowering must succeed");
        assert!(matches!(ast.kind, ExpKind::Var { .. }));
        assert_eq!(miniexp_from_ast(&ast, &it), Some(decl));
    }
}
