#![forbid(unsafe_code)]
//! Types, numeric expressions and constraints (`doc_typ`, `doc_nexp`, ...).

use sail_ast::{
    ConstraintKind, Kind, NExp, NExpKind, QuantItem, SimpleInterner, Typ, TypArg, TypArgKind,
    TypKind,
};

use crate::ctx::Ctx;
use crate::doc::{brk, chr, comma_sp, flow, separate, space, string, Doc};
use crate::names::{doc_big_int, doc_id_typ_ctor, doc_kid, name_text};

pub type Result<T> = std::result::Result<T, String>;

fn var_text(it: &SimpleInterner, s: sail_ast::Symbol) -> String {
    sail_ast::Interner::resolve(it, s).unwrap_or("?").to_owned()
}

/// `doc_nexp`.
pub fn doc_nexp(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    atomic(ctx, it, n)
}

fn plussub(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::Sum(a, b) => Ok(separate(&space(), [plussub(ctx, it, a)?, chr('+'), mul(ctx, it, b)?])),
        NExpKind::Minus(a, b) => Ok(separate(&space(), [plussub(ctx, it, a)?, chr('-'), mul(ctx, it, b)?])),
        _ => mul(ctx, it, n),
    }
}

fn mul(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::Times(a, b) => Ok(separate(&space(), [mul(ctx, it, a)?, chr('*'), uneg(ctx, it, b)?])),
        _ => uneg(ctx, it, n),
    }
}

fn uneg(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::Neg(a) => Ok(separate(&space(), [chr('-'), uneg(ctx, it, a)?]).parens()),
        _ => exp(ctx, it, n),
    }
}

fn exp(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::Exp(a) => Ok(separate(&space(), [string("2"), chr('^'), exp(ctx, it, a)?])),
        _ => app(ctx, it, n),
    }
}

fn app(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => Ok(separate(
            &space(),
            [
                string("if ("),
                doc_nconstraint(ctx, it, cond)?,
                string(" : Bool) then"),
                atomic(ctx, it, then_branch)?,
                string("else"),
                atomic(ctx, it, else_branch)?,
            ],
        )),
        NExpKind::App { func, args } if name_text(func, it) == "div" && args.len() == 2 => Ok(separate(
            &space(),
            [atomic(ctx, it, &args[0])?, chr('/'), atomic(ctx, it, &args[1])?],
        )),
        NExpKind::App { func, args } if name_text(func, it) == "mod" && args.len() == 2 => Ok(separate(
            &space(),
            [atomic(ctx, it, &args[0])?, chr('%'), atomic(ctx, it, &args[1])?],
        )),
        NExpKind::App { func, args } if name_text(func, it) == "abs" && args.len() == 1 => Ok(separate(
            &chr('.'),
            [atomic(ctx, it, &args[0])?, string("natAbs")],
        )),
        _ => atomic(ctx, it, n),
    }
}

