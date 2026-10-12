//! Move termination measures into their functions, matching
//! `move_termination_measures` (plus the trailing `move_loop_measures`)
//! in `sail/src/lib/rewrites.ml`.
//!
//! Phase A: every top-level `Measure` definition is attached to its
//! function (`RecOpt::Measure`), covering both `Fundef` and
//! `InternalMutrec` member sets, and the standalone definition is
//! dropped. `val` specs of functions called from a measure body are moved
//! to directly precede the using function (each exactly once), so the
//! result still type checks; the later sorting rewrite rearranges the
//! rest. `val` specs are also deduplicated first-wins while traversing,
//! exactly as the OCaml walk does.
//!
//! Phase B: every top-level `LoopMeasures` definition is distributed onto
//! matching `Loop` nodes inside its function (consumed head-first in
//! pre-order, by `While`/`Until` kind) and dropped afterwards.
//!
//! Gaps on the untyped AST (documented, no guessing):
//! - OCaml raises on a second measure for the same function. `TypedPass`
//!   has no error channel, so the first measure wins and later duplicates
//!   are dropped.
//! - OCaml warns about unused loop measures via `Reporting`; there is no
//!   diagnostic channel here, so leftovers vanish silently.
//! - The measure-body call check (`Env` lookups) is purely syntactic
//!   (`App` targets); valspec identity is by symbol.
//! - Measure-expression checking (`check_exp ... int_typ`) needs a type
//!   environment and is skipped.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"move_termination_measures"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 0 (erste Position,
//!   gefolgt von `"instantiate_outcomes"`).
//! - Trait-Impl-Name: `MoveTerminationMeasures` (diese Datei).
//!   `passes/mod.rs`: `pub mod move_termination_measures;` +
//!   `pub use move_termination_measures::MoveTerminationMeasures;`
//!   ergaenzen.
//!   `pipeline.rs`: `"move_termination_measures" =>
//!   Some(Arc::new(MoveTerminationMeasures))` in `make_typed_implemented`
//!   und den Namen in `is_typed_implemented_pass` aufnehmen. Dortige Tests
//!   anpassen: die `!is_typed_implemented_pass("move_termination_measures")`-
//!   Stelle (wird implementiert; der Stub-Fall dort muss auf einen weiterhin
//!   unverdrahteten Namen wie `"instantiate_outcomes"` wechseln),
//!   `typed_implemented_lists_only_wired_passes` muss danach (mit den
//!   Lane-60-Paessen, Lean-Ordnung, verifiziert)
//!   `["move_termination_measures", "remove_duplicate_valspecs",
//!   "pat_string_append", "add_unspecified_rec", "undefined",
//!   "tuple_assignments", "simple_assignments", "split",
//!   "minimise_recursive_functions", "const_prop_mutrec",
//!   "merge_function_clauses", "remove_blocks",
//!   "remove_superfluous_letbinds"]` erwarten.
//! - `tests/typed_pipeline.rs` anpassen:
//!   `typed_implemented_rewrites_covering_program` erwartet bisher
//!   `("lean", "two")` (last-extern-wins); mit verdrahtetem
//!   `move_termination_measures` (Index 0, droppt spaetere Duplikate wie
//!   OCaml `called_output`) gilt `("lean", "one")`. Das ist OCaml-Paritaet
//!   (beides erster `val`-Spec); last-extern-wins bleibt fuer spaeter
//!   entstehende Duplikate bestehen (Unit-Test von
//!   `remove_duplicate_valspecs`).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sail_ast::{Def, DefKind, Exp, ExpKind, Funcl, Loop, Node, PExpKind, Pat, RecOpt};

use crate::pass::TypedPass;

#[derive(Clone, Copy, Debug, Default)]
pub struct MoveTerminationMeasures;

impl TypedPass for MoveTerminationMeasures {
    fn name(&self) -> &'static str {
        "move_termination_measures"
    }

    fn run(&self, ast: &mut sail_ast::Ast, _interner: &mut dyn sail_ast::Interner) {
        move_measures(ast);
        move_loop_measures(ast);
    }
}

