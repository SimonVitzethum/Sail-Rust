//! Constant-argument specialization for mutually recursive functions,
//! matching `Constant_propagation_mutrec.rewrite_ast` in
//! `sail/src/lib/constant_propagation_mutrec.ml` (registered as
//! `"const_prop_mutrec"` in `sail/src/lib/rewrites.ml`; Lean passes
//! `[String_arg "lean"]`, which only seeds the constant folder and needs
//! no equivalent here).
//!
//! When a call inside an `InternalMutrec` group targets a group member
//! with (some) syntactic-constant arguments, a specialized copy
//! `f#mutrec_<suffixes>` is created with the constants substituted into
//! its clauses, and the call site is rewritten to the copy (dropping the
//! constant arguments, or passing unit when all are constant). Guards and
//! termination measures are specialized the same way. Only calls inside
//! the group are rewritten; external callers keep calling the generic
//! version, as in OCaml.
//!
//! Gaps on the untyped AST (documented, no guessing):
//! - `is_const_exp` needs a type environment for two cases: identifiers
//!   (only enum constructors count) and vectors (only bitvector-typed
//!   ones count). Here identifiers are never constant (conservative) and
//!   vectors/structs of constants always are (substituting a closed
//!   constant is semantics-preserving regardless of type). Unit, string
//!   and real literals are not constant, matching OCaml.
//! - Copy suffixes encode literals directly (`T`, `F`, `#42`, ...);
//!   OCaml `zencode`s pretty-printed expressions. Names are internal
//!   `#mutrec_` identifiers, never user-visible.
//! - No `val` specs are generated for copies (needs function types),
//!   type/kind substitutions and `check_defs` re-checking are skipped, and
//!   the command-line `targets` pre-propagation is inactive (empty in Lean
//!   runs). The global `top_sort_defs` OCaml runs at the end is a separate
//!   pass in Lean order; copies are appended in creation order directly
//!   after their group instead.
//! - The original group is kept as `InternalMutrec` (OCaml explodes it
//!   into separate fundefs as scaffolding for its typechecker round trip,
//!   which has no untyped counterpart). Later passes all understand the
//!   grouping, so this is semantics-preserving for the pipeline.
//! - A clause the constants cannot match (arity mismatch, or a constant
//!   argument against a non-identifier/non-wildcard pattern) abandons that
//!   specialization and preserves the original call. OCaml raises here;
//!   `TypedPass` has no error channel, so graceful preservation is the
//!   honest subset.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"const_prop_mutrec"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 28, zwischen
//!   `"add_register_init_function"` (27) und `"exp_lift_assign"` (29).
//! - Trait-Impl-Name: `ConstPropMutrec` (diese Datei).
//!   `passes/mod.rs`: `pub mod const_prop_mutrec;` +
//!   `pub use const_prop_mutrec::ConstPropMutrec;` ergaenzen.
//!   `pipeline.rs`: `"const_prop_mutrec" => Some(Arc::new(ConstPropMutrec))`
//!   in `make_typed_implemented` und den Namen in
//!   `is_typed_implemented_pass` aufnehmen. Dortige Tests anpassen:
//!   `typed_implemented_lists_only_wired_passes` muss danach (mit den
//!   Lane-60-Paessen, Lean-Ordnung, verifiziert)
//!   `["move_termination_measures", "remove_duplicate_valspecs",
//!   "pat_string_append", "add_unspecified_rec", "undefined",
//!   "tuple_assignments", "simple_assignments", "split",
//!   "minimise_recursive_functions", "const_prop_mutrec",
//!   "merge_function_clauses", "remove_blocks",
//!   "remove_superfluous_letbinds"]` erwarten; ggf.
//!   `tests/typed_pipeline.rs` ebenso, falls es implementierte Listen
//!   aufzählt.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{
    Def, DefKind, Exp, ExpKind, Funcl, FunclSet, Interner, LExp, LitKind, Name, Node, PExp,
    PExpKind, Pat, PatKind, RecOpt, Symbol, TAnnotOpt,
};

use crate::pass::TypedPass;

use super::typed::{mk_exp, mk_name, mk_pat};

#[derive(Clone, Copy, Debug, Default)]
pub struct ConstPropMutrec;