fn atomic(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> Result<Doc> {
    match &n.kind {
        NExpKind::Constant(c) => Ok(doc_big_int(c)),
        NExpKind::Var(s) => Ok(doc_kid(ctx, &var_text(it, *s))),
        NExpKind::Id(id) => Ok(crate::names::doc_id_ctor(ctx, it, id, false)),
        NExpKind::Sum(..)
        | NExpKind::Minus(..)
        | NExpKind::Times(..)
        | NExpKind::Neg(..)
        | NExpKind::Exp(..)
        | NExpKind::If { .. } => Ok(plussub(ctx, it, n)?.parens()),
        NExpKind::App { func, args } => {
            let name = name_text(func, it);
            if (matches!(name.as_str(), "div" | "mod") && args.len() == 2)
                || (name == "abs" && args.len() == 1)
            {
                Ok(plussub(ctx, it, n)?.parens())
            } else {
                Err(format!("NExp application {name} not translatable yet"))
            }
        }
    }
}

/// `doc_nconstraint`.
pub fn doc_nconstraint(ctx: &Ctx<'_>, it: &SimpleInterner, c: &sail_ast::Constraint) -> Result<Doc> {
    let bin = |l: Doc, op: &str, r: Doc| flow(&brk(1), [l, string(op), r]);
    Ok(match &c.kind {
        ConstraintKind::And(a, b) => bin(doc_nconstraint(ctx, it, a)?, "∧", doc_nconstraint(ctx, it, b)?),
        ConstraintKind::Or(a, b) => bin(doc_nconstraint(ctx, it, a)?, "∨", doc_nconstraint(ctx, it, b)?),
        ConstraintKind::Equal(a, b) => bin(doc_typ_arg(ctx, it, true, a)?, "=", doc_typ_arg(ctx, it, true, b)?),
        ConstraintKind::NotEqual(a, b) => bin(doc_typ_arg(ctx, it, true, a)?, "≠", doc_typ_arg(ctx, it, true, b)?),
        ConstraintKind::App { func, args } => {
            let mut parts = vec![crate::names::doc_id_ctor(ctx, it, func, false)];
            for a in args {
                parts.push(doc_typ_arg(ctx, it, true, a)?);
            }
            flow(&brk(1), parts).parens()
        }
        ConstraintKind::False => string("false"),
        ConstraintKind::True => string("true"),
        ConstraintKind::Ge(a, b) => bin(doc_nexp(ctx, it, a)?, "≥", doc_nexp(ctx, it, b)?),
        ConstraintKind::Le(a, b) => bin(doc_nexp(ctx, it, a)?, "≤", doc_nexp(ctx, it, b)?),
        ConstraintKind::Gt(a, b) => bin(doc_nexp(ctx, it, a)?, ">", doc_nexp(ctx, it, b)?),
        ConstraintKind::Lt(a, b) => bin(doc_nexp(ctx, it, a)?, "<", doc_nexp(ctx, it, b)?),
        ConstraintKind::Id(id) => crate::names::doc_id_ctor(ctx, it, id, false),
        ConstraintKind::Set { nexp, members } => {
            let items: Vec<Doc> = members.iter().map(|m| string(m)).collect();
            flow(
                &brk(1),
                [
                    string("List.elem"),
                    doc_nexp(ctx, it, nexp)?,
                    separate(&comma_sp(), items).brackets(),
                ],
            )
        }
        ConstraintKind::Var(s) => doc_kid(ctx, &var_text(it, *s)),
        // OCaml spells negation as the application `not(c)`.
        ConstraintKind::Not(a) => flow(&brk(1), [string("not"), doc_nconstraint(ctx, it, a)?.parens()]).parens(),
    })
}

/// `doc_typ_arg ctx env rel` with `all` = `` `All `` (false = `` `Only_relevant ``).
pub fn doc_typ_arg(ctx: &Ctx<'_>, it: &SimpleInterner, all: bool, a: &TypArg) -> Result<Doc> {
    match &a.kind {
        TypArgKind::Typ(t) => doc_typ(ctx, it, t),
        TypArgKind::NExp(n) => doc_nexp(ctx, it, n),
        TypArgKind::Bool(c) => {
            if all {
                Ok(doc_nconstraint(ctx, it, c)?.parens())
            } else {
                Ok(crate::doc::empty())
            }
        }
    }
}

/// `provably_nneg` over the current quantifier assumptions.
pub fn provably_nneg(ctx: &Ctx<'_>, it: &SimpleInterner, n: &NExp) -> bool {
    match ctx.quant {
        Some((quant, quant_it)) => ctx.global.oracle.provably_nneg(quant, quant_it, n, it),
        None => ctx.global.oracle.provably_nneg(&[], it, n, it),
    }
}

fn arg_nexp<'t>(a: &'t TypArg) -> Option<&'t NExp> {
    match &a.kind {
        TypArgKind::NExp(n) => Some(n),
        _ => None,
    }
}

fn arg_typ<'t>(a: &'t TypArg) -> Option<&'t Typ> {
    match &a.kind {
        TypArgKind::Typ(t) => Some(t),
        _ => None,
    }
}

