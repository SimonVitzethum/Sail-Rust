#![forbid(unsafe_code)]
//! Lane-58 acceptance: `bit_lists_to_lits`, `vector_concat_assignments`,
//! `remove_vector_subrange_pats`.
//!
//! The three pass modules live under `src/passes/` but are not wired into
//! `passes/mod.rs` yet (a later lane owns that `Insert-Spec`; this lane
//! must not edit `pipeline` / `mod` / `pass`). To exercise them through the
//! acceptance run (`cargo test -p sail-rewrite`) they are included here by
//! path. The modules only use `crate::ast` / `crate::pass` paths (exactly
//! like their wired siblings), which resolve through the re-exports below,
//! so the future `mod.rs` wiring needs no changes inside the pass files.

pub use sail_rewrite::ast;
pub use sail_rewrite::pass;

use sail_rewrite::passes::{BitListsToLits, RemoveVectorSubrangePats, VectorConcatAssignments};
use sail_ast::Interner;


use sail_rewrite::pipeline::is_known_pass;
use sail_rewrite::{Def, DefKind, Shard, Stmt};

use pass::Pass;

#[test]
fn pass_names_match_lean_rewrite_order() {
    assert_eq!(sail_rewrite::passes::BitListsToLits.name(), "bit_lists_to_lits");
    assert_eq!(
        sail_rewrite::passes::VectorConcatAssignments.name(),
        "vector_concat_assignments"
    );
    assert_eq!(
        sail_rewrite::passes::RemoveVectorSubrangePats.name(),
        "remove_vector_subrange_pats"
    );
    for name in [
        "bit_lists_to_lits",
        "vector_concat_assignments",
        "remove_vector_subrange_pats",
    ] {
        assert!(is_known_pass(name), "{name} must be a known Lean rewrite");
    }
}

#[test]
fn string_level_passes_preserve_shards_verbatim() {
    // The string statement IR carries no types and no match arms, so all
    // three passes preserve shards (documented in each module, same
    // conservative choice as `SimpleAssignments`).
    let shard = Shard::new(
        0,
        vec![
            Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Assign { lhs: "x".into(), rhs: "[0b1, 0b0]".into() },
                    Stmt::TupleAssign { targets: vec!["a".into()], rhs: "t".into() },
                    Stmt::LetTuple {
                        bindings: vec!["b".into()],
                        rhs: "t".into(),
                        body: vec![Stmt::Return("b".into())],
                    },
                    Stmt::Undefined { target: "u".into(), ty: "bits(8)".into() },
                    Stmt::Expr("x[7..0]".into()),
                    Stmt::Return("x".into()),
                ],
            ),
            Def::new("sig", DefKind::Valspec, vec![]),
        ],
    );
    assert_eq!(sail_rewrite::passes::BitListsToLits.run(shard.clone()), shard);
    assert_eq!(
        sail_rewrite::passes::VectorConcatAssignments.run(shard.clone()),
        shard
    );
    assert_eq!(
        sail_rewrite::passes::RemoveVectorSubrangePats.run(shard.clone()),
        shard
    );
}

#[test]
fn bit_lists_end_to_end_hex_msb_first() {
    use sail_rewrite::passes::BitListsToLits;
    let mut it = sail_ast::SimpleInterner::new();
    let mut bit = |text: &str| {
        sail_ast::Node::dummy(sail_ast::ExpKind::Lit(sail_ast::Node::dummy(
            sail_ast::LitKind::Bin(it.intern(text)),
        )))
    };
    // Eight bits: two hex digits, MSB first.
    let mut ast = sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
        pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
        exp: sail_ast::Node::dummy(sail_ast::ExpKind::Vector(vec![
            bit("1"),
            bit("1"),
            bit("0"),
            bit("0"),
            bit("0"),
            bit("0"),
            bit("0"),
            bit("1"),
        ])),
    })]);
    assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 1);
    let sail_ast::DefKind::Let { exp, .. } = &ast.defs[0].kind else {
        panic!("def shape")
    };
    let sail_ast::ExpKind::Lit(lit) = &exp.kind else {
        panic!("expected literal, got {:?}", exp.kind)
    };
    let sail_ast::LitKind::Hex(symbol) = lit.kind else {
        panic!("expected hex, got {:?}", lit.kind)
    };
    assert_eq!(it.resolve(symbol), Some("C1"));
}

