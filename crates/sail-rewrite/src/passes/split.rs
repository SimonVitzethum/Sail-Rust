//! Split union-constructor clauses into auxiliary functions, matching
//! `rewrite_split_fun_ctor_pats` in `sail/src/lib/rewrites.ml`.
//!
//! Lean runs this pass as `("split", [String_arg "execute"])` (see
//! `lean_rewrites` in `sail/src/sail_lean_backend/sail_plugin_lean.ml`), so
//! the target function name is fixed to `"execute"` here: the pipeline has
//! no per-pass argument channel (`pipeline::LEAN_REWRITES` is names only).
//!
//! For every clause of `execute` whose pattern applies a union constructor,
//! `execute Ctor(args) = body` becomes a forwarder
//! `execute Ctor(free args) = execute_Ctor(free args)` while the original
//! clause body moves to a new `execute_Ctor` function emitted directly after
//! `execute` (one auxiliary function per constructor, clauses in source
//! order). A single-tuple-wrapped pattern `execute (Ctor(args))` is
//! unwrapped on both sides, as in OCaml. Direct construction calls
//! `execute Ctor(args)` in any `Fundef` body or guard are rewritten to
//! `execute_Ctor(args)` (`optimize_split_call`), but only for constructors
//! that were actually split out.
//!
//! Gaps on the untyped AST (documented, no guessing):
//! - OCaml checks constructors against the type environment
//!   (`Env.get_union_id`, `Env.is_union_constructor`) and generates typed
//!   `val` specs for the auxiliary functions. Neither is available here:
//!   every top-level `P_app` clause pattern is treated as a union
//!   constructor (Sail patterns have no other applicative form), and no
//!   val specs are emitted. A later `recheck_defs` + type checking round
//!   trip regenerates what the backends need.
//! - `P_id`/`P_as` constructor arguments are treated as binders. An enum
//!   member in that position would also bind (shadowing the enum), which is
//!   value-identical for the forwarder call, so this is sound.
//! - Clauses that already have forwarder shape (pattern `Ctor(vars..)` with
//!   body `execute_Ctor(vars..)` and no guard) are left alone. OCaml would
//!   re-split them on a second run (replacing the auxiliary body with a
//!   self-call); skipping keeps this pass a fixpoint so the typed pipeline
//!   stays idempotent. Hand-written forwarders are therefore preserved
//!   rather than re-split.
//! - Clauses whose constructor name cannot be resolved through the
//!   interner, and call sites with anything but one constructor argument,
//!   are preserved (`Env.is_union_constructor` has no untyped equivalent).
//! - Only `Fundef` definitions are rewritten; `Impl`, scattered clauses and
//!   mappings are left alone, as in OCaml. Effect copying
//!   (`Effects.copy_function_effect`) has no untyped counterpart.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"split"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 22, zwischen
//!   `"nexp_ids"` (21) und `"minimise_recursive_functions"` (23).
//! - Trait-Impl-Name: `Split` (diese Datei).
//!   `passes/mod.rs`: `pub mod split;` +
//!   `pub use split::Split;` ergaenzen.
//!   `pipeline.rs`: `"split" => Some(Arc::new(Split))` in
//!   `make_typed_implemented` und den Namen in `is_typed_implemented_pass`
//!   aufnehmen. Dortige Tests anpassen:
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

use std::collections::BTreeMap;

use sail_ast::{
    DefKind, Exp, ExpKind, Funcl, Interner, Name, Node, NodeId, PExp, PExpKind, Pat, PatKind, Span,
    Symbol,
};

use crate::pass::TypedPass;

use super::typed::{Fresh, collect_used, mk_exp, mk_name, mk_pat};

/// Function this pass splits, mirroring `[String_arg "execute"]`.
const TARGET: &str = "execute";

#[derive(Clone, Copy, Debug, Default)]
pub struct Split;

