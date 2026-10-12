//! Solver-Pool: 1 Solver pro Thread, kein Sharing.
//!
//! Z3-Kontexte sind weder `Sync` noch billig teilbar; daher leiht sich jeder
//! Thread per [`SolverPool::checkout`] exklusiv eine eigene Instanz aus
//! (Guard gibt sie per `Drop` zurueck). Der [`MemoCache`] dagegen ist
//! `Send + Sync` und wird poolweit ueber `Arc` geteilt.

use std::ops::{Deref, DerefMut};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use crate::cache::{MemoCache, SolverCacheConfig};
use crate::query::SolverQuery;
use crate::quota::{QuotaConfig, QuotaPermit, QuotaRecommendation, QuotaStats, SolverQuota};
use crate::solver::{Solver, SolverVerdict};

/// Pool mit werkseitig erzeugten Solvern plus geteiltem Memo-Cache.
pub struct SolverPool<S> {
    factory: Box<dyn Fn() -> S + Send + Sync>,
    free: Mutex<Vec<S>>,
    cache: Arc<MemoCache>,
    /// Quota fuer ruhende Solver (Lane 15): Bei Rueckgabe ueber die Quote
    /// hinaus wird die Instanz verworfen statt geparkt (Z3-Kontexte sind
    /// speicherteuer). `usize::MAX` = unbegrenzt (Default, Verhalten wie bisher).
    max_idle: AtomicUsize,
    /// Anzahl je erzeugter Solver-Instanzen (Retirements nicht abgezogen;
    /// zusammen mit [`SolverPool::idle`] ergibt das die Live-Schaetzung).
    created: AtomicU64,
    /// Live-Quota (Lane 36, Spec aus `quota.rs`): begrenzt gleichzeitig
    /// ausgecheckte Solver. `try_checkout`/`prove_cached` sind fail-fast
    /// (`None`/`Unknown` bei erschoepfter Quote), [`SolverPool::checkout`]
    /// blockiert per `acquire`. Der Permit lebt im [`PooledSolver`] und
    /// faellt erst mit der Solver-Rueckgabe.
    quota: Arc<SolverQuota>,
    /// Timeout fuer [`SolverPool::checkout_with_timeout`] und als Default
    /// fuer quota-gesteuerte Checkouts mit Warten; als Nanos hinterlegt,
    /// damit `set_acquire_timeout` per `&self` geht (Default 5 s wie
    /// [`QuotaConfig::small`]/`unlimited`).
    acquire_timeout_nanos: AtomicU64,
}

impl<S: Solver> SolverPool<S> {
    /// Neuer Pool; `factory` baut frische Solver (1 pro Thread per Checkout).
    ///
    /// Quota-Default ist unbegrenzt (Verhalten wie bisher); per
    /// [`SolverPool::with_quota`] / [`SolverPool::apply_recommendation`]
    /// einhaengen.
    pub fn new(
        factory: impl Fn() -> S + Send + Sync + 'static,
        config: SolverCacheConfig,
    ) -> Self {
        SolverPool {
            factory: Box::new(factory),
            free: Mutex::new(Vec::new()),
            cache: Arc::new(MemoCache::new(config)),
            max_idle: AtomicUsize::new(usize::MAX),
            created: AtomicU64::new(0),
            quota: Arc::new(SolverQuota::new(QuotaConfig::unlimited())),
            acquire_timeout_nanos: AtomicU64::new(
                QuotaConfig::unlimited().acquire_timeout.as_nanos().min(u128::from(u64::MAX))
                    as u64,
            ),
        }
    }

    /// Pool mit Default-Cache ([`SolverCacheConfig::small`]).
    pub fn with_default_cache(factory: impl Fn() -> S + Send + Sync + 'static) -> Self {
        Self::new(factory, SolverCacheConfig::default())
    }

