//! Topological definition sorting, matching `top_sort_defs` in
//! `sail/src/lib/rewrites.ml` (`Callgraph.top_sort_defs` over the function
//! call graph, mutually recursive definitions kept together).
//!
//! Purely syntactic here: the string [`Shard`](crate::ast::Shard) carries no
//! expression AST, so the call graph is over-approximated from identifier
//! tokens in body text — def `A` depends on def `B` when `B`'s name occurs as
//! a whole-word token in one of `A`'s statement strings. Strongly connected
//! components (mutual recursion, including self-loops) keep their input
//! order; components are emitted dependencies-first with a stable
//! lowest-first-index tie-break, so the pass is deterministic and
//! idempotent. Shard-local by construction: only dependencies inside the same
//! shard can be ordered; references to defs in other shards are ignored
//! (documented limitation of the shard surface, never guessed across shards).
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"top_sort_defs"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 26, zwischen
//!   `"recheck_defs"` (25) und `"add_register_init_function"` (27).
//! - Trait-Impl-Name: `TopSortDefs` (diese Datei).
//!   `passes/mod.rs`: `pub mod top_sort_defs;` +
//!   `pub use top_sort_defs::TopSortDefs;` ergaenzen.
//!   `pipeline.rs`: `"top_sort_defs" => Some(Arc::new(TopSortDefs))` in
//!   `make_implemented` und den Namen in `is_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct TopSortDefs;

impl Pass for TopSortDefs {
    fn name(&self) -> &'static str {
        "top_sort_defs"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        let order = topo_order(&shard);
        // `order` is a permutation of `0..defs.len()` (see `topo_order`).
        let mut slots: Vec<Option<crate::ast::Def>> =
            std::mem::take(&mut shard.defs).into_iter().map(Some).collect();
        let mut sorted = Vec::with_capacity(slots.len());
        for index in order {
            sorted.push(slots[index].take().expect("topo order is a permutation"));
        }
        shard.defs = sorted;
        shard
    }
}

/// Stable topological order of def indices: dependencies first.
fn topo_order(shard: &Shard) -> Vec<usize> {
    let n = shard.defs.len();
    if n < 2 {
        return (0..n).collect();
    }
    let deps = dependencies(shard);
    let comps = strongly_connected(&deps, n);
    // Component of each node; members already ascending (built in pop order
    // then sorted).
    let comp_of = {
        let mut comp_of = vec![usize::MAX; n];
        for (id, members) in comps.iter().enumerate() {
            for &m in members {
                comp_of[m] = id;
            }
        }
        comp_of
    };
    // Component DAG: `needs[c]` = components that must come before `c`.
    let mut needs: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); comps.len()];
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); comps.len()];
    for (from, tos) in deps.iter().enumerate() {
        for &to in tos {
            let (cf, ct) = (comp_of[from], comp_of[to]);
            if cf != ct && needs[cf].insert(ct) {
                users[ct].push(cf);
            }
        }
    }
    let first_index: Vec<usize> = comps.iter().map(|m| m[0]).collect();
    // Ready queue keyed by lowest member index: deterministic tie-break that
    // preserves input order among independent components (and makes the pass
    // idempotent: re-running a topological order yields it unchanged).
    let mut ready: BTreeSet<(usize, usize)> = comps
        .iter()
        .enumerate()
        .filter(|(c, _)| needs[*c].is_empty())
        .map(|(c, _)| (first_index[c], c))
        .collect();
    let mut remaining: Vec<usize> = needs.iter().map(BTreeSet::len).collect();
    let mut order = Vec::with_capacity(n);
    while let Some((_, c)) = ready.iter().next().copied() {
        ready.remove(&(first_index[c], c));
        order.extend(comps[c].iter().copied());
        for &u in &users[c] {
            remaining[u] -= 1;
            if remaining[u] == 0 {
                ready.insert((first_index[u], u));
            }
        }
    }
    // The component DAG is acyclic by construction, so every node is
    // emitted; keep the input order as a safety net (unreachable).
    if order.len() == n {
        order
    } else {
        (0..n).collect()
    }
}

/// `deps[i]` = indices of defs whose name occurs as a token in def `i`'s body.
fn dependencies(shard: &Shard) -> Vec<BTreeSet<usize>> {
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, def) in shard.defs.iter().enumerate() {
        by_name.entry(def.name.as_str()).or_default().push(i);
    }
    let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); shard.defs.len()];
    for (i, def) in shard.defs.iter().enumerate() {
        let mut texts = Vec::new();
        collect_texts(&def.body, &mut texts);
        for text in texts {
            for token in tokens(text) {
                if let Some(targets) = by_name.get(token) {
                    for &j in targets {
                        // Self-edges carry no ordering information; dropping
                        // them keeps singletons sortable (self-recursion is
                        // still grouped correctly: a one-node component).
                        if j != i {
                            deps[i].insert(j);
                        }
                    }
                }
            }
        }
    }
    deps
}

