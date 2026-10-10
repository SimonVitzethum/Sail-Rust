//! Peak-RSS aus `/proc/self/status` (Linux), reines `std`.
//!
//! `VmHWM` ist ein High-Water-Mark: Einmallesen am Laufzeitende ==
//! Peak-RSS des Prozesses. Zusaetzlich sampled der Shard-Runner `VmRSS`
//! pro Block, um Wachstum zu protokollieren.

/// Aktueller RSS in KiB (`VmRSS`), oder `None` (nicht-Linux / unlesbar).
pub fn current_rss_kb() -> Option<u64> {
    read_status_field("VmRSS:")
}

/// Peak-RSS in KiB (`VmHWM`), oder `None` (nicht-Linux / unlesbar).
pub fn peak_rss_kb() -> Option<u64> {
    read_status_field("VmHWM:")
}

fn read_status_field(field: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            // Format: `VmHWM:    12345 kB`.
            let num: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
            return num.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_is_nonzero_and_monotone() {
        let a = peak_rss_kb().expect("VmHWM should be readable on Linux");
        assert!(a > 0);
        // Speicher anfassen, dann darf der Peak nicht sinken.
        let _keep = vec![0u8; 4 << 20];
        let b = peak_rss_kb().expect("VmHWM should be readable on Linux");
        assert!(b >= a);
    }
}
