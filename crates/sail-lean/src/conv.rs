#![forbid(unsafe_code)]
//! Bridge from canonical `sail-ast` nodes onto the printer subset
//! (Lane 61 unification, MIGRATION.md #29–#31).
//!
//! This is the future printer input path: canonical nodes lower onto
//! [`Nexp`][crate::Nexp]/[`Typ`][crate::Typ]/[`TypArg`][crate::TypArg]
//! wherever the subset covers them, `None` everywhere else:
//! - `NExpKind::{Constant,Var,Sum,Minus,Times,Neg}` map 1:1;
//!   `Exp` is the printer's `Pow2`; `If/Id/App` have no printer
//!   counterpart (div/mod/abs print via extern calls, Lane 25).
//! - Nullary `Typ_id` names lower through the `doc_typ` core:
//!   `int/nat/bool/string/unit` become [`Typ::Int`]/`Nat`/`Bool`/
//!   `String`/`Unit`; any other `Id`/`App` becomes [`Typ::Named`]
//!   (full `doc_typ` application mapping stays Lane 10).
//! - `Typ_tuple` maps when every element maps; `Typ_var` maps to
//!   [`Typ::Var`]; `Typ_fn`/`Typ_bidir`/`Typ_exist`/`Typ_unknown`
//!   map to `None`.
//! - `TypArgKind::{Typ,NExp}` map when their payload maps;
//!   `Bool` has no printer counterpart (`None`).
//!
//! The printer logic itself is untouched.

use sail_ast::{Interner, NExp as AstNExp, NExpKind, Typ as AstTyp, TypArg as AstTypArg, TypArgKind, TypKind};

use crate::{Nexp, Typ, TypArg};

/// Lower a canonical nexp onto the printer subset.
pub fn nexp_from_ast(n: &AstNExp, it: &impl Interner) -> Option<Nexp> {
    match &n.kind {
        NExpKind::Constant(s) => s.parse::<i128>().ok().map(Nexp::Const),
        NExpKind::Var(sym) => it.resolve(*sym).map(|s| Nexp::Var(s.to_owned())),
        NExpKind::Sum(a, b) => Some(Nexp::Sum(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Minus(a, b) => Some(Nexp::Minus(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Times(a, b) => Some(Nexp::Times(
            Box::new(nexp_from_ast(a, it)?),
            Box::new(nexp_from_ast(b, it)?),
        )),
        NExpKind::Neg(a) => Some(Nexp::Neg(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::Exp(a) => Some(Nexp::Pow2(Box::new(nexp_from_ast(a, it)?))),
        NExpKind::If { .. } | NExpKind::Id(_) | NExpKind::App { .. } => None,
    }
}

fn named_type(name: &str) -> Option<Typ> {
    match name {
        "int" => Some(Typ::Int),
        "nat" => Some(Typ::Nat),
        "bool" => Some(Typ::Bool),
        "string" => Some(Typ::String),
        "unit" => Some(Typ::Unit),
        _ => None,
    }
}

/// Lower a canonical type onto the printer subset.
pub fn typ_from_ast(t: &AstTyp, it: &impl Interner) -> Option<Typ> {
    match &t.kind {
        TypKind::Id(name) => {
            let text = it.resolve(name.symbol)?;
            named_type(text).or(Some(Typ::Named {
                name: text.to_owned(),
                args: Vec::new(),
                span: t.span,
            }))
        }
        TypKind::Var(sym) => it.resolve(*sym).map(|s| Typ::Var(s.to_owned())),
        TypKind::Tuple(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for e in ts {
                out.push(typ_from_ast(e, it)?);
            }
            Some(Typ::Tuple(out))
        }
        TypKind::App { name, args } => {
            let text = it.resolve(name.symbol)?;
            if args.is_empty() {
                return named_type(text).or(Some(Typ::Named {
                    name: text.to_owned(),
                    args: Vec::new(),
                    span: t.span,
                }));
            }
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(typarg_from_ast(a, it)?);
            }
            Some(Typ::Named { name: text.to_owned(), args: out, span: t.span })
        }
        TypKind::Unknown
        | TypKind::Fn { .. }
        | TypKind::Bidir { .. }
        | TypKind::Exist { .. } => None,
    }
}

/// Lower a canonical type argument onto the printer subset.
pub fn typarg_from_ast(a: &AstTypArg, it: &impl Interner) -> Option<TypArg> {
    match &a.kind {
        TypArgKind::Typ(t) => typ_from_ast(t, it).map(TypArg::Typ),
        TypArgKind::NExp(n) => nexp_from_ast(n, it).map(TypArg::Nexp),
        TypArgKind::Bool(_) => None,
    }
}
