#![forbid(unsafe_code)]
//! Lane-127-Paritaet Solver-Trait + CachingSolver (Code, E3: NICHT ausfuehren).
//!
//! Wahrheit: `sail-solver` (`sail_solver::Solver`: `prove(&SolverQuery) ->
//! SolverVerdict`, `name()`, `MemoCache` mit TTL + LRU). `sail-ty`
//! (`sail_ty::solver::Solver`: `prove(&NConstraint) -> ProveResult`,
//! `CachingSolver`) ist die Lane-05-lokale Sicht ueber Minimaltypen.
//! Abbildung: `Proved <-> Valid`, `Refuted <-> Invalid`,
//! `Unknown <-> Unknown`; beide Caches sind kapazitaetsbegrenzt (LRU).

use std::time::Duration;

use sail_solver::{
    Constraint, MemoCache, Solver as CanonicalSolver, SolverCacheConfig, SolverQuery,
    SolverVerdict, StubSolver as CanonicalStub,
};
use sail_ty::solver::{
    CachingSolver, NormalizingSolver, ProveResult, Solver as TySolver, StubSolver as TyStub,
};
use sail_ty::{NConstraint as TyNc, Nexp as TyNexp};

/// Stub-Antworten stimmen auf faltbaren Formeln ueberein.
#[test]
fn stub_verdicts_agree_on_foldable() {
    let ty = TyStub::new();
    let canon = CanonicalStub::new();
    // (sail-ty-Constraint, sail-solver-Constraint, erwartetes Paar).
    let cases: &[(TyNc, Constraint, ProveResult, SolverVerdict)] = &[
        (
            TyNc::True,
            Constraint::True,
            ProveResult::Proved,
            SolverVerdict::Valid,
        ),
        (
            TyNc::False,
            Constraint::False,
            ProveResult::Refuted,
            SolverVerdict::Invalid,
        ),
        (
            TyNc::Not(Box::new(TyNc::True)),
            Constraint::Not(Box::new(Constraint::True)),
            ProveResult::Refuted,
            SolverVerdict::Invalid,
        ),
        // `n == n` ist syntaktische Reflexivitaet (Proved/Valid),
        // `n != n` die Widerlegung (Refuted/Invalid).
        (
            TyNc::Eq(
                Box::new(TyNexp::Var("n".to_string())),
                Box::new(TyNexp::Var("n".to_string())),
            ),
            Constraint::atom("(= (var 1) (var 1))"),
            ProveResult::Proved,
            // Opakes Atom: kanonischer Stub bleibt sound-`Unknown`.
            SolverVerdict::Unknown,
        ),
    ];
    for (tync, cnc, want_ty, want_canon) in &cases[..3] {
        assert_eq!(ty.prove(tync), *want_ty);
        assert_eq!(canon.prove(&SolverQuery::new(cnc.clone())), *want_canon);
    }
    // Reflexivitaet lokal beweisbar; kanonisch sound-offen (kein Fake-Proof).
    assert_eq!(ty.prove(&cases[3].0), ProveResult::Proved);
    assert_eq!(
        canon.prove(&SolverQuery::new(cases[3].1.clone())),
        SolverVerdict::Unknown
    );
    assert!(SolverVerdict::Valid.is_valid());
    assert!(!SolverVerdict::Unknown.is_valid());
    assert!(ProveResult::Proved.is_proved());
    assert_eq!(canon.name(), "stub");
}

/// `CachingSolver` (sail-ty) ist begrenzt: Full + Repeat = Hit, Kapazitaet haelt.
#[test]
fn ty_caching_solver_is_bounded_lru() {
    let cached = CachingSolver::new(TyStub::new(), 2);
    assert!(cached.is_empty());
    let a = TyNc::True;
    let b = TyNc::False;
    let c = TyNc::Eq(Box::new(TyNexp::Const(1)), Box::new(TyNexp::Const(2)));
    assert_eq!(cached.prove(&a), ProveResult::Proved);
    assert_eq!(cached.prove(&a), ProveResult::Proved); // Hit
    assert_eq!(cached.prove(&b), ProveResult::Refuted);
    assert_eq!(cached.prove(&c), ProveResult::Refuted); // Eviction faellig
    let s = cached.stats();
    assert_eq!(s.hits, 1);
    assert_eq!(s.misses, 3);
    assert_eq!(s.evictions, 1);
    assert_eq!(cached.len(), 2);
}