    /// Pool mit geteilter Live-Quota (Lane 36). `acquire_timeout` startet
    /// bei 5 s; per [`SolverPool::set_acquire_timeout`] anpassbar.
    pub fn with_quota(
        factory: impl Fn() -> S + Send + Sync + 'static,
        config: SolverCacheConfig,
        quota: Arc<SolverQuota>,
    ) -> Self {
        SolverPool {
            factory: Box::new(factory),
            free: Mutex::new(Vec::new()),
            cache: Arc::new(MemoCache::new(config)),
            max_idle: AtomicUsize::new(usize::MAX),
            created: AtomicU64::new(0),
            quota,
            acquire_timeout_nanos: AtomicU64::new(5_000_000_000),
        }
    }

    /// Pool mit Quota aus Konfiguration (erzeugt eine eigene
    /// [`SolverQuota`]; zum Teilen einer lebenden Quote
    /// [`SolverPool::with_quota`] nutzen).
    pub fn with_quota_config(
        factory: impl Fn() -> S + Send + Sync + 'static,
        config: SolverCacheConfig,
        quota_config: QuotaConfig,
    ) -> Self {
        let timeout = quota_config.acquire_timeout;
        SolverPool {
            factory: Box::new(factory),
            free: Mutex::new(Vec::new()),
            cache: Arc::new(MemoCache::new(config)),
            max_idle: AtomicUsize::new(usize::MAX),
            created: AtomicU64::new(0),
            quota: Arc::new(SolverQuota::new(quota_config)),
            acquire_timeout_nanos: AtomicU64::new(
                timeout.as_nanos().min(u128::from(u64::MAX)) as u64,
            ),
        }
    }

    /// Geteilte Live-Quota des Pools.
    pub fn quota(&self) -> &Arc<SolverQuota> {
        &self.quota
    }

    /// Momentaufnahme der Quota-Metriken.
    pub fn quota_stats(&self) -> QuotaStats {
        self.quota.stats()
    }

    /// Timeout fuer [`SolverPool::checkout_with_timeout`].
    pub fn acquire_timeout(&self) -> Duration {
        Duration::from_nanos(self.acquire_timeout_nanos.load(Ordering::Relaxed))
    }

    /// Setzt den Checkout-Timeout (wirkt auf kuenftige
    /// `checkout_with_timeout`-Aufrufe ohne expliziten Timeout).
    pub fn set_acquire_timeout(&self, timeout: Duration) {
        self.acquire_timeout_nanos.store(
            timeout.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }

    /// Exklusiv einen Solver ausleihen (ggf. neu erzeugen).
    ///
    /// Blockiert per `SolverQuota::acquire`, bis ein Live-Slot frei wird
    /// (kein `Unknown`; fail-fast-Variante:
    /// [`SolverPool::try_checkout`]). Der Permit wird im Guard mitgefuehrt
    /// und faellt erst mit der Solver-Rueckgabe.
    pub fn checkout(&self) -> PooledSolver<'_, S> {
        let permit = self.quota.acquire();
        let solver = self
            .free
            .lock()
            .expect("solver pool lock poisoned")
            .pop()
            .unwrap_or_else(|| {
                self.created.fetch_add(1, Ordering::Relaxed);
                (self.factory)()
            });
        PooledSolver {
            solver: Some(solver),
            free: &self.free,
            max_idle: &self.max_idle,
            _permit: Some(permit),
        }
    }

