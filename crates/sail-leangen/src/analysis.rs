#![forbid(unsafe_code)]
//! Expression predicates (`has_loop`, `has_effect`, `has_early_return`).

use sail_ast::{Exp, ExpKind, LExp, LExpKind, PExp, PExpKind, SimpleInterner};

use crate::ctx::Oracle;
use crate::names::name_text;

/// Visit every expression node (pre-order); stops early when `visit` is true.
pub fn exp_exists(e: &Exp, visit: &mut dyn FnMut(&Exp) -> bool) -> bool {
    if visit(e) {
        return true;
    }
    let sub = |x: &Exp, visit: &mut dyn FnMut(&Exp) -> bool| exp_exists(x, visit);
    match &e.kind {
        ExpKind::Block(items) | ExpKind::Tuple(items) | ExpKind::Vector(items) | ExpKind::List(items) => {
            items.iter().any(|x| sub(x, visit))
        }
        ExpKind::Id(_) | ExpKind::Lit(_) | ExpKind::Sizeof(_) | ExpKind::Config(_) | ExpKind::Ref(_)
        | ExpKind::Undef | ExpKind::InternalValue(_) | ExpKind::Constraint(_) => false,
        ExpKind::Typ { exp, .. } => sub(exp, visit),
        ExpKind::App { args, .. } => args.iter().any(|x| sub(x, visit)),
        ExpKind::If { cond, then_branch, else_branch } => {
            sub(cond, visit) || sub(then_branch, visit) || sub(else_branch, visit)
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            measure.as_ref().is_some_and(|m| sub(m, visit)) || sub(cond, visit) || sub(body, visit)
        }
        ExpKind::For { from, to, step, body, .. } => {
            sub(from, visit) || sub(to, visit) || sub(step, visit) || sub(body, visit)
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => sub(a, visit) || sub(b, visit),
        ExpKind::Struct { fields, .. } => fields.iter().any(|f| sub(&f.value, visit)),
        ExpKind::StructUpdate { record, fields } => {
            sub(record, visit) || fields.iter().any(|f| sub(&f.value, visit))
        }
        ExpKind::Field { record, .. } => sub(record, visit),
        ExpKind::Match { scrutinee, arms } => sub(scrutinee, visit) || arms.iter().any(|a| pexp_exists(a, visit)),
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            sub(bind, visit) || sub(body, visit)
        }
        ExpKind::Assign { lexp, value } => lexp_exists(lexp, visit) || sub(value, visit),
        ExpKind::Return(x) | ExpKind::Exit(x) | ExpKind::Throw(x) | ExpKind::InternalReturn(x) => sub(x, visit),
        ExpKind::Try { exp, arms } => sub(exp, visit) || arms.iter().any(|a| pexp_exists(a, visit)),
        ExpKind::Assert { cond, msg } => sub(cond, visit) || sub(msg, visit),
        ExpKind::Var { lexp, init, body } => lexp_exists(lexp, visit) || sub(init, visit) || sub(body, visit),
        ExpKind::InternalAssume { exp, .. } => sub(exp, visit),
    }
}

pub fn pexp_exists(p: &PExp, visit: &mut dyn FnMut(&Exp) -> bool) -> bool {
    match &p.kind {
        PExpKind::PatExp { body, .. } => exp_exists(body, visit),
        PExpKind::PatWhen { guard, body, .. } => exp_exists(guard, visit) || exp_exists(body, visit),
    }
}

fn lexp_exists(l: &LExp, visit: &mut dyn FnMut(&Exp) -> bool) -> bool {
    match &l.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } => false,
        LExpKind::Deref(e) => exp_exists(e, visit),
        LExpKind::Tuple(items) | LExpKind::VectorConcat(items) => items.iter().any(|x| lexp_exists(x, visit)),
        LExpKind::Vector { lexp, index } => lexp_exists(lexp, visit) || exp_exists(index, visit),
        LExpKind::VectorRange { lexp, lo, hi } => {
            lexp_exists(lexp, visit) || exp_exists(lo, visit) || exp_exists(hi, visit)
        }
        LExpKind::Field { lexp, .. } => lexp_exists(lexp, visit),
    }
}

#[must_use]
pub fn is_loop_id(name: &str) -> bool {
    matches!(name, "while#" | "while#t" | "foreach#" | "until#" | "until#t")
}

#[must_use]
pub fn is_effect_id(name: &str) -> bool {
    matches!(name, "while#" | "foreach#" | "early_return")
}

fn app_name(e: &Exp, it: &SimpleInterner) -> Option<String> {
    match &e.kind {
        ExpKind::App { func, .. } => Some(name_text(func, it)),
        _ => None,
    }
}

