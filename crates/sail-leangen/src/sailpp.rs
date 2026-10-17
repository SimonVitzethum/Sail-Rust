#![forbid(unsafe_code)]
//! A compact Sail-like printer for debugging rewrites (not Lean output).

use std::fmt::Write;

use sail_ast::{
    Def, DefKind, Exp, ExpKind, Interner, LExp, LExpKind, LitKind, Pat, PatKind, PExp, PExpKind, SimpleInterner,
};

fn id(it: &SimpleInterner, n: &sail_ast::Name) -> String {
    Interner::resolve(it, n.symbol).unwrap_or("?").to_owned()
}

fn lit(it: &SimpleInterner, l: &sail_ast::Lit) -> String {
    let t = |s: sail_ast::Symbol| Interner::resolve(it, s).unwrap_or("?").to_owned();
    match &l.kind {
        LitKind::Unit => "()".into(),
        LitKind::True => "true".into(),
        LitKind::False => "false".into(),
        LitKind::Num(n) => t(*n),
        LitKind::Hex(h) => format!("0x{}", t(*h)),
        LitKind::Bin(b) => format!("0b{}", t(*b)),
        LitKind::String(s) => format!("{:?}", t(s.to_owned())),
        LitKind::Real(r) => t(*r),
    }
}

#[must_use]
pub fn pat_to_string(it: &SimpleInterner, p: &Pat) -> String {
    match &p.kind {
        PatKind::Wild => "_".into(),
        PatKind::Lit(l) => lit(it, l),
        PatKind::Id(n) => id(it, n),
        PatKind::Typ { pat, .. } => format!("({} : T)", pat_to_string(it, pat)),
        PatKind::As { pat, id: n } => format!("({} as {})", pat_to_string(it, pat), id(it, n)),
        PatKind::Var { pat, .. } => format!("{}'", pat_to_string(it, pat)),
        PatKind::App { func, args } => format!(
            "{}({})",
            id(it, func),
            args.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(", ")
        ),
        PatKind::Tuple(ps) => format!("({})", ps.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(", ")),
        PatKind::Vector(ps) => format!("[{}]", ps.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(", ")),
        PatKind::VectorConcat(ps) => ps.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(" @ "),
        PatKind::List(ps) => format!("[|{}|]", ps.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(", ")),
        PatKind::Cons(a, b) => format!("{} :: {}", pat_to_string(it, a), pat_to_string(it, b)),
        PatKind::Struct { fields, .. } => format!(
            "struct {{{}}}",
            fields
                .iter()
                .map(|(n, p)| format!("{} = {}", id(it, n), pat_to_string(it, p)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PatKind::Or(a, b) => format!("{} | {}", pat_to_string(it, a), pat_to_string(it, b)),
        PatKind::Not(a) => format!("~{}", pat_to_string(it, a)),
        PatKind::VectorSubrange { id: n, lo, hi } => format!("{}[{hi} .. {lo}]", id(it, n)),
        PatKind::StringAppend(ps) => ps.iter().map(|a| pat_to_string(it, a)).collect::<Vec<_>>().join(" ^ "),
    }
}

fn lexp_to_string(it: &SimpleInterner, l: &LExp) -> String {
    match &l.kind {
        LExpKind::Id(n) => id(it, n),
        LExpKind::Typ { id: n, .. } => format!("({} : T)", id(it, n)),
        LExpKind::Deref(e) => format!("*({})", exp_to_string(it, e)),
        LExpKind::Tuple(ls) => format!("({})", ls.iter().map(|a| lexp_to_string(it, a)).collect::<Vec<_>>().join(", ")),
        LExpKind::VectorConcat(ls) => ls.iter().map(|a| lexp_to_string(it, a)).collect::<Vec<_>>().join(" @ "),
        LExpKind::Vector { lexp, index } => format!("{}[{}]", lexp_to_string(it, lexp), exp_to_string(it, index)),
        LExpKind::VectorRange { lexp, lo, hi } => format!(
            "{}[{} .. {}]",
            lexp_to_string(it, lexp),
            exp_to_string(it, hi),
            exp_to_string(it, lo)
        ),
        LExpKind::Field { lexp, field } => format!("{}.{}", lexp_to_string(it, lexp), id(it, field)),
    }
}

fn pexp_to_string(it: &SimpleInterner, p: &PExp) -> String {
    match &p.kind {
        PExpKind::PatExp { pat, body } => format!("{} => {}", pat_to_string(it, pat), exp_to_string(it, body)),
        PExpKind::PatWhen { pat, guard, body } => format!(
            "{} if {} => {}",
            pat_to_string(it, pat),
            exp_to_string(it, guard),
            exp_to_string(it, body)
        ),
    }
}

#[must_use]
pub fn exp_to_string(it: &SimpleInterner, e: &Exp) -> String {
    let list = |xs: &[Exp]| xs.iter().map(|x| exp_to_string(it, x)).collect::<Vec<_>>().join(", ");
    match &e.kind {
        ExpKind::Block(items) => format!("{{ {} }}", items.iter().map(|x| exp_to_string(it, x)).collect::<Vec<_>>().join("; ")),
        ExpKind::Id(n) => id(it, n),
        ExpKind::Lit(l) => lit(it, l),
        ExpKind::Typ { exp, .. } => format!("({} : T)", exp_to_string(it, exp)),
        ExpKind::App { func, args } => format!("{}({})", id(it, func), list(args)),
        ExpKind::Tuple(xs) => format!("({})", list(xs)),
        ExpKind::If { cond, then_branch, else_branch } => format!(
            "if {} then {} else {}",
            exp_to_string(it, cond),
            exp_to_string(it, then_branch),
            exp_to_string(it, else_branch)
        ),
        ExpKind::Loop { cond, body, .. } => format!("loop({}, {})", exp_to_string(it, cond), exp_to_string(it, body)),
        ExpKind::For { var, from, to, step, body, .. } => format!(
            "foreach ({} from {} to {} by {}) {}",
            id(it, var),
            exp_to_string(it, from),
            exp_to_string(it, to),
            exp_to_string(it, step),
            exp_to_string(it, body)
        ),
        ExpKind::Vector(xs) => format!("[{}]", list(xs)),
        ExpKind::VectorAppend(a, b) => format!("{} @ {}", exp_to_string(it, a), exp_to_string(it, b)),
        ExpKind::List(xs) => format!("[|{}|]", list(xs)),
        ExpKind::Cons(a, b) => format!("{} :: {}", exp_to_string(it, a), exp_to_string(it, b)),
        ExpKind::Struct { fields, .. } => format!(
            "struct {{{}}}",
            fields
                .iter()
                .map(|f| format!("{} = {}", id(it, &f.field), exp_to_string(it, &f.value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ExpKind::StructUpdate { record, fields } => format!(
            "{{{} with {}}}",
            exp_to_string(it, record),
            fields
                .iter()
                .map(|f| format!("{} = {}", id(it, &f.field), exp_to_string(it, &f.value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ExpKind::Field { record, field } => format!("{}.{}", exp_to_string(it, record), id(it, field)),
        ExpKind::Match { scrutinee, arms } => format!(
            "match {} {{ {} }}",
            exp_to_string(it, scrutinee),
            arms.iter().map(|a| pexp_to_string(it, a)).collect::<Vec<_>>().join(", ")
        ),
        ExpKind::Let { pat, bind, body } => format!(
            "let {} = {} in {}",
            pat_to_string(it, pat),
            exp_to_string(it, bind),
            exp_to_string(it, body)
        ),
        ExpKind::InternalPlet { pat, bind, body } => format!(
            "plet {} = {} in {}",
            pat_to_string(it, pat),
            exp_to_string(it, bind),
            exp_to_string(it, body)
        ),
        ExpKind::Assign { lexp, value } => format!("{} = {}", lexp_to_string(it, lexp), exp_to_string(it, value)),
        ExpKind::Sizeof(_) => "sizeof(..)".into(),
        ExpKind::Return(x) => format!("return({})", exp_to_string(it, x)),
        ExpKind::InternalReturn(x) => format!("ireturn({})", exp_to_string(it, x)),
        ExpKind::Exit(x) => format!("exit({})", exp_to_string(it, x)),
        ExpKind::Config(k) => format!("%{}", k.join(".")),
        ExpKind::Ref(n) => format!("ref {}", id(it, n)),
        ExpKind::Throw(x) => format!("throw({})", exp_to_string(it, x)),
        ExpKind::Try { exp, arms } => format!(
            "try {} catch {{ {} }}",
            exp_to_string(it, exp),
            arms.iter().map(|a| pexp_to_string(it, a)).collect::<Vec<_>>().join(", ")
        ),
        ExpKind::Assert { cond, msg } => format!("assert({}, {})", exp_to_string(it, cond), exp_to_string(it, msg)),
        ExpKind::Var { lexp, init, body } => format!(
            "var {} = {} in {}",
            lexp_to_string(it, lexp),
            exp_to_string(it, init),
            exp_to_string(it, body)
        ),
        ExpKind::Undef => "undefined".into(),
        ExpKind::InternalValue(_) => "<value>".into(),
        ExpKind::InternalAssume { exp, .. } => format!("assume(.., {})", exp_to_string(it, exp)),
        ExpKind::Constraint(_) => "constraint(..)".into(),
    }
}

/// One-line rendering of a definition (functions show their clauses).
#[must_use]
pub fn def_to_string(it: &SimpleInterner, def: &Def) -> String {
    let mut out = String::new();
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            for f in funcls {
                let _ = writeln!(out, "function {} {}", id(it, &f.name), pexp_to_string(it, &f.arm));
            }
        }
        DefKind::Let { pat, exp } => {
            let _ = writeln!(out, "let {} = {}", pat_to_string(it, pat), exp_to_string(it, exp));
        }
        other => {
            let _ = writeln!(out, "<{:?}>", std::mem::discriminant(other));
        }
    }
    out
}
