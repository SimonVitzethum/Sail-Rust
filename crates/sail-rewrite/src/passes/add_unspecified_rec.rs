//! Mark self-recursive functions as `Rec`, matching
//! `rewrite_add_unspecified_rec` in `sail/src/lib/rewrites.ml`.
//!
//! OCaml flips `Rec_nonrec` to `Rec_rec` when any clause of the definition
//! calls a function in its own strongly-connected component
//! (`recursive_fn_map` over `Callgraph.function_call_graph` plus
//! `is_funcl_rec`, which folds `e_app` over the clause body). Only
//! `DEF_fundef` is touched; `InternalMutrec` sets are left alone, as in OCaml.
//!
//! No type or effect environment is needed: the pass only reads `RecOpt`
//! annotations and syntactic `App { func, .. }` call sites, both present on
//! the untyped AST. The `interner` parameter is unused and kept only so the
//! signature matches the canonical pass shape for the later wiring
//! (`TypedPass::run`).
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"add_unspecified_rec"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 8, zwischen
//!   `"mapping_patterns"` (7) und `"undefined"` (9).
//! - Trait-Impl-Name: `AddUnspecifiedRec` (diese Datei).
//!   `passes/mod.rs`: `pub mod add_unspecified_rec;` +
//!   `pub use add_unspecified_rec::AddUnspecifiedRec;` ergaenzen.
//!   `pipeline.rs`: `"add_unspecified_rec" => Some(Arc::new(AddUnspecifiedRec))`
//!   in `make_typed_implemented` und den Namen in
//!   `is_typed_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{Ast, DefKind, Exp, ExpKind, PExp, RecOpt};

#[derive(Clone, Copy, Debug, Default)]
pub struct AddUnspecifiedRec;

impl crate::pass::TypedPass for AddUnspecifiedRec {
    fn name(&self) -> &'static str {
        AddUnspecifiedRec::name(self)
    }

    fn run(&self, ast: &mut Ast, interner: &mut dyn sail_ast::Interner) {
        AddUnspecifiedRec::run(self, ast, interner)
    }
}

impl AddUnspecifiedRec {
    /// Pass name, matching the OCaml rewrite name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "add_unspecified_rec"
    }

    /// Run the pass on the whole AST (in place).
    pub fn run(&self, ast: &mut Ast, _interner: &mut dyn sail_ast::Interner) {
        let sccs = strongly_connected(&call_graph(ast));
        for def in &mut ast.defs {
            if let DefKind::Fundef { rec, funcls, .. } = &mut def.kind {
                if !matches!(rec, RecOpt::NonRec) {
                    continue;
                }
                let hits = funcls.iter().any(|f| {
                    let members = sccs.get(&f.name.symbol.0);
                    let callees = arm_callees(&f.arm);
                    match members {
                        Some(m) => callees.iter().any(|c| m.contains(c)),
                        None => false,
                    }
                });
                if hits {
                    *rec = RecOpt::Rec;
                }
            }
        }
    }
}

/// Direct callees of one match arm (OCaml `fold_pexp` with `e_app`).
fn arm_callees(arm: &PExp) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    match &arm.kind {
        sail_ast::PExpKind::PatExp { body, .. } => collect_exp(body, &mut out),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            collect_exp(guard, &mut out);
            collect_exp(body, &mut out);
        }
    }
    out
}

/// Caller (`u32` symbol) -> direct callees, over every function-like definer.
///
/// Covers `Fundef`, `InternalMutrec` sets and `Impl` bodies so the SCCs match
/// OCaml's `function_call_graph`; only `Fundef` is *rewritten* (see `run`).
fn call_graph(ast: &Ast) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut graph: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut add = |name: u32, callees: BTreeSet<u32>| {
        graph.entry(name).or_default().extend(callees);
    };
    for def in &ast.defs {
        match &def.kind {
            DefKind::Fundef { funcls, .. } => {
                let mut merged = BTreeSet::new();
                let mut names = Vec::new();
                for f in funcls {
                    names.push(f.name.symbol.0);
                    merged.extend(arm_callees(&f.arm));
                }
                for n in names {
                    add(n, merged.clone());
                }
            }
            DefKind::InternalMutrec(sets) => {
                for set in sets {
                    let mut merged = BTreeSet::new();
                    let mut names = Vec::new();
                    for f in &set.funcls {
                        names.push(f.name.symbol.0);
                        merged.extend(arm_callees(&f.arm));
                    }
                    for n in names {
                        add(n, merged.clone());
                    }
                }
            }
            DefKind::Impl(f) => add(f.name.symbol.0, arm_callees(&f.arm)),
            _ => {}
        }
    }
    // Every callee becomes a node so singletons keep their identity SCC.
    let callees: Vec<u32> = graph.values().flatten().copied().collect();
    for c in callees {
        graph.entry(c).or_default();
    }
    graph
}