impl TypedPass for Split {
    fn name(&self) -> &'static str {
        "split"
    }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        let target = interner.intern(TARGET);
        let mut fresh = Fresh::new(collect_used(ast, interner));
        // Constructor symbol -> auxiliary function symbol, over every
        // rewritten `execute` definition. Drives call-site optimization.
        let mut split_ctors: BTreeMap<u32, Symbol> = BTreeMap::new();
        let defs = std::mem::take(&mut ast.defs);
        let mut out = Vec::with_capacity(defs.len());
        for def in defs {
            let Node { id, span, kind, annotation } = def;
            match kind {
                DefKind::Fundef { rec, annot, funcls }
                    if is_target(&funcls, target) =>
                {
                    let (clauses, aux) =
                        split_clauses(funcls, &mut fresh, interner, &mut split_ctors);
                    out.push(Node {
                        id,
                        span,
                        kind: DefKind::Fundef { rec: rec.clone(), annot: annot.clone(), funcls: clauses },
                        annotation,
                    });
                    for (_, (_, aux_funcls)) in aux {
                        // Forwarder-shaped clauses register their constructor
                        // for call optimization but contribute no clause;
                        // emitting an empty definition would break the
                        // fixpoint (and Lean has no empty functions).
                        if aux_funcls.is_empty() {
                            continue;
                        }
                        out.push(Node::dummy(DefKind::Fundef {
                            rec: rec.clone(),
                            annot: annot.clone(),
                            funcls: aux_funcls,
                        }));
                    }
                }
                kind => out.push(Node { id, span, kind, annotation }),
            }
        }
        if !split_ctors.is_empty() {
            for def in &mut out {
                if let DefKind::Fundef { funcls, .. } = &mut def.kind {
                    for funcl in funcls {
                        optimize_pexp(&mut funcl.arm, target, &split_ctors);
                    }
                }
            }
        }
        ast.defs = out;
    }
}

fn is_target(funcls: &[Funcl], target: Symbol) -> bool {
    funcls.first().is_some_and(|f| f.name.symbol == target)
}

/// Split `execute` clauses; returns the replacement clauses plus per-def
/// auxiliary functions (constructor symbol -> (aux symbol, clauses)).
#[allow(clippy::too_many_lines)]
fn split_clauses(
    funcls: Vec<Funcl>,
    fresh: &mut Fresh,
    interner: &mut dyn Interner,
    split_ctors: &mut BTreeMap<u32, Symbol>,
) -> (Vec<Funcl>, BTreeMap<u32, (Symbol, Vec<Funcl>)>) {
    let mut clauses = Vec::with_capacity(funcls.len());
    let mut aux_funs: BTreeMap<u32, (Symbol, Vec<Funcl>)> = BTreeMap::new();
    for funcl in funcls {
        let (funcl_id, funcl_span, funcl_name) = (funcl.id, funcl.span, funcl.name.clone());
        let (arm_id, arm_span) = (funcl.arm.id, funcl.arm.span);
        let (pat, guard, body) = match funcl.arm.kind {
            PExpKind::PatExp { pat, body } => (pat, None, body),
            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
        };
        let (ctor, args) = match clause_ctor(&pat) {
            Some(pair) => pair,
            None => {
                clauses.push(rebuild(
                    funcl_id, funcl_span, funcl_name, arm_id, arm_span, pat, guard, body,
                ));
                continue;
            }
        };
        let Some(ctor_text) = interner.resolve(ctor.symbol) else {
            clauses.push(rebuild(
                funcl_id, funcl_span, funcl_name, arm_id, arm_span, pat, guard, body,
            ));
            continue;
        };
        // Unwrap a single tuple-pattern constructor argument, as in OCaml
        // (`match args with [P_tuple args] -> args | _ -> args`).
        let mut inner = args;
        if let [single] = inner.as_slice() {
            if let PatKind::Tuple(flat) = &single.kind {
                inner = flat.clone();
            }
        }
        let aux_text = format!("{TARGET}_{ctor_text}");
        let aux_sym = interner.intern(&aux_text);
        let aux_name = mk_name(aux_sym);
        // Forwarder: keep plain variables, freshen anything else.
        let mut fwd_pats = Vec::with_capacity(inner.len());
        let mut fwd_args = Vec::with_capacity(inner.len());
        for arg in &inner {
            let name = match pat_binder(arg) {
                Some(n) => n,
                None => mk_name(interner.intern(&fresh.fresh("arg"))),
            };
            fwd_args.push(mk_exp(ExpKind::Id(name.clone())));
            fwd_pats.push(mk_pat(PatKind::Id(name)));
        }
        // Skip clauses that already have forwarder shape (see module docs):
        // re-splitting would replace the auxiliary body with a self-call.
        let already_forwarded =
            guard.is_none() && is_forwarder_call(&body, &aux_sym, &fwd_args);
        // Auxiliary clause keeps the original guard and body, matching on
        // the unwrapped arguments.
        let aux_pat = if inner.len() == 1 {
            inner.into_iter().next().expect("len checked above")
        } else {
            mk_pat(PatKind::Tuple(inner))
        };
        let aux_arm = match guard {
            Some(g) => PExpKind::PatWhen { pat: aux_pat, guard: g, body },
            None => PExpKind::PatExp { pat: aux_pat, body },
        };
        let fwd_body = mk_exp(ExpKind::App { func: aux_name.clone(), args: fwd_args });
        let entry = aux_funs.entry(ctor.symbol.0).or_insert_with(|| (aux_sym, Vec::new()));
        debug_assert_eq!(entry.0, aux_sym);
        if !already_forwarded {
            entry.1.push(Funcl {
                id: funcl_id,
                span: funcl_span,
                name: aux_name,
                arm: PExp {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    kind: aux_arm,
                },
            });
        }
        split_ctors.insert(ctor.symbol.0, aux_sym);
        clauses.push(Funcl {
            id: funcl_id,
            span: funcl_span,
            name: funcl_name,
            arm: PExp {
                id: arm_id,
                span: arm_span,
                kind: PExpKind::PatExp {
                    pat: mk_pat(PatKind::App { func: ctor.clone(), args: fwd_pats }),
                    body: fwd_body,
                },
            },
        });
    }
    (clauses, aux_funs)
}