/// `MemoCache` (sail-solver) ist begrenzt: 3 Stores bei Kapazitaet 2 -> len 2.
#[test]
fn canonical_memo_cache_is_bounded_lru() {
    let cache = MemoCache::new(SolverCacheConfig {
        capacity: 2,
        ttl: Duration::from_secs(60),
    });
    let q = |s: &str| SolverQuery::new(Constraint::atom(s));
    cache.store(&q("a"), SolverVerdict::Valid);
    cache.store(&q("b"), SolverVerdict::Invalid);
    cache.store(&q("c"), SolverVerdict::Unknown);
    let s = cache.stats();
    assert_eq!(s.len, 2);
    assert_eq!(s.capacity, 2);
    // Juengste bleiben, aelteste ist verdrangt.
    assert!(cache.lookup(&q("a")).is_none());
    assert_eq!(cache.lookup(&q("c")), Some(SolverVerdict::Unknown));
}

/// Lane 10: Paritaet des kanonischen Pfads.
/// - Grund-Potenzen (`2^3 == 8`) falten beidseitig identisch (ty-seitig via
///   `as_const`, kanonisch als opakes Atom nur bei Konstanten — der
///   kanonische Stub bleibt bei Atomen sound-`Unknown`, dokumentiert hier
///   die Arbeitsteilung: Faltung lokal, SMT-Potenzen an das Backend).
/// - Identitaeten (`n+0 == n`) beweist der ty-Stub nach Normalisierung;
///   der kanonische Stub kennt keine Nexp-Struktur (opakes Atom) und bleibt
///   `Unknown` — kein Fake-Proof.
/// - Allgemeine Apps / opake Booleans bleiben beidseitig `Unknown`.
#[test]
fn lane10_canonical_path_powers_apps_parity() {
    use sail_ty::nexp::normalize_nexp;
    let ty = TyStub::new();
    let canon = CanonicalStub::new();
    // Grund-Potenz faltet lokal …
    assert_eq!(
        normalize_nexp(&TyNexp::Exp(Box::new(TyNexp::Const(3)))),
        TyNexp::Const(8)
    );
    assert_eq!(
        ty.prove(&TyNc::Eq(
            Box::new(TyNexp::Exp(Box::new(TyNexp::Const(3)))),
            Box::new(TyNexp::Const(8)),
        )),
        ProveResult::Proved
    );
    // … symbolische Potenz bleibt ty-seitig `Unknown` (SMT-Backend faellig),
    // ausser reflexiv …
    assert_eq!(
        ty.prove(&TyNc::Eq(
            Box::new(TyNexp::Exp(Box::new(TyNexp::Var("n".to_string())))),
            Box::new(TyNexp::Const(8)),
        )),
        ProveResult::Unknown
    );
    assert_eq!(
        ty.prove(&TyNc::Eq(
            Box::new(TyNexp::Exp(Box::new(TyNexp::Var("n".to_string())))),
            Box::new(TyNexp::Exp(Box::new(TyNexp::Var("n".to_string())))),
        )),
        ProveResult::Proved
    );
    // Identitaet lokal bewiesen, kanonisch sound-offen (opakes Atom).
    assert_eq!(
        ty.prove(&TyNc::Eq(
            Box::new(TyNexp::Add(
                Box::new(TyNexp::Var("n".to_string())),
                Box::new(TyNexp::Const(0)),
            )),
            Box::new(TyNexp::Var("n".to_string())),
        )),
        ProveResult::Proved
    );
    assert_eq!(
        canon.prove(&SolverQuery::new(Constraint::atom(
            "(= (+ (var n) (const 0)) (var n))"
        ))),
        SolverVerdict::Unknown
    );
    // Allgemeine Apps / opake Booleans: beidseitig `Unknown`.
    assert_eq!(
        ty.prove(&TyNc::App(
            "sorted".to_string(),
            vec![TyNexp::Var("n".to_string())]
        )),
        ProveResult::Unknown
    );
    assert_eq!(
        canon.prove(&SolverQuery::new(Constraint::atom("(pred sorted (var n))"))),
        SolverVerdict::Unknown
    );
}

// ---------------------------------------------------------------------------
// Lane 20 (Audit O-6): Existential-/Range-/arithmetische Inferenz-Reste.
// ---------------------------------------------------------------------------
//
// Was die lokale Schicht (sail-ty: `Nexp::simplify`, `linear_form`,
// `StubSolver`-Lineararithmetik, `match_scalar_arg`/`match_bool_arg`)
// jetzt beweist — und was auf beiden Schichten sound-offen bleibt:
// symbolische Potenzarithmetik (`2 ^ 'n`: der kanonische SMT-Pfad
// erhaelt nur Kongruenz ohne arithmetische Axiome), allgemeine
// Constraint-Applikationen (`canonical.rs`
// "constraint synonym needs expansion before proof") sowie nicht
// exakt-invertierbare abhängige Aufrufparameter (kanonisch
// `cannot infer dependent call parameter`, lokal
// `ScalarMatch::Uninferred`).

