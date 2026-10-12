//! Lane 15 — adaptive Speicherquote für Solver-Pools und Memo-Caches.
//!
//! Reine Rechenhilfe ohne Zustand/Abhängigkeiten: Aus einem cgroup-Limit
//! (`MemoryMax`) und einem gemessenen Peak leitet sie Idle-Solver-Quota,
//! Cache-Byte-Quote und Worker-Empfehlung ab. Die Durchsetzung liegt bei
//! `sail-solver` (`SolverPool::set_max_idle`/`evict_idle`,
//! `MemoCache::set_max_bytes`/`enforce_byte_quota`); dieses Modul liefert
//! nur die Zahlen, damit Treiber-Lanes dieselbe Policy teilen.

#![forbid(unsafe_code)]

/// RAM-Kostenannahmen pro Solver-Instanz (Z3-Kontexte sind speicherteuer).
/// Konservativ hoch angesetzt, damit die Quote auf kleinen Cgroups greift.
pub const BYTES_PER_IDLE_SOLVER: u64 = 32 * 1024 * 1024;

/// Max. ruhende Solver, die eine Quote je vorhält (mehr lohnt nicht:
/// aktive Last trägt der Checkout, nicht der Parkplatz).
pub const MAX_IDLE_SOLVERS: usize = 16;

/// Adaptive Quota-Empfehlung aus Limit und gemessenem Peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaAdvice {
    /// Max. ruhende Solver (`SolverPool::set_max_idle`).
    pub max_idle_solvers: usize,
    /// Max. Memo-Cache-Bytes (`MemoCache::set_max_bytes`).
    pub max_cache_bytes: u64,
    /// Empfohlene Workerzahl (skaliert mit Kopfraum, mindestens 1).
    pub workers: usize,
}

impl QuotaAdvice {
    /// Leitet die Quota aus `limit_bytes` (cgroup `MemoryMax`) und dem
    /// gemessenen `peak_bytes` ab:
    /// - Solver-Anteil = Limit/4; davon zuerst bis zu [`MAX_IDLE_SOLVERS`]
    ///   Idle-Solver à 32 MiB, der Rest als Cache-Bytes.
    /// - Worker: 1 + Kopfraum-Skalierung (pro weitere 512 MiB ein Worker,
    ///   gedeckelt auf `max_workers`), damit der OOC-Scheduler unter
    ///   Druck nicht mehr Threads als RAM füttert.
    pub fn recommend(limit_bytes: u64, peak_bytes: u64, max_workers: usize) -> Self {
        let solver_budget = limit_bytes / 4;
        let idle_wanted = solver_budget / BYTES_PER_IDLE_SOLVER.max(1);
        let max_idle = (idle_wanted.min(MAX_IDLE_SOLVERS as u64).max(1)) as usize;
        let reserved = max_idle as u64 * BYTES_PER_IDLE_SOLVER;
        let max_cache_bytes = solver_budget.saturating_sub(reserved).max(1 << 20);
        let headroom = limit_bytes.saturating_sub(peak_bytes);
        let workers = (1 + headroom / (512 * 1024 * 1024))
            .min(max_workers.max(1) as u64)
            .max(1) as usize;
        Self {
            max_idle_solvers: max_idle.max(1),
            max_cache_bytes,
            workers,
        }
    }

    /// Passt der gemessene Peak ins Limit?
    pub fn peak_fits(limit_bytes: u64, peak_bytes: u64) -> bool {
        peak_bytes <= limit_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_cgroup_yields_tight_quota() {
        // 128-MiB-Teilmenge: Solver-Anteil 32 MiB -> 1 Idle-Solver, Rest Cache.
        let a = QuotaAdvice::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        assert_eq!(a.max_idle_solvers, 1);
        assert!(a.max_cache_bytes >= 1 << 20);
        assert!(a.workers >= 1 && a.workers <= 16);
    }

    #[test]
    fn large_cgroup_allows_more_idle_and_workers() {
        let small = QuotaAdvice::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        let big = QuotaAdvice::recommend(4 * 1024 * 1024 * 1024, 1024 * 1024 * 1024, 16);
        assert!(big.max_idle_solvers > small.max_idle_solvers);
        assert!(big.max_cache_bytes > small.max_cache_bytes);
        assert!(big.workers >= small.workers);
    }

    #[test]
    fn exhausted_headroom_clamps_workers_to_one() {
        let a = QuotaAdvice::recommend(128 * 1024 * 1024, 128 * 1024 * 1024, 16);
        assert_eq!(a.workers, 1);
        assert!(QuotaAdvice::peak_fits(128, 128));
        assert!(!QuotaAdvice::peak_fits(128, 129));
    }
}
