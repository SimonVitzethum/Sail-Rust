#![forbid(unsafe_code)]
//! Bridges between canonical `sail-ast` nodes and the `sail-nexp` algebra
//! (Lane 108: `typ_arg` sorts `A_nexp`/`A_typ`/`A_bool`).
//!
//! Mapping (`Ast.ml`, `typ_arg`/`n_constraint`):
//! - `NExpKind::{Id,Var,Constant,App,If,Times,Sum,Minus,Exp,Neg}` map 1:1
//!   onto [`NExp`]; `Constant` parses the decimal `String` as `i128`,
//!   `Var` keeps the interned [`Kid`][crate::Kid] verbatim, `Id`/`App`
//!   operator names resolve through the caller's interner.
//! - `TypArgKind::{NExp,Typ,Bool}` map 1:1 onto [`TypArg`]; types map
//!   structurally onto [`NTyp`] (all `TypKind` shapes, existentials with
//!   their binders), constraints recurse.
//! - `ConstraintKind::{Equal,NotEqual}` carry [`TypArg`] pairs (as in the
//!   OCaml AST, not raw nexps); `App` carries [`TypArg`]s of any sort;
//!   `Ge/Gt/Le/Lt` carry raw nexps; `Set` members parse as `i128`.
//!
//! `*_from_ast` return `None` for unresolvable names (`Id`/`App`
//! operators, which need interner lookups) and unparsable literals;
//! `*_to_ast` are total. All synthesized nodes carry
//! `NodeId::DUMMY`/`Span::DUMMY`; names are interned through the caller's
//! interner.
//!
//! `i128` range note (Zarith-Verlust): Sail integers are arbitrary
//! precision (`Zarith` in OCaml), but [`NExp::Constant`] holds an
//! `i128`. Literals outside the `i128` range (and `Set` members) fail
//! to lift (`None`); callers must treat that as "open/unknown", never
//! as a proven fact. Inside the range every fold is exact (`checked_*`,
//! unfolded on overflow), and the Z3 backend maps even edge literals
//! exactly (Horner basis `2^32`).

use sail_ast::{
    Constraint as AstConstraint, ConstraintKind, Interner, Kind as AstKind, KOpt as AstKOpt,
    Name, NExp as AstNExp, NExpKind, Node, Typ as AstTyp, TypArg as AstTypArg, TypArgKind,
    TypKind,
};

use crate::expr::{NConstraint, NExp, NKind, NQuant, NTyp, TypArg};

fn ast_nexp(kind: NExpKind) -> AstNExp {
    Node::dummy(kind)
}

fn ast_constraint(kind: ConstraintKind) -> AstConstraint {
    Node::dummy(kind)
}

fn ast_typ(kind: TypKind) -> AstTyp {
    Node::dummy(kind)
}

fn ast_typarg(kind: TypArgKind) -> AstTypArg {
    Node::dummy(kind)
}

fn op_name(func: &Name, it: &impl Interner) -> Option<String> {
    it.resolve(func.symbol).map(str::to_owned)
}

fn ast_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

