//! Lane-21 recursion audit: iterative depth guards stay total on 10_000-deep
//! inputs where the native-recursive `depth()`/`conv` functions abort with
//! `SIGABRT` (see `evidence/2026-10-12/lane-21-recursion.md` for the probe
//! log). All 10k cases run on a 128 KiB worker thread — the same budget as
//! the existing `canonical_program` depth tests — and must *return*, never
//! overflow. Plus roundtrip coverage for the Lane-21 `conv.rs` clone
//! removals (`typarg_nexp_only` borrow, `typ_from_ast` moves).

use sail_ast::*;
use sail_ty::conv::{constraint_from_ast, typ_from_ast};
use sail_ty::nexp::{MiniTyp, NConstraint, Nexp, TypQuant};
use sail_ty::recursion::{
    constraint_exceeds_depth, mini_typ_exceeds_depth, nexp_exceeds_depth, typ_quant_exceeds_depth,
};

fn deep_add(depth: usize) -> Nexp {
    let mut n = Nexp::Const(0);
    for _ in 0..depth {
        n = Nexp::Add(Box::new(Nexp::Const(1)), Box::new(n));
    }
    n
}

fn deep_and(depth: usize) -> NConstraint {
    let mut c = NConstraint::True;
    for _ in 0..depth {
        c = NConstraint::And(Box::new(NConstraint::True), Box::new(c));
    }
    c
}

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .expect("iterative guard must return on a 128 KiB stack, never overflow");
}

#[test]
fn iterative_guards_reject_10k_inputs_on_a_small_stack() {
    // NOTE: deep values are `mem::forget`ten on purpose — dropping a
    // 10_000-deep `Box` chain runs the derived recursive `Drop`, which
    // would overflow the small stack itself (separate finding, see report
    // §4; `sail-ast` nodes have an iterative `drop_definition` for this).
    on_small_stack(|| {
        let n = deep_add(10_000);
        assert!(nexp_exceeds_depth(&n, 256));
        std::mem::forget(n);
        let c = deep_and(10_000);
        assert!(constraint_exceeds_depth(&c, 256));
        std::mem::forget(c);
        let t = MiniTyp::Bits(Box::new(deep_add(10_000)));
        assert!(mini_typ_exceeds_depth(&t, 256));
        std::mem::forget(t);
        let t = MiniTyp::Range(Box::new(deep_add(10_000)), Box::new(Nexp::Const(0)));
        assert!(mini_typ_exceeds_depth(&t, 256));
        std::mem::forget(t);
        let q = TypQuant {
            quants: vec![],
            constraint: deep_and(10_000),
            typ: MiniTyp::Int,
        };
        assert!(typ_quant_exceeds_depth(&q, 256));
        std::mem::forget(q);
    });
}

#[test]
fn iterative_guards_accept_shallow_inputs_on_a_small_stack() {
    on_small_stack(|| {
        assert!(!nexp_exceeds_depth(&deep_add(10), 256));
        assert!(!constraint_exceeds_depth(&deep_and(10), 256));
        assert!(!mini_typ_exceeds_depth(
            &MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bits(Box::new(deep_add(10)))]),
            256
        ));
    });
}

#[test]
fn typ_from_ast_moves_agree_with_clone_semantics() {
    // `bits('n)` single-arg, `range(lo,hi)` two-arg, and the multi-arg
    // `Named` fallback (incl. zero args) — the shapes the Lane-21 move
    // rewrite in `typ_from_ast` must accept exactly as before.
    let mut it = SimpleInterner::new();
    let n = it.intern("n");
    let bits_name = it.intern("bits");
    let range_name = it.intern("range");

    let bits = Node::dummy(TypKind::App {
        name: Name::dummy(bits_name),
        args: vec![Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(n))))],
    });
    assert_eq!(
        typ_from_ast(&bits, &it),
        Some(MiniTyp::Bits(Box::new(Nexp::Var("n".to_owned()))))
    );

    let range = Node::dummy(TypKind::App {
        name: Name::dummy(range_name),
        args: vec![
            Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Constant(
                "8".to_owned(),
            )))),
            Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Constant(
                "15".to_owned(),
            )))),
        ],
    });
    assert_eq!(
        typ_from_ast(&range, &it),
        Some(MiniTyp::Range(
            Box::new(Nexp::Const(8)),
            Box::new(Nexp::Const(15))
        ))
    );

    // Two-arg `bits` is NOT the unary primitive => Named fallback.
    let bits2 = Node::dummy(TypKind::App {
        name: Name::dummy(bits_name),
        args: vec![
            Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(n)))),
            Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(n)))),
        ],
    });
    assert_eq!(
        typ_from_ast(&bits2, &it),
        Some(MiniTyp::Named {
            name: "bits".to_owned(),
            args: vec![Nexp::Var("n".to_owned()), Nexp::Var("n".to_owned())],
        })
    );

    // Non-nexp argument => None (unchanged `typarg_nexp_only` semantics).
    let bad = Node::dummy(TypKind::App {
        name: Name::dummy(bits_name),
        args: vec![Node::dummy(TypArgKind::Bool(Box::new(Node::dummy(
            ConstraintKind::True,
        ))))],
    });
    assert_eq!(typ_from_ast(&bad, &it), None);
}

#[test]
fn constraint_app_with_bool_arg_stays_abstract() {
    // `NC_app` with a non-`A_nexp` argument lifts to `None` — the borrowed
    // `typarg_nexp_only` must reject exactly like the cloning version did.
    let mut it = SimpleInterner::new();
    let p = it.intern("p");
    let c = Node::dummy(ConstraintKind::App {
        func: Name::dummy(p),
        args: vec![Node::dummy(TypArgKind::Bool(Box::new(Node::dummy(
            ConstraintKind::True,
        ))))],
    });
    assert_eq!(constraint_from_ast(&c, &it), None);
}
