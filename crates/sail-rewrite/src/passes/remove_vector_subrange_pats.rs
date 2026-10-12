//! Remove vector-subrange patterns by naming each slice.
//!
//! Mirrors `rewrite_ast_remove_vector_subrange_pats` in
//! `sail/src/lib/rewrites.ml`. Every `P_vector_subrange (id, n, m)` in a
//! match arm (surface `id[n..m]`) becomes `P_typ (typ, P_id range_id)` with
//! `range_id = "{id}_{n}_{m}#"`, and both the guard and the body are wrapped
//! in `let id = slice_hi @ ... @ slice_lo` bindings that rebuild the
//! original vector from its slices via `E_vector_append`.
//!
//! Lengths and order (`Längen/Ordnung`): slices bound to the same `id` are
//! kept in OCaml `insert_into_append` order — descending, so the first
//! slice holds the highest indices — and the rebuilt concatenation follows
//! that order (`fold_left` over `E_vector_append`, most significant slice
//! first). The `(n, m)` pair maps to [`sail_ast::PatKind::VectorSubrange`]
//! fields `(lo, hi)` positionally (`id[n..m]` parses to
//! `P_vector_subrange (id, n, m)` and the converter stores the first index
//! in `lo`). Bounds that do not parse as integers keep stable source order.
//!
//! `check_exp` gap: OCaml takes each slice's type from the pattern
//! annotation and re-checks every synthesized `let` with `check_exp`. The
//! untyped [`sail_ast`] IR carries no annotations, so the replacement
//! binder is wrapped in `P_typ` with [`sail_ast::TypKind::Unknown`] (the
//! documented "typechecker fills in later" placeholder) and the lets are
//! built from well-formed nodes only. Fresh slice names carry the
//! compiler-reserved `#` suffix (as do OCaml's), so they cannot capture
//! user identifiers.
//!
//! Arm coverage mirrors OCaml dispatch (`rewriter.ml`): function-clause
//! patterns (`Fundef` / `Impl` / `InternalMutrec`) plus every `E_match` /
//! `E_try` arm are rewritten; mapping and scattered-funcl patterns go
//! through the base rewriter untouched (only their guard/body expressions
//! are descended into), `DEF_let` / `DEF_measure` patterns are untouched,
//! and `DEF_loop_measures` is skipped entirely ("haven't been typechecked").
//!
//! String-level [`crate::pass::Pass`] is the identity transform: the
//! statement IR has no match arms, so there is nothing to rewrite.

#![forbid(unsafe_code)]

use sail_ast::Interner;

use crate::ast::Shard;
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveVectorSubrangePats;

impl Pass for RemoveVectorSubrangePats {
    fn name(&self) -> &'static str {
        "remove_vector_subrange_pats"
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

impl crate::pass::TypedPass for RemoveVectorSubrangePats {
    fn name(&self) -> &'static str { "remove_vector_subrange_pats" }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        Self::run_ast(self, ast, interner);
    }
}

impl RemoveVectorSubrangePats {
    /// Rewrite every subrange pattern in `ast`.
    ///
    /// Returns the number of rewritten match arms (arms that contained at
    /// least one `P_vector_subrange`).
    pub fn run_ast(
        &self,
        ast: &mut sail_ast::Ast,
        interner: &mut dyn sail_ast::Interner,
    ) -> usize {
        let mut count = 0;
        for def in &mut ast.defs {
            count += walk_def(def, interner);
        }
        count
    }
}

/// One collected slice: the already-interned fresh name plus its raw and
/// numeric bounds. Numeric bounds mirror OCaml `(n, m)`; the raw strings
/// feed `range_id` verbatim.
struct Slice {
    range_id: sail_ast::Name,
    bounds: Option<(i128, i128)>,
}

/// Collected slices grouped by source `id` (OCaml `appends`): the
/// representative source [`sail_ast::Name`] (span-preserving, for the
/// rebuilding `let`) plus the descending slice list. `BTreeMap` iterates in
/// ascending id order like OCaml's `Bindings.fold`.
type Appends = std::collections::BTreeMap<String, (sail_ast::Name, Vec<Slice>)>;

