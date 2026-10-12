//! Split vector-concatenation assignments into per-part writes.
//!
//! Mirrors `rewrite_vector_concat_assignments` in `sail/src/lib/rewrites.ml`:
//! `E_assign (LE_vector_concat parts, rhs)` over a vector or bitvector
//! becomes `let split_vec = rhs in` (unless `rhs` is syntactically small, in
//! which case it is reused directly) followed by a block assigning each part
//! from the matching slice `vector_subrange#(base, first, second)`.
//!
//! Lengths and order (`Längen/Ordnung`): OCaml takes part widths from
//! `vector_typ_args_of` + `Type_check.solve_unique` (falling back to
//! `E_sizeof`), the start index from `vector_start_index`, and the direction
//! from `Env.get_default_order`, advancing one part after another via
//! `next`. [`split_ranges`] ports exactly that fold over explicit widths:
//! for increasing order part `k` covers
//! `[start + offset, start + offset + len - 1]`, for decreasing order it
//! covers `[start - offset - len + 1, start - offset]`, and the slice
//! arguments are passed as `(previous_i, j)` in OCaml order — `(lo, hi)`
//! for `Inc`, `(hi, lo)` for `Dec`.
//!
//! `check_exp` / `is_bitvector_typ` gap: widths, start index, and order all
//! come from the type environment, which the untyped [`sail_ast`] IR does
//! not carry. [`VectorConcatAssignments::run_ast`] therefore takes a
//! [`ConcatLengths`] oracle (the future `TypedPass` wiring implements it via
//! `Env`: `get_default_order`, `vector_start_index`, `vector_typ_args_of` plus
//! `solve_unique`). A concatenation is only lowered when every part
//! width resolves; unknown widths are skipped conservatively instead of
//! emitting `E_sizeof` guesses (OCaml's fallback needs nexps that only the
//! typechecker can justify).
//!
//! String-level [`crate::pass::Pass`] is the identity transform: the
//! statement IR has neither concatenation syntax nor part widths (the same
//! conservative choice as `SimpleAssignments`).

#![forbid(unsafe_code)]

use sail_ast::Interner;

use crate::ast::Shard;
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct VectorConcatAssignments;

impl Pass for VectorConcatAssignments {
    fn name(&self) -> &'static str {
        "vector_concat_assignments"
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

/// Type-environment answers the untyped IR cannot give.
///
/// * `order_inc` — OCaml `Env.get_default_order`: `true` for `Inc`.
/// * `start_index` — OCaml `vector_start_index` for a vector of `total`
///   elements (lowest index for `Inc`, highest for `Dec`).
/// * `part_length` — OCaml `len`: the width of one concatenation part from
///   `vector_typ_args_of` + `solve_unique`; `None` when it does not resolve.
pub trait ConcatLengths {
    fn order_inc(&self) -> bool;
    fn start_index(&self, total: u128) -> i128;
    fn part_length(&self, part: &sail_ast::LExp) -> Option<u128>;
}

impl crate::pass::TypedPass for VectorConcatAssignments {
    fn name(&self) -> &'static str { "vector_concat_assignments" }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        /// Konservative Brücke ohne Typ-Env (Main-Agent): Inc-Ordnung,
        /// 0-basiert, nur voll auflösbare Breiten feuern, sonst Skip.
        struct NoEnv;
        impl ConcatLengths for NoEnv {
            fn order_inc(&self) -> bool { true }
            fn start_index(&self, _total: u128) -> i128 { 0 }
            fn part_length(&self, _part: &sail_ast::LExp) -> Option<u128> { None }
        }
        Self::run_ast(self, ast, interner, &NoEnv);
    }
}

impl VectorConcatAssignments {
    /// Lower every resolvable concatenation assignment in `ast`, bottom-up.
    ///
    /// Returns the number of rewritten assignments.
    pub fn run_ast(
        &self,
        ast: &mut sail_ast::Ast,
        interner: &mut dyn sail_ast::Interner,
        lens: &dyn ConcatLengths,
    ) -> usize {
        let mut used = collect_used(ast, interner);
        let mut fresh_next = 0u32;
        let mut fresh = |used: &mut std::collections::BTreeSet<String>| {
            loop {
                let cand = if fresh_next == 0 {
                    // OCaml uses plain `mk_id "split_vec"`.
                    fresh_next += 1;
                    "split_vec".to_owned()
                } else {
                    fresh_next += 1;
                    format!("split_vec_{}", fresh_next - 1)
                };
                if used.insert(cand.clone()) {
                    return cand;
                }
            }
        };
        let mut count = 0;
        for def in &mut ast.defs {
            walk_def(def, &mut |exp| {
                if rewrite_assign(exp, interner, lens, &mut used, &mut fresh) {
                    count += 1;
                }
            });
        }
        count
    }
}