/// Kosaraju SCCs: symbol -> all members of its component.
fn strongly_connected(graph: &BTreeMap<u32, BTreeSet<u32>>) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    for &node in graph.keys() {
        visit(node, graph, &mut visited, &mut order);
    }
    let mut reversed: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&from, tos) in graph {
        reversed.entry(from).or_default();
        for &to in tos {
            reversed.entry(to).or_default().push(from);
        }
    }
    let mut comp_of: BTreeMap<u32, usize> = BTreeMap::new();
    let mut comps: Vec<BTreeSet<u32>> = Vec::new();
    for &node in order.iter().rev() {
        if comp_of.contains_key(&node) {
            continue;
        }
        let mut members = BTreeSet::new();
        assign(node, &reversed, &mut comp_of, comps.len(), &mut members);
        comps.push(members);
    }
    comp_of
        .into_iter()
        .map(|(sym, idx)| (sym, comps[idx].clone()))
        .collect()
}

fn visit(
    node: u32,
    graph: &BTreeMap<u32, BTreeSet<u32>>,
    visited: &mut BTreeSet<u32>,
    order: &mut Vec<u32>,
) {
    if !visited.insert(node) {
        return;
    }
    if let Some(next) = graph.get(&node) {
        for &m in next {
            visit(m, graph, visited, order);
        }
    }
    order.push(node);
}

fn assign(
    node: u32,
    reversed: &BTreeMap<u32, Vec<u32>>,
    comp_of: &mut BTreeMap<u32, usize>,
    comp: usize,
    members: &mut BTreeSet<u32>,
) {
    if comp_of.contains_key(&node) {
        return;
    }
    comp_of.insert(node, comp);
    members.insert(node);
    if let Some(prev) = reversed.get(&node) {
        for &m in prev.clone().iter() {
            assign(m, reversed, comp_of, comp, members);
        }
    }
}

