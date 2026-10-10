//! Lane 14 — RAM/Timing-Profiling-Infrastruktur.
//!
//! * [`parse_time_v_output`] parst GNU `time -v`-Stderr (`/usr/bin/time -v`).
//! * [`PhaseRecord`] ist das `-dprofile`-Äquivalent: eine Zeile JSON pro Phase (JSONL-Timings-Log).
//! * [`BUDGETS`] hält die verbindlichen Peak-RSS-Limits (32 GiB Maschine ohne Swap).

#![forbid(unsafe_code)]

use std::fmt;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Verbindliche RAM-Budgets (GiB) für 32-GiB-Maschine ohne Swap.
pub const BUDGET_AARCH64_SMALL_GB: f64 = 4.0;
pub const BUDGET_ARM_V85A_GB: f64 = 16.0;
pub const BUDGET_ARM_V94A_GB: f64 = 24.0;

/// 1 GiB in KiB (so wie GNU time `Maximum resident set size (kbytes)` meldet).
pub const KIB_PER_GIB: u64 = 1024 * 1024;

/// Budget-Eintrag pro Modell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Modellname, z. B. `aarch64_small`.
    pub model: &'static str,
    /// Limit in GiB.
    pub limit_gib: u64,
    /// Limit in KiB (Peak-RSS-Einheit von GNU time).
    pub limit_kib: u64,
}

impl Budget {
    pub const fn new(model: &'static str, limit_gib: u64) -> Self {
        Self {
            model,
            limit_gib,
            limit_kib: limit_gib * KIB_PER_GIB,
        }
    }

    /// `true` wenn `peak_rss_kb` im Budget liegt.
    pub fn fits(&self, peak_rss_kb: u64) -> bool {
        peak_rss_kb <= self.limit_kib
    }
}

/// Die drei verbindlichen Budgets aus der Lane-Spec.
pub const BUDGETS: [Budget; 3] = [
    Budget::new("aarch64_small", 4),
    Budget::new("arm-v8.5-a", 16),
    Budget::new("arm-v9.4-a", 24),
];

/// Ergebnis von GNU `/usr/bin/time -v` (Stderr-Block).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeVReport {
    /// `Maximum resident set size (kbytes)`.
    pub max_rss_kb: u64,
    /// `Elapsed (wall clock) time` in Sekunden.
    pub wall_s: f64,
    /// `User time (seconds)`.
    pub user_s: f64,
    /// `System time (seconds)`.
    pub sys_s: f64,
    /// `Major (requiring I/O) page faults`.
    pub major_faults: u64,
    /// `Minor (reclaiming a frame) page faults`.
    pub minor_faults: u64,
    /// `Swaps from disk` (muss auf swaploser Maschine 0 sein).
    pub swaps: u64,
    /// Roh-Command (`Command being timed`).
    pub command: String,
    /// Exit-Status (`Exit status`), falls vorhanden.
    pub exit_status: Option<i32>,
}

/// Fehler beim Parsen von `time -v`-Output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTimeVError(pub String);

impl fmt::Display for ParseTimeVError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time -v parse error: {}", self.0)
    }
}

impl std::error::Error for ParseTimeVError {}

fn parse_u64_field(line: &str, key: &str) -> Option<u64> {
    line.strip_prefix(key)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|tok| tok.parse::<u64>().ok())
}

fn parse_f64_field(line: &str, key: &str) -> Option<f64> {
    line.strip_prefix(key)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|tok| tok.parse::<f64>().ok())
}

/// Parst `Elapsed (wall clock) time (h:mm:ss or m:ss): M:SS.cc`-Formate.
///
/// Akzeptiert `M:SS`, `M:SS.cc`, `H:MM:SS`, `H:MM:SS.cc` sowie plain seconds.
pub fn parse_elapsed(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    let mut parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let secs: f64 = parts.pop()?.parse().ok()?;
    let mins: f64 = parts.pop()?.parse().ok()?;
    let hours: f64 = if parts.is_empty() {
        0.0
    } else {
        parts.pop()?.parse().ok()?
    };
    Some(hours * 3600.0 + mins * 60.0 + secs)
}