/// Try to rewrite one expression node. Returns whether it changed.
fn rewrite_assign(
    exp: &mut sail_ast::Exp,
    interner: &mut dyn sail_ast::Interner,
    lens: &dyn ConcatLengths,
    used: &mut std::collections::BTreeSet<String>,
    fresh: &mut impl FnMut(&mut std::collections::BTreeSet<String>) -> String,
) -> bool {
    let sail_ast::ExpKind::Assign { lexp, value } = &mut exp.kind else {
        return false;
    };
    let sail_ast::LExpKind::VectorConcat(parts) = &mut lexp.kind else {
        return false;
    };
    if parts.is_empty() {
        return false;
    }
    let mut widths = Vec::with_capacity(parts.len());
    for part in parts.iter() {
        let Some(len) = lens.part_length(part) else {
            // Conservative: OCaml would fall back to `E_sizeof`, which needs
            // typechecker nexps. Without them, leave the assignment alone.
            return false;
        };
        widths.push(len);
    }
    let total: u128 = widths.iter().sum();
    let order_inc = lens.order_inc();
    let Some(ranges) = split_ranges(lens.start_index(total), order_inc, &widths) else {
        return false;
    };
    let small = is_small(value);
    let split_name = if small {
        None
    } else {
        Some(fresh(used))
    };
    let old = std::mem::replace(&mut exp.kind, sail_ast::ExpKind::Undef);
    let sail_ast::ExpKind::Assign { lexp, value } = old else {
        exp.kind = old;
        return false;
    };
    let sail_ast::LExpKind::VectorConcat(parts) = lexp.kind else {
        exp.kind = sail_ast::ExpKind::Assign { lexp, value };
        return false;
    };
    exp.kind = lower_concat_assign(parts, *value, &ranges, split_name, interner).kind;
    true
}

/// OCaml `small`: variables and literals (through annotations) duplicate
/// freely; everything else is bound once so effects run exactly once.
fn is_small(exp: &sail_ast::Exp) -> bool {
    match &exp.kind {
        sail_ast::ExpKind::Id(_) | sail_ast::ExpKind::Lit(_) | sail_ast::ExpKind::Sizeof(_) => true,
        sail_ast::ExpKind::Typ { exp: inner, .. } => is_small(inner),
        sail_ast::ExpKind::List(es) => es.iter().all(is_small),
        sail_ast::ExpKind::Cons(first, second) => is_small(first) && is_small(second),
        // Note: `E_vector` is deliberately *not* small in OCaml, so vector
        // right-hand sides are always bound to `split_vec`.
        _ => false,
    }
}

/// Port of OCaml's `next` fold: slice bounds for each part in order.
///
/// Returns `(first, second)` slice arguments per part — `(lo, hi)` for
/// increasing order, `(hi, lo)` for decreasing order, exactly as OCaml
/// passes `(i, j)` to `vector_subrange`. `None` on integer overflow.
pub fn split_ranges(start: i128, order_inc: bool, lens: &[u128]) -> Option<Vec<(i128, i128)>> {
    let mut ranges = Vec::with_capacity(lens.len());
    if order_inc {
        let mut index = start;
        for &len in lens {
            let len = i128::try_from(len).ok()?;
            let second = index.checked_add(len)?.checked_sub(1)?;
            ranges.push((index, second));
            index = index.checked_add(len)?;
        }
    } else {
        let mut index = start;
        for &len in lens {
            let len = i128::try_from(len).ok()?;
            let second = index.checked_sub(len)?.checked_add(1)?;
            ranges.push((index, second));
            index = index.checked_sub(len)?;
        }
    }
    Some(ranges)
}

