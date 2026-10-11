//! Tuple assignment lowering, matching `rewrite_tuple_assignments` in
//! `sail/src/lib/rewrites.ml`: bind every RHS component before writing any
//! target. Repeated projections of the original RHS duplicate effects and
//! break swaps when earlier assignments mutate later components.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct TupleAssignments;

impl Pass for TupleAssignments {
    fn name(&self) -> &'static str { "tuple_assignments" }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            let mut used = BTreeSet::new();
            collect_identifiers(&def.body, &mut used);
            let mut next = 0;
            rewrite_body(&mut def.body, &mut used, &mut next);
        }
        shard
    }
}

fn collect_identifiers(body: &[Stmt], used: &mut BTreeSet<String>) {
    fn text(src: &str, used: &mut BTreeSet<String>) {
        used.extend(src.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty()).map(str::to_owned));
    }
    for stmt in body {
        match stmt {
            Stmt::Assign { lhs, rhs } => { text(lhs, used); text(rhs, used); }
            Stmt::TupleAssign { targets, rhs } => {
                for target in targets { text(target, used); }
                text(rhs, used);
            }
            Stmt::LetTuple { bindings, rhs, body } => {
                for binding in bindings { text(binding, used); }
                text(rhs, used);
                collect_identifiers(body, used);
            }
            Stmt::Undefined { target, ty } => { text(target, used); text(ty, used); }
            Stmt::Expr(src) | Stmt::Return(src) => text(src, used),
        }
    }
}

fn rewrite_body(body: &mut [Stmt], used: &mut BTreeSet<String>, next: &mut usize) {
    for stmt in body {
        match stmt {
            Stmt::TupleAssign { targets, rhs } => {
                let bindings: Vec<String> = targets.iter().map(|_| loop {
                    let name = format!("__sail_tuple_{}", *next);
                    *next += 1;
                    if used.insert(name.clone()) { break name; }
                }).collect();
                let assignments = std::mem::take(targets).into_iter()
                    .zip(bindings.iter().cloned())
                    .map(|(lhs, rhs)| Stmt::Assign { lhs, rhs }).collect();
                *stmt = Stmt::LetTuple {
                    bindings, rhs: std::mem::take(rhs), body: assignments,
                };
            }
            Stmt::LetTuple { body, .. } => rewrite_body(body, used, next),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn run_body(body: Vec<Stmt>) -> Vec<Stmt> {
        TupleAssignments.run(Shard::new(0,
            vec![Def::new("f", DefKind::Function, body)])).defs.swap_remove(0).body
    }

    #[test]
    fn binds_effectful_rhs_once_before_assigning_any_target() {
        let out = run_body(vec![Stmt::TupleAssign {
            targets: vec!["a".into(), "b".into()], rhs: "call()".into(),
        }]);
        assert_eq!(out, vec![Stmt::LetTuple {
            bindings: vec!["__sail_tuple_0".into(), "__sail_tuple_1".into()],
            rhs: "call()".into(),
            body: vec![
                Stmt::Assign { lhs: "a".into(), rhs: "__sail_tuple_0".into() },
                Stmt::Assign { lhs: "b".into(), rhs: "__sail_tuple_1".into() },
            ],
        }]);
    }

    #[test]
    fn zero_and_one_component_still_use_tuple_binding() {
        for targets in [vec![], vec!["x".into()]] {
            let out = run_body(vec![Stmt::TupleAssign {
                targets: targets.clone(), rhs: "effect()".into(),
            }]);
            let Stmt::LetTuple { bindings, rhs, body } = &out[0] else { panic!("missing binding") };
            assert_eq!(bindings.len(), targets.len());
            assert_eq!(body.len(), targets.len());
            assert_eq!(rhs, "effect()");
        }
    }

    #[test]
    fn avoids_capture_and_preserves_surrounding_statements() {
        let first = Stmt::Expr("__sail_tuple_0".into());
        let last = Stmt::Return("a".into());
        let out = run_body(vec![first.clone(), Stmt::TupleAssign {
            targets: vec!["a".into(), "__sail_tuple_1".into()],
            rhs: "(__sail_tuple_1, a)".into(),
        }, last.clone()]);
        assert_eq!(out[0], first);
        assert_eq!(out[2], last);
        let Stmt::LetTuple { bindings, .. } = &out[1] else { panic!("missing binding") };
        assert_eq!(bindings, &["__sail_tuple_2", "__sail_tuple_3"]);
        assert_eq!(run_body(out.clone()), out, "pass must be idempotent");
    }
}