/// Mirror of OCaml `insert_into_append`: keep the list in descending range
/// order (`Z.gt m1 n2` puts the new slice first, comparing the new slice's
/// second bound against the head's first bound). Unparseable bounds keep
/// stable source order (appended at the back).
fn insert_sorted(slice: Slice, list: Vec<Slice>) -> Vec<Slice> {
    let mut out = Vec::with_capacity(list.len() + 1);
    let mut rest = list.into_iter();
    let mut pending = Some(slice);
    for existing in rest.by_ref() {
        let insert_before = match (&pending, existing.bounds) {
            (Some(candidate), Some((lo2, _))) => match candidate.bounds {
                // OCaml: `if Z.gt m1 n2` with (n, m) = (first, second).
                Some((_, hi1)) => hi1 > lo2,
                None => false,
            },
            _ => false,
        };
        if insert_before {
            out.push(pending.take().expect("pending slice"));
            out.push(existing);
            out.extend(rest);
            return out;
        }
        out.push(existing);
    }
    if let Some(slice) = pending {
        out.push(slice);
    }
    out
}

/// Rewrite one match arm: replace subrange patterns, wrap guard and body.
///
/// Returns whether the arm contained a subrange pattern.
fn rewrite_arm(arm: &mut sail_ast::PExp, interner: &mut dyn sail_ast::Interner) -> bool {
    let (pat, guard, body) = match &mut arm.kind {
        sail_ast::PExpKind::PatExp { pat, body } => (pat, None, body),
        sail_ast::PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
    };
    let mut appends: Appends = Appends::new();
    rewrite_pat(pat, &mut appends, interner);
    if appends.is_empty() {
        return false;
    }
    if let Some(guard) = guard {
        let old = std::mem::replace(guard, sail_ast::Node::dummy(sail_ast::ExpKind::Undef));
        *guard = add_letbinds(old, &appends);
    }
    let old = std::mem::replace(body, sail_ast::Node::dummy(sail_ast::ExpKind::Undef));
    *body = add_letbinds(old, &appends);
    true
}

/// Replace every `P_vector_subrange` in `pat`, collecting slices per id.
fn rewrite_pat(
    pat: &mut sail_ast::Pat,
    appends: &mut Appends,
    interner: &mut dyn sail_ast::Interner,
) {
    match &mut pat.kind {
        sail_ast::PatKind::VectorSubrange { id, lo, hi } => {
            let id_text = interner
                .resolve(id.symbol)
                .unwrap_or("<unresolved>")
                .to_owned();
            // OCaml: `range_id = "{id}_{n}_{m}#"`.
            let range_text = format!("{id_text}_{lo}_{hi}#");
            let bounds = match (lo.parse::<i128>(), hi.parse::<i128>()) {
                (Ok(first), Ok(second)) => Some((first, second)),
                _ => None,
            };
            let range_id = sail_ast::Name::dummy(interner.intern(&range_text));
            let (_source, entry) = appends
                .entry(id_text)
                .or_insert_with(|| (id.clone(), Vec::new()));
            *entry = insert_sorted(
                Slice { range_id: range_id.clone(), bounds },
                std::mem::take(entry),
            );
            // OCaml wraps the fresh id in `P_typ` with the slice type; the
            // untyped IR has no slice type, so `Unknown` stands in.
            pat.kind = sail_ast::PatKind::Typ {
                typ: sail_ast::Node::dummy(sail_ast::TypKind::Unknown),
                pat: Box::new(sail_ast::Node::dummy(sail_ast::PatKind::Id(range_id))),
            };
        }
        sail_ast::PatKind::Or(first, second) | sail_ast::PatKind::Cons(first, second) => {
            rewrite_pat(first, appends, interner);
            rewrite_pat(second, appends, interner);
        }
        sail_ast::PatKind::Not(inner)
        | sail_ast::PatKind::Typ { pat: inner, .. }
        | sail_ast::PatKind::Var { pat: inner, .. } => rewrite_pat(inner, appends, interner),
        sail_ast::PatKind::As { pat: inner, .. } => rewrite_pat(inner, appends, interner),
        sail_ast::PatKind::App { args, .. }
        | sail_ast::PatKind::Vector(args)
        | sail_ast::PatKind::VectorConcat(args)
        | sail_ast::PatKind::Tuple(args)
        | sail_ast::PatKind::List(args)
        | sail_ast::PatKind::StringAppend(args) => {
            for arg in args {
                rewrite_pat(arg, appends, interner);
            }
        }
        sail_ast::PatKind::Struct { fields, .. } => {
            for (_, field) in fields {
                rewrite_pat(field, appends, interner);
            }
        }
        sail_ast::PatKind::Lit(_) | sail_ast::PatKind::Wild | sail_ast::PatKind::Id(_) => {}
    }
}

