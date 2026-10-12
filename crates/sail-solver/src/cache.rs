//! Begrenzter LRU-Memo-Cache mit TTL und Hit/Miss-Zaehlern.
//!
//! Ersatz fuer OCaml `-memo_z3` (unbegrenzt): **Bounded-LRU ist Pflicht** —
//! jeder Cache hat `capacity >= 1` (kleinere Werte werden auf 1 geklemmt),
//! bei vollem Cache greift LRU-Eviction, Eintraege altern per TTL aus.
//! Defaults bewusst klein (128 Eintraege, 5 Minuten). Der Cache ist
//! `Send + Sync` und wird poolweit von allen Threads geteilt; die teuren
//! Z3-Solver bleiben pro Thread exklusiv ([`crate::SolverPool`]).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::query::SolverQuery;
use crate::quota::QuotaRecommendation;
use crate::solver::{Solver, SolverVerdict};

/// Konfiguration des Memo-Caches.
#[derive(Debug, Clone)]
pub struct SolverCacheConfig {
    /// Max. Eintraege (LRU-Eviction). Mindestens 1.
    pub capacity: usize,
    /// Lebensdauer eines Eintrags ab dem Einfuellen.
    pub ttl: Duration,
}

impl SolverCacheConfig {
    /// Winzige Defaults: 128 Eintraege, 5 Minuten TTL.
    pub fn small() -> Self {
        SolverCacheConfig {
            capacity: 128,
            ttl: Duration::from_secs(300),
        }
    }

    /// Normalisiert (`capacity >= 1`).
    fn normalized(&self) -> (usize, Duration) {
        (self.capacity.max(1), self.ttl)
    }
}

impl Default for SolverCacheConfig {
    fn default() -> Self {
        SolverCacheConfig::small()
    }
}

#[derive(Debug, Clone)]
struct Entry {
    verdict: SolverVerdict,
    inserted: Instant,
}

/// Momentaufnahme der Cache-Metriken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub len: usize,
    pub capacity: usize,
    /// Anzahl seit Cache-Erzeugung notierter SMT-Rundläufe (Lane 15:
    /// kanonische Prover-Zähler; `MemoSolver::prove` und
    /// [`crate::SolverPool::prove_cached`] notieren hier jede echte
    /// Solver-Anfrage inkl. Wandzeit).
    pub smt_calls: u64,
    /// Summierte SMT-Wandzeit in Sekunden (nur Misses mit Solver-Kontakt).
    pub smt_secs: f64,
}

impl CacheStats {
    /// Trefferquote in `[0.0, 1.0]` (`0.0` ohne Anfragen).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

#[derive(Debug)]
struct Inner {
    capacity: usize,
    ttl: Duration,
    map: HashMap<String, Entry>,
    /// LRU-Reihenfolge: vorne aelteste, hinten juengste Nutzung.
    order: VecDeque<String>,
}

impl Inner {
    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.to_owned());
    }

    fn remove(&mut self, key: &str) {
        self.map.remove(key);
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
    }

    fn evict_if_full(&mut self) {
        while self.map.len() >= self.capacity {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.map.remove(&oldest);
                }
                None => break,
            }
        }
    }

    fn get_valid(&mut self, key: &str, now: Instant) -> Option<SolverVerdict> {
        let entry = self.map.get(key)?;
        let expired = now.duration_since(entry.inserted) > self.ttl;
        if expired {
            self.remove(key);
            return None;
        }
        let verdict = self.map.get(key).map(|e| e.verdict);
        if verdict.is_some() {
            self.touch(key);
        }
        verdict
    }
}

/// Threadteilbarer, begrenzter Memo-Cache.
///
/// Zaehler sind atomar (`hits`/`misses`), Eintraege hinter einem Mutex.
/// Klone teilen sich Zustand via interner `Arc`-aehnlicher Struktur nicht;
/// stattdessen `std::sync::Arc<MemoCache>` aussen verwenden
/// (siehe [`MemoSolver`] / [`crate::SolverPool`]).
#[derive(Debug)]
pub struct MemoCache {
    inner: Mutex<Inner>,
    hits: AtomicU64,
    misses: AtomicU64,
    /// Kanonische Prover-Zähler (Lane 15): jede echte Solver-Anfrage
    /// (Cache-Miss mit Solver-Kontakt) wird mit Wandzeit notiert.
    /// Auslesbar via [`MemoCache::smt_totals`] / [`MemoCache::stats`];
    /// der CLI-Profiler rechnet daraus Deltas pro Phase.
    smt_calls: AtomicU64,
    smt_nanos: AtomicU64,
    /// Adaptive Byte-Quote (Lane 15): `u64::MAX` = aus. Sonst wirft
    /// [`MemoCache::enforce_byte_quota`] jeweils die ältesten Einträge,
    /// bis [`MemoCache::approx_bytes`] wieder im Budget liegt.
    max_bytes: AtomicU64,
}

