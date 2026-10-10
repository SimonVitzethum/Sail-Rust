//! Peak-Messung: live-Set-Peak plus echte Peak-RSS (Linux, nur std).
//!
//! Zwei Ebenen, beide ohne `unsafe`:
//!
//! - Logischer Peak: maximale Anzahl gleichzeitig lebender Eintraege mal
//!   [`core::mem::size_of`]. Das ist allocator-unabhaengig, deterministisch
//!   und damit die Vergleichsgroesse fuer vorher/nachher (`ARENA_REPORT.md`).
//! - Echte Peak-RSS: `VmHWM` aus `/proc/self/status` (Linux only, sonst
//!   `None`). Informativ, nicht deterministisch — dokumentiert den
//!   tatsaechlichen Speichereffekt der Drop-Punkte.

/// Geschaetzte Bytes des live-Sets: `live * size_of::<T>()`.
///
/// Absichtlich eine freie, nicht-generische Funktion ueber `elem_size`:
/// So landet genau eine Kopie im Binaer (siehe `BLOAT.md`), statt einer
/// Monomorphisierung pro `T`.
#[must_use]
pub fn live_bytes_estimate(live: usize, elem_size: usize) -> u64 {
    (live as u64).saturating_mul(elem_size as u64)
}

/// Aktuelle Peak-RSS des Prozesses in KiB (`VmHWM`), Linux only.
///
/// Liest `/proc/self/status` mit reinem std-IO. Gibt `None` zurueck auf
/// Nicht-Linux-Systemen oder wenn die Datei nicht parsbar ist.
#[must_use]
#[cfg(target_os = "linux")]
pub fn peak_rss_kib() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/self/status").ok()?;
    content
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("VmHWM:")?;
            let num = rest.split_whitespace().next()?;
            // VmHWM wird in kB gemeldet.
            num.parse::<u64>().ok()
        })
}

/// Nicht-Linux-Fallback: keine Peak-RSS verfuegbar.
#[must_use]
#[cfg(not(target_os = "linux"))]
pub fn peak_rss_kib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_scales_linearly() {
        assert_eq!(live_bytes_estimate(0, 32), 0);
        assert_eq!(live_bytes_estimate(1_000, 32), 32_000);
        assert_eq!(live_bytes_estimate(1_000, 0), 0);
    }

    #[test]
    fn estimate_saturates_instead_of_wrapping() {
        assert_eq!(live_bytes_estimate(usize::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn rss_reading_does_not_panic() {
        // Nur Rauchtest: darf Some oder None liefern, nie paniken.
        let _ = peak_rss_kib();
    }
}