/// `doc_typ`.
pub fn doc_typ(ctx: &Ctx<'_>, it: &SimpleInterner, t: &Typ) -> Result<Doc> {
    match &t.kind {
        TypKind::App { name, args } => {
            let n = name_text(name, it);
            match (n.as_str(), args.as_slice()) {
                ("vector", [m, e]) if arg_nexp(m).is_some() && arg_typ(e).is_some() => Ok(flow(
                    &space(),
                    [
                        string("Vector"),
                        doc_typ(ctx, it, arg_typ(e).unwrap())?,
                        doc_nexp(ctx, it, arg_nexp(m).unwrap())?,
                    ],
                )
                .parens()
                .nest(2)),
                ("atom_bool", _) => Ok(string("Bool")),
                ("bitvector" | "bits", [m]) if arg_nexp(m).is_some() => {
                    Ok(string("BitVec ").cat(doc_nexp(ctx, it, arg_nexp(m).unwrap())?).parens())
                }
                ("atom", [x]) if arg_nexp(x).is_some() => {
                    Ok(string(if provably_nneg(ctx, it, arg_nexp(x).unwrap()) { "Nat" } else { "Int" }))
                }
                ("register", rest) => {
                    let docs: Vec<Doc> = rest
                        .iter()
                        .map(|a| doc_typ_app(ctx, it, a))
                        .collect::<Result<_>>()?;
                    Ok(string("RegisterRef ").cat(separate(&chr(','), docs)).parens())
                }
                ("implicit", [x]) if matches!(arg_nexp(x).map(|n| &n.kind), Some(NExpKind::Var(_))) => {
                    Ok(chr('_'))
                }
                ("option", [x]) if arg_typ(x).is_some() => {
                    Ok(string("Option ").cat(doc_typ(ctx, it, arg_typ(x).unwrap())?).parens())
                }
                ("list", rest) => {
                    let docs: Vec<Doc> = rest
                        .iter()
                        .map(|a| doc_typ_arg(ctx, it, false, a))
                        .collect::<Result<_>>()?;
                    Ok(string("List").cat(space()).cat(separate(&space(), docs)).parens())
                }
                ("range", [lo, _hi]) if arg_nexp(lo).is_some() => {
                    Ok(string(if provably_nneg(ctx, it, arg_nexp(lo).unwrap()) { "Nat" } else { "Int" }))
                }
                ("result", [a, b]) if arg_typ(a).is_some() && arg_typ(b).is_some() => Ok(separate(
                    &space(),
                    [
                        string("Result"),
                        doc_typ(ctx, it, arg_typ(a).unwrap())?,
                        doc_typ(ctx, it, arg_typ(b).unwrap())?,
                    ],
                )
                .parens()),
                _ => {
                    let docs: Vec<Doc> = args
                        .iter()
                        .map(|a| doc_typ_arg(ctx, it, false, a))
                        .collect::<Result<_>>()?;
                    Ok(doc_id_typ_ctor(ctx, &n).cat(space()).cat(separate(&space(), docs)).parens())
                }
            }
        }
        TypKind::Id(id) => {
            let n = name_text(id, it);
            Ok(match n.as_str() {
                "unit" => string("Unit"),
                "int" => string("Int"),
                "string" => string("String"),
                "bool" => string("Bool"),
                "bit" => string("BitVec 1").parens(),
                "nat" => string("Nat"),
                _ => doc_id_typ_ctor(ctx, &n),
            })
        }
        TypKind::Tuple(ts) => {
            let docs: Vec<Doc> = ts.iter().map(|t| doc_typ(ctx, it, t)).collect::<Result<_>>()?;
            Ok(separate(&space().cat(string("×")).cat(space()), docs).parens())
        }
        TypKind::Var(s) => Ok(doc_kid(ctx, &var_text(it, *s))),
        TypKind::Exist { binders, typ, .. } => {
            let mut inner = ctx.clone();
            for b in binders {
                inner.add_kid_id_rename("_lean_wildcard", &var_text(it, b.kid));
            }
            doc_typ(&inner, it, typ)
        }
        TypKind::Fn { .. } => Err("Type Typ_fn not translatable yet".into()),
        TypKind::Bidir { .. } => Err("Type Typ_bidir not translatable yet".into()),
        TypKind::Unknown => Err("Type Typ_internal_unknown not translatable yet".into()),
    }
}