fn collect_texts<'a>(body: &'a [Stmt], out: &mut Vec<&'a str>) {
    for stmt in body {
        match stmt {
            Stmt::Assign { lhs, rhs } => {
                out.push(lhs.as_str());
                out.push(rhs.as_str());
            }
            Stmt::TupleAssign { targets, rhs } => {
                for t in targets {
                    out.push(t.as_str());
                }
                out.push(rhs.as_str());
            }
            Stmt::LetTuple { bindings, rhs, body } => {
                for b in bindings {
                    out.push(b.as_str());
                }
                out.push(rhs.as_str());
                collect_texts(body, out);
            }
            Stmt::Undefined { target, ty } => {
                out.push(target.as_str());
                out.push(ty.as_str());
            }
            Stmt::Expr(src) | Stmt::Return(src) => out.push(src.as_str()),
        }
    }
}

/// Whole-word identifier tokens (`[0-9A-Za-z_]+` runs).
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
}

/// Kosaraju SCCs in index order; members ascending. `graph[i]` = deps of `i`.
fn strongly_connected(graph: &[BTreeSet<usize>], n: usize) -> Vec<Vec<usize>> {
    // Forward pass: iterative DFS, nodes and neighbours in ascending order.
    let mut visited = vec![false; n];
    let mut finish = Vec::with_capacity(n);
    for root in 0..n {
        if visited[root] {
            continue;
        }
        let mut stack: Vec<(usize, bool)> = vec![(root, false)];
        visited[root] = true;
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                finish.push(node);
                continue;
            }
            stack.push((node, true));
            // Push neighbours in reverse so the smallest is visited first.
            for &next in graph[node].iter().rev() {
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, false));
                }
            }
        }
    }
    // Reverse graph.
    let mut reversed: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, tos) in graph.iter().enumerate() {
        for &to in tos {
            reversed[to].push(from);
        }
    }
    for list in &mut reversed {
        list.sort_unstable();
    }
    // Backward pass in reverse finish order.
    let mut comp_of: Vec<Option<usize>> = vec![None; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for &root in finish.iter().rev() {
        if comp_of[root].is_some() {
            continue;
        }
        let id = comps.len();
        let mut members = Vec::new();
        let mut stack = vec![root];
        comp_of[root] = Some(id);
        while let Some(node) = stack.pop() {
            members.push(node);
            for &prev in &reversed[node] {
                if comp_of[prev].is_none() {
                    comp_of[prev] = Some(id);
                    stack.push(prev);
                }
            }
        }
        members.sort_unstable();
        comps.push(members);
    }
    comps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn fun(name: &str, body: Vec<Stmt>) -> Def {
        Def::new(name, DefKind::Function, body)
    }

    fn names(shard: &Shard) -> Vec<&str> {
        shard.defs.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn moves_callee_before_caller() {
        let shard = Shard::new(
            0,
            vec![
                fun("f", vec![Stmt::Expr("g(x) + 1".into())]),
                fun("g", vec![Stmt::Expr("x".into())]),
            ],
        );
        let out = TopSortDefs.run(shard);
        assert_eq!(names(&out), vec!["g", "f"]);
        // Idempotent.
        let again = TopSortDefs.run(out.clone());
        assert_eq!(again, out);
    }

    #[test]
    fn keeps_mutual_recursion_together_in_input_order() {
        let shard = Shard::new(
            0,
            vec![
                fun("h", vec![Stmt::Expr("f(0)".into())]),
                fun("f", vec![Stmt::Expr("g(1)".into())]),
                fun("g", vec![Stmt::Expr("f(2)".into())]),
            ],
        );
        let out = TopSortDefs.run(shard);
        // `{f, g}` is one SCC and `h` depends on it: SCC first, input order.
        assert_eq!(names(&out), vec!["f", "g", "h"]);
        let again = TopSortDefs.run(out.clone());
        assert_eq!(again, out);
    }

    #[test]
    fn ignores_substring_matches_and_cross_shard_names() {
        let shard = Shard::new(
            0,
            vec![
                fun("f", vec![Stmt::Expr("foo + other".into())]),
                fun("g", vec![Stmt::Expr("1".into())]),
            ],
        );
        let out = TopSortDefs.run(shard.clone());
        // No whole-word edge (`foo` is not `f`): input order preserved.
        assert_eq!(out, shard);
    }

    #[test]
    fn handles_empty_and_singleton_shards() {
        assert_eq!(TopSortDefs.run(Shard::new(0, vec![])), Shard::new(0, vec![]));
        let one = Shard::new(0, vec![fun("f", vec![Stmt::Expr("f(f)".into())])]);
        assert_eq!(TopSortDefs.run(one.clone()), one);
    }
}