struct Measure {
    pat: Pat,
    exp: Exp,
    /// Functions called from the measure body, first-encounter order.
    called: Vec<u32>,
}

fn move_measures(ast: &mut sail_ast::Ast) {
    // First measure per function wins; later duplicates are dropped
    // (OCaml raises; `TypedPass` has no error channel).
    let mut measures: BTreeMap<u32, Measure> = BTreeMap::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for def in &ast.defs {
        if let DefKind::Measure { name, pat, exp } = &def.kind {
            if !seen.insert(name.symbol.0) {
                continue;
            }
            let mut dedup = BTreeSet::new();
            let mut called = Vec::new();
            collect_calls(exp, &mut dedup, &mut called);
            measures.insert(name.symbol.0, Measure {
                pat: pat.clone(),
                exp: exp.clone(),
                called,
            });
        }
    }

    let called_all: BTreeSet<u32> =
        measures.values().flat_map(|m| m.called.iter().copied()).collect();
    // Last valspec per called function wins the move (OCaml `Bindings.add`
    // overwrites while folding).
    let mut specs_of_called: BTreeMap<u32, Def> = BTreeMap::new();
    for def in &ast.defs {
        if let DefKind::Val(vs) = &def.kind {
            if called_all.contains(&vs.name.symbol.0) {
                specs_of_called.insert(vs.name.symbol.0, def.clone());
            }
        }
    }

    // Emitted valspec ids (moved or kept): later occurrences are dropped.
    let mut emitted: BTreeSet<u32> = BTreeSet::new();
    let defs = std::mem::take(&mut ast.defs);
    let mut out = Vec::with_capacity(defs.len());
    for def in defs {
        let Node { id, span, kind, annotation } = def;
        match kind {
            DefKind::Fundef { rec, annot, funcls } => {
                let key = funcls.first().map(|f| f.name.symbol.0);
                match key.and_then(|k| measures.get(&k)) {
                    Some(m) => {
                        out.extend(take_moved(m, &specs_of_called, &mut emitted));
                        out.push(Node {
                            id,
                            span,
                            kind: DefKind::Fundef {
                                rec: RecOpt::Measure { pat: m.pat.clone(), exp: m.exp.clone() },
                                annot,
                                funcls,
                            },
                            annotation,
                        });
                    }
                    None => out.push(Node {
                        id,
                        span,
                        kind: DefKind::Fundef { rec, annot, funcls },
                        annotation,
                    }),
                }
            }
            DefKind::InternalMutrec(sets) => {
                let mut sets = sets;
                let mut moved: Vec<Def> = Vec::new();
                for set in &mut sets {
                    let key = set.funcls.first().map(|f| f.name.symbol.0);
                    if let Some(m) = key.and_then(|k| measures.get(&k)) {
                        set.rec = RecOpt::Measure { pat: m.pat.clone(), exp: m.exp.clone() };
                        moved.extend(take_moved(m, &specs_of_called, &mut emitted));
                    }
                }
                out.extend(moved);
                out.push(Node {
                    id,
                    span,
                    kind: DefKind::InternalMutrec(sets),
                    annotation,
                });
            }
            DefKind::Val(vs) => {
                if emitted.contains(&vs.name.symbol.0) {
                    continue;
                }
                emitted.insert(vs.name.symbol.0);
                out.push(Node {
                    id,
                    span,
                    kind: DefKind::Val(vs),
                    annotation,
                });
            }
            DefKind::Measure { .. } => continue,
            kind => out.push(Node { id, span, kind, annotation }),
        }
    }
    ast.defs = out;
}

