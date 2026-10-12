//! Z3-Quote (Audit O-9): begrenzte Permit-Vergabe + adaptive Speicherquote.
//!
//! Befund O-9: [`crate::SolverPool`] (Basisstand) erzeugt per Checkout
//! unbegrenzt Solver-Instanzen (`factory()` bei leerem `free`-Vec) und der
//! [`crate::MemoCache`] war nur per Eintragszahl/TTL begrenzt — kein Limit
//! auf gleichzeitig lebende Z3-Kontexte (speicherteuer) und kein Byte-Budget.
//!
//! Dieses Modul liefert die **Vergabe- und Empfehlungsseite** der Quote,
//! bewusst ohne `pool.rs`/`cache.rs` anzufassen (Lane 15 besitzt beide):
//!
//! - [`SolverQuota`]: zaehlende Permit-Semaphore (Mutex + Condvar, std-only,
//!   `Send + Sync`). `try_acquire` / `acquire` / `acquire_timeout` vergeben
//!   maximal `max_live` gleichzeitige Permits; [`QuotaPermit`] (RAII-Guard)
//!   gibt per `Drop` zurueck. `set_max_live` schrumpft/wachst adaptiv
//!   (In-Flight-Permits laufen natuerlich aus, kein Abbruch).
//! - [`recommend_quota`]: reine Rechenhilfe ohne Zustand — aus cgroup-Limit
//!   (`limit_bytes`) und gemessenem Peak (`peak_bytes`) folgen
//!   Live-Permit-Zahl, Idle-Parkplatz und Cache-Byte-Budget. Die
//!   Durchsetzung bleibt bei Lane-15-Primitiven (siehe Skizze unten).
//!
//! # Anbindungsskizze (NACH dem Lane-15-Merge; `pool.rs`/`cache.rs` lesen)
//!
//! Zeilennummern beziehen sich auf den Lane-15-Stand (Files unveraendert
//! aus Lane-23-Sicht; Merge ist sauber, da Lane 23 beide Files nicht
//! anfasst):
//!
//! 1. `crates/sail-solver/src/pool.rs` (Lane 15):
//!    - Feldablage: `SolverPool`-Struct Zeile 19, Felder `max_idle` Zeile 26
//!      und `created` Zeile 29 — daneben `quota: Arc<SolverQuota>`
//!      (dieses Modul) ergaenzen; Konstruktor `new` Zeilen 42–43 analog
//!      initialisieren.
//!    - `checkout` Zeilen 53–70: vor der `factory()`-Erzeugung
//!      (`created.fetch_add` Zeile 60) ein `QuotaPermit` anfordern
//!      (`try_acquire` fuer fail-fast oder `acquire_timeout` mit der
//!      `QuotaConfig::acquire_timeout`); bei Verweigerung `Unknown`
//!      zurueckgeben statt neuen Solver zu bauen. Den Permit im
//!      `PooledSolver` (Struct Zeile 136, Felder Zeile 139, `Drop`
//!      Quota-Zweig Zeile 159) mitfuehren, damit der Permit erst mit der
//!      Solver-Rueckgabe faellt.
//!    - `prove_cached` Zeilen 76–82: Permit um den exklusiven Checkout
//!      legen; `note_smt_call` Zeile 82 bleibt unveraendert.
//!    - Idle-Quote (`set_max_idle` Zeile 104, `evict_idle` Zeile 115)
//!      mit `QuotaRecommendation::max_idle` aus [`recommend_quota`]
//!      speisen (adaptiver Drop-Punkt, keine neue Logik noetig).
//! 2. `crates/sail-solver/src/cache.rs` (Lane 15):
//!    - `set_max_bytes` Zeile 278 / `enforce_byte_quota` Zeile 289 /
//!      `evict_oldest` Zeile 306: Byte-Budget aus
//!      `QuotaRecommendation::max_cache_bytes` uebernehmen und
//!      `enforce_byte_quota` als periodischen Drop-Punkt rufen (kein
//!      implizites Raeumen im Lookup-Pfad — Lane-15-Semantik bleibt).
//!    - Eintragszahl-Math (`entries_for_byte_budget` Zeile 324) NICHT
//!      duplizieren — dieses Modul verweist darauf statt eigene
//!      Budget-Arithmetik zu pflegen.
//! 3. `crates/sail-solver/src/lib.rs`: `pub mod quota;` plus Re-Export
//!    (einzige Verdrahtung; `pool.rs`/`cache.rs` bleiben unberuehrt).
//!
//! Abgrenzung zu Lane 15 (`sail-perf/src/quota.rs`, `QuotaAdvice`):
//! jene Policy lebt im Perf-Treiber und kennt Worker-Skalierung; diese
//! hier lebt im Solver (`sail-solver`, keine neuen Deps) und kennt zusaetzlich
//! die blockierende Permit-Vergabe mit Timeout. Beide teilen die
//! 32-MiB-Annahme pro Solver ([`BYTES_PER_SOLVER`]); bei Aenderung beide
//! Stellen synchron halten.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Condvar, Mutex,
};
use std::time::{Duration, Instant};