impl TypedPass for ConstPropMutrec {
    fn name(&self) -> &'static str {
        "const_prop_mutrec"
    }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        let defs = std::mem::take(&mut ast.defs);
        let mut out = Vec::with_capacity(defs.len());
        for def in defs {
            let Node { id, span, kind, annotation } = def;
            match kind {
                DefKind::InternalMutrec(sets) if !sets.is_empty() => {
                    let mut ctx = GroupCtx::new(interner, &sets);
                    let mut sets = sets;
                    for set in &mut sets {
                        for funcl in &mut set.funcls {
                            rewrite_pexp(&mut funcl.arm, &mut ctx);
                        }
                    }
                    let copies = ctx.into_copies();
                    out.push(Node {
                        id,
                        span,
                        kind: DefKind::InternalMutrec(sets),
                        annotation,
                    });
                    out.extend(copies);
                }
                kind => out.push(Node { id, span, kind, annotation }),
            }
        }
        ast.defs = out;
    }
}

/// Per-group working state. Member clauses are cloned up front so call
/// rewriting (which holds `&mut` into one clause) can read any other
/// member's clauses without aliasing.
struct GroupCtx<'a> {
    interner: &'a mut dyn Interner,
    /// Symbols of the functions in the current group.
    members: BTreeSet<u32>,
    /// Cloned (name symbol, rec, clauses) per group member.
    snapshot: Vec<(u32, RecOpt, Vec<Funcl>)>,
    /// Emitted copies in creation order (name symbol, definition).
    copies: Vec<(u32, Def)>,
    copy_syms: BTreeSet<u32>,
}

impl<'a> GroupCtx<'a> {
    fn new(interner: &'a mut dyn Interner, sets: &[FunclSet]) -> Self {
        let mut members = BTreeSet::new();
        let mut snapshot = Vec::with_capacity(sets.len());
        for set in sets {
            if let Some(first) = set.funcls.first() {
                members.insert(first.name.symbol.0);
                snapshot.push((first.name.symbol.0, set.rec.clone(), set.funcls.clone()));
            }
        }
        Self {
            interner,
            members,
            snapshot,
            copies: Vec::new(),
            copy_syms: BTreeSet::new(),
        }
    }

    fn into_copies(self) -> Vec<Def> {
        self.copies.into_iter().map(|(_, d)| d).collect()
    }
}

/// Whether `exp` is a syntactic constant (`is_const_exp` in OCaml, minus
/// the environment-dependent identifier/vector-typing cases).
fn is_const_exp(exp: &Exp) -> bool {
    match &exp.kind {
        ExpKind::Lit(lit) => matches!(
            &lit.kind,
            LitKind::True | LitKind::False | LitKind::Num(_) | LitKind::Hex(_) | LitKind::Bin(_)
        ),
        ExpKind::Vector(es) => es.iter().all(is_const_exp),
        ExpKind::Struct { fields, .. } => fields.iter().all(|f| is_const_exp(&f.value)),
        ExpKind::Typ { exp: inner, .. } => is_const_exp(inner),
        _ => false,
    }
}

/// Suffix encoding one constant argument (`generate_fun_id` in OCaml,
/// with direct literal rendering instead of `zencode_string`).
fn const_suffix(exp: &Exp, interner: &dyn Interner) -> String {
    match &exp.kind {
        ExpKind::Lit(lit) => match &lit.kind {
            LitKind::True => "T".to_owned(),
            LitKind::False => "F".to_owned(),
            LitKind::Num(s) | LitKind::Hex(s) | LitKind::Bin(s) | LitKind::String(s) => {
                format!("#{}", sanitize(interner.resolve(*s).unwrap_or("c")))
            }
            LitKind::Real(s) => {
                format!("#{}", sanitize(interner.resolve(*s).unwrap_or("c")))
            }
            LitKind::Unit => "unit".to_owned(),
        },
        ExpKind::Vector(es) => es.iter().map(|e| const_suffix(e, interner)).collect(),
        ExpKind::Struct { fields, .. } => {
            let mut out = String::from("struct");
            for f in fields {
                out.push_str(&const_suffix(&f.value, interner));
            }
            out
        }
        ExpKind::Typ { exp: inner, .. } => const_suffix(inner, interner),
        _ => "c".to_owned(),
    }
}

fn sanitize(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if clean.is_empty() {
        "c".to_owned()
    } else {
        clean
    }
}

