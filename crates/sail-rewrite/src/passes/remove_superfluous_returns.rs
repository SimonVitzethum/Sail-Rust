//! Redundant-return elimination, matching
//! `rewrite_ast_remove_superfluous_returns` in `sail/src/lib/rewrites.ml`.
//!
//! A `let pat = bind in body` (or internal `plet`) collapses to `bind` when
//! `body` just returns the bound value back:
//! - `let lit = bind in return lit` with equal literals;
//! - `let _ = bind in return ()` (or `()` ) when `bind` has unit type;
//! - `let x = bind in return x`;
//! - `let (x, y, ..) = bind in return (x, y, ..)` with pairwise matching
//!   identifiers (patterns and tuple elements are peeled through one type
//!   ascription each, as OCaml maps `untyp_pat`/`uncast_exp` over them).
//!
//! What works without a type environment:
//! - the `has_early_return` guard is purely syntactic (a `return` node or a
//!   call to `early_return` anywhere inside `bind`, mirroring
//!   `has_early_return` in `sail/src/lib/rewriter.ml`, where
//!   `E_internal_return` itself does *not* count) and is implemented exactly;
//! - all four pattern/body shapes, including the one-level `E_typ`/`P_typ`
//!   peeling and the pairwise tuple comparison;
//! - `add_opt_cast`: when the pattern or the body carried a syntactic type,
//!   `bind` is re-wrapped in the same `E_typ` (OCaml re-annotates through
//!   the environment, which has no untyped-AST equivalent).
//!
//! Documented gaps (need elaboration, so they stay conservative rather than
//! guessed):
//! - OCaml also fires when `bind` is merely `effectful`. Effect facts come
//!   from the typechecker, so this pass only fires on `has_early_return`.
//!   That is a strict, sound subset: whenever this pass fires, OCaml fires
//!   too. Pure or merely effectful binds without an early return are
//!   preserved (see the `effectful_bind_without_early_return_survives` test).
//! - the wildcard rule needs `is_unit_typ (typ_of bind)`. Only what is
//!   syntactically provable counts: a unit literal, or an `E_typ` whose
//!   ascribed type is syntactically `unit`. Anything else is preserved.
//!
//! Wiring note: this file is deliberately self-contained (only `sail_ast`)
//! so it compiles before the canonical-path helpers land. The bottom-up
//! walk below mirrors the shared `visit_all` traversal; once
//! `crate::passes::typed` exists, the local `mk_exp` helper can be
//! replaced by its import, and the inherent [`RemoveSuperfluousReturns::run`]
//! gains a trivial `TypedPass` delegate (see `LANE59_INSERT_SPEC.md`).

#![forbid(unsafe_code)]

use sail_ast::{Def, DefKind, Exp, ExpKind, Interner, LitKind, PatKind, Typ, TypKind};

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveSuperfluousReturns;

impl crate::pass::TypedPass for RemoveSuperfluousReturns {
    fn name(&self) -> &'static str { RemoveSuperfluousReturns::name(self) }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        RemoveSuperfluousReturns::run(self, ast, interner)
    }
}

impl RemoveSuperfluousReturns {
    /// Pass name, matching the OCaml rewrite name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "remove_superfluous_returns"
    }

    /// Run the pass on the whole canonical AST: a single bottom-up walk
    /// (OCaml `rewrite_ast_base`, no fixpoint). Children are rewritten
    /// before their parent, and the replacement (`bind`, itself already
    /// visited) is not re-scanned.
    pub fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn Interner) {
        let ids: &dyn Interner = &*interner;
        for def in &mut ast.defs {
            rewrite_def(def, ids);
        }
    }
}

/// Make a synthesized expression (dummy id/span, as OCaml desugared output).
fn mk_exp(kind: ExpKind) -> Exp {
    sail_ast::Node::dummy(kind)
}

/// Make a synthesized pattern.
fn mk_pat(kind: PatKind) -> sail_ast::Pat {
    sail_ast::Node::dummy(kind)
}

/// Make a synthesized identifier.
fn mk_name(symbol: sail_ast::Symbol) -> sail_ast::Name {
    sail_ast::Name::dummy(symbol)
}