/// RAM-Kostenannahme pro lebender Solver-Instanz (Z3-Kontexte sind
/// speicherteuer). Konservativ hoch, damit die Quote auf kleinen Cgroups
/// greift. Spiegelt `sail-perf::quota::BYTES_PER_IDLE_SOLVER` (Lane 15);
/// bei Aenderung beide Stellen synchron halten.
pub const BYTES_PER_SOLVER: u64 = 32 * 1024 * 1024;

/// Deckel fuer gleichzeitig lebende Solver (mehr lohnt nicht: aktive Last
/// traegt der Checkout, nicht der Vorrat).
pub const MAX_LIVE_SOLVERS: usize = 16;

/// Untergrenze fuer das Cache-Byte-Budget, damit die Quote den Cache nie
/// auf null dreht.
pub const MIN_CACHE_BYTES: u64 = 1 << 20;

/// Konfiguration der Permit-Vergabe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaConfig {
    /// Max. gleichzeitig lebende Permits (`>= 1`, kleinere Werte klemmen).
    pub max_live: usize,
    /// Default-Timeout fuer [`SolverQuota::acquire_timeout`]-Aufrufe,
    /// die keinen expliziten Timeout mitgeben.
    pub acquire_timeout: Duration,
}

impl QuotaConfig {
    /// Kleine, sichere Defaults: 4 Live-Permits, 5 s Vergabe-Timeout.
    pub fn small() -> Self {
        QuotaConfig {
            max_live: 4,
            acquire_timeout: Duration::from_secs(5),
        }
    }

    /// Unbegrenzt (Verhalten wie ohne Quote; nur als Migrationshilfe).
    pub fn unlimited() -> Self {
        QuotaConfig {
            max_live: usize::MAX,
            acquire_timeout: Duration::from_secs(5),
        }
    }

    /// Normalisiert (`max_live >= 1`).
    fn normalized(self) -> Self {
        QuotaConfig {
            max_live: self.max_live.max(1),
            acquire_timeout: self.acquire_timeout,
        }
    }
}

impl Default for QuotaConfig {
    fn default() -> Self {
        QuotaConfig::small()
    }
}

#[derive(Debug)]
struct Inner {
    max_live: usize,
    live: usize,
}

/// Momentaufnahme der Quota-Metriken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaStats {
    /// Aktuell vergebene Permits.
    pub live: usize,
    /// Aktuelles Live-Limit.
    pub max_live: usize,
    /// Je vergebene Permits (kumulativ).
    pub acquired_total: u64,
    /// Abgelehnte `try_acquire`-Versuche (kumulativ).
    pub refused_total: u64,
    /// Zeitueberschreitungen bei `acquire_timeout` (kumulativ).
    pub timed_out_total: u64,
}

/// Zaehlende Permit-Vergabe fuer Solver-Instanzen.
///
/// Begrenzt die Zahl **gleichzeitig lebender** Solver (Lane-15-Luecke:
/// `max_idle` deckelt nur geparkte, nicht aktive Instanzen). Blockierendes
/// Warten via Condvar; `Send + Sync` und damit poolweit teilbar. Fehlende
/// Lock-Poison-Toleranz wie im Rest des Crates: vergiftetes Lock = Bug,
/// daher `expect` statt Stillhalte-Fallback.
#[derive(Debug)]
pub struct SolverQuota {
    inner: Mutex<Inner>,
    cvar: Condvar,
    acquired: AtomicU64,
    refused: AtomicU64,
    timed_out: AtomicU64,
}