fn rewrite_pexp(arm: &mut sail_ast::PExp, ctx: &mut GroupCtx<'_>) {
    match &mut arm.kind {
        PExpKind::PatExp { body, .. } => rewrite_exp(body, ctx),
        PExpKind::PatWhen { guard, body, .. } => {
            rewrite_exp(guard, ctx);
            rewrite_exp(body, ctx);
        }
    }
}

/// Bottom-up call rewriting: children first (mirroring `fold_pexp` with
/// `e_aux`), then the node itself.
fn rewrite_exp(exp: &mut Exp, ctx: &mut GroupCtx<'_>) {
    rewrite_children(exp, ctx);
    let (func_sym, args) = match &mut exp.kind {
        ExpKind::App { func, args }
            if ctx.members.contains(&func.symbol.0) && args.iter().any(is_const_exp) =>
        {
            (func.symbol, std::mem::take(args))
        }
        _ => return,
    };
    match specialize(ctx, func_sym, &args) {
        Some(copy) => {
            let remaining: Vec<Exp> =
                args.into_iter().filter(|a| !is_const_exp(a)).collect();
            let args = if remaining.is_empty() {
                vec![mk_exp(ExpKind::Lit(Node::dummy(LitKind::Unit)))]
            } else {
                remaining
            };
            *exp = mk_exp(ExpKind::App { func: mk_name(copy), args });
        }
        // Abandoned specialization: restore the original call.
        None => {
            if let ExpKind::App { args: slot, .. } = &mut exp.kind {
                *slot = args;
            }
        }
    }
}

/// Create (or reuse) the specialized copy for one call site. Returns the
/// copy's symbol, or `None` when the call must be preserved.
fn specialize(ctx: &mut GroupCtx<'_>, func: Symbol, args: &[Exp]) -> Option<Symbol> {
    let func_text: String = ctx.interner.resolve(func).unwrap_or("unknown").to_owned();
    let mut name = format!("{func_text}#mutrec_");
    for arg in args.iter().filter(|a| is_const_exp(a)) {
        name.push_str(&const_suffix(arg, ctx.interner));
    }
    let copy_sym = ctx.interner.intern(&name);
    if ctx.copy_syms.contains(&copy_sym.0) {
        return Some(copy_sym);
    }
    let (_, rec, funcls) = ctx.snapshot.iter().find(|(sym, _, _)| *sym == func.0)?;
    let (rec, funcls) = (rec.clone(), funcls.clone());
    let copy_name = mk_name(copy_sym);
    let mut new_funcls = Vec::with_capacity(funcls.len());
    for funcl in &funcls {
        new_funcls.push(specialize_funcl(funcl, &copy_name, args)?);
    }
    let new_rec = match rec {
        RecOpt::Measure { pat, exp } => {
            let (pat, _, exp) = specialize_arm(&pat, None, &exp, args)?;
            RecOpt::Measure { pat, exp }
        }
        rec => rec,
    };
    ctx.copy_syms.insert(copy_sym.0);
    ctx.copies.push((
        copy_sym.0,
        Node::dummy(DefKind::Fundef {
            rec: new_rec,
            annot: TAnnotOpt::None,
            funcls: new_funcls,
        }),
    ));
    Some(copy_sym)
}

/// Specialize one clause: match call args against the clause patterns
/// (`prop_args_pexp` in OCaml). `None` abandons the whole copy.
fn specialize_funcl(funcl: &Funcl, copy: &Name, args: &[Exp]) -> Option<Funcl> {
    let (pat, guard, body) = match &funcl.arm.kind {
        PExpKind::PatExp { pat, body } => (pat, None, body),
        PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
    };
    let (pat, guard, body) = specialize_arm(pat, guard, body, args)?;
    let kind = match guard {
        Some(g) => PExpKind::PatWhen { pat, guard: g, body },
        None => PExpKind::PatExp { pat, body },
    };
    Some(Funcl {
        id: funcl.id,
        span: funcl.span,
        name: copy.clone(),
        arm: PExp { id: funcl.id, span: funcl.span, kind },
    })
}

