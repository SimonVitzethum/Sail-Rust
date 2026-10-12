//! `-dprofile`-Aequivalent als Timings-Log (Lane 38, angelehnt an Lane 14).
//!
//! Format (stderr, eine Zeile pro Phase, key=value, stabil parsbar):
//! ```text
//! [profile] phase="<name>" wall_s=<secs> rss_peak_mb=<mb> smt_calls=<n> smt_s=<secs>
//! ```
//! OCaml-Sail druckt stattdessen `Profiled <phase>: <s>s` (+ SMT-Zeile); das
//! obige Format ist das dokumentierte sail-rs-Aequivalent und meldet
//! zusaetzlich Peak-RSS pro Phase (Lane-14-Budget-Sicht).
#![forbid(unsafe_code)]

use std::time::Instant;

/// Eine Profiling-Phase mit Wandzeit, Peak-RSS und SMT-Zaehlern.
#[derive(Debug, Clone)]
pub struct PhaseSample {
    pub phase: &'static str,
    pub wall_s: f64,
    pub rss_peak_mb: u64,
    pub smt_calls: u64,
    pub smt_s: f64,
}

impl PhaseSample {
    pub fn format(&self) -> String {
        format!(
            "[profile] phase=\"{}\" wall_s={:.6} rss_peak_mb={} smt_calls={} smt_s={:.6}",
            self.phase, self.wall_s, self.rss_peak_mb, self.smt_calls, self.smt_s
        )
    }
}

/// Liest Peak-RSS (VmHWM) aus `/proc/self/status`. Reines `std`, kein
/// `unsafe`. Gibt `0` zurueck, wenn unlesbar (nicht-Linux).
pub fn peak_rss_mb() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in status.lines() {
        if line.starts_with("VmHWM:") {
            // Format: "VmHWM:   12345 kB"
            let kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0);
            return kb / 1024;
        }
    }
    0
}

/// Kanonische Prover-Zähler einer Phase (Lane 15).
///
/// Quelle sind die Pool-/Cache-Zähler aus `sail-solver`
/// (`SolverPool::smt_totals`, `MemoCache::smt_totals`): Der Aufrufer
/// snapshotet vor/nach der Phase und übergibt das Delta via
/// [`Profiler::phase_with_counters`]. Dadurch sind `smt_calls`/`smt_s`
/// im `[profile]`-Log echte Messwerte statt Nullen (TESTING.md:
/// SMT-Nullzähler waren bisher nicht verdrahtet).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProverCounters {
    /// Anzahl SMT-Rundläufe (Solver-Kontakt, Cache-Misses).
    pub calls: u64,
    /// Summierte SMT-Wandzeit in Sekunden.
    pub secs: f64,
}

impl ProverCounters {
    pub const ZERO: Self = Self {
        calls: 0,
        secs: 0.0,
    };

    pub fn new(calls: u64, secs: f64) -> Self {
        Self { calls, secs }
    }

    /// Sättigendes Delta `after - before` (Zähler sind monoton).
    pub fn diff(before: Self, after: Self) -> Self {
        Self {
            calls: after.calls.saturating_sub(before.calls),
            secs: (after.secs - before.secs).max(0.0),
        }
    }
}

/// Sammler fuer Phasen-Timings. `emit_stderr` druckt im obigen Format.
#[derive(Debug, Default)]
pub struct Profiler {
    samples: Vec<PhaseSample>,
    total_start: Option<Instant>,
}