impl SolverQuota {
    /// Neue Quote aus `config` (`max_live` wird auf `>= 1` geklemmt).
    pub fn new(config: QuotaConfig) -> Self {
        let cfg = config.normalized();
        SolverQuota {
            inner: Mutex::new(Inner {
                max_live: cfg.max_live,
                live: 0,
            }),
            cvar: Condvar::new(),
            acquired: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            timed_out: AtomicU64::new(0),
        }
    }

    /// Quote mit Default-Konfiguration ([`QuotaConfig::small`]).
    pub fn with_defaults() -> Self {
        Self::new(QuotaConfig::small())
    }

    /// Nicht-blockierende Vergabe: `Some(Permit)` bei freiem Slot,
    /// sonst `None` (zaehlt `refused_total`).
    pub fn try_acquire(&self) -> Option<QuotaPermit<'_>> {
        let mut inner = self.inner.lock().expect("solver quota lock poisoned");
        if inner.live < inner.max_live {
            inner.live += 1;
            self.acquired.fetch_add(1, Ordering::Relaxed);
            Some(QuotaPermit { quota: self })
        } else {
            self.refused.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Blockierende Vergabe (ohne Timeout). Bricht nur per Permit-Rueckgabe
    /// anderer Threads auf — mit `max_live >= 1` (Konstruktor klemmt) gibt
    /// es kein Selbst-Deadlock durch ein Null-Limit.
    pub fn acquire(&self) -> QuotaPermit<'_> {
        let mut inner = self.inner.lock().expect("solver quota lock poisoned");
        while inner.live >= inner.max_live {
            inner = self.cvar.wait(inner).expect("solver quota lock poisoned");
        }
        inner.live += 1;
        self.acquired.fetch_add(1, Ordering::Relaxed);
        QuotaPermit { quota: self }
    }

    /// Blockierende Vergabe mit Timeout: `None` nach Ablauf (zaehlt
    /// `timed_out_total`), `Some(Permit)` bei Erfolg.
    pub fn acquire_timeout(&self, timeout: Duration) -> Option<QuotaPermit<'_>> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().expect("solver quota lock poisoned");
        while inner.live >= inner.max_live {
            let now = Instant::now();
            if now >= deadline {
                self.timed_out.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            let (guard, res) = self
                .cvar
                .wait_timeout(inner, deadline - now)
                .expect("solver quota lock poisoned");
            inner = guard;
            if res.timed_out() && inner.live >= inner.max_live {
                self.timed_out.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        }
        inner.live += 1;
        self.acquired.fetch_add(1, Ordering::Relaxed);
        Some(QuotaPermit { quota: self })
    }

    /// Aktuell vergebene Permits.
    pub fn live(&self) -> usize {
        self.inner.lock().expect("solver quota lock poisoned").live
    }

    /// Aktuelles Live-Limit.
    pub fn max_live(&self) -> usize {
        self.inner.lock().expect("solver quota lock poisoned").max_live
    }

    /// Freie Slots (`max_live - live`, saturierend).
    pub fn available(&self) -> usize {
        let inner = self.inner.lock().expect("solver quota lock poisoned");
        inner.max_live.saturating_sub(inner.live)
    }

    /// Passt das Live-Limit adaptiv an (`0` klemmt auf `1`, damit
    /// `acquire` nicht dauerhaft blockiert). Schrumpfen wirkt nur auf
    /// kuenftige Vergaben — In-Flight-Permits laufen natuerlich aus.
    /// Wachsen weckt blockierte `acquire`-Rufer.
    pub fn set_max_live(&self, max_live: usize) {
        let clamped = max_live.max(1);
        let mut inner = self.inner.lock().expect("solver quota lock poisoned");
        inner.max_live = clamped;
        if inner.live < clamped {
            self.cvar.notify_all();
        }
    }

    /// Metriken-Momentaufnahme.
    pub fn stats(&self) -> QuotaStats {
        let inner = self.inner.lock().expect("solver quota lock poisoned");
        QuotaStats {
            live: inner.live,
            max_live: inner.max_live,
            acquired_total: self.acquired.load(Ordering::Relaxed),
            refused_total: self.refused.load(Ordering::Relaxed),
            timed_out_total: self.timed_out.load(Ordering::Relaxed),
        }
    }

    fn release(&self) {
        let mut inner = self.inner.lock().expect("solver quota lock poisoned");
        debug_assert!(inner.live > 0, "quota permit released without acquire");
        inner.live = inner.live.saturating_sub(1);
        drop(inner);
        self.cvar.notify_one();
    }
}

/// Exklusiver Permit-Guard; gibt den Slot bei `Drop` zurueck und weckt
/// genau einen blockierten `acquire`-Rufer.
#[derive(Debug)]
pub struct QuotaPermit<'a> {
    quota: &'a SolverQuota,
}