    /// Fail-fast-Checkout (Lane 36): `None` bei erschoepfter Live-Quota
    /// (kein neuer Solver, kein Warten), sonst Guard mit Permit.
    ///
    /// Der Permit wird **vor** der `factory()`-Erzeugung angefordert;
    /// Wiederverwendung aus `free` braucht ebenfalls einen Slot, damit
    /// gleichzeitig ausgecheckte Solver die Quote nie ueberschreiten.
    pub fn try_checkout(&self) -> Option<PooledSolver<'_, S>> {
        let permit = self.quota.try_acquire()?;
        let solver = self
            .free
            .lock()
            .expect("solver pool lock poisoned")
            .pop()
            .unwrap_or_else(|| {
                self.created.fetch_add(1, Ordering::Relaxed);
                (self.factory)()
            });
        Some(PooledSolver {
            solver: Some(solver),
            free: &self.free,
            max_idle: &self.max_idle,
            _permit: Some(permit),
        })
    }

    /// Checkout mit explizitem Timeout: `None` nach Ablauf ohne freien
    /// Live-Slot (zaehlt `timed_out_total` in der Quote).
    pub fn checkout_with_timeout(&self, timeout: Duration) -> Option<PooledSolver<'_, S>> {
        let permit = self.quota.acquire_timeout(timeout)?;
        let solver = self
            .free
            .lock()
            .expect("solver pool lock poisoned")
            .pop()
            .unwrap_or_else(|| {
                self.created.fetch_add(1, Ordering::Relaxed);
                (self.factory)()
            });
        Some(PooledSolver {
            solver: Some(solver),
            free: &self.free,
            max_idle: &self.max_idle,
            _permit: Some(permit),
        })
    }

    /// Kanonischer Prover-Pfad mit Zaehlung (Lane 15): Cache-Lookup, bei
    /// Miss exklusiver Checkout + zeitgestoppter Solver-Lauf + Store.
    /// Kuenftige `Program::prove`-Adoption ersetzt dort die handverdrahtete
    /// lookup/checkout/store-Folge durch genau diesen Aufruf; die
    /// SMT-Zeit landet ueber den geteilten Cache in
    /// [`MemoCache::smt_totals`] und damit im CLI-Profiler.
    ///
    /// Lane 36: der exklusive Checkout ist per [`SolverPool::try_checkout`]
    /// quota-umschlossen (fail-fast); bei erschoepfter Quote kommt
    /// [`SolverVerdict::Unknown`] zurueck statt ein neuer Solver gebaut zu
    /// werden (kein Store, keine `note_smt_call` — ein spaeterer Retry kann
    /// nach Freiwerden eines Slots noch beweisen).
    pub fn prove_cached(&self, query: &SolverQuery) -> SolverVerdict {
        if let Some(v) = self.cache.lookup(query) {
            return v;
        }
        let start = std::time::Instant::now();
        let Some(guard) = self.try_checkout() else {
            return SolverVerdict::Unknown;
        };
        let v = guard.prove(query);
        self.cache.note_smt_call(start.elapsed());
        self.cache.store(query, v);
        v
    }

    /// Geteilter Memo-Cache des Pools.
    pub fn cache(&self) -> &Arc<MemoCache> {
        &self.cache
    }

    /// Anzahl aktuell freier (zurueckgegebener) Solver.
    pub fn idle(&self) -> usize {
        self.free.lock().expect("solver pool lock poisoned").len()
    }

    /// Anzahl je erzeugter Solver-Instanzen (inkl. verworfener).
    pub fn created_total(&self) -> u64 {
        self.created.load(Ordering::Relaxed)
    }

    /// Setzt die Idle-Quota (ruhende Solver). Ueberschuss wird bei der
    /// naechsten Rueckgabe/`evict_idle` verworfen.
    pub fn set_max_idle(&self, max_idle: usize) {
        self.max_idle.store(max_idle, Ordering::Relaxed);
    }

    /// Aktuelle Idle-Quota (`usize::MAX` = unbegrenzt).
    pub fn max_idle(&self) -> usize {
        self.max_idle.load(Ordering::Relaxed)
    }

    /// Verwirft ruhende Solver bis zur Quote. Gibt die Anzahl
    /// verworfener Instanzen zurueck (adaptiver Speicher-Drop-Punkt).
    pub fn evict_idle(&self) -> usize {
        let max = self.max_idle.load(Ordering::Relaxed);
        let mut free = self.free.lock().expect("solver pool lock poisoned");
        let mut evicted = 0;
        while free.len() > max {
            free.pop();
            evicted += 1;
        }
        evicted
    }

    /// Uebernimmt eine [`QuotaRecommendation`]: Live-Limit in die
    /// [`SolverQuota`], Idle-Parkplatz per `set_max_idle`, Cache-Bytes per
    /// `MemoCache::set_max_bytes`. Adaptiver Drop-Punkt ohne neue Logik;
    /// Durchsetzung per [`SolverPool::enforce_quotas`].
    pub fn apply_recommendation(&self, rec: &QuotaRecommendation) {
        rec.apply_to(&self.quota);
        self.set_max_idle(rec.max_idle);
        self.cache.set_max_bytes(rec.max_cache_bytes);
    }

    /// Periodischer Drop-Punkt (Lane 36): verwirft Idle-Ueberschuss und
    /// Cache-Bytes ueber der Quote. Gibt `(idle_evicted, cache_evicted)`
    /// zurueck. Kein Lookup-Raeumen (Lane-15-Semantik bleibt).
    pub fn enforce_quotas(&self) -> (usize, usize) {
        (self.evict_idle(), self.cache.enforce_byte_quota())
    }

    /// Kanonische Prover-Zähler des Pools: `(SMT-Rundläufe, SMT-Sekunden)`,
    /// summiert über den geteilten Cache (alle Threads).
    pub fn smt_totals(&self) -> (u64, f64) {
        self.cache.smt_totals()
    }
}