fn rebuild(
    funcl_id: sail_ast::NodeId,
    funcl_span: sail_ast::Span,
    funcl_name: Name,
    arm_id: sail_ast::NodeId,
    arm_span: sail_ast::Span,
    pat: Pat,
    guard: Option<Exp>,
    body: Exp,
) -> Funcl {
    let kind = match guard {
        Some(g) => PExpKind::PatWhen { pat, guard: g, body },
        None => PExpKind::PatExp { pat, body },
    };
    Funcl {
        id: funcl_id,
        span: funcl_span,
        name: funcl_name,
        arm: PExp { id: arm_id, span: arm_span, kind },
    }
}

/// Top-level constructor pattern of a clause, unwrapping a single
/// outer tuple (`P_tuple [P_app ..]`).
fn clause_ctor(pat: &Pat) -> Option<(Name, Vec<Pat>)> {
    match &pat.kind {
        PatKind::App { func, args } => Some((func.clone(), args.clone())),
        PatKind::Tuple(inner) => match inner.as_slice() {
            [single] => match &single.kind {
                PatKind::App { func, args } => Some((func.clone(), args.clone())),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Plain-variable constructor argument reused by the forwarder
/// (`pat_var` in OCaml, minus the environment check; see module docs).
fn pat_binder(pat: &Pat) -> Option<Name> {
    match &pat.kind {
        PatKind::Id(name) => Some(name.clone()),
        PatKind::As { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// Whether `body` is exactly `aux(vars..)` with no extra structure.
fn is_forwarder_call(body: &Exp, aux: &Symbol, vars: &[Exp]) -> bool {
    match &body.kind {
        ExpKind::App { func, args } if func.symbol == *aux => {
            args.len() == vars.len()
                && args.iter().zip(vars).all(|(a, v)| a == v)
        }
        _ => false,
    }
}

fn optimize_pexp(arm: &mut sail_ast::PExp, target: Symbol, split_ctors: &BTreeMap<u32, Symbol>) {
    match &mut arm.kind {
        PExpKind::PatExp { body, .. } => optimize_exp(body, target, split_ctors),
        PExpKind::PatWhen { guard, body, .. } => {
            optimize_exp(guard, target, split_ctors);
            optimize_exp(body, target, split_ctors);
        }
    }
}

/// Bottom-up call-site rewrite over one `Fundef` clause body/guard.
fn optimize_exp(exp: &mut Exp, target: Symbol, split_ctors: &BTreeMap<u32, Symbol>) {
    optimize_children(exp, target, split_ctors);
    if let ExpKind::App { func, args } = &mut exp.kind {
        if func.symbol == target && args.len() == 1 {
            if let ExpKind::App { func: ctor, args: ctor_args } = &args[0].kind {
                if let Some(aux) = split_ctors.get(&ctor.symbol.0) {
                    let flat: Option<Vec<Exp>> = match ctor_args.as_slice() {
                        [single] => match &single.kind {
                            ExpKind::Tuple(inner) => Some(inner.clone()),
                            _ => Some(vec![single.clone()]),
                        },
                        // OCaml reports unreachable here; without a
                        // diagnostic channel the call is preserved.
                        _ => None,
                    };
                    if let Some(flat) = flat {
                        *exp = mk_exp(ExpKind::App { func: mk_name(*aux), args: flat });
                    }
                }
            }
        }
    }
}

fn optimize_children(exp: &mut Exp, target: Symbol, split_ctors: &BTreeMap<u32, Symbol>) {
    match &mut exp.kind {
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                optimize_exp(e, target, split_ctors);
            }
        }
        ExpKind::Typ { exp: e, .. } => optimize_exp(e, target, split_ctors),
        ExpKind::App { args, .. } => {
            for e in args {
                optimize_exp(e, target, split_ctors);
            }
        }
        ExpKind::If { cond, then_branch, else_branch } => {
            optimize_exp(cond, target, split_ctors);
            optimize_exp(then_branch, target, split_ctors);
            optimize_exp(else_branch, target, split_ctors);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                optimize_exp(m, target, split_ctors);
            }
            optimize_exp(cond, target, split_ctors);
            optimize_exp(body, target, split_ctors);
        }
        ExpKind::For { from, to, step, body, .. } => {
            optimize_exp(from, target, split_ctors);
            optimize_exp(to, target, split_ctors);
            optimize_exp(step, target, split_ctors);
            optimize_exp(body, target, split_ctors);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            optimize_exp(a, target, split_ctors);
            optimize_exp(b, target, split_ctors);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                optimize_exp(&mut f.value, target, split_ctors);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            optimize_exp(record, target, split_ctors);
            for f in fields {
                optimize_exp(&mut f.value, target, split_ctors);
            }
        }
        ExpKind::Field { record, .. } => optimize_exp(record, target, split_ctors),
        ExpKind::Match { scrutinee, arms } => {
            optimize_exp(scrutinee, target, split_ctors);
            for a in arms {
                optimize_pexp(a, target, split_ctors);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            optimize_exp(bind, target, split_ctors);
            optimize_exp(body, target, split_ctors);
        }
        ExpKind::Assign { lexp, value } => {
            optimize_lexp(lexp, target, split_ctors);
            optimize_exp(value, target, split_ctors);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => optimize_exp(e, target, split_ctors),
        ExpKind::Try { exp: e, arms } => {
            optimize_exp(e, target, split_ctors);
            for a in arms {
                optimize_pexp(a, target, split_ctors);
            }
        }
        ExpKind::Assert { cond, msg } => {
            optimize_exp(cond, target, split_ctors);
            optimize_exp(msg, target, split_ctors);
        }
        ExpKind::Var { lexp, init, body } => {
            optimize_lexp(lexp, target, split_ctors);
            optimize_exp(init, target, split_ctors);
            optimize_exp(body, target, split_ctors);
        }
        ExpKind::InternalAssume { exp: e, .. } => optimize_exp(e, target, split_ctors),
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

fn optimize_lexp(lexp: &mut sail_ast::LExp, target: Symbol, split_ctors: &BTreeMap<u32, Symbol>) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(e) => optimize_exp(e, target, split_ctors),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            optimize_lexp(inner, target, split_ctors);
            optimize_exp(index, target, split_ctors);
        }
        sail_ast::LExpKind::VectorRange { lexp: inner, lo, hi } => {
            optimize_lexp(inner, target, split_ctors);
            optimize_exp(lo, target, split_ctors);
            optimize_exp(hi, target, split_ctors);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => {
            optimize_lexp(inner, target, split_ctors);
        }
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                optimize_lexp(p, target, split_ctors);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, Def, LitKind, RecOpt, SimpleInterner, TAnnotOpt};

    fn nm(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn pexp(kind: PExpKind) -> PExp {
        PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind,
        }
    }

    fn var_pat(it: &mut SimpleInterner, text: &str) -> Pat {
        mk_pat(PatKind::Id(nm(it, text)))
    }

    fn ctor_clause(
        it: &mut SimpleInterner,
        ctor: &str,
        args: Vec<Pat>,
        body: Exp,
    ) -> Funcl {
        Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, "execute"),
            arm: pexp(PExpKind::PatExp {
                pat: mk_pat(PatKind::App { func: nm(it, ctor), args }),
                body,
            }),
        }
    }

    fn fundef(_name: &str, _it: &mut SimpleInterner, funcls: Vec<Funcl>) -> Def {
        Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls,
        })
    }

    fn app(it: &mut SimpleInterner, func: &str, args: Vec<Exp>) -> Exp {
        mk_exp(ExpKind::App { func: nm(it, func), args })
    }

    fn clause_pat(funcl: &Funcl) -> &Pat {
        match &funcl.arm.kind {
            PExpKind::PatExp { pat, .. } | PExpKind::PatWhen { pat, .. } => pat,
        }
    }

    fn clause_body(funcl: &Funcl) -> &Exp {
        match &funcl.arm.kind {
            PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. } => body,
        }
    }

    #[test]
    fn splits_ctor_clause_into_aux_and_forwarder() {
        let mut it = SimpleInterner::new();
        let body = mk_exp(ExpKind::Id(nm(&mut it, "x")));
        let px = var_pat(&mut it, "x");
        let py = var_pat(&mut it, "y");
        let clause = ctor_clause(&mut it, "Instr", vec![px, py], body.clone());
        let mut ast = Ast::new(vec![fundef("execute", &mut it, vec![clause])]);
        Split.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 2);
        // Main definition keeps one forwarder clause in place.
        let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def 0 shape")
        };
        assert_eq!(funcls.len(), 1);
        let PatKind::App { func, args } = &clause_pat(&funcls[0]).kind else {
            panic!("forwarder shape")
        };
        assert_eq!(it.resolve(func.symbol), Some("Instr"));
        assert_eq!(args.len(), 2);
        let ExpKind::App { func: target, args: call_args } = &clause_body(&funcls[0]).kind
        else {
            panic!("forwarder body")
        };
        assert_eq!(it.resolve(target.symbol), Some("execute_Instr"));
        assert_eq!(call_args.len(), 2);
        // Auxiliary function carries the original body.
        let DefKind::Fundef { funcls: aux, .. } = &ast.defs[1].kind else {
            panic!("def 1 shape")
        };
        assert_eq!(aux.len(), 1);
        assert_eq!(it.resolve(aux[0].name.symbol), Some("execute_Instr"));
        assert_eq!(clause_body(&aux[0]), &body);
        // Fixpoint: a second run changes nothing.
        let snapshot = ast.clone();
        Split.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn unwraps_single_tuple_pattern_and_freshens_non_vars() {
        let mut it = SimpleInterner::new();
        let body = mk_exp(ExpKind::Lit(Node::dummy(LitKind::Unit)));
        let inner = mk_pat(PatKind::Tuple(vec![
            mk_pat(PatKind::Lit(Node::dummy(LitKind::True))),
            var_pat(&mut it, "y"),
        ]));
        let clause = Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(&mut it, "execute"),
            arm: pexp(PExpKind::PatExp {
                pat: mk_pat(PatKind::Tuple(vec![mk_pat(PatKind::App {
                    func: nm(&mut it, "Branch"),
                    args: vec![inner],
                })])),
                body,
            }),
        };
        let mut ast = Ast::new(vec![fundef("execute", &mut it, vec![clause])]);
        Split.run(&mut ast, &mut it);
        let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def 0 shape")
        };
        // Non-variable argument becomes a fresh `arg0`, `y` is reused.
        let PatKind::App { args, .. } = &clause_pat(&funcls[0]).kind else {
            panic!("forwarder shape")
        };
        assert_eq!(args.len(), 2);
        let names: Vec<_> = args
            .iter()
            .map(|p| match &p.kind {
                PatKind::Id(n) => it.resolve(n.symbol).unwrap_or("?"),
                _ => "?",
            })
            .collect();
        assert_eq!(names, vec!["arg0", "y"]);
        // Auxiliary clause matches the unwrapped tuple (not freshened).
        let DefKind::Fundef { funcls: aux, .. } = &ast.defs[1].kind else {
            panic!("def 1 shape")
        };
        assert!(matches!(&clause_pat(&aux[0]).kind, PatKind::Tuple(_)));
    }

    #[test]
    fn leaves_other_functions_and_plain_clauses_alone() {
        let mut it = SimpleInterner::new();
        let plain = Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(&mut it, "other"),
            arm: pexp(PExpKind::PatExp {
                pat: mk_pat(PatKind::App {
                    func: nm(&mut it, "Instr"),
                    args: vec![var_pat(&mut it, "x")],
                }),
                body: mk_exp(ExpKind::Undef),
            }),
        };
        let wild = Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(&mut it, "execute"),
            arm: pexp(PExpKind::PatExp {
                pat: mk_pat(PatKind::Wild),
                body: mk_exp(ExpKind::Undef),
            }),
        };
        let mut ast = Ast::new(vec![
            fundef("other", &mut it, vec![plain]),
            fundef("execute", &mut it, vec![wild]),
        ]);
        let snapshot = ast.clone();
        Split.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn rewrites_direct_calls_only_for_split_constructors() {
        let mut it = SimpleInterner::new();
        let px = var_pat(&mut it, "x");
        let clause = ctor_clause(&mut it, "Instr", vec![px], mk_exp(ExpKind::Undef));
        let na = mk_exp(ExpKind::Id(nm(&mut it, "a")));
        let nb = mk_exp(ExpKind::Id(nm(&mut it, "b")));
        let nc = mk_exp(ExpKind::Id(nm(&mut it, "c")));
        let ca = app(&mut it, "Instr", vec![na]);
        let cb = app(&mut it, "Other", vec![nb]);
        let cc = app(&mut it, "Instr", vec![nc]);
        let s0 = app(&mut it, "execute", vec![ca]);
        let s1 = app(&mut it, "execute", vec![cb]);
        let s2 = app(&mut it, "helper", vec![cc]);
        let caller_body = mk_exp(ExpKind::Block(vec![s0, s1, s2]));
        let caller_name = nm(&mut it, "caller");
        let caller = Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: caller_name,
            arm: pexp(PExpKind::PatExp {
                pat: mk_pat(PatKind::Wild),
                body: caller_body,
            }),
        };
        let mut ast = Ast::new(vec![
            fundef("execute", &mut it, vec![clause]),
            fundef("caller", &mut it, vec![caller]),
        ]);
        Split.run(&mut ast, &mut it);
        let DefKind::Fundef { funcls, .. } = &ast.defs[2].kind else {
            panic!("caller shape")
        };
        let ExpKind::Block(stmts) = &clause_body(&funcls[0]).kind else {
            panic!("caller body")
        };
        let ExpKind::App { func, args } = &stmts[0].kind else {
            panic!("call 0")
        };
        assert_eq!(it.resolve(func.symbol), Some("execute_Instr"));
        assert_eq!(args.len(), 1);
        // Unknown constructor and foreign callee are preserved.
        assert!(matches!(&stmts[1].kind, ExpKind::App { .. }));
        let ExpKind::App { func: f1, .. } = &stmts[1].kind else {
            unreachable!()
        };
        assert_eq!(it.resolve(f1.symbol), Some("execute"));
        assert!(matches!(&stmts[2].kind, ExpKind::App { .. }));
    }

    #[test]
    fn accumulates_repeated_constructors_in_clause_order() {
        let mut it = SimpleInterner::new();
        let e1 = mk_exp(ExpKind::Id(nm(&mut it, "e1")));
        let e2 = mk_exp(ExpKind::Id(nm(&mut it, "e2")));
        let e3 = mk_exp(ExpKind::Id(nm(&mut it, "e3")));
        let pa = var_pat(&mut it, "a");
        let pb = var_pat(&mut it, "b");
        let pc = var_pat(&mut it, "c");
        let c1 = ctor_clause(&mut it, "Instr", vec![pa], e1);
        let c2 = ctor_clause(&mut it, "Other", vec![pb], e2);
        let c3 = ctor_clause(&mut it, "Instr", vec![pc], e3);
        let mut ast = Ast::new(vec![fundef("execute", &mut it, vec![c1, c2, c3])]);
        Split.run(&mut ast, &mut it);
        // Main def + two auxiliaries.
        assert_eq!(ast.defs.len(), 3);
        let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def 0 shape")
        };
        assert_eq!(funcls.len(), 3);
        let names: Vec<_> = ast.defs[1..]
            .iter()
            .map(|d| match &d.kind {
                DefKind::Fundef { funcls, .. } => it.resolve(funcls[0].name.symbol).unwrap_or("?").to_owned(),
                _ => "?".to_owned(),
            })
            .collect();
        assert!(names.contains(&"execute_Instr".to_owned()));
        assert!(names.contains(&"execute_Other".to_owned()));
        let DefKind::Fundef { funcls: instr, .. } = &ast
            .defs
            .iter()
            .find(|d| matches!(&d.kind, DefKind::Fundef { funcls, .. } if it.resolve(funcls[0].name.symbol) == Some("execute_Instr")))
            .expect("aux")
            .kind
        else {
            unreachable!()
        };
        assert_eq!(instr.len(), 2);
        let snapshot = ast.clone();
        Split.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
