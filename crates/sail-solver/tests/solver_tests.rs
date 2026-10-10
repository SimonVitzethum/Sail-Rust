#![forbid(unsafe_code)]
//! Integrations-Tests: Cache-Eviction + Stub (Default ohne C-Dep).

use sail_solver::{
    Constraint, MemoSolver, Solver, SolverCacheConfig, SolverPool, SolverVerdict, StubSolver,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn cache_eviction_bounds_entries() {
    let solver = MemoSolver::new(
        StubSolver::new(),
        SolverCacheConfig {
            capacity: 2,
            ttl: Duration::from_secs(60),
        },
    );
    // Stub liefert Unknown fuer Atome, aber der Cache muss trotzdem
    // begrenzen + Treffer zaehlen.
    let a = sail_solver::SolverQuery::new(Constraint::atom("a"));
    let b = sail_solver::SolverQuery::new(Constraint::atom("b"));
    let c = sail_solver::SolverQuery::new(Constraint::atom("c"));
    assert_eq!(solver.prove(&a), SolverVerdict::Unknown); // miss
    assert_eq!(solver.prove(&a), SolverVerdict::Unknown); // hit
    assert_eq!(solver.prove(&b), SolverVerdict::Unknown); // miss
    assert_eq!(solver.prove(&c), SolverVerdict::Unknown); // miss + evict a
    let s = solver.stats();
    assert_eq!(s.len, 2);
    assert_eq!((s.hits, s.misses), (1, 3));
}

#[test]
fn stub_terminates_without_proving_opaque() {
    let pool = SolverPool::with_default_cache(StubSolver::new);
    let s = pool.checkout();
    assert_eq!(s.name(), "stub");
    let v = s.prove(&sail_solver::SolverQuery::new(Constraint::atom(
        "forall x. x > 0",
    )));
    assert_eq!(v, SolverVerdict::Unknown);
}

#[test]
fn pool_shares_cache_across_threads() {
    let pool = Arc::new(SolverPool::with_default_cache(StubSolver::new));
    let cache = Arc::clone(pool.cache());
    let q = sail_solver::SolverQuery::new(Constraint::True);
    cache.store(&q, SolverVerdict::Valid);
    let mut hs = Vec::new();
    for _ in 0..4 {
        let c = Arc::clone(&cache);
        let qq = q.clone();
        hs.push(std::thread::spawn(move || c.lookup(&qq)));
    }
    for h in hs {
        assert_eq!(h.join().unwrap(), Some(SolverVerdict::Valid));
    }
    assert_eq!(cache.stats().hits, 4);
}