/// Moved valspec clones for one measure's called functions, in
/// first-encounter order; each moves at most once per run.
fn take_moved(
    m: &Measure,
    specs: &BTreeMap<u32, Def>,
    emitted: &mut BTreeSet<u32>,
) -> Vec<Def> {
    let mut moved = Vec::new();
    for id in &m.called {
        if emitted.contains(id) {
            continue;
        }
        emitted.insert(*id);
        if let Some(spec) = specs.get(id) {
            moved.push(spec.clone());
        }
    }
    moved
}

/// Syntactic call targets in an expression (`called_fns_in_exp` in
/// OCaml), first-encounter pre-order.
fn collect_calls(exp: &Exp, seen: &mut BTreeSet<u32>, order: &mut Vec<u32>) {
    match &exp.kind {
        ExpKind::App { func, args } => {
            if seen.insert(func.symbol.0) {
                order.push(func.symbol.0);
            }
            for a in args {
                collect_calls(a, seen, order);
            }
        }
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                collect_calls(e, seen, order);
            }
        }
        ExpKind::Typ { exp: e, .. } => collect_calls(e, seen, order),
        ExpKind::If { cond, then_branch, else_branch } => {
            collect_calls(cond, seen, order);
            collect_calls(then_branch, seen, order);
            collect_calls(else_branch, seen, order);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                collect_calls(m, seen, order);
            }
            collect_calls(cond, seen, order);
            collect_calls(body, seen, order);
        }
        ExpKind::For { from, to, step, body, .. } => {
            collect_calls(from, seen, order);
            collect_calls(to, seen, order);
            collect_calls(step, seen, order);
            collect_calls(body, seen, order);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            collect_calls(a, seen, order);
            collect_calls(b, seen, order);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                collect_calls(&f.value, seen, order);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            collect_calls(record, seen, order);
            for f in fields {
                collect_calls(&f.value, seen, order);
            }
        }
        ExpKind::Field { record, .. } => collect_calls(record, seen, order),
        ExpKind::Match { scrutinee, arms } => {
            collect_calls(scrutinee, seen, order);
            for a in arms {
                collect_arm_calls(a, seen, order);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            collect_calls(bind, seen, order);
            collect_calls(body, seen, order);
        }
        ExpKind::Assign { lexp, value } => {
            collect_lexp_calls(lexp, seen, order);
            collect_calls(value, seen, order);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => collect_calls(e, seen, order),
        ExpKind::Try { exp: e, arms } => {
            collect_calls(e, seen, order);
            for a in arms {
                collect_arm_calls(a, seen, order);
            }
        }
        ExpKind::Assert { cond, msg } => {
            collect_calls(cond, seen, order);
            collect_calls(msg, seen, order);
        }
        ExpKind::Var { lexp, init, body } => {
            collect_lexp_calls(lexp, seen, order);
            collect_calls(init, seen, order);
            collect_calls(body, seen, order);
        }
        ExpKind::InternalAssume { exp: e, .. } => collect_calls(e, seen, order),
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

fn collect_arm_calls(arm: &sail_ast::PExp, seen: &mut BTreeSet<u32>, order: &mut Vec<u32>) {
    match &arm.kind {
        PExpKind::PatExp { body, .. } => collect_calls(body, seen, order),
        PExpKind::PatWhen { guard, body, .. } => {
            collect_calls(guard, seen, order);
            collect_calls(body, seen, order);
        }
    }
}

fn collect_lexp_calls(lexp: &sail_ast::LExp, seen: &mut BTreeSet<u32>, order: &mut Vec<u32>) {
    match &lexp.kind {
        sail_ast::LExpKind::Deref(e) => collect_calls(e, seen, order),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            collect_lexp_calls(inner, seen, order);
            collect_calls(index, seen, order);
        }
        sail_ast::LExpKind::VectorRange { lexp: inner, lo, hi } => {
            collect_lexp_calls(inner, seen, order);
            collect_calls(lo, seen, order);
            collect_calls(hi, seen, order);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => {
            collect_lexp_calls(inner, seen, order);
        }
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                collect_lexp_calls(p, seen, order);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

fn move_loop_measures(ast: &mut sail_ast::Ast) {
    // Multiple definitions concatenate, as in OCaml.
    let mut table: BTreeMap<u32, VecDeque<(Loop, Exp)>> = BTreeMap::new();
    for def in &ast.defs {
        if let DefKind::LoopMeasures { name, measures } = &def.kind {
            table.entry(name.symbol.0).or_default().extend(measures.iter().cloned());
        }
    }
    if table.is_empty() {
        ast.defs.retain(|d| !matches!(d.kind, DefKind::LoopMeasures { .. }));
        return;
    }
    for def in &mut ast.defs {
        match &mut def.kind {
            DefKind::Fundef { funcls, .. } => {
                attach_funcls(funcls, &mut table);
            }
            DefKind::InternalMutrec(sets) => {
                for set in sets {
                    attach_funcls(&mut set.funcls, &mut table);
                }
            }
            _ => {}
        }
    }
    ast.defs.retain(|d| !matches!(d.kind, DefKind::LoopMeasures { .. }));
}

fn attach_funcls(funcls: &mut [Funcl], table: &mut BTreeMap<u32, VecDeque<(Loop, Exp)>>) {
    for funcl in funcls {
        let Some(queue) = table.get_mut(&funcl.name.symbol.0) else {
            continue;
        };
        if queue.is_empty() {
            continue;
        }
        match &mut funcl.arm.kind {
            PExpKind::PatExp { body, .. } => attach_exp(body, queue),
            PExpKind::PatWhen { guard, body, .. } => {
                attach_exp(guard, queue);
                attach_exp(body, queue);
            }
        }
    }
}

/// Pre-order walk: a matching loop consumes the queue head, then its
/// children see the remainder.
fn attach_exp(exp: &mut Exp, queue: &mut VecDeque<(Loop, Exp)>) {
    if queue.is_empty() {
        return;
    }
    if let ExpKind::Loop { loop_kind, measure, cond, body, .. } = &mut exp.kind {
        if queue.front().is_some_and(|(kind, _)| *kind == *loop_kind) {
            let (_, m) = queue.pop_front().expect("checked above");
            *measure = Some(Box::new(m));
            attach_exp(cond, queue);
            attach_exp(body, queue);
            return;
        }
    }
    attach_children(exp, queue);
}

fn attach_children(exp: &mut Exp, queue: &mut VecDeque<(Loop, Exp)>) {
    match &mut exp.kind {
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                attach_exp(e, queue);
            }
        }
        ExpKind::Typ { exp: e, .. } => attach_exp(e, queue),
        ExpKind::App { args, .. } => {
            for e in args {
                attach_exp(e, queue);
            }
        }
        ExpKind::If { cond, then_branch, else_branch } => {
            attach_exp(cond, queue);
            attach_exp(then_branch, queue);
            attach_exp(else_branch, queue);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                attach_exp(m, queue);
            }
            attach_exp(cond, queue);
            attach_exp(body, queue);
        }
        ExpKind::For { from, to, step, body, .. } => {
            attach_exp(from, queue);
            attach_exp(to, queue);
            attach_exp(step, queue);
            attach_exp(body, queue);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            attach_exp(a, queue);
            attach_exp(b, queue);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                attach_exp(&mut f.value, queue);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            attach_exp(record, queue);
            for f in fields {
                attach_exp(&mut f.value, queue);
            }
        }
        ExpKind::Field { record, .. } => attach_exp(record, queue),
        ExpKind::Match { scrutinee, arms } => {
            attach_exp(scrutinee, queue);
            for a in arms {
                match &mut a.kind {
                    PExpKind::PatExp { body, .. } => attach_exp(body, queue),
                    PExpKind::PatWhen { guard, body, .. } => {
                        attach_exp(guard, queue);
                        attach_exp(body, queue);
                    }
                }
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            attach_exp(bind, queue);
            attach_exp(body, queue);
        }
        ExpKind::Assign { lexp, value } => {
            attach_lexp(lexp, queue);
            attach_exp(value, queue);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => attach_exp(e, queue),
        ExpKind::Try { exp: e, arms } => {
            attach_exp(e, queue);
            for a in arms {
                match &mut a.kind {
                    PExpKind::PatExp { body, .. } => attach_exp(body, queue),
                    PExpKind::PatWhen { guard, body, .. } => {
                        attach_exp(guard, queue);
                        attach_exp(body, queue);
                    }
                }
            }
        }
        ExpKind::Assert { cond, msg } => {
            attach_exp(cond, queue);
            attach_exp(msg, queue);
        }
        ExpKind::Var { lexp, init, body } => {
            attach_lexp(lexp, queue);
            attach_exp(init, queue);
            attach_exp(body, queue);
        }
        ExpKind::InternalAssume { exp: e, .. } => attach_exp(e, queue),
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

fn attach_lexp(lexp: &mut sail_ast::LExp, queue: &mut VecDeque<(Loop, Exp)>) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(e) => attach_exp(e, queue),
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            attach_lexp(inner, queue);
            attach_exp(index, queue);
        }
        sail_ast::LExpKind::VectorRange { lexp: inner, lo, hi } => {
            attach_lexp(inner, queue);
            attach_exp(lo, queue);
            attach_exp(hi, queue);
        }
        sail_ast::LExpKind::Field { lexp: inner, .. } => attach_lexp(inner, queue),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                attach_lexp(p, queue);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{
        Ast, Exp, FunclSet, Interner, Name, Node, NodeId, PExpKind, PatKind, QuantItem,
        SimpleInterner, Span, TAnnotOpt, TypKind, TypSchm,
    };

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

    fn fundef(it: &mut SimpleInterner, name: &str, rec: RecOpt, body: Exp) -> Def {
        Def::dummy(DefKind::Fundef {
            rec,
            annot: TAnnotOpt::None,
            funcls: vec![Funcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: nm(it, name),
                arm: pexp(PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body,
                }),
            }],
        })
    }

    fn measure(it: &mut SimpleInterner, name: &str, body: Exp) -> Def {
        Def::dummy(DefKind::Measure {
            name: nm(it, name),
            pat: Node::dummy(PatKind::Wild),
            exp: body,
        })
    }

    fn rec_of(ast: &Ast, index: usize) -> RecOpt {
        let DefKind::Fundef { rec, .. } = &ast.defs[index].kind else {
            panic!("def {index} is not a fundef")
        };
        rec.clone()
    }

    #[test]
    fn attaches_measure_and_drops_standalone_def() {
        let mut it = SimpleInterner::new();
        let body = Node::dummy(ExpKind::Id(nm(&mut it, "x")));
        let mut ast = Ast::new(vec![
            fundef(&mut it, "f", RecOpt::NonRec, Node::dummy(ExpKind::Undef)),
            measure(&mut it, "f", body.clone()),
        ]);
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 1);
        let RecOpt::Measure { exp, .. } = rec_of(&ast, 0) else {
            panic!("measure not attached")
        };
        assert_eq!(exp, body);
        // Idempotent.
        let snapshot = ast.clone();
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    fn valspec(it: &mut SimpleInterner, name: &str) -> Def {
        Def::dummy(DefKind::Val(sail_ast::ValSpec {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, name),
            typschm: TypSchm {
                quant: Vec::<QuantItem>::new(),
                typ: Node::dummy(TypKind::Unknown),
            },
            extern_opt: None,
        }))
    }

    #[test]
    fn moves_called_valspecs_before_fundef_and_dedups() {
        let mut it = SimpleInterner::new();
        let gspec = valspec(&mut it, "g");
        let gx = Node::dummy(ExpKind::Id(nm(&mut it, "x")));
        let gcall = Node::dummy(ExpKind::App { func: nm(&mut it, "g"), args: vec![gx] });
        let undef = Node::dummy(ExpKind::Undef);
        let fdef = fundef(&mut it, "f", RecOpt::NonRec, undef);
        let mdef = measure(&mut it, "f", gcall);
        let mut ast = Ast::new(vec![fdef, gspec.clone(), gspec.clone(), mdef]);
        MoveTerminationMeasures.run(&mut ast, &mut it);
        // Moved valspec, function, then nothing: duplicates collapsed and
        // the measure definition is gone.
        assert_eq!(ast.defs.len(), 2);
        let DefKind::Val(vs) = &ast.defs[0].kind else {
            panic!("def 0 shape")
        };
        assert_eq!(it.resolve(vs.name.symbol), Some("g"));
        assert!(matches!(rec_of(&ast, 1), RecOpt::Measure { .. }));
    }

    #[test]
    fn first_duplicate_measure_wins() {
        let mut it = SimpleInterner::new();
        let first = Node::dummy(ExpKind::Id(nm(&mut it, "a")));
        let second = Node::dummy(ExpKind::Id(nm(&mut it, "b")));
        let mut ast = Ast::new(vec![
            fundef(&mut it, "f", RecOpt::NonRec, Node::dummy(ExpKind::Undef)),
            measure(&mut it, "f", first.clone()),
            measure(&mut it, "f", second),
        ]);
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 1);
        let RecOpt::Measure { exp, .. } = rec_of(&ast, 0) else {
            panic!("measure not attached")
        };
        assert_eq!(exp, first);
    }

    #[test]
    fn attaches_measure_inside_mutrec_sets() {
        let mut it = SimpleInterner::new();
        let mk_set = |it: &mut SimpleInterner, name: &str| FunclSet {
            rec: RecOpt::Rec,
            annot: TAnnotOpt::None,
            funcls: vec![Funcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: nm(it, name),
                arm: pexp(PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body: Node::dummy(ExpKind::Undef),
                }),
            }],
        };
        let mut ast = Ast::new(vec![
            Def::dummy(DefKind::InternalMutrec(vec![
                mk_set(&mut it, "p"),
                mk_set(&mut it, "q"),
            ])),
            measure(&mut it, "q", Node::dummy(ExpKind::Undef)),
        ]);
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 1);
        let DefKind::InternalMutrec(sets) = &ast.defs[0].kind else {
            panic!("group shape")
        };
        assert!(matches!(sets[0].rec, RecOpt::Rec));
        assert!(matches!(sets[1].rec, RecOpt::Measure { .. }));
    }

    #[test]
    fn distributes_loop_measures_by_kind_and_drops_defs() {
        let mut it = SimpleInterner::new();
        let loop_exp = Node::dummy(ExpKind::Loop {
            loop_kind: Loop::While,
            measure: None,
            cond: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(sail_ast::LitKind::True)))),
            body: Box::new(Node::dummy(ExpKind::Undef)),
        });
        let until_exp = Node::dummy(ExpKind::Loop {
            loop_kind: Loop::Until,
            measure: None,
            cond: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(sail_ast::LitKind::True)))),
            body: Box::new(Node::dummy(ExpKind::Undef)),
        });
        let f = fundef(
            &mut it,
            "f",
            RecOpt::NonRec,
            Node::dummy(ExpKind::Block(vec![loop_exp, until_exp])),
        );
        let lm = Def::dummy(DefKind::LoopMeasures {
            name: nm(&mut it, "f"),
            measures: vec![(Loop::While, Node::dummy(ExpKind::Lit(Node::dummy(
                sail_ast::LitKind::False,
            ))))],
        });
        let mut ast = Ast::new(vec![f, lm]);
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 1);
        let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        let PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
            panic!("arm shape")
        };
        let ExpKind::Block(stmts) = &body.kind else {
            panic!("body shape")
        };
        assert!(matches!(&stmts[0].kind, ExpKind::Loop { measure: Some(_), .. }));
        assert!(matches!(&stmts[1].kind, ExpKind::Loop { measure: None, .. }));
        // Idempotent.
        let snapshot = ast.clone();
        MoveTerminationMeasures.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
