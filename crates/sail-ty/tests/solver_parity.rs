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
use sail_ty::solver::{CachingSolver, ProveResult, Solver as TySolver, StubSolver as TyStub};
use sail_ty::{NConstraint as TyNc, Nexp as TyNexp};

/// Stub-Antworten stimmen auf faltbaren Formeln ueberein.
#[test]
fn stub_verdicts_agree_on_foldable() {
    let ty = TyStub::new();
    let canon = CanonicalStub::new();
    // (sail-ty-Constraint, sail-solver-Constraint, erwartetes Paar).
    let cases: &[(TyNc, Constraint, ProveResult, SolverVerdict)] = &[
        (TyNc::True, Constraint::True, ProveResult::Proved, SolverVerdict::Valid),
        (TyNc::False, Constraint::False, ProveResult::Refuted, SolverVerdict::Invalid),
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