#[test]
fn vector_concat_end_to_end_descending_with_effectful_rhs() {
    use sail_rewrite::passes::{ConcatLengths, VectorConcatAssignments};
    struct Ctx;
    impl ConcatLengths for Ctx {
        fn order_inc(&self) -> bool {
            false
        }
        fn start_index(&self, _total: u128) -> i128 {
            3
        }
        fn part_length(&self, _part: &sail_ast::LExp) -> Option<u128> {
            Some(2)
        }
    }
    let mut it = sail_ast::SimpleInterner::new();
    let mut id = |text: &str| sail_ast::Name::dummy(it.intern(text));
    let mut ast = sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
        pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
        exp: sail_ast::Node::dummy(sail_ast::ExpKind::Assign {
            lexp: Box::new(sail_ast::Node::dummy(sail_ast::LExpKind::VectorConcat(vec![
                sail_ast::Node::dummy(sail_ast::LExpKind::Id(id("hi"))),
                sail_ast::Node::dummy(sail_ast::LExpKind::Id(id("lo"))),
            ]))),
            value: Box::new(sail_ast::Node::dummy(sail_ast::ExpKind::App {
                func: id("read"),
                args: vec![],
            })),
        }),
    })]);
    assert_eq!(VectorConcatAssignments.run_ast(&mut ast, &mut it, &Ctx), 1);
    let sail_ast::DefKind::Let { exp, .. } = &ast.defs[0].kind else {
        panic!("def shape")
    };
    // `let split_vec = read() in { hi = slice(split_vec,3,2);
    // lo = slice(split_vec,1,0) }`.
    let sail_ast::ExpKind::Let { pat, bind, body } = &exp.kind else {
        panic!("expected let, got {:?}", exp.kind)
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
    for (assign, (target, first, second)) in
        assigns.iter().zip([("hi", "3", "2"), ("lo", "1", "0")])
    {
        let sail_ast::ExpKind::Assign { lexp, value } = &assign.kind else {
            panic!("expected assign")
        };
        let sail_ast::LExpKind::Id(name) = &lexp.kind else {
            panic!("expected id lexp")
        };
        assert_eq!(it.resolve(name.symbol), Some(target));
        let sail_ast::ExpKind::App { args, .. } = &value.kind else {
            panic!("expected slice")
        };
        let mut nums = args[1..].iter().map(|arg| {
            let sail_ast::ExpKind::Lit(lit) = &arg.kind else {
                panic!("expected lit")
            };
            let sail_ast::LitKind::Num(symbol) = lit.kind else {
                panic!("expected num")
            };
            it.resolve(symbol).unwrap().to_owned()
        });
        assert_eq!((nums.next().as_deref(), nums.next().as_deref()), (Some(first), Some(second)));
    }
}

#[test]
fn subrange_end_to_end_two_ids_sorted_by_ascending_id() {
    use sail_rewrite::passes::RemoveVectorSubrangePats;
    let mut it = sail_ast::SimpleInterner::new();
    let mut sub = |id: &str, lo: &str, hi: &str| {
        sail_ast::Node::dummy(sail_ast::PatKind::VectorSubrange {
            id: sail_ast::Name::dummy(it.intern(id)),
            lo: lo.to_owned(),
            hi: hi.to_owned(),
        })
    };
    let arm = |pat: sail_ast::Pat| sail_ast::PExp {
        id: sail_ast::NodeId::DUMMY,
        span: sail_ast::Span::DUMMY,
        kind: sail_ast::PExpKind::PatExp {
            pat,
            body: sail_ast::Node::dummy(sail_ast::ExpKind::Undef),
        },
    };
    let mut ast = sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Fundef {
        rec: sail_ast::RecOpt::NonRec,
        annot: sail_ast::TAnnotOpt::None,
        funcls: vec![sail_ast::Funcl {
            id: sail_ast::NodeId::DUMMY,
            span: sail_ast::Span::DUMMY,
            name: sail_ast::Name::dummy(sail_ast::Symbol::DUMMY),
            arm: arm(sail_ast::Node::dummy(sail_ast::PatKind::Tuple(vec![
                sub("y", "1", "0"),
                sub("x", "7", "4"),
            ]))),
        }],
    })]);
    assert_eq!(RemoveVectorSubrangePats.run_ast(&mut ast, &mut it), 1);
    let sail_ast::DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
        panic!("def shape")
    };
    let sail_ast::PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
        panic!("arm shape")
    };
    // Ascending id order: `Bindings.fold` wraps `x` first, so the outer
    // layer is `let y` and the inner layer is `let x`.
    let sail_ast::ExpKind::Let { pat, bind: _, body: inner } = &body.kind else {
        panic!("expected lets, got {:?}", body.kind)
    };
    let sail_ast::PatKind::Id(y) = &pat.kind else {
        panic!("expected id pat")
    };
    assert_eq!(it.resolve(y.symbol), Some("y"));
    let sail_ast::ExpKind::Let { pat, bind, body: _ } = &inner.kind else {
        panic!("expected nested let")
    };
    let sail_ast::PatKind::Id(x) = &pat.kind else {
        panic!("expected id pat")
    };
    assert_eq!(it.resolve(x.symbol), Some("x"));
    let sail_ast::ExpKind::Id(slice) = &bind.kind else {
        panic!("expected slice id")
    };
    assert_eq!(it.resolve(slice.symbol), Some("x_7_4#"));
}
