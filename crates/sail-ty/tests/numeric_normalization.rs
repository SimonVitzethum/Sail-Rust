//! Ground folding and bounded-memory rebuilding of deep numeric expressions.
use sail_ty::nexp::{normalize_nexp, Nexp};

#[test]
fn unary_ground_subterms_fold_inside_symbolic_expressions() {
    for (inner, expected) in [
        (Nexp::Exp(Box::new(Nexp::Const(3))), 8),
        (Nexp::Neg(Box::new(Nexp::Const(3))), -3),
        (Nexp::Abs(Box::new(Nexp::Const(-3))), 3),
    ] {
        let expression = Nexp::Add(Box::new(Nexp::Var("n".into())), Box::new(inner));
        assert_eq!(
            normalize_nexp(&expression),
            Nexp::Add(
                Box::new(Nexp::Var("n".into())),
                Box::new(Nexp::Const(expected))
            )
        );
    }
}

#[test]
fn ten_thousand_symbolic_nodes_rebuild_without_prefix_copies() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            fn release_and_check(mut expression: Nexp) {
                let mut depth = 0;
                while let Nexp::Add(left, right) = expression {
                    assert_eq!(*right, Nexp::Const(1));
                    expression = *left;
                    depth += 1;
                }
                assert_eq!(expression, Nexp::Var("n".into()));
                assert_eq!(depth, 10_000);
            }
            let mut expression = Nexp::Var("n".into());
            for _ in 0..10_000 {
                expression = Nexp::Add(Box::new(expression), Box::new(Nexp::Const(1)));
            }
            release_and_check(expression.simplify());
            release_and_check(normalize_nexp(&expression));
            release_and_check(expression);
        })
        .unwrap()
        .join()
        .unwrap();
}
