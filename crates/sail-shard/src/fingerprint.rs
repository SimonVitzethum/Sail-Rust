//! Fingerprint: stabiler Hash ueber Shard-Inhalt + Pass-Version.

use std::fmt;

/// 256-Bit-Fingerprint (blake3) ueber `(pass_version, shard-Inhalt)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    /// Hex-Darstellung (64 Zeichen).
    #[must_use]
    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
        s
    }

    /// Parst eine [`Fingerprint::to_hex`]-Ausgabe zurueck.
    /// Akzeptiert nur exakt 64 Klein-/Grossbuchstaben-Hexzeichen.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        fn nibble(b: u8) -> Option<u8> {
            match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            }
        }
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, pair) in bytes.chunks(2).enumerate() {
            out[i] = nibble(pair[0])? << 4 | nibble(pair[1])?;
        }
        Some(Self(out))
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Berechnet den Fingerprint ueber Inhalt + Pass-Version.
///
/// Format: `pass_version` (mit Laengen-Prefix, zur Trennschaerfe), dann
/// `content`-Bytes. Gleiche Eingaben liefern gleiche Hashes (Stabilitaet),
/// jede Aenderung an Inhalt oder Version aendert den Hash.
#[must_use]
pub fn fingerprint(content: &str, pass_version: &str) -> Fingerprint {
    fingerprint_bytes(content.as_bytes(), pass_version.as_bytes())
}

/// Byte-Variante von [`fingerprint`].
#[must_use]
pub fn fingerprint_bytes(content: &[u8], pass_version: &[u8]) -> Fingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(pass_version.len() as u64).to_le_bytes());
    hasher.update(pass_version);
    hasher.update(&(content.len() as u64).to_le_bytes());
    hasher.update(content);
    Fingerprint(hasher.finalize().into())
}

/// Parallele Fingerprints ueber viele Shard-Inhalte (rayon).
///
/// Reihenfolge des Outputs entspricht der Input-Reihenfolge.
#[must_use]
pub fn fingerprint_all(contents: &[&str], pass_version: &str) -> Vec<Fingerprint> {
    use rayon::prelude::*;
    contents.par_iter().map(|c| fingerprint(c, pass_version)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const V1: &str = "lane11-passes-v1";

    #[test]
    fn fingerprint_stable() {
        let a = fingerprint("insn ADD x0, x1", V1);
        let b = fingerprint("insn ADD x0, x1", V1);
        assert_eq!(a, b);
        assert_eq!(a.to_hex(), b.to_hex());
        assert_eq!(a.to_hex().len(), 64);
    }

    #[test]
    fn fingerprint_changes_on_content() {
        let a = fingerprint("insn ADD", V1);
        let b = fingerprint("insn SUB", V1);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_changes_on_pass_version() {
        let a = fingerprint("same body", "passes-v1");
        let b = fingerprint("same body", "passes-v2");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_no_prefix_collision() {
        // Laengen-Prefix verhindert, dass ("ab","c") == ("a","bc") kollidiert.
        let seen: HashSet<Fingerprint> = [
            fingerprint("ab", "c"),
            fingerprint("a", "bc"),
            fingerprint("abc", ""),
            fingerprint("", "abc"),
        ]
        .into_iter()
        .collect();
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn fingerprint_hex_roundtrip() {
        let a = fingerprint("insn ADD x0, x1", V1);
        assert_eq!(Fingerprint::from_hex(&a.to_hex()), Some(a));
        assert_eq!(Fingerprint::from_hex(&a.to_hex().to_uppercase()), Some(a));
        assert_eq!(Fingerprint::from_hex("xyz"), None);
        assert_eq!(Fingerprint::from_hex(&a.to_hex()[..63]), None);
    }

    #[test]
    fn fingerprint_all_parallel_matches_serial() {
        let inputs = vec!["alpha", "beta", "gamma", "delta"];
        let par = fingerprint_all(&inputs, V1);
        let ser: Vec<_> = inputs.iter().map(|c| fingerprint(c, V1)).collect();
        assert_eq!(par, ser);
    }
}