/// Build `let split_vec = rhs in { part = slice; ... }` (or just the block
/// for small right-hand sides), mirroring OCaml `assign_tuple`.
pub fn lower_concat_assign(
    parts: Vec<sail_ast::LExp>,
    rhs: sail_ast::Exp,
    ranges: &[(i128, i128)],
    split_vec: Option<String>,
    interner: &mut dyn sail_ast::Interner,
) -> sail_ast::Exp {
    debug_assert_eq!(parts.len(), ranges.len());
    let subrange = sail_ast::Name::dummy(interner.intern("vector_subrange#"));
    let num = |value: i128, interner: &mut dyn sail_ast::Interner| {
        sail_ast::Node::dummy(sail_ast::ExpKind::Lit(sail_ast::Node::dummy(
            sail_ast::LitKind::Num(interner.intern(&value.to_string())),
        )))
    };
    let base = |split_vec: &Option<sail_ast::Name>, rhs: &sail_ast::Exp| match split_vec {
        Some(name) => sail_ast::Node::dummy(sail_ast::ExpKind::Id(name.clone())),
        None => rhs.clone(),
    };
    // OCaml `strip_exp` / `strip_lexp` only remap annotations; the untyped
    // IR has none, so both are the identity here.
    let split_name = split_vec
        .map(|text| sail_ast::Name::dummy(interner.intern(&text)));
    let mut assigns = Vec::with_capacity(parts.len());
    for (part, (first, second)) in parts.into_iter().zip(ranges.iter()) {
        let slice = sail_ast::Node::dummy(sail_ast::ExpKind::App {
            func: subrange.clone(),
            args: vec![
                base(&split_name, &rhs),
                num(*first, &mut *interner),
                num(*second, &mut *interner),
            ],
        });
        assigns.push(sail_ast::Node::dummy(sail_ast::ExpKind::Assign {
            lexp: Box::new(part),
            value: Box::new(slice),
        }));
    }
    let block = sail_ast::Node::dummy(sail_ast::ExpKind::Block(assigns));
    match split_name {
        None => block,
        Some(name) => sail_ast::Node::dummy(sail_ast::ExpKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Id(name)),
            bind: Box::new(rhs),
            body: Box::new(block),
        }),
    }
}

/// Seed the `split_vec` fresh-name generator (OCaml `mk_id` checks the
/// whole environment; collect every value-level name text in `ast`).
fn collect_used(
    ast: &sail_ast::Ast,
    interner: &dyn sail_ast::Interner,
) -> std::collections::BTreeSet<String> {
    let mut used = std::collections::BTreeSet::new();
    let mut name = |n: &sail_ast::Name| {
        if let Some(text) = interner.resolve(n.symbol) {
            used.insert(text.to_owned());
        }
    };
    for def in &ast.defs {
        collect_def_names(def, &mut name);
    }
    used
}

fn collect_def_names(def: &sail_ast::Def, name: &mut impl FnMut(&sail_ast::Name)) {
    match &def.kind {
        sail_ast::DefKind::Fundef { funcls, .. } => {
            for funcl in funcls {
                name(&funcl.name);
                collect_pexp_names(&funcl.arm, name);
            }
        }
        sail_ast::DefKind::Mapdef { name: n, clauses, .. } => {
            name(n);
            for clause in clauses {
                match &clause.kind {
                    sail_ast::MapclKind::Bidir { forwards, backwards } => {
                        collect_mpexp_names(forwards, name);
                        collect_mpexp_names(backwards, name);
                    }
                    sail_ast::MapclKind::Forwards(pexp)
                    | sail_ast::MapclKind::Backwards(pexp) => collect_pexp_names(pexp, name),
                }
            }
        }
        sail_ast::DefKind::Impl(funcl) => {
            name(&funcl.name);
            collect_pexp_names(&funcl.arm, name);
        }
        sail_ast::DefKind::Let { pat, exp } => {
            collect_pat_names(pat, name);
            collect_exp_names(exp, name);
        }
        sail_ast::DefKind::Register(spec) => {
            name(&spec.name);
            if let Some(init) = &spec.init {
                collect_exp_names(init, name);
            }
        }
        sail_ast::DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &set.funcls {
                    name(&funcl.name);
                    collect_pexp_names(&funcl.arm, name);
                }
            }
        }
        sail_ast::DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => {
                name(&funcl.name);
                collect_pexp_names(&funcl.arm, name);
            }
            sail_ast::ScatteredDef::Mapcl { name: n, clause } => {
                name(n);
                match &clause.kind {
                    sail_ast::MapclKind::Bidir { forwards, backwards } => {
                        collect_mpexp_names(forwards, name);
                        collect_mpexp_names(backwards, name);
                    }
                    sail_ast::MapclKind::Forwards(pexp)
                    | sail_ast::MapclKind::Backwards(pexp) => collect_pexp_names(pexp, name),
                }
            }
            sail_ast::ScatteredDef::Function { name: n, .. }
            | sail_ast::ScatteredDef::Mapping { name: n, .. }
            | sail_ast::ScatteredDef::Enum(n)
            | sail_ast::ScatteredDef::End(n) => name(n),
            sail_ast::ScatteredDef::Variant { name: n, .. }
            | sail_ast::ScatteredDef::Unioncl { name: n, .. } => name(n),
            sail_ast::ScatteredDef::Enumcl { enum_name, member } => {
                name(enum_name);
                name(member);
            }
        },
        sail_ast::DefKind::Val(spec) => name(&spec.name),
        _ => {}
    }
}