/// Wrap `exp` in `let id = slice @ ... @ slice in exp` per source id.
///
/// Mirrors OCaml `add_letbind`: the append chain starts from the first
/// (highest) slice and left-nests `E_vector_append`
/// (`((r1 @ r2) @ r3)`); ids nest in ascending order.
fn add_letbinds(exp: sail_ast::Exp, appends: &Appends) -> sail_ast::Exp {
    let mut exp = exp;
    for (source, slices) in appends.values() {
        let [first, rest @ ..] = slices.as_slice() else {
            continue;
        };
        let mut append =
            sail_ast::Node::dummy(sail_ast::ExpKind::Id(first.range_id.clone()));
        for slice in rest {
            append = sail_ast::Node::dummy(sail_ast::ExpKind::VectorAppend(
                Box::new(append),
                Box::new(sail_ast::Node::dummy(sail_ast::ExpKind::Id(
                    slice.range_id.clone(),
                ))),
            ));
        }
        // The source `id` is shadowed by the rebuilt concatenation, exactly
        // like OCaml's `Env.add_local`-shadowing `let`. The source `Name`
        // node (real span) is reused for the binder.
        exp = sail_ast::Node::dummy(sail_ast::ExpKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Id(source.clone())),
            bind: Box::new(append),
            body: Box::new(exp),
        });
    }
    exp
}

// ---------------------------------------------------------------------------
// Arm-focused walk with OCaml dispatch (see module docs).
// ---------------------------------------------------------------------------

/// Walk one definition; returns the number of rewritten arms.
fn walk_def(def: &mut sail_ast::Def, interner: &mut dyn sail_ast::Interner) -> usize {
    match &mut def.kind {
        sail_ast::DefKind::Fundef { funcls, .. } => {
            funcls.iter_mut().map(|f| rewrite_funcl(f, interner)).sum()
        }
        sail_ast::DefKind::Impl(funcl) => rewrite_funcl(funcl, interner),
        sail_ast::DefKind::InternalMutrec(sets) => sets
            .iter_mut()
            .flat_map(|set| set.funcls.iter_mut())
            .map(|f| rewrite_funcl(f, interner))
            .sum(),
        // Patterns below go through the base rewriter untouched in OCaml;
        // only guard/body expressions are descended into.
        sail_ast::DefKind::Mapdef { clauses, .. } => clauses
            .iter_mut()
            .map(|c| walk_mapcl(c, interner))
            .sum(),
        sail_ast::DefKind::Let { exp, .. } | sail_ast::DefKind::Measure { exp, .. } => {
            walk_exp(exp, interner)
        }
        sail_ast::DefKind::Outcome { defs, .. } => {
            defs.iter_mut().map(|d| walk_def(d, interner)).sum()
        }
        sail_ast::DefKind::Register(spec) => spec
            .init
            .as_mut()
            .map(|init| walk_exp(init, interner))
            .unwrap_or(0),
        sail_ast::DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => walk_funcl_exp(&mut funcl.arm, interner),
            sail_ast::ScatteredDef::Mapcl { clause, .. } => walk_mapcl(clause, interner),
            _ => 0,
        },
        // `DEF_loop_measures` is skipped; the rest carries no expressions.
        _ => 0,
    }
}

/// Rewrite a function-clause arm (pattern + guard + body).
fn rewrite_funcl(funcl: &mut sail_ast::Funcl, interner: &mut dyn sail_ast::Interner) -> usize {
    let mut count = usize::from(rewrite_arm(&mut funcl.arm, interner));
    count += walk_pexp_exps(&mut funcl.arm, interner);
    count
}

/// Descend into a scattered-funcl arm's guard/body only (OCaml base
/// rewriter leaves the pattern alone there).
fn walk_funcl_exp(arm: &mut sail_ast::PExp, interner: &mut dyn sail_ast::Interner) -> usize {
    walk_pexp_exps(arm, interner)
}

fn walk_mapcl(clause: &mut sail_ast::Mapcl, interner: &mut dyn sail_ast::Interner) -> usize {
    match &mut clause.kind {
        sail_ast::MapclKind::Bidir { forwards, backwards } => {
            walk_mpexp(forwards, interner) + walk_mpexp(backwards, interner)
        }
        sail_ast::MapclKind::Forwards(pexp) | sail_ast::MapclKind::Backwards(pexp) => {
            walk_pexp_exps(pexp, interner)
        }
    }
}

