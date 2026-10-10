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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub len: usize,
    pub capacity: usize,
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
        }
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
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            len: inner.map.len(),
            capacity: inner.capacity,
        }
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
        let v = self.solver.prove(query);
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
}