fn collect_pexp_names(pexp: &sail_ast::PExp, name: &mut impl FnMut(&sail_ast::Name)) {
    match &pexp.kind {
        sail_ast::PExpKind::PatExp { pat, body } => {
            collect_pat_names(pat, name);
            collect_exp_names(body, name);
        }
        sail_ast::PExpKind::PatWhen { pat, guard, body } => {
            collect_pat_names(pat, name);
            collect_exp_names(guard, name);
            collect_exp_names(body, name);
        }
    }
}

fn collect_mpexp_names(mpexp: &sail_ast::MPExp, name: &mut impl FnMut(&sail_ast::Name)) {
    match &mpexp.kind {
        sail_ast::MPExpKind::Pat(pat) => collect_pat_names(pat, name),
        sail_ast::MPExpKind::When { pat, guard } => {
            collect_pat_names(pat, name);
            collect_exp_names(guard, name);
        }
    }
}

fn collect_pat_names(pat: &sail_ast::Pat, name: &mut impl FnMut(&sail_ast::Name)) {
    match &pat.kind {
        sail_ast::PatKind::Id(n) | sail_ast::PatKind::VectorSubrange { id: n, .. } => name(n),
        sail_ast::PatKind::App { func: n, args } => {
            name(n);
            for arg in args {
                collect_pat_names(arg, name);
            }
        }
        sail_ast::PatKind::As { pat, id } => {
            name(id);
            collect_pat_names(pat, name);
        }
        sail_ast::PatKind::Or(first, second) | sail_ast::PatKind::Cons(first, second) => {
            collect_pat_names(first, name);
            collect_pat_names(second, name);
        }
        sail_ast::PatKind::Not(inner) | sail_ast::PatKind::Var { pat: inner, .. } => {
            collect_pat_names(inner, name);
        }
        sail_ast::PatKind::Typ { pat: inner, .. } => collect_pat_names(inner, name),
        sail_ast::PatKind::Tuple(ps)
        | sail_ast::PatKind::Vector(ps)
        | sail_ast::PatKind::VectorConcat(ps)
        | sail_ast::PatKind::List(ps)
        | sail_ast::PatKind::StringAppend(ps) => {
            for p in ps {
                collect_pat_names(p, name);
            }
        }
        sail_ast::PatKind::Struct { name: s, fields, .. } => {
            if let sail_ast::StructName::Id(n) = s {
                name(n);
            }
            for (field, p) in fields {
                name(field);
                collect_pat_names(p, name);
            }
        }
        sail_ast::PatKind::Lit(_) | sail_ast::PatKind::Wild => {}
    }
}