fn rewrite_def(def: &mut Def, interner: &dyn Interner) {
    match &mut def.kind {
        DefKind::Fundef { funcls, .. } => {
            for f in funcls {
                rewrite_pexp(&mut f.arm, interner);
            }
        }
        DefKind::Mapdef { clauses, .. } => {
            for c in clauses {
                match &mut c.kind {
                    sail_ast::MapclKind::Bidir {
                        forwards,
                        backwards,
                    } => {
                        rewrite_mpexp(forwards, interner);
                        rewrite_mpexp(backwards, interner);
                    }
                    sail_ast::MapclKind::Forwards(p) | sail_ast::MapclKind::Backwards(p) => {
                        rewrite_pexp(p, interner);
                    }
                }
            }
        }
        DefKind::Impl(f) => rewrite_pexp(&mut f.arm, interner),
        DefKind::Let { exp, .. } => rewrite_exp(exp, interner),
        DefKind::Outcome { defs, .. } => {
            for d in defs {
                rewrite_def(d, interner);
            }
        }
        DefKind::Measure { exp, .. } => rewrite_exp(exp, interner),
        DefKind::LoopMeasures { measures, .. } => {
            for (_, e) in measures {
                rewrite_exp(e, interner);
            }
        }
        DefKind::Register(spec) => {
            if let Some(e) = &mut spec.init {
                rewrite_exp(e, interner);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for s in sets {
                for f in &mut s.funcls {
                    rewrite_pexp(&mut f.arm, interner);
                }
            }
        }
        DefKind::Scattered(s) => match s {
            sail_ast::ScatteredDef::Funcl(f) => rewrite_pexp(&mut f.arm, interner),
            sail_ast::ScatteredDef::Mapcl { clause, .. } => match &mut clause.kind {
                sail_ast::MapclKind::Bidir {
                    forwards,
                    backwards,
                } => {
                    rewrite_mpexp(forwards, interner);
                    rewrite_mpexp(backwards, interner);
                }
                sail_ast::MapclKind::Forwards(p) | sail_ast::MapclKind::Backwards(p) => {
                    rewrite_pexp(p, interner);
                }
            },
            sail_ast::ScatteredDef::Function { .. }
            | sail_ast::ScatteredDef::Variant { .. }
            | sail_ast::ScatteredDef::Unioncl { .. }
            | sail_ast::ScatteredDef::Mapping { .. }
            | sail_ast::ScatteredDef::Enum(_)
            | sail_ast::ScatteredDef::Enumcl { .. }
            | sail_ast::ScatteredDef::End(_) => {}
        },
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Val(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
}

fn rewrite_mpexp(m: &mut sail_ast::MPExp, interner: &dyn Interner) {
    if let sail_ast::MPExpKind::When { guard, .. } = &mut m.kind {
        rewrite_exp(guard, interner);
    }
}

fn rewrite_pexp(p: &mut sail_ast::PExp, interner: &dyn Interner) {
    match &mut p.kind {
        sail_ast::PExpKind::PatExp { body, .. } => rewrite_exp(body, interner),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            rewrite_exp(guard, interner);
            rewrite_exp(body, interner);
        }
    }
}

fn rewrite_exp(exp: &mut Exp, interner: &dyn Interner) {
    rewrite_exp_children(exp, interner);
    if matches!(
        &exp.kind,
        ExpKind::Let { .. } | ExpKind::InternalPlet { .. }
    ) {
        if let Some(replacement) = simpler(exp, interner) {
            *exp = replacement;
        }
    }
}

fn rewrite_exp_children(exp: &mut Exp, interner: &dyn Interner) {
    match &mut exp.kind {
        ExpKind::Block(es) | ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            for e in es {
                rewrite_exp(e, interner);
            }
        }
        ExpKind::Typ { exp: e, .. } => rewrite_exp(e, interner),
        ExpKind::App { args, .. } => {
            for e in args {
                rewrite_exp(e, interner);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            rewrite_exp(cond, interner);
            rewrite_exp(then_branch, interner);
            rewrite_exp(else_branch, interner);
        }
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            if let Some(m) = measure {
                rewrite_exp(m, interner);
            }
            rewrite_exp(cond, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::For {
            from,
            to,
            step,
            body,
            ..
        } => {
            rewrite_exp(from, interner);
            rewrite_exp(to, interner);
            rewrite_exp(step, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            rewrite_exp(a, interner);
            rewrite_exp(b, interner);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                rewrite_exp(&mut f.value, interner);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            rewrite_exp(record, interner);
            for f in fields {
                rewrite_exp(&mut f.value, interner);
            }
        }
        ExpKind::Field { record, .. } => rewrite_exp(record, interner),
        ExpKind::Match { scrutinee, arms } => {
            rewrite_exp(scrutinee, interner);
            for a in arms {
                rewrite_pexp(a, interner);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            rewrite_exp(bind, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::Assign { lexp, value } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(value, interner);
        }
        ExpKind::Return(e) | ExpKind::Exit(e) | ExpKind::Throw(e) | ExpKind::InternalReturn(e) => {
            rewrite_exp(e, interner)
        }
        ExpKind::Try { exp: e, arms } => {
            rewrite_exp(e, interner);
            for a in arms {
                rewrite_pexp(a, interner);
            }
        }
        ExpKind::Assert { cond, msg } => {
            rewrite_exp(cond, interner);
            rewrite_exp(msg, interner);
        }
        ExpKind::Var { lexp, init, body } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(init, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::InternalAssume { exp: e, .. } => rewrite_exp(e, interner),
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
    }
}

fn rewrite_lexp_children(lexp: &mut sail_ast::LExp, interner: &dyn Interner) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(e) => rewrite_exp(e, interner),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            rewrite_lexp_children(inner, interner);
            rewrite_exp(index, interner);
        }
        sail_ast::LExpKind::VectorRange {
            lexp: inner,
            lo,
            hi,
        } => {
            rewrite_lexp_children(inner, interner);
            rewrite_exp(lo, interner);
            rewrite_exp(hi, interner);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => {
            rewrite_lexp_children(inner, interner);
        }
        sail_ast::LExpKind::Id(_)
        | sail_ast::LExpKind::Typ { .. }
        | sail_ast::LExpKind::Tuple(_)
        | sail_ast::LExpKind::VectorConcat(_) => match &mut lexp.kind {
            sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
                for p in parts {
                    rewrite_lexp_children(p, interner);
                }
            }
            _ => {}
        },
    }
}

/// What OCaml `uncast_exp` sees after peeling one type ascription off the
/// `let` body: either an internal return (whose payload is already peeled
/// once) or anything else.
enum Uncast<'a> {
    InternalReturn(&'a ExpKind),
    Other(&'a ExpKind),
}

/// Peel one `E_typ`, or one `E_typ` under `E_internal_return`, exactly like
/// `uncast_exp` in `sail/src/lib/ast_util.ml`.
fn uncast_exp(exp: &Exp) -> (Uncast<'_>, Option<&Typ>) {
    match &exp.kind {
        ExpKind::Typ { typ, exp: inner } => match &inner.kind {
            ExpKind::InternalReturn(payload) => (Uncast::InternalReturn(&payload.kind), Some(typ)),
            kind => (Uncast::Other(kind), Some(typ)),
        },
        ExpKind::InternalReturn(inner) => match &inner.kind {
            ExpKind::Typ { typ, exp: payload } => {
                (Uncast::InternalReturn(&payload.kind), Some(typ))
            }
            kind => (Uncast::InternalReturn(kind), None),
        },
        kind => (Uncast::Other(kind), None),
    }
}

/// Peel one `P_typ`, exactly like `untyp_pat` in `ast_util.ml`.
fn untyp_pat(pat: &sail_ast::Pat) -> (&PatKind, Option<&Typ>) {
    match &pat.kind {
        PatKind::Typ { typ, pat: inner } => (&inner.kind, Some(typ)),
        kind => (kind, None),
    }
}

/// Compute the collapsed replacement for a let/plet node, if any OCaml rule
/// fires within the type-environment-free subset documented above.
fn simpler(exp: &Exp, interner: &dyn Interner) -> Option<Exp> {
    let (pat, bind, body) = match &exp.kind {
        ExpKind::Let { pat, bind, body } => (pat, bind, body),
        ExpKind::InternalPlet { pat, bind, body } => (pat, bind, body),
        _ => return None,
    };
    if !has_early_return(bind, interner) {
        return None;
    }
    let (pat_kind, ptyp) = untyp_pat(pat);
    let (body_view, etyp) = uncast_exp(body);
    let fires = match (pat_kind, body_view) {
        // `let lit = bind in return lit` with equal literals.
        (PatKind::Lit(l), Uncast::InternalReturn(ExpKind::Lit(l2))) => l.kind == l2.kind,
        // `let _ = bind in return ()` (or `()`), with a provably-unit bind.
        (PatKind::Wild, Uncast::InternalReturn(ExpKind::Lit(unit)))
        | (PatKind::Wild, Uncast::Other(ExpKind::Lit(unit)))
            if matches!(unit.kind, LitKind::Unit) =>
        {
            syntactic_unit(bind, interner)
        }
        // `let x = bind in return x`.
        (PatKind::Id(id), Uncast::InternalReturn(ExpKind::Id(id2))) => id.symbol == id2.symbol,
        // `let (xs..) = bind in return (xs..)`, pairwise peeled.
        (PatKind::Tuple(ps), Uncast::InternalReturn(ExpKind::Tuple(es))) => {
            ps.len() == es.len() && ps.iter().zip(es.iter()).all(|(p, e)| ids_match(p, e))
        }
        _ => false,
    };
    if !fires {
        return None;
    }
    // OCaml `add_opt_cast`: prefer the pattern's type, else the body's.
    let mut out = (**bind).clone();
    if let Some(typ) = ptyp.or(etyp) {
        out = mk_exp(ExpKind::Typ {
            typ: typ.clone(),
            exp: Box::new(out),
        });
    }
    Some(out)
}

/// Pairwise tuple check: peeled pattern is `P_id a` and peeled element is
/// `E_id b` with `a == b` (OCaml `same_id` after mapping
/// `untyp_pat`/`uncast_exp` over both sides).
fn ids_match(pat: &sail_ast::Pat, exp: &Exp) -> bool {
    match (untyp_pat(pat).0, uncast_exp(exp).0) {
        (PatKind::Id(a), Uncast::InternalReturn(ExpKind::Id(b)))
        | (PatKind::Id(a), Uncast::Other(ExpKind::Id(b))) => a.symbol == b.symbol,
        _ => false,
    }
}

/// Syntactic subset of `is_unit_typ (typ_of bind)`: a unit literal, or an
/// ascription whose type is syntactically the `unit` type identifier.
fn syntactic_unit(bind: &Exp, interner: &dyn Interner) -> bool {
    match &bind.kind {
        ExpKind::Lit(lit) => matches!(lit.kind, LitKind::Unit),
        ExpKind::Typ { typ, .. } => match &typ.kind {
            TypKind::Id(name) => interner.resolve(name.symbol) == Some("unit"),
            _ => false,
        },
        _ => false,
    }
}

/// Whether `exp` contains a `return` or a call to `early_return`, mirroring
/// `has_early_return` in `sail/src/lib/rewriter.ml` (`pure_exp_alg false
/// (||)` with `e_app`/`e_return` overridden). `E_internal_return` does not
/// count by itself; only its payload is searched.
fn has_early_return(exp: &Exp, interner: &dyn Interner) -> bool {
    match &exp.kind {
        ExpKind::Return(_) => true,
        ExpKind::App { func, args } => {
            interner.resolve(func.symbol) == Some("early_return")
                || args.iter().any(|e| has_early_return(e, interner))
        }
        ExpKind::Block(es) | ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            es.iter().any(|e| has_early_return(e, interner))
        }
        ExpKind::Typ { exp: e, .. } => has_early_return(e, interner),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            has_early_return(cond, interner)
                || has_early_return(then_branch, interner)
                || has_early_return(else_branch, interner)
        }
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            measure
                .as_ref()
                .is_some_and(|m| has_early_return(m, interner))
                || has_early_return(cond, interner)
                || has_early_return(body, interner)
        }
        ExpKind::For {
            from,
            to,
            step,
            body,
            ..
        } => {
            has_early_return(from, interner)
                || has_early_return(to, interner)
                || has_early_return(step, interner)
                || has_early_return(body, interner)
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            has_early_return(a, interner) || has_early_return(b, interner)
        }
        ExpKind::Struct { fields, .. } => {
            fields.iter().any(|f| has_early_return(&f.value, interner))
        }
        ExpKind::StructUpdate { record, fields } => {
            has_early_return(record, interner)
                || fields.iter().any(|f| has_early_return(&f.value, interner))
        }
        ExpKind::Field { record, .. } => has_early_return(record, interner),
        ExpKind::Match { scrutinee, arms } => {
            has_early_return(scrutinee, interner)
                || arms.iter().any(|a| has_early_return_in_pexp(a, interner))
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            has_early_return(bind, interner) || has_early_return(body, interner)
        }
        ExpKind::Assign { lexp, value } => {
            has_early_return_in_lexp(lexp, interner) || has_early_return(value, interner)
        }
        ExpKind::Exit(e) | ExpKind::Throw(e) | ExpKind::InternalReturn(e) => {
            has_early_return(e, interner)
        }
        ExpKind::Try { exp: e, arms } => {
            has_early_return(e, interner)
                || arms.iter().any(|a| has_early_return_in_pexp(a, interner))
        }
        ExpKind::Assert { cond, msg } => {
            has_early_return(cond, interner) || has_early_return(msg, interner)
        }
        ExpKind::Var { lexp, init, body } => {
            has_early_return_in_lexp(lexp, interner)
                || has_early_return(init, interner)
                || has_early_return(body, interner)
        }
        ExpKind::InternalAssume { exp: e, .. } => has_early_return(e, interner),
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => false,
    }
}

fn has_early_return_in_pexp(arm: &sail_ast::PExp, interner: &dyn Interner) -> bool {
    match &arm.kind {
        sail_ast::PExpKind::PatExp { body, .. } => has_early_return(body, interner),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            has_early_return(guard, interner) || has_early_return(body, interner)
        }
    }
}

fn has_early_return_in_lexp(lexp: &sail_ast::LExp, interner: &dyn Interner) -> bool {
    match &lexp.kind {
        sail_ast::LExpKind::Deref(e) => has_early_return(e, interner),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            has_early_return_in_lexp(inner, interner) || has_early_return(index, interner)
        }
        sail_ast::LExpKind::VectorRange {
            lexp: inner,
            lo,
            hi,
        } => {
            has_early_return_in_lexp(inner, interner)
                || has_early_return(lo, interner)
                || has_early_return(hi, interner)
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => has_early_return_in_lexp(inner, interner),
        sail_ast::LExpKind::Id(_)
        | sail_ast::LExpKind::Typ { .. }
        | sail_ast::LExpKind::Tuple(_)
        | sail_ast::LExpKind::VectorConcat(_) => match &lexp.kind {
            sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
                parts.iter().any(|p| has_early_return_in_lexp(p, interner))
            }
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, SimpleInterner};

    fn run(exp: Exp, it: &mut SimpleInterner) -> Exp {
        let mut ast = Ast::new(vec![Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp,
        })]);
        RemoveSuperfluousReturns.run(&mut ast, it);
        let DefKind::Let { exp, .. } = ast.defs.swap_remove(0).kind else {
            panic!("def shape")
        };
        exp
    }

    /// `if c then return v else w`: a bind with an early return.
    fn returning_bind(it: &mut SimpleInterner) -> Exp {
        mk_exp(ExpKind::If {
            cond: Box::new(mk_exp(ExpKind::Id(mk_name(it.intern("c"))))),
            then_branch: Box::new(mk_exp(ExpKind::Return(Box::new(mk_exp(ExpKind::Id(
                mk_name(it.intern("v")),
            )))))),
            else_branch: Box::new(mk_exp(ExpKind::Id(mk_name(it.intern("w"))))),
        })
    }

    fn ident(it: &mut SimpleInterner, name: &str) -> Exp {
        mk_exp(ExpKind::Id(mk_name(it.intern(name))))
    }

    fn internal_return(exp: Exp) -> Exp {
        mk_exp(ExpKind::InternalReturn(Box::new(exp)))
    }

    #[test]
    fn id_rule_fires_only_with_early_return() {
        let mut it = SimpleInterner::new();
        let pat = mk_pat(PatKind::Id(mk_name(it.intern("x"))));
        // Pure bind: preserved (OCaml would need `effectful` to fire).
        let pure = mk_exp(ExpKind::Let {
            pat: pat.clone(),
            bind: Box::new(ident(&mut it, "e")),
            body: Box::new(internal_return(ident(&mut it, "x"))),
        });
        assert!(matches!(run(pure, &mut it).kind, ExpKind::Let { .. }));
        // Early-returning bind: collapses to the bind.
        let impure = mk_exp(ExpKind::Let {
            pat,
            bind: Box::new(returning_bind(&mut it)),
            body: Box::new(internal_return(ident(&mut it, "x"))),
        });
        assert!(matches!(run(impure, &mut it).kind, ExpKind::If { .. }));
    }

    #[test]
    fn mismatched_ids_and_bodies_survive() {
        let mut it = SimpleInterner::new();
        // `let x = bind in return y`: different identifiers survive.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(internal_return(ident(&mut it, "y"))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
        // `let x = bind in x` (no `return` wrapper) survives: that is the
        // sibling `remove_superfluous_letbinds` rule, not this pass.
        // (On this base that sibling does not exist yet; the shape is still
        // out of scope for this pass.)
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(ident(&mut it, "x")),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
    }

    #[test]
    fn lit_and_tuple_rules_fire_through_ascriptions() {
        let mut it = SimpleInterner::new();
        let num = |it: &mut SimpleInterner, text: &str| {
            mk_exp(ExpKind::Lit(sail_ast::Node::dummy(LitKind::Num(
                it.intern(text),
            ))))
        };
        // `let 1 = bind in return 1`, both sides under `E_typ`/`P_typ`.
        let typ = sail_ast::Node::dummy(sail_ast::TypKind::Unknown);
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Typ {
                    typ: typ.clone(),
                    pat: Box::new(mk_pat(PatKind::Lit(sail_ast::Node::dummy(LitKind::Num(
                        it.intern("1"),
                    ))))),
                }),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(mk_exp(ExpKind::Typ {
                    typ,
                    exp: Box::new(internal_return(num(&mut it, "1"))),
                })),
            }),
            &mut it,
        );
        // Collapses to the bind, re-wrapped in the surviving ascription
        // (OCaml `add_opt_cast`).
        assert!(matches!(out.kind, ExpKind::Typ { .. }));
        // Unequal literals survive.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Lit(sail_ast::Node::dummy(LitKind::Num(
                    it.intern("1"),
                )))),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(internal_return(num(&mut it, "2"))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
        // Tuple rule with matching identifiers elementwise.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Tuple(vec![
                    mk_pat(PatKind::Id(mk_name(it.intern("a")))),
                    mk_pat(PatKind::Id(mk_name(it.intern("b")))),
                ])),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(internal_return(mk_exp(ExpKind::Tuple(vec![
                    ident(&mut it, "a"),
                    ident(&mut it, "b"),
                ])))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::If { .. }));
        // Length mismatch survives.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Tuple(vec![mk_pat(PatKind::Id(mk_name(
                    it.intern("a"),
                )))])),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(internal_return(mk_exp(ExpKind::Tuple(vec![
                    ident(&mut it, "a"),
                    ident(&mut it, "b"),
                ])))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
    }

    #[test]
    fn wild_unit_rule_needs_provable_unit_bind() {
        let mut it = SimpleInterner::new();
        let unit = mk_exp(ExpKind::Lit(sail_ast::Node::dummy(LitKind::Unit)));
        // Ascribed-unit bind with an early return: collapses.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Wild),
                bind: Box::new(mk_exp(ExpKind::Typ {
                    typ: sail_ast::Node::dummy(sail_ast::TypKind::Id(mk_name(it.intern("unit")))),
                    exp: Box::new(returning_bind(&mut it)),
                })),
                body: Box::new(internal_return(unit.clone())),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Typ { .. }));
        // Same shape, but the bind's type is unknown: preserved, because
        // `is_unit_typ (typ_of bind)` cannot be proven without inference.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Wild),
                bind: Box::new(returning_bind(&mut it)),
                body: Box::new(internal_return(unit)),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
    }

    #[test]
    fn effectful_bind_without_early_return_survives() {
        let mut it = SimpleInterner::new();
        // A call looks effectful to OCaml (`effectful`), but without an
        // early return this pass cannot see that, so it preserves the node
        // instead of guessing. This is the documented `Env`-free gap.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
                bind: Box::new(mk_exp(ExpKind::App {
                    func: mk_name(it.intern("print")),
                    args: vec![ident(&mut it, "msg")],
                })),
                body: Box::new(internal_return(ident(&mut it, "x"))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::Let { .. }));
        // ...while a call literally named `early_return` does count,
        // mirroring OCaml `has_early_return`.
        let out = run(
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
                bind: Box::new(mk_exp(ExpKind::App {
                    func: mk_name(it.intern("early_return")),
                    args: vec![ident(&mut it, "v")],
                })),
                body: Box::new(internal_return(ident(&mut it, "x"))),
            }),
            &mut it,
        );
        assert!(matches!(out.kind, ExpKind::App { .. }));
    }

    #[test]
    fn pass_is_idempotent() {
        let mut it = SimpleInterner::new();
        let exp = mk_exp(ExpKind::Let {
            pat: mk_pat(PatKind::Id(mk_name(it.intern("x")))),
            bind: Box::new(returning_bind(&mut it)),
            body: Box::new(internal_return(ident(&mut it, "x"))),
        });
        let once = run(exp, &mut it);
        let again = run(once.clone(), &mut it);
        assert_eq!(again, once);
    }
}
