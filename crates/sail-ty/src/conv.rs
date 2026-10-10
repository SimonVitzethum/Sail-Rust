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
//!   compares type arguments, not raw nexps). `NC_set`/`NC_app`/`NC_var`
//!   map back to `None`.
//! - `Kind::{KInt,KBool,KType}` erase onto `sail_ast::Kind::{Int,Bool,Type}`;
//!   `KNat` (Lean-`Nat` refinement, Lane 04/10) erases to `Int` and is
//!   NEVER silently recovered: [`kind_from_ast`] yields `KInt`.
//!
//! All synthesized nodes carry `NodeId::DUMMY`/`Span::DUMMY`; names are
//! interned through the caller's interner. Behaviour of the local types
//! (folding, kind checking, solving) is untouched.

use sail_ast::{
    Constraint, ConstraintKind, Interner, Kind as AstKind, Name, NExp, NExpKind, Node, TypArg,
    TypArgKind,
};

use crate::kind::Kind;
use crate::nexp::{NConstraint, Nexp};

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
        ConstraintKind::App { .. }
        | ConstraintKind::Id(_)
        | ConstraintKind::Var(_)
        | ConstraintKind::Set { .. } => None,
    }
}
