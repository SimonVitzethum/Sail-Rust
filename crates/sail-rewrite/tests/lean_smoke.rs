//! End-to-end smoke test: Lean-default pipeline over sharded definitions.
//!
//! Models a tiny Sail shape (`undefined` + tuple assignment + self-assign)
//! of the kind found under `sail/test/lean`, shards it for low RAM, and runs
//! the full [`Pipeline::lean_default`] (implemented passes + stubs) in
//! parallel via rayon.

#![forbid(unsafe_code)]

use sail_rewrite::{Def, DefKind, Pipeline, Shard, Stmt};

#[test]
fn lean_default_pipeline_parallel_smoke() {
    let defs: Vec<Def> = (0..8)
        .map(|i| {
            Def::new(
                format!("f{i}"),
                DefKind::Function,
                vec![
                    Stmt::Undefined {
                        target: "n".to_string(),
                        ty: "int".to_string(),
                    },
                    Stmt::TupleAssign {
                        targets: vec!["a".to_string(), "b".to_string()],
                        rhs: "t".to_string(),
                    },
                    Stmt::Assign {
                        lhs: "x".to_string(),
                        rhs: "x".to_string(),
                    },
                ],
            )
        })
        .collect();

    let shards = Shard::chunk(defs, 2);
    assert_eq!(shards.len(), 4);

    let pipe = Pipeline::lean_default();
    // Full Lean order: 50 entries incl. repeated recheck_defs/attach_effects.
    assert_eq!(pipe.len(), sail_rewrite::pipeline::LEAN_REWRITES.len());

    let out = pipe.run_parallel(shards);
    assert_eq!(out.len(), 4);
    for shard in &out {
        for def in &shard.defs {
            // No Undefined / TupleAssign / self-assign may survive.
            for stmt in &def.body {
                assert!(
                    !matches!(stmt, Stmt::Undefined { .. }),
                    "undefined survived"
                );
                assert!(!matches!(stmt, Stmt::TupleAssign { .. }), "tuple survived");
                assert!(
                    !matches!(stmt, Stmt::Assign { lhs, rhs } if lhs == rhs),
                    "self-assign survived"
                );
            }
            assert_eq!(def.body.len(), 3);
        }
    }
}