use sail_ast::{
    Constraint as AstConstraint, ConstraintKind as AstConstraintKind, Interner, NExp as AstNExp,
    NExpKind as AstNExpKind, Name, Node, SimpleInterner, TypArg as AstTypArg,
    TypArgKind as AstTypArgKind,
};
use sail_solver::canonical::{CanonicalQuery, Context, View};
use sail_ty::subsume::{match_bool_arg, match_scalar_arg, BoolMatch, ScalarMatch};
use std::collections::{BTreeMap, HashSet};

fn ty_var(v: &str) -> TyNexp {
    TyNexp::Var(v.to_string())
}

fn ty_const(c: i128) -> TyNexp {
    TyNexp::Const(c)
}

/// Der kanonische Stub und der sail-ty-Stub antworten auf opaken Formeln
/// identisch sound-`Unknown` (Abbildung `Unknown <-> Unknown`) — und die
/// `NormalizingSolver`-Schicht aendert daran nichts (sie schliesst nur
/// exakte lineare Luecken, raet nie).
#[test]
fn lane20_stubs_agree_on_opaque_sound_unknown() {
    let ty = TyStub::new();
    let norm = NormalizingSolver::new(TyStub::new());
    let canon = CanonicalStub::new();
    let opaque_ty = [
        TyNc::App("p".to_string(), vec![ty_var("n")]),
        TyNc::Var("b".to_string()),
        TyNc::Id("p".to_string()),
        TyNc::Set(Box::new(ty_var("n")), vec![1, 2]),
    ];
    for nc in &opaque_ty {
        assert_eq!(ty.prove(nc), ProveResult::Unknown, "{nc}");
        assert_eq!(norm.prove(nc), ProveResult::Unknown, "{nc}");
    }
    for atom in ["opaque", "p(n)", "another"] {
        let q = SolverQuery::new(Constraint::atom(atom));
        assert_eq!(canon.prove(&q), SolverVerdict::Unknown, "{atom}");
    }
    // Symbolische Potenz und nichtlineare Multiplikation bleiben auch
    // normalisiert offen (kein Raten).
    let power = TyNc::Eq(
        Box::new(TyNexp::Exp(Box::new(ty_var("n")))),
        Box::new(ty_const(8)),
    );
    assert_eq!(ty.prove(&power), ProveResult::Unknown);
    assert_eq!(norm.prove(&power), ProveResult::Unknown);
    let nonlinear = TyNc::Eq(
        Box::new(TyNexp::Mul(Box::new(ty_var("n")), Box::new(ty_var("m")))),
        Box::new(TyNexp::Mul(Box::new(ty_var("m")), Box::new(ty_var("n")))),
    );
    assert_eq!(ty.prove(&nonlinear), ProveResult::Unknown);
    assert_eq!(norm.prove(&nonlinear), ProveResult::Unknown);
}

/// Range-Inklusionen und Bits-Kongruenzen, wie `check_subtype` (Regeln
/// `S-RangeRange`/`S-BitsEq`) sie als Obligationen stellt, entscheidet die
/// `NormalizingSolver`-Schicht exakt — inklusive symbolischer Schranken.
/// Der blosse Stub bleibt dort per Vertrag offen (gepinnt in
/// `tests/typechecker_full.rs`).
#[test]
fn lane20_stub_discharges_checker_range_and_bits_obligations() {
    let ty = NormalizingSolver::new(TyStub::new());
    let n = || ty_var("n");
    // `range('n, 'n + 5) <: range('n - 1, 'n + 10)`:
    // `('n - 1 <= 'n) & ('n + 5 <= 'n + 10)` — konstante Differenz.
    let inclusion = TyNc::And(
        Box::new(TyNc::Le(
            Box::new(TyNexp::Sub(Box::new(n()), Box::new(ty_const(1)))),
            Box::new(n()),
        )),
        Box::new(TyNc::Le(
            Box::new(TyNexp::Add(Box::new(n()), Box::new(ty_const(5)))),
            Box::new(TyNexp::Add(Box::new(n()), Box::new(ty_const(10)))),
        )),
    );
    assert_eq!(ty.prove(&inclusion), ProveResult::Proved);
    // Verletzte Inklusion `range(0, 9) <: range(0, 8)` widerlegt konstant.
    let violated = TyNc::And(
        Box::new(TyNc::Le(Box::new(ty_const(0)), Box::new(ty_const(0)))),
        Box::new(TyNc::Le(Box::new(ty_const(9)), Box::new(ty_const(8)))),
    );
    assert_eq!(ty.prove(&violated), ProveResult::Refuted);
    // Offene Inklusion `'n <= 10` bleibt offen (Annahmen wohnen im
    // `SolverContext`, nicht im Stub).
    assert_eq!(
        ty.prove(&TyNc::Le(Box::new(n()), Box::new(ty_const(10)))),
        ProveResult::Unknown
    );
    // `bits('n + 0) <: bits('n)`: Normalisierung schliesst die Kongruenz.
    let congruence = TyNc::Eq(
        Box::new(TyNexp::Add(Box::new(n()), Box::new(ty_const(0)))),
        Box::new(n()),
    );
    assert_eq!(ty.prove(&congruence), ProveResult::Proved);
}