/// Match `args` against `pat`, dropping constant-matched patterns and
/// substituting them into guard/body. Returns the new arm triple.
fn specialize_arm(
    pat: &Pat,
    guard: Option<&Exp>,
    body: &Exp,
    args: &[Exp],
) -> Option<(Pat, Option<Exp>, Exp)> {
    let pats: Vec<Pat> = match &pat.kind {
        PatKind::Tuple(ps) => ps.clone(),
        _ => vec![pat.clone()],
    };
    if pats.len() != args.len() {
        return None;
    }
    let mut kept = Vec::with_capacity(pats.len());
    let mut substs: BTreeMap<u32, Exp> = BTreeMap::new();
    for (arg, pat) in args.iter().zip(&pats) {
        if !is_const_exp(arg) {
            kept.push(pat.clone());
            continue;
        }
        match &pat.kind {
            PatKind::Id(n) => {
                substs.insert(n.symbol.0, arg.clone());
            }
            PatKind::Typ { pat: inner, .. } => match &inner.kind {
                PatKind::Id(n) => {
                    substs.insert(n.symbol.0, arg.clone());
                }
                PatKind::Wild => {}
                _ => return None,
            },
            PatKind::Wild => {}
            _ => return None,
        }
    }
    let new_pat = match kept.as_slice() {
        [single] => single.clone(),
        _ => mk_pat(PatKind::Tuple(kept)),
    };
    let new_guard = guard.map(|g| {
        let mut g = g.clone();
        subst_exp(&mut g, &substs);
        g
    });
    let mut new_body = body.clone();
    subst_exp(&mut new_body, &substs);
    Some((new_pat, new_guard, new_body))
}

/// Substitute bound constants, respecting shadowing. The substituted
/// constants are closed literals, so capture is impossible; only shadowing
/// (re-binding the same name) must narrow the map.
fn subst_exp(exp: &mut Exp, substs: &BTreeMap<u32, Exp>) {
    if substs.is_empty() {
        return;
    }
    match &mut exp.kind {
        ExpKind::Id(n) => {
            if let Some(rep) = substs.get(&n.symbol.0) {
                *exp = rep.clone();
            }
        }
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                subst_exp(e, substs);
            }
        }
        ExpKind::Typ { exp: e, .. } => subst_exp(e, substs),
        ExpKind::App { args, .. } => {
            for e in args {
                subst_exp(e, substs);
            }
        }
        ExpKind::If { cond, then_branch, else_branch } => {
            subst_exp(cond, substs);
            subst_exp(then_branch, substs);
            subst_exp(else_branch, substs);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                subst_exp(m, substs);
            }
            subst_exp(cond, substs);
            subst_exp(body, substs);
        }
        ExpKind::For { var, from, to, step, body, .. } => {
            subst_exp(from, substs);
            subst_exp(to, substs);
            subst_exp(step, substs);
            subst_exp(body, &narrow(substs, &single(var.symbol.0)));
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            subst_exp(a, substs);
            subst_exp(b, substs);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                subst_exp(&mut f.value, substs);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            subst_exp(record, substs);
            for f in fields {
                subst_exp(&mut f.value, substs);
            }
        }
        ExpKind::Field { record, .. } => subst_exp(record, substs),
        ExpKind::Match { scrutinee, arms } => {
            subst_exp(scrutinee, substs);
            for a in arms {
                subst_arm(a, substs);
            }
        }
        ExpKind::Let { pat, bind, body } | ExpKind::InternalPlet { pat, bind, body, .. } => {
            subst_exp(bind, substs);
            subst_exp(body, &narrow(substs, &pat_bound_ids(pat)));
        }
        ExpKind::Assign { lexp, value } => {
            subst_lexp(lexp, substs);
            subst_exp(value, substs);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => subst_exp(e, substs),
        ExpKind::Try { exp: e, arms } => {
            subst_exp(e, substs);
            for a in arms {
                subst_arm(a, substs);
            }
        }
        ExpKind::Assert { cond, msg } => {
            subst_exp(cond, substs);
            subst_exp(msg, substs);
        }
        ExpKind::Var { lexp, init, body } => {
            subst_lexp(lexp, substs);
            subst_exp(init, substs);
            subst_exp(body, &narrow(substs, &lexp_bound_ids(lexp)));
        }
        ExpKind::InternalAssume { exp: e, .. } => subst_exp(e, substs),
        // `Ref` names a register, not a variable: never substituted.
        ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
    }
}

