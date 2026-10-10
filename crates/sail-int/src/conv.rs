#![forbid(unsafe_code)]
//! Bridge between the local [`NExp`][crate::nexp::NExp] fragment and the
//! canonical `sail-ast` nodes (Lane 61 unification, MIGRATION.md #34).
//!
//! Mapping: `Constant/Var/Add/Sub/Mul/Neg` map 1:1 onto
//! `NExpKind::{Constant,Var,Sum,Minus,Times,Neg}` (`Constant` is `i128`
//! here vs decimal `String` there); `Abs/Pow2/Div/Mod` become `NExp_app`
//! nodes (`abs`, `2^` is `Exp`, `div`, `mod`). The canonical
//! `If/Id/App` shapes with any other operator map back to `None`
//! (this crate has no conditional/VAR-id nodes; [`Facts`][crate::nexp::Facts]
//! context logic is untouched).
//!
//! All synthesized nodes carry `NodeId::DUMMY`/`Span::DUMMY`; names are
//! interned through the caller's interner.

use sail_ast::{Interner, Name, NExp as AstNExp, NExpKind, Node};

use crate::nexp::NExp;

fn mk(kind: NExpKind) -> AstNExp {
    Node::dummy(kind)
}

fn op_name(it: &mut impl Interner, op: &str) -> Name {
    Name::dummy(it.intern(op))
}

/// Lower a local nexp onto a canonical node.
pub fn nexp_to_ast(n: &NExp, it: &mut impl Interner) -> AstNExp {
    match n {
        NExp::Constant(c) => mk(NExpKind::Constant(c.to_string())),
        NExp::Var(v) => mk(NExpKind::Var(it.intern(v))),
        NExp::Add(a, b) => mk(NExpKind::Sum(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Sub(a, b) => mk(NExpKind::Minus(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Mul(a, b) => mk(NExpKind::Times(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Neg(a) => mk(NExpKind::Neg(Box::new(nexp_to_ast(a, it)))),
        NExp::Abs(a) => mk(NExpKind::App {
            func: op_name(it, "abs"),
            args: vec![nexp_to_ast(a, it)],
        }),
        NExp::Pow2(a) => mk(NExpKind::Exp(Box::new(nexp_to_ast(a, it)))),
        NExp::Div(a, b) => mk(NExpKind::App {
            func: op_name(it, "div"),
            args: vec![nexp_to_ast(a, it), nexp_to_ast(b, it)],
        }),
        NExp::Mod(a, b) => mk(NExpKind::App {
            func: op_name(it, "mod"),
            args: vec![nexp_to_ast(a, it), nexp_to_ast(b, it)],
        }),
    }
}

/// Lift a canonical node back onto a local nexp. `None` for shapes with
/// no local counterpart (`If`, `Id`, foreign `App` operators).
pub fn nexp_from_ast(n: &AstNExp, it: &impl Interner) -> Option<NExp> {
    match &n.kind {
        NExpKind::Constant(s) => s.parse::<i128>().ok().map(NExp::Constant),
        NExpKind::Var(sym) => it.resolve(*sym).map(|s| NExp::Var(s.to_owned())),
        NExpKind::Sum(a, b) => Some(NExp::Add(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Minus(a, b) => Some(NExp::Sub(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Times(a, b) => Some(NExp::Mul(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Neg(a) => Some(NExp::Neg(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::Exp(a) => Some(NExp::Pow2(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::App { func, args } => {
            let op = it.resolve(func.symbol)?;
            match (op, args.as_slice()) {
                ("abs", [a]) => Some(NExp::Abs(Box::new(nexp_from_ast(a, it)?))),
                ("div", [a, b]) => Some(NExp::Div(
                    Box::new(nexp_from_ast(a, it)?),
                    Box::new(nexp_from_ast(b, it)?),
                )),
                ("mod", [a, b]) => Some(NExp::Mod(
                    Box::new(nexp_from_ast(a, it)?),
                    Box::new(nexp_from_ast(b, it)?),
                )),
                _ => None,
            }
        }
        NExpKind::If { .. } | NExpKind::Id(_) => None,
    }
}