impl MemoCache {
    pub fn new(config: SolverCacheConfig) -> Self {
        let (capacity, ttl) = config.normalized();
        MemoCache {
            inner: Mutex::new(Inner {
                capacity,
                ttl,
                map: HashMap::new(),
                order: VecDeque::new(),
            }),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            smt_calls: AtomicU64::new(0),
            smt_nanos: AtomicU64::new(0),
            max_bytes: AtomicU64::new(u64::MAX),
        }
    }

    /// Notiert einen SMT-Rundlauf mit Wandzeit (kanonischer Prover-Zähler).
    /// Rufer: jede Stelle, die nach einem Cache-Miss wirklich den Solver
    /// befragt ([`MemoSolver::prove`], [`crate::SolverPool::prove_cached`]).
    pub fn note_smt_call(&self, elapsed: Duration) {
        self.smt_calls.fetch_add(1, Ordering::Relaxed);
        self.smt_nanos.fetch_add(
            elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }

    /// `(Anzahl SMT-Rundläufe, summierte SMT-Wandzeit in Sekunden)`.
    pub fn smt_totals(&self) -> (u64, f64) {
        (
            self.smt_calls.load(Ordering::Relaxed),
            self.smt_nanos.load(Ordering::Relaxed) as f64 / 1_000_000_000.0,
        )
    }

    /// Lookup; zaehlt Hit/Miss. Abgelaufene Eintraege zaehlen als Miss.
    pub fn lookup(&self, query: &SolverQuery) -> Option<SolverVerdict> {
        let key = query.cache_key();
        let now = Instant::now();
        let hit = self
            .inner
            .lock()
            .expect("solver memo lock poisoned")
            .get_valid(&key, now);
        match hit {
            Some(v) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(v)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Einfuellen (mit LRU-Eviction bei vollem Cache).
    pub fn store(&self, query: &SolverQuery, verdict: SolverVerdict) {
        let key = query.cache_key();
        let mut inner = self.inner.lock().expect("solver memo lock poisoned");
        if inner.map.contains_key(&key) {
            if let Some(e) = inner.map.get_mut(&key) {
                e.verdict = verdict;
                e.inserted = Instant::now();
            }
            inner.touch(&key);
            return;
        }
        inner.evict_if_full();
        inner.map.insert(
            key.clone(),
            Entry {
                verdict,
                inserted: Instant::now(),
            },
        );
        inner.order.push_back(key);
    }

    /// Abgelaufene Eintraege entfernen (RAM-Regel, z. B. periodisch rufen).
    pub fn evict_expired(&self) -> usize {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("solver memo lock poisoned");
        let ttl = inner.ttl;
        let expired: Vec<String> = inner
            .map
            .iter()
            .filter(|(_, e)| now.duration_since(e.inserted) > ttl)
            .map(|(k, _)| k.clone())
            .collect();
        let n = expired.len();
        for k in expired {
            inner.remove(&k);
        }
        n
    }

    /// Leeren (z. B. vor RAM-Messungen in Lane 14).
    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("solver memo lock poisoned");
        inner.map.clear();
        inner.order.clear();
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().expect("solver memo lock poisoned");
        let (smt_calls, smt_secs) = self.smt_totals();
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            len: inner.map.len(),
            capacity: inner.capacity,
            smt_calls,
            smt_secs,
        }
    }

    /// Setzt die adaptive Byte-Quote (`u64::MAX` = aus, Default).
    /// Die Quote begrenzt [`MemoCache::approx_bytes`]; Durchsetzung per
    /// [`MemoCache::enforce_byte_quota`] (expliziter Drop-Punkt, kein
    /// implizites Räumen im Lookup-Pfad).
    pub fn set_max_bytes(&self, max_bytes: u64) {
        self.max_bytes.store(max_bytes, Ordering::Relaxed);
    }

    /// Uebernimmt das Byte-Budget aus einer [`QuotaRecommendation`]
    /// (Lane 36): `max_cache_bytes` -> `set_max_bytes`. Keine eigene
    /// Eintragszahl-Arithmetik — dafuer [`MemoCache::entries_for_byte_budget`].
    pub fn apply_recommendation(&self, rec: &QuotaRecommendation) {
        self.set_max_bytes(rec.max_cache_bytes);
    }

    /// Aktuelle Byte-Quote (`u64::MAX` = aus).
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes.load(Ordering::Relaxed)
    }