impl Drop for QuotaPermit<'_> {
    fn drop(&mut self) {
        self.quota.release();
    }
}

/// Adaptive Quota-Empfehlung aus Limit und gemessenem Peak (reine
/// Rechenhilfe ohne Zustand; Durchsetzung bei Lane-15-Primitiven,
/// siehe Modul-Doku).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaRecommendation {
    /// Max. gleichzeitig lebende Solver (`SolverQuota::set_max_live`).
    pub max_live: usize,
    /// Max. ruhende Solver (`SolverPool::set_max_idle`, Lane 15).
    pub max_idle: usize,
    /// Max. Memo-Cache-Bytes (`MemoCache::set_max_bytes`, Lane 15).
    pub max_cache_bytes: u64,
    /// Empfohlene Workerzahl (skaliert mit Kopfraum, mindestens 1).
    pub workers: usize,
}

impl QuotaRecommendation {
    /// Leitet die Quota aus `limit_bytes` (cgroup `MemoryMax`) und dem
    /// gemessenen `peak_bytes` ab:
    /// - Solver-Budget = Limit/4; davon zuerst bis zu [`MAX_LIVE_SOLVERS`]
    ///   Live-Permits a [`BYTES_PER_SOLVER`], der Rest als Cache-Bytes
    ///   (mindestens [`MIN_CACHE_BYTES`).
    /// - Idle-Parkplatz = Live-Permits (parken lohnt nur, was auch lebt).
    /// - Worker: 1 + Kopfraum-Skalierung (pro weitere 512 MiB ein Worker,
    ///   gedeckelt auf `max_workers`), damit der Scheduler unter Druck
    ///   nicht mehr Threads als RAM fuettert.
    pub fn recommend(
        limit_bytes: u64,
        peak_bytes: u64,
        max_workers: usize,
    ) -> Self {
        let solver_budget = limit_bytes / 4;
        let live_wanted = solver_budget / BYTES_PER_SOLVER.max(1);
        let max_live = (live_wanted.min(MAX_LIVE_SOLVERS as u64).max(1)) as usize;
        let reserved = max_live as u64 * BYTES_PER_SOLVER;
        let max_cache_bytes = solver_budget.saturating_sub(reserved).max(MIN_CACHE_BYTES);
        let headroom = limit_bytes.saturating_sub(peak_bytes);
        let workers = (1 + headroom / (512 * 1024 * 1024))
            .min(max_workers.max(1) as u64)
            .max(1) as usize;
        QuotaRecommendation {
            max_live,
            max_idle: max_live,
            max_cache_bytes,
            workers,
        }
    }

    /// Passt der gemessene Peak ins Limit?
    pub fn peak_fits(limit_bytes: u64, peak_bytes: u64) -> bool {
        peak_bytes <= limit_bytes
    }