/// `doc_typ_app`.
pub fn doc_typ_app(ctx: &Ctx<'_>, it: &SimpleInterner, a: &TypArg) -> Result<Doc> {
    match &a.kind {
        TypArgKind::Typ(t) => doc_typ(ctx, it, t),
        TypArgKind::Bool(c) => doc_nconstraint(ctx, it, c),
        TypArgKind::NExp(n) => doc_nexp(ctx, it, n),
    }
}

/// `doc_kind`.
pub fn doc_kind(ctx: &Ctx<'_>, it: &SimpleInterner, kid: sail_ast::Symbol, kind: Kind) -> Doc {
    match kind {
        Kind::Int => {
            let n = NExp::new(
                sail_ast::NodeId::DUMMY,
                sail_ast::Span::DUMMY,
                NExpKind::Var(kid),
            );
            string(if provably_nneg(ctx, it, &n) { "Nat" } else { "Int" })
        }
        Kind::Bool => string("Bool"),
        Kind::Type => string("Type"),
    }
}

/// `doc_quant_item_all`.
pub fn doc_quant_item_all(ctx: &Ctx<'_>, it: &SimpleInterner, q: &QuantItem) -> Result<Doc> {
    Ok(match q {
        QuantItem::Id(k) => flow(
            &brk(1),
            [doc_kid(ctx, &var_text(it, k.kid)), chr(':'), doc_kind(ctx, it, k.kid, k.kind)],
        ),
        QuantItem::Constraint(c) => doc_nconstraint(ctx, it, c)?,
    })
}

/// `doc_typ_quant_in_comment`.
pub fn doc_typ_quant_in_comment(ctx: &Ctx<'_>, it: &SimpleInterner, quant: &[QuantItem]) -> Result<Doc> {
    let items: Vec<Doc> = quant
        .iter()
        .map(|q| doc_quant_item_all(ctx, it, q))
        .collect::<Result<_>>()?;
    if items.is_empty() {
        return Ok(crate::doc::empty());
    }
    Ok(string("/-- Type quantifiers: ")
        .cat(separate(&comma_sp(), items).nest(2))
        .cat(string(" -/"))
        .cat(crate::doc::hardline()))
}

/// `doc_typ_quant_relevant`: binders only, with the constraints in scope for `Nat`.
pub fn doc_typ_quant_relevant(ctx: &Ctx<'_>, it: &SimpleInterner, quant: &[QuantItem]) -> Vec<Doc> {
    let mut inner = ctx.clone();
    inner.quant = None;
    let local = (quant, it);
    // Re-borrow with a shorter lifetime for the call below.
    let inner_ctx = Ctx {
        quant: Some(local),
        ..inner
    };
    quant
        .iter()
        .filter_map(|q| match q {
            QuantItem::Id(k) => Some(flow(
                &brk(1),
                [
                    doc_kid(&inner_ctx, &var_text(it, k.kid)),
                    chr(':'),
                    doc_kind(&inner_ctx, it, k.kid, k.kind),
                ],
            )),
            QuantItem::Constraint(_) => None,
        })
        .collect()
}

/// `doc_typ_quant_only_vars`.
pub fn doc_typ_quant_only_vars(ctx: &Ctx<'_>, it: &SimpleInterner, quant: &[QuantItem]) -> Vec<Doc> {
    quant
        .iter()
        .filter_map(|q| match q {
            QuantItem::Id(k) => Some(doc_kid(ctx, &var_text(it, k.kid))),
            QuantItem::Constraint(_) => None,
        })
        .collect()
}