    /// Wirft die jeweils ältesten Einträge, bis die Quote wieder hält
    /// oder der Cache leer ist. Gibt die Anzahl entfernter Einträge zurück.
    ///
    /// Periodischer Drop-Punkt (Lane 36): explizit rufen (z. B. via
    /// [`crate::SolverPool::enforce_quotas`]); der Lookup-Pfad raeumt
    /// bewusst nicht implizit (Lane-15-Semantik bleibt).
    pub fn enforce_byte_quota(&self) -> usize {
        let max = self.max_bytes.load(Ordering::Relaxed);
        if max == u64::MAX {
            return 0;
        }
        let mut evicted = 0;
        while (self.approx_bytes() as u64) > max {
            if self.evict_oldest(1) == 0 {
                break;
            }
            evicted += 1;
        }
        evicted
    }

    /// Periodischer Quota-Drop-Punkt (Lane 36): Alias fuer
    /// [`MemoCache::enforce_byte_quota`] zum expliziten periodischen Rufen
    /// (kein implizites Raeumen im Lookup-Pfad).
    pub fn enforce_quota(&self) -> usize {
        self.enforce_byte_quota()
    }

    /// Entfernt bis zu `n` älteste Einträge (LRU-Front). Gibt die Anzahl
    /// tatsächlich entfernter Einträge zurück.
    ///
    /// Speisung per [`QuotaRecommendation`]: Budget via
    /// [`MemoCache::apply_recommendation`] uebernehmen, dann diesen
    /// Drop-Punkt periodisch rufen (kein Lookup-Raeumen).
    pub fn evict_oldest(&self, n: usize) -> usize {
        let mut inner = self.inner.lock().expect("solver memo lock poisoned");
        let mut removed = 0;
        for _ in 0..n {
            match inner.order.pop_front() {
                Some(oldest) => {
                    inner.map.remove(&oldest);
                    removed += 1;
                }
                None => break,
            }
        }
        removed
    }

    /// Kapazitätsvorschlag für ein Byte-Budget: so viele Einträge passen
    /// bei `bytes_per_entry` Bytes pro Eintrag hinein (mindestens 1).
    /// Reine Rechenhilfe für die adaptive Quote, ohne Zustand.
    pub fn entries_for_byte_budget(budget_bytes: usize, bytes_per_entry: usize) -> usize {
        (budget_bytes / bytes_per_entry.max(1)).max(1)
    }

    /// Grobe Speicherabschaetzung (Eintraege + Keys), fuer Lane-14-Messungen.
    pub fn approx_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("solver memo lock poisoned");
        let keys: usize = inner.map.keys().map(|k| k.len()).sum();
        keys + inner.map.len() * (std::mem::size_of::<Entry>() + 32)
    }
}

/// Solver-Huelle mit Memo-Cache: erst Lookup, bei Miss delegieren + fuellen.
pub struct MemoSolver<S> {
    solver: S,
    cache: MemoCache,
}

impl<S: Solver> MemoSolver<S> {
    pub fn new(solver: S, config: SolverCacheConfig) -> Self {
        MemoSolver {
            solver,
            cache: MemoCache::new(config),
        }
    }

    pub fn prove(&self, query: &SolverQuery) -> SolverVerdict {
        if let Some(v) = self.cache.lookup(query) {
            return v;
        }
        let start = Instant::now();
        let v = self.solver.prove(query);
        self.cache.note_smt_call(start.elapsed());
        self.cache.store(query, v);
        v
    }

    pub fn stats(&self) -> CacheStats {
        self.cache.stats()
    }

    pub fn cache(&self) -> &MemoCache {
        &self.cache
    }