    /// Uebernimmt die Empfehlung in eine lebende [`SolverQuota`]
    /// (reine Limit-Uebernahme; In-Flight-Permits laufen aus).
    pub fn apply_to(&self, quota: &SolverQuota) {
        quota.set_max_live(self.max_live);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn config_clamps_zero_to_one() {
        let q = SolverQuota::new(QuotaConfig {
            max_live: 0,
            acquire_timeout: Duration::from_secs(1),
        });
        assert_eq!(q.max_live(), 1);
        let _held = q.try_acquire().expect("single slot");
        assert!(q.try_acquire().is_none());
    }

    #[test]
    fn try_acquire_caps_live_and_returns_on_drop() {
        let q = SolverQuota::new(QuotaConfig {
            max_live: 2,
            acquire_timeout: Duration::from_secs(1),
        });
        let _a = q.try_acquire().expect("slot 1");
        assert_eq!((q.live(), q.available()), (1, 1));
        let _b = q.try_acquire().expect("slot 2");
        assert_eq!((q.live(), q.available()), (2, 0));
        assert!(q.try_acquire().is_none());
        let s = q.stats();
        assert_eq!((s.acquired_total, s.refused_total), (2, 1));
        drop(_b);
        assert_eq!((q.live(), q.available()), (1, 1));
        assert!(q.try_acquire().is_some());
    }

    #[test]
    fn blocking_acquire_wakes_on_release() {
        let q = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_secs(5),
        }));
        let _held = q.try_acquire().expect("initial permit");
        let q2 = Arc::clone(&q);
        let h = std::thread::spawn(move || {
            let _p = q2.acquire();
            assert_eq!(q2.live(), 1);
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(_held);
        h.join().expect("worker panicked");
        assert_eq!(q.live(), 0);
    }

    #[test]
    fn acquire_timeout_returns_none_on_exhaustion() {
        let q = SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_secs(1),
        });
        let _held = q.try_acquire().expect("initial permit");
        let start = Instant::now();
        assert!(q.acquire_timeout(Duration::from_millis(50)).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(q.stats().timed_out_total, 1);
        assert_eq!(q.live(), 1);
    }

    #[test]
    fn shrink_only_affects_future_acquisitions() {
        let q = SolverQuota::new(QuotaConfig {
            max_live: 3,
            acquire_timeout: Duration::from_secs(1),
        });
        let _a = q.try_acquire().unwrap();
        let _b = q.try_acquire().unwrap();
        q.set_max_live(1);
        assert_eq!(q.max_live(), 1);
        // In-Flight-Permits laufen weiter, Neuzugang wird abgewiesen.
        assert!(q.try_acquire().is_none());
        drop(_a);
        drop(_b);
        // Nach Drain greift das neue Limit wieder.
        assert!(q.try_acquire().is_some());
    }

    #[test]
    fn grow_wakes_blocked_acquirers() {
        let q = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 1,
            acquire_timeout: Duration::from_secs(5),
        }));
        let _held = q.try_acquire().expect("initial permit");
        let q2 = Arc::clone(&q);
        let h = std::thread::spawn(move || {
            let _p = q2.acquire();
            assert_eq!(q2.live(), 2);
        });
        std::thread::sleep(Duration::from_millis(50));
        q.set_max_live(2);
        h.join().expect("worker panicked");
        assert_eq!(q.live(), 1);
    }

    #[test]
    fn concurrent_permits_never_exceed_limit() {
        let q = Arc::new(SolverQuota::new(QuotaConfig {
            max_live: 3,
            acquire_timeout: Duration::from_secs(5),
        }));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..12 {
            let (q2, p2) = (Arc::clone(&q), Arc::clone(&peak));
            hs.push(std::thread::spawn(move || {
                let _permit = q2.acquire();
                let live = q2.live();
                p2.fetch_max(live, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(5));
            }));
        }
        for h in hs {
            h.join().expect("worker panicked");
        }
        assert!(peak.load(Ordering::Relaxed) <= 3);
        assert_eq!(q.live(), 0);
        assert_eq!(q.stats().acquired_total, 12);
    }

    #[test]
    fn recommend_small_cgroup_yields_tight_quota() {
        // 128-MiB-Cgroup: Solver-Budget 32 MiB -> genau 1 Live-Permit.
        let r = QuotaRecommendation::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        assert_eq!((r.max_live, r.max_idle), (1, 1));
        assert!(r.max_cache_bytes >= MIN_CACHE_BYTES);
        assert!((1..=16).contains(&r.workers));
        // Empfehlung laesst sich in eine lebende Quote uebernehmen.
        let q = SolverQuota::with_defaults();
        r.apply_to(&q);
        assert_eq!(q.max_live(), 1);
    }

    #[test]
    fn recommend_large_cgroup_allows_more() {
        let small = QuotaRecommendation::recommend(128 * 1024 * 1024, 100 * 1024 * 1024, 16);
        let big =
            QuotaRecommendation::recommend(4 * 1024 * 1024 * 1024, 1024 * 1024 * 1024, 16);
        assert!(big.max_live > small.max_live);
        assert!(big.max_cache_bytes > small.max_cache_bytes);
        assert!(big.workers >= small.workers);
    }

    #[test]
    fn recommend_exhausted_headroom_clamps_workers() {
        let r = QuotaRecommendation::recommend(128 * 1024 * 1024, 128 * 1024 * 1024, 16);
        assert_eq!(r.workers, 1);
        assert!(QuotaRecommendation::peak_fits(128, 128));
        assert!(!QuotaRecommendation::peak_fits(128, 129));
    }
}
