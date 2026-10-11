#![forbid(unsafe_code)]
//! Lane-127-Paritaet IntKind/Facts (Code, E3: NICHT ausfuehren).
//!
//! Wahrheit: `sail-nexp` (`sail_nexp::{IntKind, Facts, provably_nneg_in,
//! int_kind_in}` mit internierten `Kid`s, Lower-+Upper-Bounds,
//! Nonzero-Mengen). `sail-int` (`sail_int::{IntKind, Facts, NExp}` mit
//! `String`-Schluesseln, nur Lower-Bounds) stimmt auf allen geteilten
//! Shapes ueberein; was `sail-int` nicht kennt (Upper-Bounds,
//! Nonzero, Constraints), faellt konservativ auf `Int` zurueck.

use sail_ast::{Interner as _, SimpleInterner};
use sail_int::{Facts as IntFacts, NExp as IntExp, NumType};
use sail_nexp::{int_kind_in, provably_nneg_in, Facts, NExp};

fn kid(ix: &mut SimpleInterner, name: &str) -> sail_nexp::Kid {
    ix.intern(name)
}

/// Konstanten und geschlossene Shapes klassifizieren beide gleich.
#[test]
fn closed_shapes_agree() {
    let mut ix = SimpleInterner::new();
    let n = kid(&mut ix, "n");
    // (sail-int-Ausdruck, sail-nexp-Ausdruck, erwartetes Lean-Name).
    let cases: &[(IntExp, NExp, &str)] = &[
        (
            IntExp::Constant(0),
            NExp::Constant(0),
            "Nat",
        ),
        (
            IntExp::Constant(7),
            NExp::Constant(7),
            "Nat",
        ),
        (
            IntExp::Constant(-1),
            NExp::Constant(-1),
            "Int",
        ),
        (
            IntExp::abs(IntExp::var("n")),
            NExp::abs(NExp::Var(n)),
            "Nat",
        ),
        (
            IntExp::Pow2(Box::new(IntExp::var("n"))),
            NExp::Exp(Box::new(NExp::Var(n))),
            "Int",
        ),
        (
            IntExp::negated(IntExp::Constant(1)),
            NExp::Neg(Box::new(NExp::Constant(1))),
            "Int",
        ),
    ];
    for (ie, ne, want) in cases {
        let inat = NumType::Atom(ie.clone()).lean_name() == "Nat";
        let nnat = int_kind_in(ne, &Facts::new()) == sail_nexp::IntKind::Nat;
        assert_eq!(inat, nnat, "Drift bei {ie:?}");
        assert_eq!(if nnat { "Nat" } else { "Int" }, *want);
        assert_eq!(
            sail_int::provably_nneg(ie),
            sail_nexp::provably_nneg(ne),
            "provably_nneg-Drift bei {ie:?}"
        );
    }
}

/// Variablen mit Lower-Bound-Fakt `n >= 0` sind beidseitig `Nat`,
/// ohne Fakten beidseitig `Int`.
#[test]
fn var_with_lower_bound_agrees() {
    let mut ix = SimpleInterner::new();
    let n = kid(&mut ix, "n");
    let ie = IntExp::var("n");
    let ne = NExp::Var(n);
    // Ohne Fakten: unbekannt -> `Int`.
    assert_eq!(NumType::Atom(ie.clone()).lean_name(), "Int");
    assert_eq!(int_kind_in(&ne, &Facts::new()), sail_nexp::IntKind::Int);
    // Mit `n >= 0`: `Nat`.
    let ifacts = IntFacts::new().with_lower("n", 0);
    let facts = Facts::new().with_lower(n, 0);
    assert_eq!(
        NumType::Atom(ie.clone()).lean_name_in(&ifacts),
        "Nat"
    );
    assert_eq!(int_kind_in(&ne, &facts), sail_nexp::IntKind::Nat);
    assert!(sail_int::provably_nneg_in(&ie, &ifacts));
    assert!(provably_nneg_in(&ne, &facts));
    // Schwaecherer Bound (`n >= -5`) reicht beidseitig nicht.
    let ifacts = IntFacts::new().with_lower("n", -5);
    let facts = Facts::new().with_lower(n, -5);
    assert_eq!(NumType::Atom(ie.clone()).lean_name_in(&ifacts), "Int");
    assert_eq!(int_kind_in(&ne, &facts), sail_nexp::IntKind::Int);
}