fn walk_mpexp(mpexp: &mut sail_ast::MPExp, interner: &mut dyn sail_ast::Interner) -> usize {
    if let sail_ast::MPExpKind::When { guard, .. } = &mut mpexp.kind {
        walk_exp(guard, interner)
    } else {
        0
    }
}

/// Descend into guard/body expressions, rewriting nested match arms.
fn walk_pexp_exps(pexp: &mut sail_ast::PExp, interner: &mut dyn sail_ast::Interner) -> usize {
    match &mut pexp.kind {
        sail_ast::PExpKind::PatExp { body, .. } => walk_exp(body, interner),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            walk_exp(guard, interner) + walk_exp(body, interner)
        }
    }
}

/// Descend into an expression, rewriting nested `E_match` / `E_try` arms.
///
/// Returns the number of rewritten arms in the subtree.
fn walk_exp(exp: &mut sail_ast::Exp, interner: &mut dyn sail_ast::Interner) -> usize {
    match &mut exp.kind {
        sail_ast::ExpKind::Match { scrutinee, arms } => {
            let mut count = walk_exp(scrutinee, interner);
            for arm in arms {
                count += usize::from(rewrite_arm(arm, interner));
                count += walk_pexp_exps(arm, interner);
            }
            count
        }
        sail_ast::ExpKind::Try { exp: inner, arms } => {
            let mut count = walk_exp(inner, interner);
            for arm in arms {
                count += usize::from(rewrite_arm(arm, interner));
                count += walk_pexp_exps(arm, interner);
            }
            count
        }
        sail_ast::ExpKind::Block(es)
        | sail_ast::ExpKind::Tuple(es)
        | sail_ast::ExpKind::Vector(es)
        | sail_ast::ExpKind::List(es) => es.iter_mut().map(|e| walk_exp(e, interner)).sum(),
        sail_ast::ExpKind::Typ { exp: inner, .. } => walk_exp(inner, interner),
        sail_ast::ExpKind::App { args, .. } => {
            args.iter_mut().map(|e| walk_exp(e, interner)).sum()
        }
        sail_ast::ExpKind::If { cond, then_branch, else_branch } => {
            walk_exp(cond, interner)
                + walk_exp(then_branch, interner)
                + walk_exp(else_branch, interner)
        }
        sail_ast::ExpKind::Loop { measure, cond, body, .. } => {
            measure.as_mut().map(|m| walk_exp(m, interner)).unwrap_or(0)
                + walk_exp(cond, interner)
                + walk_exp(body, interner)
        }
        sail_ast::ExpKind::For { from, to, step, body, .. } => {
            walk_exp(from, interner)
                + walk_exp(to, interner)
                + walk_exp(step, interner)
                + walk_exp(body, interner)
        }
        sail_ast::ExpKind::VectorAppend(first, second) | sail_ast::ExpKind::Cons(first, second) => {
            walk_exp(first, interner) + walk_exp(second, interner)
        }
        sail_ast::ExpKind::Struct { fields, .. } => fields
            .iter_mut()
            .map(|field| walk_exp(&mut field.value, interner))
            .sum(),
        sail_ast::ExpKind::StructUpdate { record, fields } => {
            walk_exp(record, interner)
                + fields
                    .iter_mut()
                    .map(|field| walk_exp(&mut field.value, interner))
                    .sum::<usize>()
        }
        sail_ast::ExpKind::Field { record, .. } => walk_exp(record, interner),
        sail_ast::ExpKind::Let { bind, body, .. } => {
            walk_exp(bind, interner) + walk_exp(body, interner)
        }
        sail_ast::ExpKind::Assign { lexp, value } => {
            walk_lexp(lexp, interner) + walk_exp(value, interner)
        }
        sail_ast::ExpKind::Return(inner)
        | sail_ast::ExpKind::Exit(inner)
        | sail_ast::ExpKind::Throw(inner)
        | sail_ast::ExpKind::InternalReturn(inner) => walk_exp(inner, interner),
        sail_ast::ExpKind::Assert { cond, msg } => {
            walk_exp(cond, interner) + walk_exp(msg, interner)
        }
        sail_ast::ExpKind::Var { lexp, init, body } => {
            walk_lexp(lexp, interner) + walk_exp(init, interner) + walk_exp(body, interner)
        }
        sail_ast::ExpKind::InternalPlet { bind, body, .. } => {
            walk_exp(bind, interner) + walk_exp(body, interner)
        }
        sail_ast::ExpKind::InternalAssume { exp: inner, .. } => walk_exp(inner, interner),
        sail_ast::ExpKind::Id(_)
        | sail_ast::ExpKind::Lit(_)
        | sail_ast::ExpKind::Sizeof(_)
        | sail_ast::ExpKind::Config(_)
        | sail_ast::ExpKind::Ref(_)
        | sail_ast::ExpKind::Undef
        | sail_ast::ExpKind::InternalValue(_)
        | sail_ast::ExpKind::Constraint(_) => 0,
    }
}