/// Powers retain congruence without assuming arithmetic facts. General
/// constraint applications still require elaboration.
#[test]
fn lane20_canonical_emitter_abstracts_powers_and_refuses_general_apps() {
    let mut it = SimpleInterner::new();
    let n_sym = it.intern("n");
    let m_sym = it.intern("m");
    let pred = it.intern("my_pred");
    let mut variables = BTreeMap::new();
    variables.insert("n".to_string(), 0);
    let context = Context {
        interner: &it,
        variables: &variables,
        bindings: None,
    };
    fn view<'a>(constraint: &'a AstConstraint, context: Context<'a>) -> View<'a> {
        View {
            negated: false,
            constraint,
            context,
        }
    }
    // `2 ^ 'n == 8` cannot follow from congruence without assumptions.
    let power = Node::dummy(AstNExpKind::Exp(Box::new(Node::dummy(AstNExpKind::Var(
        n_sym,
    )))));
    let eight: AstNExp = Node::dummy(AstNExpKind::Constant("8".to_string()));
    let goal: AstConstraint = Node::dummy(AstConstraintKind::Equal(
        Box::new(Node::dummy(AstTypArgKind::NExp(power))),
        Box::new(Node::dummy(AstTypArgKind::NExp(eight))),
    ));
    let query = CanonicalQuery::implication(&[], view(&goal, context), None).unwrap();
    let solver = sail_solver::SmtLibSolver::new();
    assert_eq!(
        solver.prove(&sail_solver::SolverQuery::new(
            sail_solver::Constraint::SmtLib(query)
        )),
        SolverVerdict::Unknown,
    );
    // Allgemeines Constraint-Prädikat `my_pred('n)`: braucht Expansion.
    let arg: AstTypArg = Node::dummy(AstTypArgKind::NExp(Node::dummy(AstNExpKind::Var(m_sym))));
    let app: AstConstraint = Node::dummy(AstConstraintKind::App {
        func: Name::dummy(pred),
        args: vec![arg],
    });
    let error = CanonicalQuery::implication(&[], view(&app, context), None).unwrap_err();
    assert_eq!(error, "constraint synonym needs expansion before proof");
    // Lineare Ziele emittieren dagegen fehlerfrei (Solver entscheidet).
    let linear: AstConstraint = Node::dummy(AstConstraintKind::Ge(
        Node::dummy(AstNExpKind::Sum(
            Box::new(Node::dummy(AstNExpKind::Var(n_sym))),
            Box::new(Node::dummy(AstNExpKind::Constant("1".to_string()))),
        )),
        Node::dummy(AstNExpKind::Var(n_sym)),
    ));
    let query = CanonicalQuery::implication(&[], view(&linear, context), None).unwrap();
    assert!(query.script().contains("(>= "));
}

/// Abhaengige Aufrufparameter: exakte lokale Bindungen
/// (`match_scalar_arg`/`match_bool_arg`) gegenueber sound-offenen Resten.
#[test]
fn lane20_dependent_parameters_bind_exactly_or_stay_open() {
    let int = HashSet::from(["n".to_string()]);
    // Arithmetisch exakt: `'n + 1 = 5` bindet `'n := 4` (kanonisch waere
    // dies `cannot infer dependent call parameter` — Verdrahtung folgt).
    let formal = TyNexp::Add(Box::new(ty_var("n")), Box::new(ty_const(1)));
    assert_eq!(
        match_scalar_arg(&formal, &ty_const(5), &int),
        ScalarMatch::Bind {
            var: "n".to_string(),
            to: ty_const(4)
        }
    );
    // Potenz-Formal `2 ^ 'n = 8`: lokal wie kanonisch sound-offen.
    let power = TyNexp::Exp(Box::new(ty_var("n")));
    assert!(matches!(
        match_scalar_arg(&power, &ty_const(8), &int),
        ScalarMatch::Uninferred { .. }
    ));
    // Boolesch exakt: bare Variable bindet; App bleibt offen.
    let bools = HashSet::from(["p".to_string()]);
    assert_eq!(
        match_bool_arg(&TyNc::Var("p".to_string()), &TyNc::True, &bools),
        BoolMatch::Bind {
            var: "p".to_string(),
            to: TyNc::True
        }
    );
    assert!(matches!(
        match_bool_arg(
            &TyNc::App("p".to_string(), vec![ty_var("n")]),
            &TyNc::True,
            &bools
        ),
        BoolMatch::Uninferred { .. }
    ));
}