/// `has_loop`.
#[must_use]
pub fn has_loop(e: &Exp, it: &SimpleInterner) -> bool {
    exp_exists(e, &mut |x| {
        matches!(x.kind, ExpKind::Return(_)) || app_name(x, it).is_some_and(|n| is_loop_id(&n))
    })
}

/// `has_effect_app`.
#[must_use]
pub fn has_effect_app(e: &Exp, it: &SimpleInterner) -> bool {
    exp_exists(e, &mut |x| {
        matches!(x.kind, ExpKind::Return(_)) || app_name(x, it).is_some_and(|n| is_effect_id(&n))
    })
}

/// `has_early_return`.
#[must_use]
pub fn has_early_return(e: &Exp, it: &SimpleInterner) -> bool {
    exp_exists(e, &mut |x| {
        matches!(x.kind, ExpKind::Return(_)) || app_name(x, it).is_some_and(|n| n == "early_return")
    })
}

fn has_incomplete(e: &Exp) -> bool {
    e.annotation
        .as_ref()
        .is_some_and(|a| a.attributes.iter().any(|attr| attr.name == "incomplete"))
}

/// `effectful (effect_of e)`: register access, throw, exit/assert, undefined,
/// calls of impure functions, incomplete matches, terminating loops.
#[must_use]
pub fn effectful(e: &Exp, it: &SimpleInterner, oracle: &dyn Oracle, asserts_termination: bool) -> bool {
    exp_exists(e, &mut |x| match &x.kind {
        ExpKind::Id(id) => oracle.is_register(&name_text(id, it)),
        ExpKind::Undef | ExpKind::Throw(_) | ExpKind::Exit(_) | ExpKind::Assert { .. } => true,
        ExpKind::App { func, .. } => {
            let name = name_text(func, it);
            name == "__deref" || !oracle.function_is_pure(&name)
        }
        ExpKind::Match { .. } => has_incomplete(x),
        ExpKind::Loop { measure, .. } => asserts_termination && measure.is_some(),
        ExpKind::Assign { lexp, .. } => lexp_register_write(lexp, it, oracle),
        ExpKind::Var { lexp, .. } => lexp_register_write(lexp, it, oracle),
        _ => false,
    })
}

fn lexp_register_write(l: &LExp, it: &SimpleInterner, oracle: &dyn Oracle) -> bool {
    match &l.kind {
        LExpKind::Id(id) | LExpKind::Typ { id, .. } => oracle.is_register(&name_text(id, it)),
        LExpKind::Deref(_) => true,
        LExpKind::Tuple(items) | LExpKind::VectorConcat(items) => {
            items.iter().any(|x| lexp_register_write(x, it, oracle))
        }
        LExpKind::Vector { lexp, .. } | LExpKind::VectorRange { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_register_write(lexp, it, oracle)
        }
    }
}

/// `has_effect e = effectful (effect_of e) || has_effect_app e`.
#[must_use]
pub fn has_effect(e: &Exp, it: &SimpleInterner, oracle: &dyn Oracle) -> bool {
    effectful(e, it, oracle, true) || has_effect_app(e, it)
}

/// Annotated type of an expression (`typ_of`), if elaboration set one.
fn annotated_typ(e: &Exp) -> Option<&sail_ast::Typ> {
    e.annotation.as_ref().and_then(|a| a.typ.as_deref())
}

/// `is_unit (typ_of e)`, when the node carries a type.
#[must_use]
pub fn exp_is_unit(e: &Exp, it: &SimpleInterner, oracle: &dyn Oracle) -> Option<bool> {
    let t = annotated_typ(e)?;
    Some(oracle.typ_head(t, it).is_some_and(|h| h == "unit"))
}

/// `is_bitvector_typ (typ_of e)`, when the node carries a type.
#[must_use]
pub fn exp_is_bitvector(e: &Exp, it: &SimpleInterner, oracle: &dyn Oracle) -> Option<bool> {
    let t = annotated_typ(e)?;
    Some(oracle.typ_head(t, it).is_some_and(|h| h == "bits" || h == "bitvector"))
}

/// Constant width of a pattern of type `bits(n)`/`bitvector(n)`.
#[must_use]
pub fn pat_const_bits(p: &sail_ast::Pat, it: &SimpleInterner) -> Option<i128> {
    use sail_ast::{NExpKind, TypArgKind, TypKind};
    let t = p.annotation.as_ref().and_then(|a| a.typ.as_deref())?;
    if let TypKind::App { name, args } = &t.kind {
        let n = name_text(name, it);
        if n == "bits" || n == "bitvector" {
            if let [a] = args.as_slice() {
                if let TypArgKind::NExp(ne) = &a.kind {
                    if let NExpKind::Constant(c) = &ne.kind {
                        return c.parse().ok();
                    }
                }
            }
        }
    }
    None
}