/// Lower a canonical nexp onto the algebra. `None` for unresolvable
/// operator names and unparsable constants.
pub fn nexp_from_ast(n: &AstNExp, it: &impl Interner) -> Option<NExp> {
    match &n.kind {
        NExpKind::Id(name) => op_name(name, it).map(NExp::Id),
        NExpKind::Var(kid) => Some(NExp::Var(*kid)),
        NExpKind::Constant(text) => text.parse::<i128>().ok().map(NExp::Constant),
        NExpKind::App { func, args } => {
            let f = op_name(func, it)?;
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(nexp_from_ast(a, it)?);
            }
            Some(NExp::App(f, out))
        }
        NExpKind::If { cond, then_branch, else_branch } => Some(NExp::If(
            Box::new(constraint_from_ast(cond, it)?),
            Box::new(nexp_from_ast(then_branch, it)?),
            Box::new(nexp_from_ast(else_branch, it)?),
        )),
        NExpKind::Times(a, b) => Some(NExp::Times(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Sum(a, b) => Some(NExp::Sum(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Minus(a, b) => Some(NExp::Minus(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Exp(x) => Some(NExp::Exp(Box::new(nexp_from_ast(x, it)?))),
        NExpKind::Neg(x) => Some(NExp::Neg(Box::new(nexp_from_ast(x, it)?))),
    }
}

/// Lower an algebra nexp onto a canonical node (total).
pub fn nexp_to_ast(e: &NExp, it: &mut impl Interner) -> AstNExp {
    match e {
        NExp::Id(name) => ast_nexp(NExpKind::Id(ast_name(it, name))),
        NExp::Var(kid) => ast_nexp(NExpKind::Var(*kid)),
        NExp::Constant(v) => ast_nexp(NExpKind::Constant(v.to_string())),
        NExp::App(f, args) => ast_nexp(NExpKind::App {
            func: ast_name(it, f),
            args: args.iter().map(|a| nexp_to_ast(a, it)).collect(),
        }),
        NExp::If(c, t, b) => ast_nexp(NExpKind::If {
            cond: Box::new(constraint_to_ast(c, it)),
            then_branch: Box::new(nexp_to_ast(t, it)),
            else_branch: Box::new(nexp_to_ast(b, it)),
        }),
        NExp::Times(a, b) => ast_nexp(NExpKind::Times(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Sum(a, b) => ast_nexp(NExpKind::Sum(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Minus(a, b) => ast_nexp(NExpKind::Minus(
            Box::new(nexp_to_ast(a, it)),
            Box::new(nexp_to_ast(b, it)),
        )),
        NExp::Exp(x) => ast_nexp(NExpKind::Exp(Box::new(nexp_to_ast(x, it)))),
        NExp::Neg(x) => ast_nexp(NExpKind::Neg(Box::new(nexp_to_ast(x, it)))),
    }
}

/// Erase an algebra kind onto the canonical kind.
#[must_use]
pub fn nkind_erase(kind: NKind) -> AstKind {
    match kind {
        NKind::Type => AstKind::Type,
        NKind::Int => AstKind::Int,
        NKind::Bool => AstKind::Bool,
    }
}

/// Recover an algebra kind from a canonical one.
#[must_use]
pub fn nkind_from_ast(kind: AstKind) -> NKind {
    match kind {
        AstKind::Type => NKind::Type,
        AstKind::Int => NKind::Int,
        AstKind::Bool => NKind::Bool,
    }
}

/// Lower a canonical type argument onto the algebra. `None` for
/// unresolvable names and unparsable literals below the node.
pub fn typarg_from_ast(a: &AstTypArg, it: &impl Interner) -> Option<TypArg> {
    match &a.kind {
        TypArgKind::NExp(n) => nexp_from_ast(n, it).map(TypArg::NExp),
        TypArgKind::Typ(t) => ntyp_from_ast(t, it).map(TypArg::Typ),
        TypArgKind::Bool(c) => constraint_from_ast(c, it).map(|c| TypArg::Bool(Box::new(c))),
    }
}

/// Lower an algebra type argument onto a canonical node (total).
pub fn typarg_to_ast(a: &TypArg, it: &mut impl Interner) -> AstTypArg {
    match a {
        TypArg::NExp(e) => ast_typarg(TypArgKind::NExp(nexp_to_ast(e, it))),
        TypArg::Typ(t) => ast_typarg(TypArgKind::Typ(ntyp_to_ast(t, it))),
        TypArg::Bool(c) => ast_typarg(TypArgKind::Bool(Box::new(constraint_to_ast(c, it)))),
    }
}

/// Lower a canonical type onto the algebra. `None` for unresolvable
/// names below the node.
pub fn ntyp_from_ast(t: &AstTyp, it: &impl Interner) -> Option<NTyp> {
    match &t.kind {
        TypKind::Unknown => Some(NTyp::Unknown),
        TypKind::Id(name) => op_name(name, it).map(NTyp::Id),
        TypKind::Var(kid) => Some(NTyp::Var(*kid)),
        TypKind::Fn { args, ret } => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(ntyp_from_ast(a, it)?);
            }
            Some(NTyp::Fn(out, Box::new(ntyp_from_ast(ret, it)?)))
        }
        TypKind::Bidir { lhs, rhs } => Some(NTyp::Bidir(
            Box::new(ntyp_from_ast(lhs, it)?),
            Box::new(ntyp_from_ast(rhs, it)?),
        )),
        TypKind::Tuple(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for e in ts {
                out.push(ntyp_from_ast(e, it)?);
            }
            Some(NTyp::Tuple(out))
        }
        TypKind::App { name, args } => {
            let f = op_name(name, it)?;
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(typarg_from_ast(a, it)?);
            }
            Some(NTyp::App(f, out))
        }
        TypKind::Exist { binders, constraint, typ } => {
            let mut out = Vec::with_capacity(binders.len());
            for b in binders {
                out.push(NQuant { kind: nkind_from_ast(b.kind), kid: b.kid });
            }
            Some(NTyp::Exist(
                out,
                Box::new(constraint_from_ast(constraint, it)?),
                Box::new(ntyp_from_ast(typ, it)?),
            ))
        }
    }
}

/// Lower an algebra type onto a canonical node (total).
pub fn ntyp_to_ast(t: &NTyp, it: &mut impl Interner) -> AstTyp {
    match t {
        NTyp::Unknown => ast_typ(TypKind::Unknown),
        NTyp::Id(name) => ast_typ(TypKind::Id(ast_name(it, name))),
        NTyp::Var(kid) => ast_typ(TypKind::Var(*kid)),
        NTyp::Fn(args, ret) => ast_typ(TypKind::Fn {
            args: args.iter().map(|a| ntyp_to_ast(a, it)).collect(),
            ret: Box::new(ntyp_to_ast(ret, it)),
        }),
        NTyp::Bidir(lhs, rhs) => ast_typ(TypKind::Bidir {
            lhs: Box::new(ntyp_to_ast(lhs, it)),
            rhs: Box::new(ntyp_to_ast(rhs, it)),
        }),
        NTyp::Tuple(ts) => ast_typ(TypKind::Tuple(
            ts.iter().map(|e| ntyp_to_ast(e, it)).collect(),
        )),
        NTyp::App(f, args) => ast_typ(TypKind::App {
            name: ast_name(it, f),
            args: args.iter().map(|a| typarg_to_ast(a, it)).collect(),
        }),
        NTyp::Exist(binders, c, ty) => ast_typ(TypKind::Exist {
            binders: binders
                .iter()
                .map(|b| AstKOpt { kind: nkind_erase(b.kind), kid: b.kid })
                .collect(),
            constraint: Box::new(constraint_to_ast(c, it)),
            typ: Box::new(ntyp_to_ast(ty, it)),
        }),
    }
}

/// Lower a canonical constraint onto the algebra. `None` for
/// unresolvable names and unparsable literals below the node.
pub fn constraint_from_ast(c: &AstConstraint, it: &impl Interner) -> Option<NConstraint> {
    match &c.kind {
        ConstraintKind::True => Some(NConstraint::True),
        ConstraintKind::False => Some(NConstraint::False),
        ConstraintKind::Id(name) => op_name(name, it).map(NConstraint::Id),
        ConstraintKind::Var(kid) => Some(NConstraint::Var(*kid)),
        ConstraintKind::Equal(a, b) => Some(NConstraint::Equal(
            Box::new(typarg_from_ast(a, it)?),
            Box::new(typarg_from_ast(b, it)?),
        )),
        ConstraintKind::NotEqual(a, b) => Some(NConstraint::NotEqual(
            Box::new(typarg_from_ast(a, it)?),
            Box::new(typarg_from_ast(b, it)?),
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
        ConstraintKind::Set { nexp, members } => {
            let mut vs = Vec::with_capacity(members.len());
            for m in members {
                vs.push(m.parse::<i128>().ok()?);
            }
            Some(NConstraint::Set(Box::new(nexp_from_ast(nexp, it)?), vs))
        }
        ConstraintKind::And(a, b) => Some(NConstraint::And(
            Box::new(constraint_from_ast(a, it)?),
            Box::new(constraint_from_ast(b, it)?),
        )),
        ConstraintKind::Or(a, b) => Some(NConstraint::Or(
            Box::new(constraint_from_ast(a, it)?),
            Box::new(constraint_from_ast(b, it)?),
        )),
        ConstraintKind::Not(x) => Some(NConstraint::Not(Box::new(constraint_from_ast(x, it)?))),
        ConstraintKind::App { func, args } => {
            let f = op_name(func, it)?;
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(typarg_from_ast(a, it)?);
            }
            Some(NConstraint::App(f, out))
        }
    }
}

/// Lower an algebra constraint onto a canonical node (total).
pub fn constraint_to_ast(c: &NConstraint, it: &mut impl Interner) -> AstConstraint {
    match c {
        NConstraint::True => ast_constraint(ConstraintKind::True),
        NConstraint::False => ast_constraint(ConstraintKind::False),
        NConstraint::Id(name) => ast_constraint(ConstraintKind::Id(ast_name(it, name))),
        NConstraint::Var(kid) => ast_constraint(ConstraintKind::Var(*kid)),
        NConstraint::Equal(a, b) => ast_constraint(ConstraintKind::Equal(
            Box::new(typarg_to_ast(a, it)),
            Box::new(typarg_to_ast(b, it)),
        )),
        NConstraint::NotEqual(a, b) => ast_constraint(ConstraintKind::NotEqual(
            Box::new(typarg_to_ast(a, it)),
            Box::new(typarg_to_ast(b, it)),
        )),
        NConstraint::Ge(a, b) => ast_constraint(ConstraintKind::Ge(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Gt(a, b) => ast_constraint(ConstraintKind::Gt(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Le(a, b) => ast_constraint(ConstraintKind::Le(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Lt(a, b) => ast_constraint(ConstraintKind::Lt(
            nexp_to_ast(a, it),
            nexp_to_ast(b, it),
        )),
        NConstraint::Set(n, vs) => ast_constraint(ConstraintKind::Set {
            nexp: nexp_to_ast(n, it),
            members: vs.iter().map(|v| v.to_string()).collect(),
        }),
        NConstraint::And(a, b) => ast_constraint(ConstraintKind::And(
            Box::new(constraint_to_ast(a, it)),
            Box::new(constraint_to_ast(b, it)),
        )),
        NConstraint::Or(a, b) => ast_constraint(ConstraintKind::Or(
            Box::new(constraint_to_ast(a, it)),
            Box::new(constraint_to_ast(b, it)),
        )),
        NConstraint::Not(x) => {
            ast_constraint(ConstraintKind::Not(Box::new(constraint_to_ast(x, it))))
        }
        NConstraint::App(f, args) => ast_constraint(ConstraintKind::App {
            func: ast_name(it, f),
            args: args.iter().map(|a| typarg_to_ast(a, it)).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::SimpleInterner;

    fn table() -> SimpleInterner {
        SimpleInterner::new()
    }

    #[test]
    fn nexp_roundtrips() {
        let mut it = table();
        let n = it.intern("n");
        let e = NExp::Sum(Box::new(NExp::Var(n)), Box::new(NExp::Constant(2)));
        let ast = nexp_to_ast(&e, &mut it);
        assert_eq!(nexp_from_ast(&ast, &it), Some(e));
    }

    #[test]
    fn typarg_sorts_roundtrip() {
        let mut it = table();
        let n = it.intern("n");
        // A_nexp.
        let a = TypArg::NExp(NExp::Var(n));
        assert_eq!(typarg_from_ast(&typarg_to_ast(&a, &mut it), &it), Some(a));
        // A_typ (`bits('n)`-shaped application).
        let t = TypArg::Typ(NTyp::App("bits".to_string(), vec![TypArg::NExp(NExp::Var(n))]));
        assert_eq!(typarg_from_ast(&typarg_to_ast(&t, &mut it), &it), Some(t));
        // A_bool.
        let b = TypArg::Bool(Box::new(NConstraint::Var(n)));
        assert_eq!(typarg_from_ast(&typarg_to_ast(&b, &mut it), &it), Some(b));
    }

    #[test]
    fn constraint_typ_bool_shapes_roundtrip() {
        let mut it = table();
        let n = it.intern("n");
        // NC_equal over A_typ args.
        let eq = NConstraint::Equal(
            Box::new(TypArg::Typ(NTyp::Var(n))),
            Box::new(TypArg::Typ(NTyp::Var(n))),
        );
        let ast = constraint_to_ast(&eq, &mut it);
        assert!(matches!(ast.kind, ConstraintKind::Equal(_, _)));
        assert_eq!(constraint_from_ast(&ast, &it), Some(eq));
        // NC_app with mixed sorts.
        let app = NConstraint::App(
            "ok".to_string(),
            vec![
                TypArg::NExp(NExp::Constant(1)),
                TypArg::Typ(NTyp::Id("bool".to_string())),
                TypArg::Bool(Box::new(NConstraint::True)),
            ],
        );
        let ast = constraint_to_ast(&app, &mut it);
        assert!(matches!(ast.kind, ConstraintKind::App { .. }));
        assert_eq!(constraint_from_ast(&ast, &it), Some(app));
    }
}
