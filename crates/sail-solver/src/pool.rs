//! Solver-Pool: 1 Solver pro Thread, kein Sharing.
//!
//! Z3-Kontexte sind weder `Sync` noch billig teilbar; daher leiht sich jeder
//! Thread per [`SolverPool::checkout`] exklusiv eine eigene Instanz aus
//! (Guard gibt sie per `Drop` zurueck). Der [`MemoCache`] dagegen ist
//! `Send + Sync` und wird poolweit ueber `Arc` geteilt.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

use crate::cache::{MemoCache, SolverCacheConfig};
use crate::solver::Solver;

/// Pool mit werkseitig erzeugten Solvern plus geteiltem Memo-Cache.
pub struct SolverPool<S> {
    factory: Box<dyn Fn() -> S + Send + Sync>,
    free: Mutex<Vec<S>>,
    cache: Arc<MemoCache>,
}

impl<S: Solver> SolverPool<S> {
    /// Neuer Pool; `factory` baut frische Solver (1 pro Thread per Checkout).
    pub fn new(
        factory: impl Fn() -> S + Send + Sync + 'static,
        config: SolverCacheConfig,
    ) -> Self {
        SolverPool {
            factory: Box::new(factory),
            free: Mutex::new(Vec::new()),
            cache: Arc::new(MemoCache::new(config)),
        }
    }

    /// Pool mit Default-Cache ([`SolverCacheConfig::small`]).
    pub fn with_default_cache(factory: impl Fn() -> S + Send + Sync + 'static) -> Self {
        Self::new(factory, SolverCacheConfig::default())
    }

    /// Exklusiv einen Solver ausleihen (ggf. neu erzeugen).
    pub fn checkout(&self) -> PooledSolver<'_, S> {
        let solver = self
            .free
            .lock()
            .expect("solver pool lock poisoned")
            .pop()
            .unwrap_or_else(|| (self.factory)());
        PooledSolver {
            solver: Some(solver),
            free: &self.free,
        }
    }

    /// Geteilter Memo-Cache des Pools.
    pub fn cache(&self) -> &Arc<MemoCache> {
        &self.cache
    }

    /// Anzahl aktuell freier (zurueckgegebener) Solver.
    pub fn idle(&self) -> usize {
        self.free.lock().expect("solver pool lock poisoned").len()
    }
}

/// Exklusiver Leih-Guard; gibt den Solver bei `Drop` zurueck.
pub struct PooledSolver<'a, S> {
    solver: Option<S>,
    free: &'a Mutex<Vec<S>>,
}

impl<S> Deref for PooledSolver<'_, S> {
    type Target = S;
    fn deref(&self) -> &S {
        self.solver.as_ref().expect("pooled solver taken")
    }
}

impl<S> DerefMut for PooledSolver<'_, S> {
    fn deref_mut(&mut self) -> &mut S {
        self.solver.as_mut().expect("pooled solver taken")
    }
}

impl<S> Drop for PooledSolver<'_, S> {
    fn drop(&mut self) {
        if let Some(s) = self.solver.take() {
            if let Ok(mut free) = self.free.lock() {
                free.push(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Constraint, SolverQuery};
    use crate::stub::StubSolver;

    #[test]
    fn checkout_returns_solver_to_pool() {
        let pool = SolverPool::with_default_cache(StubSolver::new);
        assert_eq!(pool.idle(), 0);
        {
            let s = pool.checkout();
            assert_eq!(s.name(), "stub");
            assert_eq!(pool.idle(), 0);
        }
        assert_eq!(pool.idle(), 1);
        // Wiederverwendung, kein zweiter Solver noetig.
        let _a = pool.checkout();
        assert_eq!(pool.idle(), 0);
    }

    #[test]
    fn pool_is_multithreaded_exclusive() {
        let pool = Arc::new(SolverPool::with_default_cache(StubSolver::new));
        let mut hs = Vec::new();
        for i in 0..8 {
            let p = Arc::clone(&pool);
            hs.push(std::thread::spawn(move || {
                let s = p.checkout();
                s.prove(&SolverQuery::new(Constraint::atom(format!("t{i}"))));
            }));
        }
        for h in hs {
            h.join().expect("worker panicked");
        }
        assert!(pool.idle() >= 1);
    }
}
