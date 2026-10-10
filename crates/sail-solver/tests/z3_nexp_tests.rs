#![cfg(feature = "z3")]
#![forbid(unsafe_code)]
//! Z3-Mapping-Tests Lane 80 (Code, E3: nicht ausfuehren): exakte Operatoren
//! beweisen `Valid`/`Invalid`, konservative fallen auf `Unknown` zurueck.
//! Laeuft nur mit `--features z3` (C-Dep).

use sail_nexp::{NConstraint, NExp, TypArg};
use sail_solver::{Constraint, Solver, SolverQuery, SolverVerdict, Z3Solver};

fn v(n: &str) -> NExp {
    NExp::var(n)
}
fn c(i: i128) -> NExp {
    NExp::Constant(i)
}
fn eq(a: NExp, b: NExp) -> NConstraint {
    NConstraint::Equal(Box::new(TypArg::NExp(a)), Box::new(TypArg::NExp(b)))
}

#[test]
fn z3_proves_exact_operators() {
    let s = Z3Solver::new();
    assert_eq!(s.name(), "z3");
    // Arithmetik + Vergleiche exakt.
    let two_plus_two = eq(NExp::Sum(Box::new(c(2)), Box::new(c(2))), c(4));
    assert_eq!(s.prove_nconstraint(&two_plus_two), SolverVerdict::Valid);
    assert_eq!(s.prove_nconstraint(&eq(c(2), c(3))), SolverVerdict::Invalid);
    // `abs`/`If`/`2^x` exakt.
    let abs_nneg = NConstraint::Ge(Box::new(NExp::abs(v("n"))), Box::new(c(0)));
    assert_eq!(s.prove_nconstraint(&abs_nneg), SolverVerdict::Valid);
    let ite = eq(
        NExp::If(Box::new(NConstraint::True), Box::new(c(1)), Box::new(c(2))),
        c(1),
    );
    assert_eq!(s.prove_nconstraint(&ite), SolverVerdict::Valid);
    assert_eq!(
        s.prove_nconstraint(&eq(NExp::Exp(Box::new(c(3))), c(8))),
        SolverVerdict::Valid
    );
    // Bignum-Literal jenseits `i64` exakt (Horner-Pfad).
    assert_eq!(
        s.prove_nconstraint(&eq(c(i128::MAX), c(i128::MAX))),
        SolverVerdict::Valid
    );
    assert_eq!(
        s.prove_nconstraint(&eq(c(i128::MIN), c(i128::MAX))),
        SolverVerdict::Invalid
    );
    // `Set` als Disjunktion.
    assert_eq!(
        s.prove_nconstraint(&NConstraint::Set(Box::new(c(5)), vec![1, 5])),
        SolverVerdict::Valid
    );
    assert_eq!(
        s.prove_nconstraint(&NConstraint::Set(Box::new(c(7)), vec![1, 5])),
        SolverVerdict::Invalid
    );
    assert_eq!(
        s.prove_nconstraint(&NConstraint::Set(Box::new(v("n")), vec![])),
        SolverVerdict::Invalid
    );
}

#[test]
fn z3_is_conservative_where_inexact() {
    let s = Z3Solver::new();
    // Offene Variable: erfuellbar und widerlegbar => Unknown.
    let open = NConstraint::Gt(Box::new(v("n")), Box::new(c(5)));
    assert_eq!(s.prove_nconstraint(&open), SolverVerdict::Unknown);
    // `div` ohne exakte Entsprechung: frische Konstante => Unknown
    // (kein falsches Valid/Invalid).
    let div_eq = eq(NExp::div(c(4), c(2)), c(2));
    assert_eq!(s.prove_nconstraint(&div_eq), SolverVerdict::Unknown);
    // Unbekannte Apps/Praedikate/Ids: Unknown.
    let uf = eq(NExp::App("uf".to_string(), vec![v("n")]), c(0));
    assert_eq!(s.prove_nconstraint(&uf), SolverVerdict::Unknown);
    assert_eq!(
        s.prove_nconstraint(&NConstraint::App("ok".to_string(), vec![TypArg::NExp(v("n"))])),
        SolverVerdict::Unknown
    );
    assert_eq!(
        s.prove_nconstraint(&NConstraint::Id("p".to_string())),
        SolverVerdict::Unknown
    );
}

#[test]
fn z3_timeout_and_opaque_query_plumbing() {
    let s = Z3Solver::new().with_timeout(5_000);
    assert_eq!(
        s.prove_nconstraint(&eq(c(1), c(1))),
        SolverVerdict::Valid
    );
    // Query-Timeout schlaegt Solver-Timeout (beide gesetzt, kein Fehler).
    let q = SolverQuery::new(Constraint::True).with_timeout(5_000);
    assert_eq!(s.prove(&q), SolverVerdict::Valid);
    assert_eq!(
        s.prove(&SolverQuery::new(Constraint::False)),
        SolverVerdict::Invalid
    );
    assert_eq!(
        s.prove(&SolverQuery::new(Constraint::atom("opaque"))),
        SolverVerdict::Unknown
    );
}