fn collect_exp_names(exp: &sail_ast::Exp, name: &mut impl FnMut(&sail_ast::Name)) {
    match &exp.kind {
        sail_ast::ExpKind::Id(n) | sail_ast::ExpKind::Ref(n) => name(n),
        sail_ast::ExpKind::App { func, args } => {
            name(func);
            for arg in args {
                collect_exp_names(arg, name);
            }
        }
        sail_ast::ExpKind::Block(es)
        | sail_ast::ExpKind::Tuple(es)
        | sail_ast::ExpKind::Vector(es)
        | sail_ast::ExpKind::List(es) => {
            for e in es {
                collect_exp_names(e, name);
            }
        }
        sail_ast::ExpKind::Typ { exp: inner, .. } => collect_exp_names(inner, name),
        sail_ast::ExpKind::If { cond, then_branch, else_branch } => {
            collect_exp_names(cond, name);
            collect_exp_names(then_branch, name);
            collect_exp_names(else_branch, name);
        }
        sail_ast::ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(measure) = measure {
                collect_exp_names(measure, name);
            }
            collect_exp_names(cond, name);
            collect_exp_names(body, name);
        }
        sail_ast::ExpKind::For { var, from, to, step, body, .. } => {
            name(var);
            collect_exp_names(from, name);
            collect_exp_names(to, name);
            collect_exp_names(step, name);
            collect_exp_names(body, name);
        }
        sail_ast::ExpKind::VectorAppend(first, second) | sail_ast::ExpKind::Cons(first, second) => {
            collect_exp_names(first, name);
            collect_exp_names(second, name);
        }
        sail_ast::ExpKind::Struct { fields, .. } => {
            for field in fields {
                name(&field.field);
                collect_exp_names(&field.value, name);
            }
        }
        sail_ast::ExpKind::StructUpdate { record, fields } => {
            collect_exp_names(record, name);
            for field in fields {
                name(&field.field);
                collect_exp_names(&field.value, name);
            }
        }
        sail_ast::ExpKind::Field { record, field } => {
            collect_exp_names(record, name);
            name(field);
        }
        sail_ast::ExpKind::Match { scrutinee, arms } => {
            collect_exp_names(scrutinee, name);
            for arm in arms {
                collect_pexp_names(arm, name);
            }
        }
        sail_ast::ExpKind::Let { bind, body, .. } => {
            collect_exp_names(bind, name);
            collect_exp_names(body, name);
        }
        sail_ast::ExpKind::Assign { lexp, value } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(value, name);
        }
        sail_ast::ExpKind::Return(inner)
        | sail_ast::ExpKind::Exit(inner)
        | sail_ast::ExpKind::Throw(inner)
        | sail_ast::ExpKind::InternalReturn(inner) => collect_exp_names(inner, name),
        sail_ast::ExpKind::Try { exp: inner, arms } => {
            collect_exp_names(inner, name);
            for arm in arms {
                collect_pexp_names(arm, name);
            }
        }
        sail_ast::ExpKind::Assert { cond, msg } => {
            collect_exp_names(cond, name);
            collect_exp_names(msg, name);
        }
        sail_ast::ExpKind::Var { lexp, init, body } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(init, name);
            collect_exp_names(body, name);
        }
        sail_ast::ExpKind::InternalPlet { bind, body, .. } => {
            collect_exp_names(bind, name);
            collect_exp_names(body, name);
        }
        sail_ast::ExpKind::InternalAssume { exp: inner, .. } => collect_exp_names(inner, name),
        sail_ast::ExpKind::Lit(_)
        | sail_ast::ExpKind::Sizeof(_)
        | sail_ast::ExpKind::Config(_)
        | sail_ast::ExpKind::Undef
        | sail_ast::ExpKind::InternalValue(_)
        | sail_ast::ExpKind::Constraint(_) => {}
    }
}

fn collect_lexp_names(lexp: &sail_ast::LExp, name: &mut impl FnMut(&sail_ast::Name)) {
    match &lexp.kind {
        sail_ast::LExpKind::Id(n) | sail_ast::LExpKind::Typ { id: n, .. } => name(n),
        sail_ast::LExpKind::Deref(exp) => collect_exp_names(exp, name),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for part in parts {
                collect_lexp_names(part, name);
            }
        }
        sail_ast::LExpKind::Vector { lexp, index } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(index, name);
        }
        sail_ast::LExpKind::VectorRange { lexp, lo, hi } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(lo, name);
            collect_exp_names(hi, name);
        }
        sail_ast::LExpKind::Field { lexp, field } => {
            collect_lexp_names(lexp, name);
            name(field);
        }
    }
}

// ---------------------------------------------------------------------------
// Bottom-up expression walk (OCaml `fold_exp` order: children first).
// ---------------------------------------------------------------------------

fn walk_def(def: &mut sail_ast::Def, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut def.kind {
        sail_ast::DefKind::Fundef { funcls, .. } => {
            for funcl in funcls {
                walk_funcl(funcl, f);
            }
        }
        sail_ast::DefKind::Mapdef { clauses, .. } => {
            for clause in clauses {
                walk_mapcl(clause, f);
            }
        }
        sail_ast::DefKind::Impl(funcl) => walk_funcl(funcl, f),
        sail_ast::DefKind::Let { exp, .. } => walk_exp(exp, f),
        sail_ast::DefKind::Outcome { defs, .. } => {
            for def in defs {
                walk_def(def, f);
            }
        }
        sail_ast::DefKind::Measure { exp, .. } => walk_exp(exp, f),
        // OCaml `rewrite_def` leaves `DEF_loop_measures` alone ("haven't
        // been typechecked").
        sail_ast::DefKind::LoopMeasures { .. } => {}
        sail_ast::DefKind::Register(spec) => {
            if let Some(init) = &mut spec.init {
                walk_exp(init, f);
            }
        }
        sail_ast::DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &mut set.funcls {
                    walk_funcl(funcl, f);
                }
            }
        }
        sail_ast::DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => walk_funcl(funcl, f),
            sail_ast::ScatteredDef::Mapcl { clause, .. } => walk_mapcl(clause, f),
            _ => {}
        },
        _ => {}
    }
}