fn subst_arm(arm: &mut sail_ast::PExp, substs: &BTreeMap<u32, Exp>) {
    match &mut arm.kind {
        PExpKind::PatExp { pat, body } => {
            subst_exp(body, &narrow(substs, &pat_bound_ids(pat)));
        }
        PExpKind::PatWhen { pat, guard, body } => {
            let narrowed = narrow(substs, &pat_bound_ids(pat));
            subst_exp(guard, &narrowed);
            subst_exp(body, &narrowed);
        }
    }
}

/// Substitute inside lvalue index expressions only; bound roots are
/// targets, never substitution sites.
fn subst_lexp(lexp: &mut LExp, substs: &BTreeMap<u32, Exp>) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(e) => subst_exp(e, substs),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            subst_lexp(inner, substs);
            subst_exp(index, substs);
        }
        sail_ast::LExpKind::VectorRange { lexp: inner, lo, hi } => {
            subst_lexp(inner, substs);
            subst_exp(lo, substs);
            subst_exp(hi, substs);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => subst_lexp(inner, substs),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                subst_lexp(p, substs);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

fn single(sym: u32) -> BTreeSet<u32> {
    BTreeSet::from([sym])
}

fn narrow(substs: &BTreeMap<u32, Exp>, bound: &BTreeSet<u32>) -> BTreeMap<u32, Exp> {
    if substs.keys().any(|k| bound.contains(k)) {
        substs.iter().filter(|(k, _)| !bound.contains(k)).map(|(k, v)| (*k, v.clone())).collect()
    } else {
        substs.clone()
    }
}

/// Value variables bound by a pattern.
fn pat_bound_ids(pat: &Pat) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    collect_pat_bound(pat, &mut out);
    out
}

fn collect_pat_bound(pat: &Pat, out: &mut BTreeSet<u32>) {
    match &pat.kind {
        PatKind::Id(n) => {
            out.insert(n.symbol.0);
        }
        PatKind::As { pat: inner, id } => {
            out.insert(id.symbol.0);
            collect_pat_bound(inner, out);
        }
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            collect_pat_bound(a, out);
            collect_pat_bound(b, out);
        }
        PatKind::Not(inner) | PatKind::Typ { pat: inner, .. } | PatKind::Var { pat: inner, .. } => {
            collect_pat_bound(inner, out);
        }
        PatKind::App { args, .. }
        | PatKind::Tuple(args)
        | PatKind::Vector(args)
        | PatKind::VectorConcat(args)
        | PatKind::List(args)
        | PatKind::StringAppend(args) => {
            for p in args {
                collect_pat_bound(p, out);
            }
        }
        PatKind::Struct { fields, .. } => {
            for (_, p) in fields {
                collect_pat_bound(p, out);
            }
        }
        PatKind::VectorSubrange { id, .. } => {
            out.insert(id.symbol.0);
        }
        PatKind::Lit(_) | PatKind::Wild => {}
    }
}

/// Variables bound by an lvalue.
fn lexp_bound_ids(lexp: &LExp) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    collect_lexp_bound(lexp, &mut out);
    out
}

fn collect_lexp_bound(lexp: &LExp, out: &mut BTreeSet<u32>) {
    match &lexp.kind {
        sail_ast::LExpKind::Id(n) => {
            out.insert(n.symbol.0);
        }
        sail_ast::LExpKind::Typ { id, .. } => {
            out.insert(id.symbol.0);
        }
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                collect_lexp_bound(p, out);
            }
        }
        sail_ast::LExpKind::Vector { lexp: inner, .. }
        | sail_ast::LExpKind::VectorRange { lexp: inner, .. }
        | sail_ast::LExpKind::Field { lexp: inner, .. } => collect_lexp_bound(inner, out),
        sail_ast::LExpKind::Deref(_) => {}
    }
}

