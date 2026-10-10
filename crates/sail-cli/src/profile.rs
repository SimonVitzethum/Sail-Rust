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
    pub fn phase<F>(&mut self, phase: &'static str, f: F)
    where
        F: FnOnce(),
    {
        let start = Instant::now();
        f();
        self.samples.push(PhaseSample {
            phase,
            wall_s: start.elapsed().as_secs_f64(),
            rss_peak_mb: peak_rss_mb(),
            smt_calls: 0,
            smt_s: 0.0,
        });
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
}