fn walk_funcl(funcl: &mut sail_ast::Funcl, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    walk_pexp(&mut funcl.arm, f);
}

fn walk_mapcl(clause: &mut sail_ast::Mapcl, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut clause.kind {
        sail_ast::MapclKind::Bidir { forwards, backwards } => {
            walk_mpexp(forwards, f);
            walk_mpexp(backwards, f);
        }
        sail_ast::MapclKind::Forwards(pexp) | sail_ast::MapclKind::Backwards(pexp) => {
            walk_pexp(pexp, f);
        }
    }
}

fn walk_mpexp(mpexp: &mut sail_ast::MPExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    if let sail_ast::MPExpKind::When { guard, .. } = &mut mpexp.kind {
        walk_exp(guard, f);
    }
}

fn walk_pexp(pexp: &mut sail_ast::PExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut pexp.kind {
        sail_ast::PExpKind::PatExp { body, .. } => walk_exp(body, f),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            walk_exp(guard, f);
            walk_exp(body, f);
        }
    }
}

fn walk_lexp(lexp: &mut sail_ast::LExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(exp) => walk_exp(exp, f),
        sail_ast::LExpKind::Vector { lexp, index } => {
            walk_lexp(lexp, f);
            walk_exp(index, f);
        }
        sail_ast::LExpKind::VectorRange { lexp, lo, hi } => {
            walk_lexp(lexp, f);
            walk_exp(lo, f);
            walk_exp(hi, f);
        }
        sail_ast::LExpKind::Field { lexp, .. } => walk_lexp(lexp, f),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for part in parts {
                walk_lexp(part, f);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

#[allow(clippy::too_many_lines)]
fn walk_exp(exp: &mut sail_ast::Exp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut exp.kind {
        sail_ast::ExpKind::Block(es)
        | sail_ast::ExpKind::Tuple(es)
        | sail_ast::ExpKind::Vector(es)
        | sail_ast::ExpKind::List(es) => {
            for e in es {
                walk_exp(e, f);
            }
        }
        sail_ast::ExpKind::Typ { exp: inner, .. } => walk_exp(inner, f),
        sail_ast::ExpKind::App { args, .. } => {
            for e in args {
                walk_exp(e, f);
            }
        }
        sail_ast::ExpKind::If { cond, then_branch, else_branch } => {
            walk_exp(cond, f);
            walk_exp(then_branch, f);
            walk_exp(else_branch, f);
        }
        sail_ast::ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(measure) = measure {
                walk_exp(measure, f);
            }
            walk_exp(cond, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::For { from, to, step, body, .. } => {
            walk_exp(from, f);
            walk_exp(to, f);
            walk_exp(step, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::VectorAppend(first, second) | sail_ast::ExpKind::Cons(first, second) => {
            walk_exp(first, f);
            walk_exp(second, f);
        }
        sail_ast::ExpKind::Struct { fields, .. } => {
            for field in fields {
                walk_exp(&mut field.value, f);
            }
        }
        sail_ast::ExpKind::StructUpdate { record, fields } => {
            walk_exp(record, f);
            for field in fields {
                walk_exp(&mut field.value, f);
            }
        }
        sail_ast::ExpKind::Field { record, .. } => walk_exp(record, f),
        sail_ast::ExpKind::Match { scrutinee, arms } => {
            walk_exp(scrutinee, f);
            for arm in arms {
                walk_pexp(arm, f);
            }
        }
        sail_ast::ExpKind::Let { bind, body, .. } => {
            walk_exp(bind, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::Assign { lexp, value } => {
            walk_lexp(lexp, f);
            walk_exp(value, f);
        }
        sail_ast::ExpKind::Return(inner)
        | sail_ast::ExpKind::Exit(inner)
        | sail_ast::ExpKind::Throw(inner)
        | sail_ast::ExpKind::InternalReturn(inner) => walk_exp(inner, f),
        sail_ast::ExpKind::Try { exp: inner, arms } => {
            walk_exp(inner, f);
            for arm in arms {
                walk_pexp(arm, f);
            }
        }
        sail_ast::ExpKind::Assert { cond, msg } => {
            walk_exp(cond, f);
            walk_exp(msg, f);
        }
        sail_ast::ExpKind::Var { lexp, init, body } => {
            walk_lexp(lexp, f);
            walk_exp(init, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::InternalPlet { bind, body, .. } => {
            walk_exp(bind, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::InternalAssume { exp: inner, .. } => walk_exp(inner, f),
        sail_ast::ExpKind::Id(_)
        | sail_ast::ExpKind::Lit(_)
        | sail_ast::ExpKind::Sizeof(_)
        | sail_ast::ExpKind::Config(_)
        | sail_ast::ExpKind::Ref(_)
        | sail_ast::ExpKind::Undef
        | sail_ast::ExpKind::InternalValue(_)
        | sail_ast::ExpKind::Constraint(_) => {}
    }
    f(exp);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-width oracle: every part is `width` bits, total `parts * width`.
    struct FixedWidths {
        order_inc: bool,
        start: i128,
        width: u128,
    }

    impl ConcatLengths for FixedWidths {
        fn order_inc(&self) -> bool {
            self.order_inc
        }
        fn start_index(&self, _total: u128) -> i128 {
            self.start
        }
        fn part_length(&self, _part: &sail_ast::LExp) -> Option<u128> {
            Some(self.width)
        }
    }

    struct Unknown;

    impl ConcatLengths for Unknown {
        fn order_inc(&self) -> bool {
            false
        }
        fn start_index(&self, _total: u128) -> i128 {
            0
        }
        fn part_length(&self, _part: &sail_ast::LExp) -> Option<u128> {
            None
        }
    }

    #[test]
    fn split_ranges_respects_lengths_and_order() {
        // Decreasing (bitvector): first part takes the top slice.
        assert_eq!(
            split_ranges(3, false, &[2, 2]),
            Some(vec![(3, 2), (1, 0)])
        );
        // Increasing: first part takes the bottom slice.
        assert_eq!(
            split_ranges(0, true, &[2, 2]),
            Some(vec![(0, 1), (2, 3)])
        );
        // Uneven parts advance by their own widths.
        assert_eq!(
            split_ranges(7, false, &[4, 1, 3]),
            Some(vec![(7, 4), (3, 3), (2, 0)])
        );
        // Overflow is refused instead of wrapping.
        assert_eq!(split_ranges(i128::MAX, true, &[2]), None);
    }

    fn concat_ast(
        it: &mut sail_ast::SimpleInterner,
        parts: &[&str],
        rhs: sail_ast::Exp,
    ) -> sail_ast::Ast {
        let lexps = parts
            .iter()
            .map(|p| {
                sail_ast::Node::dummy(sail_ast::LExpKind::Id(sail_ast::Name::dummy(it.intern(p))))
            })
            .collect();
        let assign = sail_ast::Node::dummy(sail_ast::ExpKind::Assign {
            lexp: Box::new(sail_ast::Node::dummy(sail_ast::LExpKind::VectorConcat(lexps))),
            value: Box::new(rhs),
        });
        sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
            exp: assign,
        })])
    }

    fn single_exp(ast: &sail_ast::Ast) -> &sail_ast::Exp {
        let sail_ast::DefKind::Let { exp, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        exp
    }

    #[test]
    fn small_rhs_is_reused_without_binding() {
        let mut it = sail_ast::SimpleInterner::new();
        let rhs = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("v"),
        )));
        let mut ast = concat_ast(&mut it, &["x", "y"], rhs);
        let lens = FixedWidths { order_inc: false, start: 1, width: 1 };
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &lens), 1);
        let sail_ast::ExpKind::Block(assigns) = &single_exp(&ast).kind else {
            panic!("expected block, got {:?}", single_exp(&ast).kind)
        };
        assert_eq!(assigns.len(), 2);
        // `x = vector_subrange#(v, 1, 1)`, `y = vector_subrange#(v, 0, 0)`.
        for (assign, bound) in assigns.iter().zip([1, 0]) {
            let sail_ast::ExpKind::Assign { value, .. } = &assign.kind else {
                panic!("expected assign")
            };
            let sail_ast::ExpKind::App { func, args } = &value.kind else {
                panic!("expected slice app")
            };
            assert_eq!(it.resolve(func.symbol), Some("vector_subrange#"));
            assert_eq!(args.len(), 3);
            assert!(matches!(args[0].kind, sail_ast::ExpKind::Id(_)));
            for arg in &args[1..] {
                let sail_ast::ExpKind::Lit(lit) = &arg.kind else {
                    panic!("expected num lit")
                };
                let sail_ast::LitKind::Num(symbol) = lit.kind else {
                    panic!("expected num, got {:?}", lit.kind)
                };
                assert_eq!(it.resolve(symbol), Some(if bound == 1 { "1" } else { "0" }));
            }
        }
        // Idempotent: lowered assigns carry no concatenation lexp.
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &lens), 0);
    }

    #[test]
    fn effectful_rhs_is_bound_once_before_any_write() {
        let mut it = sail_ast::SimpleInterner::new();
        let rhs = sail_ast::Node::dummy(sail_ast::ExpKind::App {
            func: sail_ast::Name::dummy(it.intern("f")),
            args: vec![],
        });
        let mut ast = concat_ast(&mut it, &["x", "y"], rhs);
        let lens = FixedWidths { order_inc: false, start: 1, width: 1 };
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &lens), 1);
        let sail_ast::ExpKind::Let { pat, bind, body } = &single_exp(&ast).kind else {
            panic!("expected let, got {:?}", single_exp(&ast).kind)
        };
        let sail_ast::PatKind::Id(name) = &pat.kind else {
            panic!("expected id pat")
        };
        assert_eq!(it.resolve(name.symbol), Some("split_vec"));
        assert!(matches!(bind.kind, sail_ast::ExpKind::App { .. }));
        let sail_ast::ExpKind::Block(assigns) = &body.kind else {
            panic!("expected block")
        };
        assert_eq!(assigns.len(), 2);
    }

    #[test]
    fn avoids_capture_of_existing_split_vec() {
        let mut it = sail_ast::SimpleInterner::new();
        // Plant a user `split_vec` so the pass must pick a fresh binder.
        let user = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("split_vec"),
        )));
        let rhs = sail_ast::Node::dummy(sail_ast::ExpKind::App {
            func: sail_ast::Name::dummy(it.intern("f")),
            args: vec![],
        });
        let mut ast = sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
            exp: sail_ast::Node::dummy(sail_ast::ExpKind::Block(vec![
                user,
                sail_ast::Node::dummy(sail_ast::ExpKind::Assign {
                    lexp: Box::new(sail_ast::Node::dummy(sail_ast::LExpKind::VectorConcat(vec![
                        sail_ast::Node::dummy(sail_ast::LExpKind::Id(sail_ast::Name::dummy(
                            it.intern("x"),
                        ))),
                    ]))),
                    value: Box::new(rhs),
                }),
            ])),
        })]);
        let lens = FixedWidths { order_inc: true, start: 0, width: 2 };
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &lens), 1);
        let sail_ast::DefKind::Let { exp, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        let sail_ast::ExpKind::Block(es) = &exp.kind else {
            panic!("exp shape")
        };
        let sail_ast::ExpKind::Let { pat, .. } = &es[1].kind else {
            panic!("expected let, got {:?}", es[1].kind)
        };
        let sail_ast::PatKind::Id(name) = &pat.kind else {
            panic!("expected id pat")
        };
        assert_eq!(it.resolve(name.symbol), Some("split_vec_1"));
    }

    #[test]
    fn unknown_widths_and_empty_concats_survive() {
        let mut it = sail_ast::SimpleInterner::new();
        let rhs = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("v"),
        )));
        let mut ast = concat_ast(&mut it, &["x"], rhs);
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &Unknown), 0);
        assert!(matches!(single_exp(&ast).kind, sail_ast::ExpKind::Assign { .. }));
        // An empty concatenation lexp is degenerate and left alone.
        let v = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("v"),
        )));
        let mut ast = concat_ast(&mut it, &[], v);
        let lens = FixedWidths { order_inc: true, start: 0, width: 1 };
        assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &lens), 0);
    }

    #[test]
    fn string_level_is_identity() {
        use crate::ast::{Def, DefKind, Shard, Stmt};
        let shard = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Expr("(x @ y) = f()".into())],
            )],
        );
        assert_eq!(VectorConcatAssignments.run(shard.clone()), shard);
        assert_eq!(VectorConcatAssignments.name(), "vector_concat_assignments");
    }
}