fn rewrite_children(exp: &mut Exp, ctx: &mut GroupCtx<'_>) {
    match &mut exp.kind {
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                rewrite_exp(e, ctx);
            }
        }
        ExpKind::Typ { exp: e, .. } => rewrite_exp(e, ctx),
        ExpKind::App { args, .. } => {
            for e in args {
                rewrite_exp(e, ctx);
            }
        }
        ExpKind::If { cond, then_branch, else_branch } => {
            rewrite_exp(cond, ctx);
            rewrite_exp(then_branch, ctx);
            rewrite_exp(else_branch, ctx);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                rewrite_exp(m, ctx);
            }
            rewrite_exp(cond, ctx);
            rewrite_exp(body, ctx);
        }
        ExpKind::For { from, to, step, body, .. } => {
            rewrite_exp(from, ctx);
            rewrite_exp(to, ctx);
            rewrite_exp(step, ctx);
            rewrite_exp(body, ctx);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            rewrite_exp(a, ctx);
            rewrite_exp(b, ctx);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                rewrite_exp(&mut f.value, ctx);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            rewrite_exp(record, ctx);
            for f in fields {
                rewrite_exp(&mut f.value, ctx);
            }
        }
        ExpKind::Field { record, .. } => rewrite_exp(record, ctx),
        ExpKind::Match { scrutinee, arms } => {
            rewrite_exp(scrutinee, ctx);
            for a in arms {
                rewrite_pexp(a, ctx);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            rewrite_exp(bind, ctx);
            rewrite_exp(body, ctx);
        }
        ExpKind::Assign { lexp, value } => {
            rewrite_lexp(lexp, ctx);
            rewrite_exp(value, ctx);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => rewrite_exp(e, ctx),
        ExpKind::Try { exp: e, arms } => {
            rewrite_exp(e, ctx);
            for a in arms {
                rewrite_pexp(a, ctx);
            }
        }
        ExpKind::Assert { cond, msg } => {
            rewrite_exp(cond, ctx);
            rewrite_exp(msg, ctx);
        }
        ExpKind::Var { lexp, init, body } => {
            rewrite_lexp(lexp, ctx);
            rewrite_exp(init, ctx);
            rewrite_exp(body, ctx);
        }
        ExpKind::InternalAssume { exp: e, .. } => rewrite_exp(e, ctx),
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

fn rewrite_lexp(lexp: &mut LExp, ctx: &mut GroupCtx<'_>) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(e) => rewrite_exp(e, ctx),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            rewrite_lexp(inner, ctx);
            rewrite_exp(index, ctx);
        }
        sail_ast::LExpKind::VectorRange { lexp: inner, lo, hi } => {
            rewrite_lexp(inner, ctx);
            rewrite_exp(lo, ctx);
            rewrite_exp(hi, ctx);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => rewrite_lexp(inner, ctx),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                rewrite_lexp(p, ctx);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, NodeId, SimpleInterner, Span};

    fn nm(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn pexp(kind: PExpKind) -> sail_ast::PExp {
        sail_ast::PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind,
        }
    }

    fn id(it: &mut SimpleInterner, text: &str) -> Exp {
        mk_exp(ExpKind::Id(nm(it, text)))
    }

    fn num(it: &mut SimpleInterner, text: &str) -> Exp {
        mk_exp(ExpKind::Lit(Node::dummy(LitKind::Num(it.intern(text)))))
    }

    fn call(it: &mut SimpleInterner, func: &str, args: Vec<Exp>) -> Exp {
        mk_exp(ExpKind::App { func: nm(it, func), args })
    }

    fn funcl(it: &mut SimpleInterner, name: &str, pat: Pat, body: Exp) -> Funcl {
        Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, name),
            arm: pexp(PExpKind::PatExp { pat, body }),
        }
    }

    fn wild() -> Pat {
        mk_pat(PatKind::Wild)
    }

    fn group(members: Vec<(RecOpt, Vec<Funcl>)>) -> Def {
        Node::dummy(DefKind::InternalMutrec(
            members
                .into_iter()
                .map(|(rec, funcls)| FunclSet {
                    rec,
                    annot: TAnnotOpt::None,
                    funcls,
                })
                .collect(),
        ))
    }

    fn copy_name_of(def: &Def, it: &SimpleInterner) -> String {
        let DefKind::Fundef { funcls, .. } = &def.kind else {
            panic!("not a fundef")
        };
        it.resolve(funcls[0].name.symbol).unwrap_or("?").to_owned()
    }

    #[test]
    fn specializes_single_const_call_and_passes_unit() {
        let mut it = SimpleInterner::new();
        // g(n) = n ; f() = g(1)
        let gn = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let gb = id(&mut it, "n");
        let g = funcl(&mut it, "g", gn, gb);
        let one = num(&mut it, "1");
        let fc = call(&mut it, "g", vec![one]);
        let f = funcl(&mut it, "f", wild(), fc);
        let mut ast = Ast::new(vec![group(
            vec![(RecOpt::NonRec, vec![g]), (RecOpt::NonRec, vec![f])],
        )]);
        ConstPropMutrec.run(&mut ast, &mut it);
        // Group kept, one copy appended.
        assert_eq!(ast.defs.len(), 2);
        assert_eq!(copy_name_of(&ast.defs[1], &it), "g#mutrec_#1");
        // Copy clause dropped the constant pattern and substituted it.
        let DefKind::Fundef { funcls, .. } = &ast.defs[1].kind else {
            panic!("copy shape")
        };
        assert!(matches!(&funcls[0].arm.kind, PExpKind::PatExp { pat, .. } if matches!(&pat.kind, PatKind::Tuple(ps) if ps.is_empty())));
        let PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
            unreachable!()
        };
        assert!(matches!(&body.kind, ExpKind::Lit(_)));
        // Call site now targets the copy with unit.
        let DefKind::InternalMutrec(sets) = &ast.defs[0].kind else {
            panic!("group shape")
        };
        let PExpKind::PatExp { body, .. } = &sets[1].funcls[0].arm.kind else {
            panic!("arm shape")
        };
        let ExpKind::App { func, args } = &body.kind else {
            panic!("call shape")
        };
        assert_eq!(it.resolve(func.symbol), Some("g#mutrec_#1"));
        assert!(matches!(args.as_slice(), [single] if matches!(&single.kind, ExpKind::Lit(l) if matches!(&l.kind, LitKind::Unit))));
        // Fixpoint: a second run changes nothing.
        let snapshot = ast.clone();
        ConstPropMutrec.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn keeps_var_and_mixed_args_positional() {
        let mut it = SimpleInterner::new();
        // h(a, b) = a ; f(y) = h(0, y)
        let pa = mk_pat(PatKind::Id(nm(&mut it, "a")));
        let pb = mk_pat(PatKind::Id(nm(&mut it, "b")));
        let ha = id(&mut it, "a");
        let h = funcl(&mut it, "h", mk_pat(PatKind::Tuple(vec![pa, pb])), ha);
        let py = mk_pat(PatKind::Id(nm(&mut it, "y")));
        let zero = num(&mut it, "0");
        let yy = id(&mut it, "y");
        let fc = call(&mut it, "h", vec![zero, yy]);
        let f = funcl(&mut it, "f", py, fc);
        let mut ast = Ast::new(vec![group(
            vec![(RecOpt::NonRec, vec![h]), (RecOpt::NonRec, vec![f])],
        )]);
        ConstPropMutrec.run(&mut ast, &mut it);
        assert_eq!(copy_name_of(&ast.defs[1], &it), "h#mutrec_#0");
        let DefKind::Fundef { funcls, .. } = &ast.defs[1].kind else {
            unreachable!()
        };
        // Only the variable pattern remains; body uses the constant.
        let PExpKind::PatExp { pat, body } = &funcls[0].arm.kind else {
            unreachable!()
        };
        assert!(matches!(&pat.kind, PatKind::Id(n) if it.resolve(n.symbol) == Some("b")));
        assert!(matches!(&body.kind, ExpKind::Lit(_)));
        let DefKind::InternalMutrec(sets) = &ast.defs[0].kind else {
            panic!("group shape")
        };
        let PExpKind::PatExp { body, .. } = &sets[1].funcls[0].arm.kind else {
            panic!("arm shape")
        };
        let ExpKind::App { func, args } = &body.kind else {
            panic!("call shape")
        };
        assert_eq!(it.resolve(func.symbol), Some("h#mutrec_#0"));
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0].kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("y")));
    }

    #[test]
    fn ignores_non_const_external_and_unsupported_calls() {
        let mut it = SimpleInterner::new();
        // g(n) = n ; f(x) = (g(x), ext(1), litmatch(2))
        let gn = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let gb = id(&mut it, "n");
        let g = funcl(&mut it, "g", gn, gb);
        let zero = Node::dummy(LitKind::Num(it.intern("0")));
        let litmatch = funcl(
            &mut it,
            "litmatch",
            mk_pat(PatKind::Lit(zero)),
            mk_exp(ExpKind::Undef),
        );
        let px = mk_pat(PatKind::Id(nm(&mut it, "x")));
        let cx = id(&mut it, "x");
        let one = num(&mut it, "1");
        let two = num(&mut it, "2");
        let t0 = call(&mut it, "g", vec![cx]);
        let t1 = call(&mut it, "ext", vec![one]);
        let t2 = call(&mut it, "litmatch", vec![two]);
        let f = funcl(&mut it, "f", px, mk_exp(ExpKind::Tuple(vec![t0, t1, t2])));
        let mut ast = Ast::new(vec![group(
            vec![
                (RecOpt::NonRec, vec![g]),
                (RecOpt::NonRec, vec![litmatch]),
                (RecOpt::NonRec, vec![f]),
            ],
        )]);
        let snapshot = ast.clone();
        ConstPropMutrec.run(&mut ast, &mut it);
        // Variable call, non-member call, and constant-against-literal
        // (unsupported shape) all preserved; no copies emitted.
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn substitution_respects_shadowing() {
        let mut it = SimpleInterner::new();
        // g(n) = (let n = 5 in n, n) ; f() = g(1)
        let lp = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let lb = num(&mut it, "5");
        let li = id(&mut it, "n");
        let inner_let = mk_exp(ExpKind::Let {
            pat: lp,
            bind: Box::new(lb),
            body: Box::new(li),
        });
        let gp = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let free = id(&mut it, "n");
        let g = funcl(&mut it, "g", gp, mk_exp(ExpKind::Tuple(vec![inner_let, free])));
        let one = num(&mut it, "1");
        let fc = call(&mut it, "g", vec![one]);
        let f = funcl(&mut it, "f", wild(), fc);
        let mut ast = Ast::new(vec![group(
            vec![(RecOpt::NonRec, vec![g]), (RecOpt::NonRec, vec![f])],
        )]);
        ConstPropMutrec.run(&mut ast, &mut it);
        let DefKind::Fundef { funcls, .. } = &ast.defs[1].kind else {
            panic!("copy shape")
        };
        let PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
            panic!("arm shape")
        };
        // Shadowed `n` keeps its local binding; free `n` became `1`.
        let ExpKind::Tuple(parts) = &body.kind else {
            panic!("body shape")
        };
        let ExpKind::Let { bind, body: inner, .. } = &parts[0].kind else {
            panic!("let shape")
        };
        assert!(matches!(&bind.kind, ExpKind::Lit(_)));
        assert!(matches!(&inner.kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("n")));
        assert!(matches!(&parts[1].kind, ExpKind::Lit(_)));
    }

    #[test]
    fn specializes_measures_and_reuses_copies() {
        let mut it = SimpleInterner::new();
        // g(n) = n with measure (n) ; f() = (g(2), g(2))
        let gp = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let gb = id(&mut it, "n");
        let g = funcl(&mut it, "g", gp, gb);
        let two_a = num(&mut it, "2");
        let two_b = num(&mut it, "2");
        let ca = call(&mut it, "g", vec![two_a]);
        let cb = call(&mut it, "g", vec![two_b]);
        let f = funcl(&mut it, "f", wild(), mk_exp(ExpKind::Tuple(vec![ca, cb])));
        let mp = mk_pat(PatKind::Id(nm(&mut it, "n")));
        let me = id(&mut it, "n");
        let mut ast = Ast::new(vec![group(
            vec![
                (
                    RecOpt::Measure { pat: mp, exp: me },
                    vec![g],
                ),
                (RecOpt::NonRec, vec![f]),
            ],
        )]);
        ConstPropMutrec.run(&mut ast, &mut it);
        // One copy despite two identical call sites.
        assert_eq!(ast.defs.len(), 2);
        let DefKind::Fundef { rec, funcls, .. } = &ast.defs[1].kind else {
            panic!("copy shape")
        };
        assert_eq!(funcls.len(), 1);
        // Measure pattern `m` matched the constant call argument, so it
        // is dropped (empty tuple) while the measure body is substituted.
        let RecOpt::Measure { pat, exp } = rec else {
            panic!("measure shape")
        };
        assert!(matches!(&pat.kind, PatKind::Tuple(ps) if ps.is_empty()));
        assert!(matches!(&exp.kind, ExpKind::Lit(_)));
    }
}