impl Profiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_total(&mut self) {
        self.total_start = Some(Instant::now());
    }

    /// Misst `f` als Phase `phase` und speichert das Sample.
    ///
    /// Ohne Prover-Anbindung (keine Zähler übergeben): `smt_calls`/`smt_s`
    /// bleiben 0. Mit Anbindung [`Profiler::phase_with_counters`] nutzen.
    pub fn phase<T, F>(&mut self, phase: &'static str, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        self.phase_with_counters(phase, ProverCounters::ZERO, f)
    }

    /// Misst `f` als Phase `phase` mit Prover-Zähler-Delta.
    ///
    /// Verdrahtung (Call-Site, z. B. `typecheck`): Pool-Zähler vorher/nachher
    /// snapshoten (`pool.smt_totals()`), [`ProverCounters::diff`] bilden,
    /// hier übergeben. Wandzeit/RSS misst der Profiler selbst.
    pub fn phase_with_counters<T, F>(
        &mut self,
        phase: &'static str,
        counters: ProverCounters,
        f: F,
    ) -> T
    where
        F: FnOnce() -> T,
    {
        let start = Instant::now();
        let result = f();
        self.samples.push(PhaseSample {
            phase,
            wall_s: start.elapsed().as_secs_f64(),
            rss_peak_mb: peak_rss_mb(),
            smt_calls: counters.calls,
            smt_s: counters.secs,
        });
        result
    }

    /// Rechnet Prover-Zähler nachträglich auf das jüngste Sample an
    /// (für Worker-Aggregation, deren Pool-Stats erst nach Phasenende
    /// zusammengeführt werden). Trifft auf die jüngste Phase mit
    /// passendem Namen zu, sonst auf das jüngste Sample überhaupt.
    pub fn add_smt_to_last(&mut self, phase: &'static str, counters: ProverCounters) {
        let target = self
            .samples
            .iter()
            .rposition(|s| s.phase == phase)
            .or_else(|| self.samples.len().checked_sub(1));
        if let Some(i) = target {
            let s = &mut self.samples[i];
            s.smt_calls = s.smt_calls.saturating_add(counters.calls);
            s.smt_s += counters.secs;
        }
    }

    /// Simulierte Phase (fuer `--dry-run`: Format bleibt testbar, ohne Arbeit).
    pub fn simulated(&mut self, phase: &'static str) {
        self.samples.push(PhaseSample {
            phase,
            wall_s: 0.0,
            rss_peak_mb: peak_rss_mb(),
            smt_calls: 0,
            smt_s: 0.0,
        });
    }

    pub fn finish_total(&mut self) {
        let wall_s = self
            .total_start
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let smt_calls = self.samples.iter().map(|s| s.smt_calls).sum();
        let smt_s: f64 = self.samples.iter().map(|s| s.smt_s).sum();
        self.samples.push(PhaseSample {
            phase: "total",
            wall_s,
            rss_peak_mb: peak_rss_mb(),
            smt_calls,
            smt_s,
        });
    }

    pub fn samples(&self) -> &[PhaseSample] {
        &self.samples
    }

    pub fn emit_stderr(&self) {
        for s in &self.samples {
            eprintln!("{}", s.format());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_matches_documented_shape() {
        let s = PhaseSample {
            phase: "typecheck",
            wall_s: 1.5,
            rss_peak_mb: 42,
            smt_calls: 3,
            smt_s: 0.25,
        };
        assert_eq!(
            s.format(),
            "[profile] phase=\"typecheck\" wall_s=1.500000 rss_peak_mb=42 smt_calls=3 smt_s=0.250000"
        );
    }

    #[test]
    fn profiler_records_phases_and_total() {
        let mut p = Profiler::new();
        p.begin_total();
        p.phase("parse", || {});
        p.phase("lean-emit", || {});
        p.finish_total();
        let names: Vec<_> = p.samples().iter().map(|s| s.phase).collect();
        assert_eq!(names, vec!["parse", "lean-emit", "total"]);
        assert!(p.samples().iter().all(|s| s.wall_s >= 0.0));
    }

    #[test]
    fn simulated_phases_have_zero_wall_time() {
        let mut p = Profiler::new();
        p.simulated("parse");
        p.finish_total();
        assert_eq!(p.samples()[0].wall_s, 0.0);
        assert_eq!(p.samples().last().unwrap().phase, "total");
    }

    #[test]
    fn peak_rss_does_not_panic() {
        let _ = peak_rss_mb();
    }

    #[test]
    fn phase_with_counters_records_prover_delta() {
        let mut p = Profiler::new();
        p.begin_total();
        let before = ProverCounters::new(10, 1.5);
        let after = ProverCounters::new(13, 1.75);
        p.phase_with_counters("typecheck", ProverCounters::diff(before, after), || {});
        let s = &p.samples()[0];
        assert_eq!(s.smt_calls, 3);
        assert!((s.smt_s - 0.25).abs() < 1e-9);
        p.finish_total();
        let total = p.samples().last().unwrap();
        assert_eq!(total.smt_calls, 3);
        assert!(total.phase == "total");
    }

    #[test]
    fn add_smt_to_last_attaches_to_named_phase() {
        let mut p = Profiler::new();
        p.begin_total();
        p.phase("frontend", || {});
        p.phase("typecheck", || {});
        p.add_smt_to_last("typecheck", ProverCounters::new(2, 0.5));
        // `frontend` behält Nullen, `typecheck` trägt echte Zähler.
        assert_eq!(p.samples()[0].smt_calls, 0);
        assert_eq!(p.samples()[1].smt_calls, 2);
        assert!((p.samples()[1].smt_s - 0.5).abs() < 1e-9);
        p.finish_total();
        assert_eq!(p.samples().last().unwrap().smt_calls, 2);
    }

    #[test]
    fn prover_counters_diff_saturates() {
        let d = ProverCounters::diff(ProverCounters::new(5, 2.0), ProverCounters::new(3, 1.0));
        assert_eq!(d, ProverCounters::ZERO);
    }
}
