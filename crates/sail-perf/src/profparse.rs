//! Lane 15 — Parser für das CLI-`-dprofile`-Format (`sail-cli/src/profile.rs`).
//!
//! Format (stderr, eine Zeile pro Phase):
//! ```text
//! [profile] phase="<name>" wall_s=<secs> rss_peak_mb=<mb> smt_calls=<n> smt_s=<secs>
//! ```
//! Dient der Gate-Auswertung: `summarize_gate_log` extrahiert alle Samples
//! aus einem Compiler-Log und meldet Peaks (RSS, SMT) für
//! `evidence/.../lane-15-profiler.log`.

#![forbid(unsafe_code)]

/// Ein geparstes `[profile]`-Sample.
#[derive(Debug, Clone, PartialEq)]
pub struct CliProfileSample {
    pub phase: String,
    pub wall_s: f64,
    pub rss_peak_mb: u64,
    pub smt_calls: u64,
    pub smt_s: f64,
}

/// Fehler beim Parsen einer `[profile]`-Zeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseProfileError(pub String);

impl std::fmt::Display for ParseProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "profile parse error: {}", self.0)
    }
}

impl std::error::Error for ParseProfileError {}

fn parse_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let rest = line.split_whitespace().find(|t| t.starts_with(&needle))?;
    Some(&rest[needle.len()..])
}

/// Parst genau eine `[profile]`-Zeile (führende Ausgaben davor sind ok,
/// solange die Zeile mit `[profile]` beginnt).
pub fn parse_profile_line(line: &str) -> Result<CliProfileSample, ParseProfileError> {
    let line = line.trim();
    if !line.starts_with("[profile]") {
        return Err(ParseProfileError("not a [profile] line".into()));
    }
    let phase = line
        .split("phase=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .ok_or_else(|| ParseProfileError("missing phase".into()))?;
    let num = |key: &str| {
        parse_kv(line, key)
            .ok_or_else(|| ParseProfileError(format!("missing {key}")))
            .and_then(|v| {
                v.parse::<f64>()
                    .map_err(|_| ParseProfileError(format!("invalid {key}={v}")))
            })
    };
    let wall_s = num("wall_s")?;
    let rss_peak_mb = num("rss_peak_mb")? as u64;
    let smt_calls = num("smt_calls")? as u64;
    let smt_s = num("smt_s")?;
    Ok(CliProfileSample {
        phase: phase.to_owned(),
        wall_s,
        rss_peak_mb,
        smt_calls,
        smt_s,
    })
}

/// Zusammenfassung eines Compiler-Logs: alle `[profile]`-Samples plus
/// abgeleitete Peaks.
#[derive(Debug, Clone, PartialEq)]
pub struct GateLogSummary {
    pub samples: Vec<CliProfileSample>,
    /// Max. `rss_peak_mb` über alle Phasen.
    pub peak_rss_mb: u64,
    /// Summe `smt_calls` (ohne `total`-Doppelzählung: nur echte Phasen).
    pub smt_calls: u64,
    /// Summe `smt_s` (dto.).
    pub smt_s: f64,
    /// Max. Phasen-Wandzeit.
    pub max_wall_s: f64,
}

/// Extrahiert alle `[profile]`-Zeilen aus `log` (voller Compiler-Stderr).
/// Gibt `Err`, wenn keine einzige Sample-Zeile enthalten ist.
pub fn summarize_gate_log(log: &str) -> Result<GateLogSummary, ParseProfileError> {
    let mut samples = Vec::new();
    for line in log.lines() {
        if line.trim_start().starts_with("[profile]") {
            samples.push(parse_profile_line(line)?);
        }
    }
    if samples.is_empty() {
        return Err(ParseProfileError("no [profile] samples in log".into()));
    }
    let mut peak_rss_mb = 0u64;
    let mut smt_calls = 0u64;
    let mut smt_s = 0.0f64;
    let mut max_wall_s = 0.0f64;
    for s in &samples {
        peak_rss_mb = peak_rss_mb.max(s.rss_peak_mb);
        max_wall_s = max_wall_s.max(s.wall_s);
        if s.phase != "total" {
            smt_calls = smt_calls.saturating_add(s.smt_calls);
            smt_s += s.smt_s;
        }
    }
    Ok(GateLogSummary {
        samples,
        peak_rss_mb,
        smt_calls,
        smt_s,
        max_wall_s,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "sail-rs: frontend converted 3 definitions\n\
        [profile] phase=\"frontend\" wall_s=1.500000 rss_peak_mb=42 smt_calls=0 smt_s=0.000000\n\
        [profile] phase=\"typecheck\" wall_s=2.000000 rss_peak_mb=64 smt_calls=3 smt_s=0.250000\n\
        [profile] phase=\"total\" wall_s=3.500000 rss_peak_mb=64 smt_calls=3 smt_s=0.250000\n";

    #[test]
    fn parses_single_line() {
        let s = parse_profile_line(
            "[profile] phase=\"typecheck\" wall_s=2.000000 rss_peak_mb=64 smt_calls=3 smt_s=0.250000",
        )
        .unwrap();
        assert_eq!(s.phase, "typecheck");
        assert!((s.wall_s - 2.0).abs() < 1e-9);
        assert_eq!((s.rss_peak_mb, s.smt_calls), (64, 3));
        assert!((s.smt_s - 0.25).abs() < 1e-9);
    }

    #[test]
    fn rejects_non_profile_and_broken_lines() {
        assert!(parse_profile_line("hello").is_err());
        assert!(parse_profile_line("[profile] phase=\"x\" wall_s=1.0").is_err());
    }

    #[test]
    fn summary_peaks_skip_total_double_count() {
        let g = summarize_gate_log(LOG).unwrap();
        assert_eq!(g.samples.len(), 3);
        assert_eq!(g.peak_rss_mb, 64);
        assert_eq!(g.smt_calls, 3);
        assert!((smt_abs(g.smt_s) - 0.25).abs() < 1e-9);
        assert!((g.max_wall_s - 3.5).abs() < 1e-9);
    }

    fn smt_abs(v: f64) -> f64 {
        v.abs()
    }

    #[test]
    fn empty_log_is_error() {
        assert!(summarize_gate_log("no samples here\n").is_err());
    }
}