fn collect_exp(exp: &Exp, out: &mut BTreeSet<u32>) {
    match &exp.kind {
        ExpKind::App { func, args } => {
            out.insert(func.symbol.0);
            for a in args {
                collect_exp(a, out);
            }
        }
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                collect_exp(e, out);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_exp(cond, out);
            collect_exp(then_branch, out);
            collect_exp(else_branch, out);
        }
        ExpKind::Loop {
            measure, cond, body, ..
        } => {
            if let Some(m) = measure {
                collect_exp(m, out);
            }
            collect_exp(cond, out);
            collect_exp(body, out);
        }
        ExpKind::For {
            from, to, step, body, ..
        } => {
            collect_exp(from, out);
            collect_exp(to, out);
            collect_exp(step, out);
            collect_exp(body, out);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            collect_exp(a, out);
            collect_exp(b, out);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                collect_exp(&f.value, out);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            collect_exp(record, out);
            for f in fields {
                collect_exp(&f.value, out);
            }
        }
        ExpKind::Field { record, .. } => collect_exp(record, out),
        ExpKind::Match { scrutinee, arms } => {
            collect_exp(scrutinee, out);
            for a in arms {
                collect_arm(a, out);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            collect_exp(bind, out);
            collect_exp(body, out);
        }
        ExpKind::Assign { value, .. } => collect_exp(value, out),
        ExpKind::Typ { exp, .. }
        | ExpKind::Return(exp)
        | ExpKind::Exit(exp)
        | ExpKind::Throw(exp)
        | ExpKind::InternalReturn(exp) => collect_exp(exp, out),
        ExpKind::Try { exp, arms } => {
            collect_exp(exp, out);
            for a in arms {
                collect_arm(a, out);
            }
        }
        ExpKind::Assert { cond, msg } => {
            collect_exp(cond, out);
            collect_exp(msg, out);
        }
        ExpKind::Var { init, body, .. } => {
            collect_exp(init, out);
            collect_exp(body, out);
        }
        ExpKind::InternalAssume { exp, .. } => collect_exp(exp, out),
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

fn collect_arm(arm: &PExp, out: &mut BTreeSet<u32>) {
    match &arm.kind {
        sail_ast::PExpKind::PatExp { body, .. } => collect_exp(body, out),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            collect_exp(guard, out);
            collect_exp(body, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Def, Funcl, Interner, Name, Node, NodeId, PExpKind, PatKind, SimpleInterner, Span};

    fn nm(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn body_call(it: &mut SimpleInterner, func: &str) -> Exp {
        Node::dummy(ExpKind::App {
            func: nm(it, func),
            args: Vec::new(),
        })
    }

    fn fundef(it: &mut SimpleInterner, name: &str, rec: RecOpt, bodies: Vec<Exp>) -> Def {
        let funcls = bodies
            .into_iter()
            .map(|body| Funcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: nm(it, name),
                arm: PExp {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    kind: PExpKind::PatExp {
                        pat: Node::dummy(PatKind::Wild),
                        body,
                    },
                },
            })
            .collect();
        Def::dummy(DefKind::Fundef {
            rec,
            annot: sail_ast::TAnnotOpt::None,
            funcls,
        })
    }

    fn rec_of(ast: &Ast, index: usize) -> RecOpt {
        let DefKind::Fundef { rec, .. } = &ast.defs[index].kind else {
            panic!("def {index} is not a fundef")
        };
        rec.clone()
    }

    #[test]
    fn marks_self_recursive_nonrec_as_rec() {
        let mut it = SimpleInterner::new();
        let body = body_call(&mut it, "f");
        let mut ast = Ast::new(vec![fundef(&mut it, "f", RecOpt::NonRec, vec![body])]);
        AddUnspecifiedRec.run(&mut ast, &mut it);
        assert!(matches!(rec_of(&ast, 0), RecOpt::Rec));
        // Idempotent.
        let snapshot = ast.clone();
        AddUnspecifiedRec.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn leaves_non_recursive_and_already_rec_alone() {
        let mut it = SimpleInterner::new();
        let plain = Node::dummy(ExpKind::Lit(Node::dummy(sail_ast::LitKind::Unit)));
        let mut ast = Ast::new(vec![
            fundef(&mut it, "g", RecOpt::NonRec, vec![plain.clone()]),
            fundef(&mut it, "h", RecOpt::Rec, vec![plain]),
        ]);
        let snapshot = ast.clone();
        AddUnspecifiedRec.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn marks_mutually_recursive_pair() {
        let mut it = SimpleInterner::new();
        let p_body = body_call(&mut it, "q");
        let q_body = body_call(&mut it, "p");
        let mut ast = Ast::new(vec![
            fundef(&mut it, "p", RecOpt::NonRec, vec![p_body]),
            fundef(&mut it, "q", RecOpt::NonRec, vec![q_body]),
        ]);
        AddUnspecifiedRec.run(&mut ast, &mut it);
        assert!(matches!(rec_of(&ast, 0), RecOpt::Rec));
        assert!(matches!(rec_of(&ast, 1), RecOpt::Rec));
    }

    #[test]
    fn non_calling_mutual_pair_stays_nonrec() {
        let mut it = SimpleInterner::new();
        let plain = || Node::dummy(ExpKind::Lit(Node::dummy(sail_ast::LitKind::Unit)));
        let mut ast = Ast::new(vec![
            fundef(&mut it, "p", RecOpt::NonRec, vec![plain()]),
            fundef(&mut it, "q", RecOpt::NonRec, vec![plain()]),
        ]);
        let snapshot = ast.clone();
        AddUnspecifiedRec.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