    pub fn into_inner(self) -> S {
        self.solver
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::Constraint;

    fn q(s: &str) -> SolverQuery {
        SolverQuery::new(Constraint::atom(s))
    }

    #[test]
    fn hit_miss_counters() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 8,
            ttl: Duration::from_secs(60),
        });
        assert!(cache.lookup(&q("a")).is_none());
        cache.store(&q("a"), SolverVerdict::Valid);
        assert_eq!(cache.lookup(&q("a")), Some(SolverVerdict::Valid));
        assert_eq!(cache.lookup(&q("b")), None);
        let s = cache.stats();
        assert_eq!((s.hits, s.misses, s.len), (1, 2, 1));
        assert!((s.hit_rate() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn lru_eviction_bounds_memory() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 3,
            ttl: Duration::from_secs(60),
        });
        for i in 0..5 {
            cache.store(&q(&format!("k{i}")), SolverVerdict::Unknown);
        }
        let s = cache.stats();
        assert_eq!(s.len, 3);
        // Aelteste (k0, k1) verdrängt, juengste bleiben.
        assert!(cache.lookup(&q("k0")).is_none());
        assert!(cache.lookup(&q("k1")).is_none());
        assert!(cache.lookup(&q("k4")).is_some());
    }

    #[test]
    fn ttl_expiry_counts_as_miss() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 8,
            ttl: Duration::from_millis(20),
        });
        cache.store(&q("t"), SolverVerdict::Valid);
        assert_eq!(cache.lookup(&q("t")), Some(SolverVerdict::Valid));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(cache.lookup(&q("t")), None);
        let s = cache.stats();
        assert_eq!((s.hits, s.misses), (1, 1));
    }

    #[test]
    fn capacity_zero_clamped_to_one() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 0,
            ttl: Duration::from_secs(60),
        });
        cache.store(&q("x"), SolverVerdict::Valid);
        assert_eq!(cache.stats().len, 1);
    }

    #[test]
    fn smt_counters_start_at_zero_and_note_records() {
        use crate::stub::StubSolver;
        let solver = MemoSolver::new(
            StubSolver::new(),
            SolverCacheConfig {
                capacity: 8,
                ttl: Duration::from_secs(60),
            },
        );
        assert_eq!(solver.cache().smt_totals().0, 0);
        // Miss mit Solver-Kontakt zaehlt genau einen SMT-Rundlauf.
        solver.prove(&q("smt-a"));
        // Treffer zaehlt keinen weiteren Rundlauf.
        solver.prove(&q("smt-a"));
        let (calls, secs) = solver.cache().smt_totals();
        assert_eq!(calls, 1);
        assert!(secs >= 0.0);
        let s = solver.stats();
        assert_eq!(s.smt_calls, 1);
        assert!((s.smt_secs - secs).abs() < 1e-9);
        assert_eq!((s.hits, s.misses), (1, 1));
    }

    #[test]
    fn byte_quota_evicts_oldest_first() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 16,
            ttl: Duration::from_secs(60),
        });
        for i in 0..6 {
            cache.store(&q(&format!("quota-{i}")), SolverVerdict::Unknown);
        }
        let full = cache.approx_bytes();
        assert!(full > 0);
        // Quote unter aktuellem Stand: aelteste muessen weichen.
        cache.set_max_bytes((full / 2) as u64);
        assert_eq!(cache.max_bytes(), (full / 2) as u64);
        let evicted = cache.enforce_byte_quota();
        assert!(evicted > 0);
        assert!((cache.approx_bytes() as u64) <= (full / 2) as u64);
        // Juengste Eintraege bleiben bevorrechtigt.
        assert!(cache.lookup(&q("quota-5")).is_some());
        // Quote aus: kein weiterer Evict.
        cache.set_max_bytes(u64::MAX);
        assert_eq!(cache.enforce_byte_quota(), 0);
    }

    #[test]
    fn entries_for_byte_budget_clamps_to_one() {
        assert_eq!(MemoCache::entries_for_byte_budget(10_000, 100), 100);
        assert_eq!(MemoCache::entries_for_byte_budget(50, 100), 1);
        assert_eq!(MemoCache::entries_for_byte_budget(1_000, 0), 1_000);
    }

    #[test]
    fn apply_recommendation_sets_byte_budget() {
        use crate::quota::QuotaRecommendation;
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 16,
            ttl: Duration::from_secs(60),
        });
        let rec = QuotaRecommendation::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        cache.apply_recommendation(&rec);
        assert_eq!(cache.max_bytes(), rec.max_cache_bytes);
        // Empfehlung nutzt die geteilte Eintragszahl-Math, kein Duplikat:
        // Budget passt fuer mindestens einen Eintrag je 1 KiB.
        assert!(MemoCache::entries_for_byte_budget(rec.max_cache_bytes as usize, 1024) >= 1);
        assert_eq!(cache.enforce_quota(), 0);
    }

    #[test]
    fn lookup_does_not_implicitly_enforce_byte_quota() {
        let cache = MemoCache::new(SolverCacheConfig {
            capacity: 16,
            ttl: Duration::from_secs(60),
        });
        for i in 0..6 {
            cache.store(&q(&format!("implicit-{i}")), SolverVerdict::Unknown);
        }
        let len_before = cache.stats().len;
        assert_eq!(len_before, 6);
        // Winzige Quote setzen: Lookup (Hit wie Miss) darf NICHT raeumen.
        cache.set_max_bytes(1);
        assert!(cache.lookup(&q("implicit-5")).is_some());
        assert!(cache.lookup(&q("missing")).is_none());
        assert_eq!(cache.stats().len, len_before);
        // Erst der periodische Drop-Punkt raeumt.
        assert!(cache.enforce_quota() > 0);
        assert!((cache.approx_bytes() as u64) <= 1);
    }
}