/// Parst den kompletten Stderr von GNU `/usr/bin/time -v`.
///
/// Robust gegen: führende Tabs, zusätzliche Leerzeilen, fehlende optionale
/// Felder (faults/swaps default 0, command default leer). `Maximum resident
/// set size` + `Elapsed (wall clock)` sind Pflicht — ohne sie gibt es einen Fehler.
pub fn parse_time_v_output(stderr: &str) -> Result<TimeVReport, ParseTimeVError> {
    let mut max_rss_kb: Option<u64> = None;
    let mut wall_s: Option<f64> = None;
    let mut user_s = 0.0f64;
    let mut sys_s = 0.0f64;
    let mut major_faults = 0u64;
    let mut minor_faults = 0u64;
    let mut swaps = 0u64;
    let mut command = String::new();
    let mut exit_status: Option<i32> = None;

    for raw in stderr.lines() {
        let line = raw.trim();
        if let Some(v) = line.strip_prefix("Maximum resident set size (kbytes):") {
            if let Ok(n) = v.split_whitespace().next().unwrap_or("").parse() {
                max_rss_kb = Some(n);
            }
        } else if let Some(v) = line.strip_prefix("Elapsed (wall clock) time (h:mm:ss or m:ss):") {
            wall_s = v.split_whitespace().next().and_then(parse_elapsed);
        } else if line.starts_with("User time (seconds):") {
            if let Some(v) = parse_f64_field(line, "User time (seconds):") {
                user_s = v;
            }
        } else if line.starts_with("System time (seconds):") {
            if let Some(v) = parse_f64_field(line, "System time (seconds):") {
                sys_s = v;
            }
        } else if line.starts_with("Major (requiring I/O) page faults:") {
            if let Some(v) = parse_u64_field(line, "Major (requiring I/O) page faults:") {
                major_faults = v;
            }
        } else if line.starts_with("Minor (reclaiming a frame) page faults:") {
            if let Some(v) = parse_u64_field(line, "Minor (reclaiming a frame) page faults:") {
                minor_faults = v;
            }
        } else if line.starts_with("Swaps from disk") {
            // Heuristik: letzte Zahl der Zeile ("Swaps from disk (now unused): 0").
            if let Some(tok) = line.split_whitespace().last().and_then(|t| t.parse().ok()) {
                swaps = tok;
            }
        } else if let Some(v) = line.strip_prefix("Command being timed:") {
            command = v.trim().trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("Exit status:") {
            exit_status = v.split_whitespace().next().and_then(|t| t.parse().ok());
        }
    }

    match (max_rss_kb, wall_s) {
        (Some(max_rss_kb), Some(wall_s)) => Ok(TimeVReport {
            max_rss_kb,
            wall_s,
            user_s,
            sys_s,
            major_faults,
            minor_faults,
            swaps,
            command,
            exit_status,
        }),
        (None, _) => Err(ParseTimeVError(
            "missing 'Maximum resident set size (kbytes)'".to_string(),
        )),
        (_, None) => Err(ParseTimeVError(
            "missing 'Elapsed (wall clock) time'".to_string(),
        )),
    }
}

/// `-dprofile`-Äquivalent: ein Timings-Eintrag pro Compiler-Phase.
///
/// Serialisiert als eine JSON-Zeile (JSONL) pro Phase; siehe
/// [`append_record`] / [`read_records`]. Format (Beispiel):
/// `{"phase":"parse","wall_ms":12,"peak_rss_kb":3456,"note":""}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseRecord {
    /// Phasenname, z. B. `parse`, `resolve`, `typecheck`, `lean-emit`.
    pub phase: String,
    /// Wall-Clock in Millisekunden.
    pub wall_ms: u64,
    /// Peak-RSS dieser Phase in KiB (GNU-time-Einheit).
    pub peak_rss_kb: u64,
    /// Freitext (Shard, Modell, Host ...).
    #[serde(default)]
    pub note: String,
}

impl PhaseRecord {
    pub fn new(phase: impl Into<String>, wall_ms: u64, peak_rss_kb: u64) -> Self {
        Self {
            phase: phase.into(),
            wall_ms,
            peak_rss_kb,
            note: String::new(),
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    /// Baut einen Record aus einem [`TimeVReport`] (vom `measure.sh`-Wrapper).
    pub fn from_report(phase: impl Into<String>, r: &TimeVReport, note: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            wall_ms: (r.wall_s * 1000.0).round() as u64,
            peak_rss_kb: r.max_rss_kb,
            note: note.into(),
        }
    }
}

/// Hängt einen [`PhaseRecord`] als JSON-Zeile an `path` an (erstellt Datei ggf.).
pub fn append_record(path: &Path, rec: &PhaseRecord) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut f, rec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    f.write_all(b"\n")?;
    Ok(())
}

/// Liest ein JSONL-Timings-Log; leere Zeilen werden übersprungen.
pub fn read_records(path: &Path) -> std::io::Result<Vec<PhaseRecord>> {
    let f = OpenOptions::new().read(true).open(path)?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PhaseRecord = serde_json::from_str(&line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        out.push(rec);
    }
    Ok(out)
}

/// Max-Peak über alle Records (0 bei leerem Log).
pub fn peak_of(records: &[PhaseRecord]) -> u64 {
    records.iter().map(|r| r.peak_rss_kb).max().unwrap_or(0)
}

/// Prüft einen Peak gegen ein [`Budget`]; `Ok(())` bei Einhaltung,
/// `Err(msg)` mit Shard-Verkleinerungs-Hinweis bei Überschreitung.
pub fn check_budget(budget: Budget, peak_rss_kb: u64) -> Result<(), String> {
    if budget.fits(peak_rss_kb) {
        Ok(())
    } else {
        Err(format!(
            "budget exceeded: model={} peak={} KiB ({:.2} GiB) > limit={} KiB ({} GiB); shard verkleinern oder Pass in Lane 07/09 zurueckweisen",
            budget.model,
            peak_rss_kb,
            peak_rss_kb as f64 / KIB_PER_GIB as f64,
            budget.limit_kib,
            budget.limit_gib,
        ))
    }
}

#[cfg(test)]
mod tests;
