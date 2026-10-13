//! `E_sizeof` (`'n` in Ausdruckstellung, OCaml `infer_exp`) und
//! Vergleichs-Guard-Fakten (OCaml `E_if`-Annahmen): `m < 'n` liefert den
//! Typ von `<`, und atomare Vergleichen steuern `'m < 'n` als Annahme
//! bei, sodass Rumpf-Constraints (`truncate` braucht `'m <= 'n`)
//! schliessen. Wie OCaml braucht `'n` eine `__size`-Ueberladung im
//! Scope; ohne sie ehrlich `UnboundVar`.

use sail_ast::*;
use sail_parse::{ParserContext, RealParser};
use sail_ty::{error::TypeErrorKind, program::Program};
use std::sync::Arc;

fn parse(sources: &[&str]) -> Vec<(Def, Arc<SimpleInterner>)> {
    let mut context = ParserContext::default();
    sources
        .iter()
        .map(|source| {
            let mut parser = RealParser::with_context(context.clone());
            let def = parser
                .parse_def(source, Span::DUMMY)
                .unwrap_or_else(|e| panic!("parse failed for {source:?}: {e}"));
            context = parser.context();
            (def, Arc::new(parser.into_interner()))
        })
        .collect()
}

fn declare(defs: &[(Def, Arc<SimpleInterner>)]) -> Program {
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program.declare(def, it.clone(), position).unwrap();
    }
    program
}

const PRELUDE: &[&str] = &[
    "default Order dec",
    "type bits('n) = bitvector('n)",
    "val less_than : forall 'n 'm. (int('n), int('m)) -> bool",
    "overload operator < = {less_than}",
    "val bitvector_length : forall 'n. bits('n) -> int('n)",
    "overload __size = {bitvector_length}",
    "val truncate : forall ('m : Int) ('n : Int), 'm >= 0 & 'm <= 'n. (bits('n), int('m)) -> bits('m)",
    "val sail_zero_extend : forall ('n : Int) ('m : Int), 'm >= 'n. (bits('n), int('m)) -> bits('m)",
];

fn check_with(prelude: &[&str], bodies: &[&str]) -> Result<usize, sail_ty::error::TypeError> {
    let mut sources: Vec<&str> = Vec::with_capacity(prelude.len() + bodies.len());
    sources.extend(prelude.iter().copied());
    sources.extend(bodies.iter().copied());
    let defs = parse(&sources);
    let program = declare(&defs);
    let mut checked = 0;
    for (position, (def, it)) in defs.iter().enumerate().skip(prelude.len()) {
        checked += program.check_definition(def, (*it).clone(), position)?;
    }
    Ok(checked)
}

#[test]
fn sizeof_comparison_checks() {
    check_with(
        PRELUDE,
        &[
            "val c1 : forall 'n. (int, bits('n)) -> bool",
            "function c1(m, v) = m < 'n",
        ],
    )
    .expect("m < 'n should check with __size in scope");
}

#[test]
fn sizeof_needs_size_overload_in_scope() {
    // Wie OCaml (`No possible overloading for __size`): ohne
    // `__size`-Ueberladung ist `'n` ungebunden, nie still akzeptiert.
    let prelude: Vec<&str> = PRELUDE
        .iter()
        .copied()
        .filter(|line| !line.contains("__size"))
        .collect();
    let err = check_with(
        &prelude,
        &[
            "val c1 : forall 'n. (int, bits('n)) -> bool",
            "function c1(m, v) = m < 'n",
        ],
    )
    .expect_err("sizeof without __size must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::UnboundVar(_)),
        "missing __size should be UnboundVar, got {err:?}"
    );
}

#[test]
fn guard_fact_discharges_callee_constraint() {
    // `truncate` braucht `'m <= 'n`; der Guard `m < 'n` liefert es.
    check_with(
        PRELUDE,
        &[
            "val extzv : forall 'n ('m : Int), 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
            "function extzv(m, v) = {
  if m < 'n then truncate(v, m) else sail_zero_extend(v, m)
}",
        ],
    )
    .expect("guarded truncate should check");
}

#[test]
fn unguarded_call_stays_unproved() {
    // Ohne Guard ist `'m <= 'n` unbeweisbar: ehrliches
    // `ConstraintNotProved`, kein Raten.
    let err = check_with(
        PRELUDE,
        &[
            "val bad : forall 'n ('m : Int), 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
            "function bad(m, v) = truncate(v, m)",
        ],
    )
    .expect_err("unguarded truncate must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)),
        "unguarded call should be ConstraintNotProved, got {err:?}"
    );
}