fn walk_lexp(lexp: &mut sail_ast::LExp, interner: &mut dyn sail_ast::Interner) -> usize {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(exp) => walk_exp(exp, interner),
        sail_ast::LExpKind::Vector { lexp, index } => {
            walk_lexp(lexp, interner) + walk_exp(index, interner)
        }
        sail_ast::LExpKind::VectorRange { lexp, lo, hi } => {
            walk_lexp(lexp, interner) + walk_exp(lo, interner) + walk_exp(hi, interner)
        }
        sail_ast::LExpKind::Field { lexp, .. } => walk_lexp(lexp, interner),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            parts.iter_mut().map(|p| walk_lexp(p, interner)).sum()
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_pat(it: &mut sail_ast::SimpleInterner, text: &str) -> sail_ast::Pat {
        sail_ast::Node::dummy(sail_ast::PatKind::Id(sail_ast::Name::dummy(it.intern(text))))
    }

    fn subrange_pat(
        it: &mut sail_ast::SimpleInterner,
        id: &str,
        lo: &str,
        hi: &str,
    ) -> sail_ast::Pat {
        sail_ast::Node::dummy(sail_ast::PatKind::VectorSubrange {
            id: sail_ast::Name::dummy(it.intern(id)),
            lo: lo.to_owned(),
            hi: hi.to_owned(),
        })
    }

    fn funcl_ast(arm: sail_ast::PExp) -> sail_ast::Ast {
        sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Fundef {
            rec: sail_ast::RecOpt::NonRec,
            annot: sail_ast::TAnnotOpt::None,
            funcls: vec![sail_ast::Funcl {
                id: sail_ast::NodeId::DUMMY,
                span: sail_ast::Span::DUMMY,
                name: sail_ast::Name::dummy(sail_ast::Symbol::DUMMY),
                arm,
            }],
        })])
    }

    fn arm_of(ast: &sail_ast::Ast) -> &sail_ast::PExp {
        let sail_ast::DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        &funcls[0].arm
    }

    #[test]
    fn single_subrange_becomes_typed_id_with_rebuilding_let() {
        let mut it = sail_ast::SimpleInterner::new();
        let body = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("x"),
        )));
        let arm = sail_ast::PExp {
            id: sail_ast::NodeId::DUMMY,
            span: sail_ast::Span::DUMMY,
            kind: sail_ast::PExpKind::PatExp {
                pat: subrange_pat(&mut it, "x", "7", "4"),
                body,
            },
        };
        let mut ast = funcl_ast(arm);
        assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 1);
        let sail_ast::PExpKind::PatExp { pat, body } = &arm_of(&ast).kind else {
            panic!("arm shape")
        };
        // Pattern is `P_typ (Unknown, x_7_4#)`.
        let sail_ast::PatKind::Typ { pat: inner, .. } = &pat.kind else {
            panic!("expected typ wrapper, got {:?}", pat.kind)
        };
        let sail_ast::PatKind::Id(name) = &inner.kind else {
            panic!("expected id")
        };
        assert_eq!(it.resolve(name.symbol), Some("x_7_4#"));
        // Body is `let x = x_7_4# in x`.
        let sail_ast::ExpKind::Let { pat, bind, body: inner_body } = &body.kind else {
            panic!("expected let, got {:?}", body.kind)
        };
        let sail_ast::PatKind::Id(bound) = &pat.kind else {
            panic!("expected id pat")
        };
        // The let reuses the source `Name` node (real span, not a dummy).
        assert_eq!(bound.symbol, it.intern("x"));
        let sail_ast::ExpKind::Id(slice) = &bind.kind else {
            panic!("expected slice id")
        };
        assert_eq!(it.resolve(slice.symbol), Some("x_7_4#"));
        assert!(matches!(inner_body.kind, sail_ast::ExpKind::Id(_)));
        // Idempotent: no subrange pattern remains.
        assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 0);
    }

    #[test]
    fn slices_sort_descending_regardless_of_source_order() {
        let mut it = sail_ast::SimpleInterner::new();
        // Source order is low slice first; the concat must still be high @ low.
        let pat = sail_ast::Node::dummy(sail_ast::PatKind::VectorConcat(vec![
            subrange_pat(&mut it, "x", "3", "0"),
            subrange_pat(&mut it, "x", "7", "4"),
        ]));
        let arm = sail_ast::PExp {
            id: sail_ast::NodeId::DUMMY,
            span: sail_ast::Span::DUMMY,
            kind: sail_ast::PExpKind::PatExp {
                pat,
                body: sail_ast::Node::dummy(sail_ast::ExpKind::Undef),
            },
        };
        let mut ast = funcl_ast(arm);
        assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 1);
        let sail_ast::PExpKind::PatExp { pat, body } = &arm_of(&ast).kind else {
            panic!("arm shape")
        };
        let sail_ast::PatKind::VectorConcat(parts) = &pat.kind else {
            panic!("expected concat pat")
        };
        assert_eq!(parts.len(), 2);
        for (part, expect) in parts.iter().zip(["x_3_0#", "x_7_4#"]) {
            let sail_ast::PatKind::Typ { pat: inner, .. } = &part.kind else {
                panic!("expected typ wrapper")
            };
            let sail_ast::PatKind::Id(name) = &inner.kind else {
                panic!("expected id")
            };
            assert_eq!(it.resolve(name.symbol), Some(expect));
        }
        // `let x = x_7_4# @ x_3_0# in ...` (highest slice first).
        let sail_ast::ExpKind::Let { bind, .. } = &body.kind else {
            panic!("expected let")
        };
        let sail_ast::ExpKind::VectorAppend(first, second) = &bind.kind else {
            panic!("expected append, got {:?}", bind.kind)
        };
        let sail_ast::ExpKind::Id(hi) = &first.kind else {
            panic!("expected id")
        };
        let sail_ast::ExpKind::Id(lo) = &second.kind else {
            panic!("expected id")
        };
        assert_eq!(it.resolve(hi.symbol), Some("x_7_4#"));
        assert_eq!(it.resolve(lo.symbol), Some("x_3_0#"));
    }

    #[test]
    fn guard_and_body_are_both_wrapped() {
        let mut it = sail_ast::SimpleInterner::new();
        let guard = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("x"),
        )));
        let arm = sail_ast::PExp {
            id: sail_ast::NodeId::DUMMY,
            span: sail_ast::Span::DUMMY,
            kind: sail_ast::PExpKind::PatWhen {
                pat: subrange_pat(&mut it, "x", "1", "1"),
                guard,
                body: sail_ast::Node::dummy(sail_ast::ExpKind::Undef),
            },
        };
        let mut ast = funcl_ast(arm);
        assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 1);
        let sail_ast::PExpKind::PatWhen { guard, body, .. } = &arm_of(&ast).kind else {
            panic!("arm shape")
        };
        assert!(matches!(guard.kind, sail_ast::ExpKind::Let { .. }));
        assert!(matches!(body.kind, sail_ast::ExpKind::Let { .. }));
    }

    #[test]
    fn plain_arm_survives_and_counts_zero() {
        let mut it = sail_ast::SimpleInterner::new();
        let arm = sail_ast::PExp {
            id: sail_ast::NodeId::DUMMY,
            span: sail_ast::Span::DUMMY,
            kind: sail_ast::PExpKind::PatExp {
                pat: id_pat(&mut it, "x"),
                body: sail_ast::Node::dummy(sail_ast::ExpKind::Undef),
            },
        };
        let mut ast = funcl_ast(arm);
        let snapshot = ast.clone();
        assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 0);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn string_level_is_identity() {
        use crate::ast::{Def, DefKind, Shard, Stmt};
        let shard = Shard::new(
            0,
            vec![Def::new("f", DefKind::Function, vec![Stmt::Expr("match x { _ => () }".into())])],
        );
        assert_eq!(RemoveVectorSubrangePats.run(shard.clone()), shard);
        assert_eq!(RemoveVectorSubrangePats.name(), "remove_vector_subrange_pats");
    }
}