/// Exklusiver Leih-Guard; gibt den Solver bei `Drop` zurueck.
/// Ueber die Pool-Quota ([`SolverPool::set_max_idle`]) hinaus wird die
/// Instanz verworfen statt geparkt.
///
/// Der mitgefuehrte [`QuotaPermit`] (Lane 36) faellt erst nach der
/// Solver-Rueckgabe (Feld-Drop nach `Drop::drop`) und gibt damit den
/// Live-Slot zurueck.
pub struct PooledSolver<'a, S> {
    solver: Option<S>,
    free: &'a Mutex<Vec<S>>,
    max_idle: &'a AtomicUsize,
    _permit: Option<QuotaPermit<'a>>,
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
                if free.len() < self.max_idle.load(Ordering::Relaxed) {
                    free.push(s);
                }
                // Sonst: Quota-Ueberschuss verwerfen (Solver-Drop ausserhalb
                // des Locks ist hier nicht noetig; `s` faellt am Ende raus).
            }
        }
        // `_permit` faellt hier (nach der Rueckgabe) und weckt genau einen
        // blockierten `acquire`-Rufer.
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

    #[test]
    fn prove_cached_counts_smt_and_reuses_cache() {
        let pool = SolverPool::with_default_cache(StubSolver::new);
        let q = SolverQuery::new(Constraint::atom("pooled"));
        pool.prove_cached(&q);
        pool.prove_cached(&q);
        let (calls, secs) = pool.smt_totals();
        assert_eq!(calls, 1);
        assert!(secs >= 0.0);
        let s = pool.cache().stats();
        assert_eq!((s.hits, s.misses, s.smt_calls), (1, 1, 1));
        assert!(pool.created_total() >= 1);
    }

    #[test]
    fn idle_quota_discards_surplus_solvers() {
        let pool = SolverPool::with_default_cache(StubSolver::new);
        pool.set_max_idle(1);
        assert_eq!(pool.max_idle(), 1);
        {
            let _a = pool.checkout();
            let _b = pool.checkout();
            assert_eq!(pool.created_total(), 2);
        }
        // Nur ein Solver darf parken, der zweite wird verworfen.
        assert_eq!(pool.idle(), 1);
        assert_eq!(pool.evict_idle(), 0);
        pool.set_max_idle(0);
        assert_eq!(pool.evict_idle(), 1);
        assert_eq!(pool.idle(), 0);
    }

    #[test]
    fn quota_try_checkout_fail_fast_and_recovers() {
        let quota = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_millis(10),
        }));
        let pool = SolverPool::with_quota(StubSolver::new, SolverCacheConfig::small(), Arc::clone(&quota));
        let _held = pool.try_checkout().expect("first slot");
        assert_eq!(quota.live(), 1);
        // Kein Warten, kein neuer Solver bei erschoepfter Quote.
        let created_before = pool.created_total();
        assert!(pool.try_checkout().is_none());
        assert_eq!(pool.created_total(), created_before);
        drop(_held);
        assert_eq!(quota.live(), 0);
        assert!(pool.try_checkout().is_some());
    }

    #[test]
    fn prove_cached_returns_unknown_when_quota_exhausted() {
        let quota = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_millis(10),
        }));
        let pool = SolverPool::with_quota(StubSolver::new, SolverCacheConfig::small(), Arc::clone(&quota));
        let q = SolverQuery::new(Constraint::atom("quota-blocked"));
        // Quote von aussen erschoepfen (kein Checkout moeglich).
        let _block = quota.try_acquire().expect("block quota");
        let created_before = pool.created_total();
        let (calls_before, _) = pool.smt_totals();
        assert_eq!(pool.prove_cached(&q), SolverVerdict::Unknown);
        // Kein Solver gebaut, kein SMT-Lauf notiert, nichts gecacht.
        assert_eq!(pool.created_total(), created_before);
        assert_eq!(pool.smt_totals().0, calls_before);
        assert!(pool.cache().lookup(&q).is_none());
        drop(_block);
        // Nach Freiwerden laeuft der kanonische Pfad wieder.
        assert_eq!(pool.prove_cached(&q), SolverVerdict::Unknown);
        assert_eq!(pool.smt_totals().0, calls_before + 1);
    }

    #[test]
    fn checkout_with_timeout_respects_deadline() {
        let quota = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_millis(10),
        }));
        let pool = SolverPool::with_quota(StubSolver::new, SolverCacheConfig::small(), Arc::clone(&quota));
        let _held = pool.try_checkout().expect("first slot");
        let start = std::time::Instant::now();
        assert!(pool.checkout_with_timeout(Duration::from_millis(20)).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(quota.stats().timed_out_total, 1);
    }

    #[test]
    fn blocking_checkout_wakes_on_return() {
        let quota = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_secs(5),
        }));
        let pool = Arc::new(SolverPool::with_quota(
            StubSolver::new,
            SolverCacheConfig::small(),
            Arc::clone(&quota),
        ));
        let held = pool.try_checkout().expect("first slot");
        let p2 = Arc::clone(&pool);
        let h = std::thread::spawn(move || {
            let _s = p2.checkout();
            assert_eq!(p2.quota().live(), 1);
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(held);
        h.join().expect("worker panicked");
    }

    #[test]
    fn apply_recommendation_feeds_pool_and_cache() {
        let pool = SolverPool::with_default_cache(StubSolver::new);
        let rec = QuotaRecommendation::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        pool.apply_recommendation(&rec);
        assert_eq!(pool.quota().max_live(), rec.max_live);
        assert_eq!(pool.max_idle(), rec.max_idle);
        assert_eq!(pool.cache().max_bytes(), rec.max_cache_bytes);
    }

    #[test]
    fn enforce_quotas_drops_idle_and_cache_bytes() {
        let pool = SolverPool::with_default_cache(StubSolver::new);
        // Idle-Ueberschuss aufbauen.
        {
            let _a = pool.checkout();
            let _b = pool.checkout();
        }
        assert_eq!(pool.idle(), 2);
        for i in 0..6 {
            let q = SolverQuery::new(Constraint::atom(format!("eq-{i}")));
            pool.cache().store(&q, SolverVerdict::Unknown);
        }
        let full = pool.cache().approx_bytes() as u64;
        assert!(full > 0);
        pool.set_max_idle(0);
        pool.cache().set_max_bytes(full / 2);
        let (idle_evicted, cache_evicted) = pool.enforce_quotas();
        assert_eq!(idle_evicted, 2);
        assert!(cache_evicted > 0);
        assert_eq!(pool.idle(), 0);
        assert!((pool.cache().approx_bytes() as u64) <= full / 2);
    }
}
