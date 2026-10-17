//! Powers are abstracted conservatively; only UNSAT implications prove facts.
use sail_ast::*;
use sail_solver::canonical::{CanonicalQuery, Context, View};
use sail_solver::SmtLibSolver;
use sail_solver::{Constraint as SolverConstraint, Solver, SolverQuery, SolverVerdict};
use std::collections::BTreeMap;

fn variable(symbol: Symbol) -> NExp {
    Node::dummy(NExpKind::Var(symbol))
}
fn power(symbol: Symbol) -> NExp {
    Node::dummy(NExpKind::Exp(Box::new(variable(symbol))))
}
fn equal(left: NExp, right: NExp) -> Constraint {
    Node::dummy(ConstraintKind::Equal(
        Box::new(Node::dummy(TypArgKind::NExp(left))),
        Box::new(Node::dummy(TypArgKind::NExp(right))),
    ))
}
fn prove(
    interner: &SimpleInterner,
    names: &[&str],
    assumptions: &[Constraint],
    goal: &Constraint,
) -> SolverVerdict {
    let variables: BTreeMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.to_string(), i as u64))
        .collect();
    let context = Context {
        interner,
        variables: &variables,
        bindings: None,
    };
    let view = |constraint| View {
        constraint,
        context,
        negated: false,
    };
    let query = CanonicalQuery::implication(
        &assumptions.iter().map(view).collect::<Vec<_>>(),
        view(goal),
        None,
    )
    .unwrap();
    assert_eq!(query.script().matches("(declare-fun sail_pow2").count(), 1);
    SmtLibSolver::new().prove(&SolverQuery::new(SolverConstraint::SmtLib(query)))
}

#[test]
fn powers_preserve_congruence_of_proven_equal_exponents() {
    let mut interner = SimpleInterner::new();
    let n = interner.intern("n");
    let m = interner.intern("m");
    assert_eq!(
        prove(
            &interner,
            &["n", "m"],
            &[equal(variable(n), variable(m))],
            &equal(power(n), power(m))
        ),
        SolverVerdict::Valid
    );
}

#[test]
fn explicit_power_witness_evidence_transfers_a_bound() {
    let mut interner = SimpleInterner::new();
    let n = interner.intern("n");
    let p = interner.intern("p");
    let r = interner.intern("r");
    let assumptions = [
        equal(variable(p), power(n)),
        Node::dummy(ConstraintKind::Lt(variable(r), variable(p))),
    ];
    let goal = Node::dummy(ConstraintKind::Lt(variable(r), power(n)));
    assert_eq!(
        prove(&interner, &["n", "p", "r"], &assumptions, &goal),
        SolverVerdict::Valid
    );
}

#[test]
fn an_abstract_power_does_not_invent_positivity_or_values() {
    let mut interner = SimpleInterner::new();
    let n = interner.intern("n");
    let zero = Node::dummy(NExpKind::Constant("0".into()));
    let goal = Node::dummy(ConstraintKind::Gt(power(n), zero));
    assert_eq!(prove(&interner, &["n"], &[], &goal), SolverVerdict::Unknown);
    let eight = Node::dummy(NExpKind::Constant("8".into()));
    assert_eq!(
        prove(&interner, &["n"], &[], &equal(power(n), eight)),
        SolverVerdict::Unknown
    );
}
