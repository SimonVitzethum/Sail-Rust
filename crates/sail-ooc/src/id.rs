//! Stabile Content-Hash-IDs (E4-Voraussetzung, vgl. Lane 37).
//!
//! Alle OoC-Schluessel sind Content-Hashes (blake3-256), keine Zaehler:
//! Ein Zaehler invalidiert bei jedem Lauf den Cache, ein Content-Hash ist
//! run-uebergreifend stabil. Jede Hashung nutzt Laengen-Prefixe, damit
//! `("ab","c") != ("a","bc")` (Trennschaerfe wie in `sail-shard`).

use std::fmt;

/// Stabile, run-uebergreifende ID: blake3-256 ueber Inhalt (+ Domaene).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId([u8; 32]);

impl ContentId {
    /// Hash ueber rohe Bytes (ohne Domaene; nur fuer bereits eindeutige Blobs).
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Hash ueber Domaene + Teile. Domaene trennt Namensraeume
    /// (`"ooc/shard-v1"`, `"ooc/memo-v1/..."`), jedes Teil ist
    /// laengen-praefixiert.
    #[must_use]
    pub fn of_domain_parts(domain: &str, parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&(domain.len() as u64).to_le_bytes());
        hasher.update(domain.as_bytes());
        hasher.update(&(parts.len() as u64).to_le_bytes());
        for part in parts {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        Self(hasher.finalize().into())
    }

    /// Shard-Inhalts-ID aus Lane-11-Identitaet + Inhalt + Pass-Version.
    ///
    /// Die [`sail_shard::ShardId`] (Name + Chunk) geht in den Hash ein, damit
    /// gleiche Inhalte in verschiedenen Shards verschiedene IDs tragen und
    /// keine Shard-Grenze je mit einer anderen verwechselt wird.
    #[must_use]
    pub fn shard_content(
        shard: &sail_shard::ShardId,
        content: &[u8],
        pass_version: &str,
    ) -> Self {
        let chunk = shard.chunk.to_le_bytes();
        Self::of_domain_parts(
            "ooc/shard-v1",
            &[shard.name.as_bytes(), &chunk, content, pass_version.as_bytes()],
        )
    }

    /// Rohe Bytes (z. B. fuer Datei-Layouts ausserhalb dieses Crates).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Hex-Darstellung (64 Kleinbuchstaben).
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

    /// Gegenstueck zu [`ContentId::to_hex`]; akzeptiert Klein-/Grossbuchstaben.
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

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Lane-11-Fingerprints sind dieselbe Hashfamilie und dasselbe
/// Laengen-Prefix-Schema; die Uebernahme ist verlustfrei.
impl From<sail_shard::Fingerprint> for ContentId {
    fn from(fp: sail_shard::Fingerprint) -> Self {
        Self(fp.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stabil_ueber_lauefe() {
        let a = ContentId::of_bytes(b"shard-inhalt");
        let b = ContentId::of_bytes(b"shard-inhalt");
        assert_eq!(a, b);
        assert_eq!(a.to_hex().len(), 64);
    }

    #[test]
    fn sensitiv_auf_inhalt_und_domaene() {
        let base = ContentId::of_domain_parts("d", &[b"x"]);
        assert_ne!(base, ContentId::of_domain_parts("d", &[b"y"]));
        assert_ne!(base, ContentId::of_domain_parts("e", &[b"x"]));
    }

    #[test]
    fn keine_prefix_kollision() {
        // Ohne Laengen-Prefix waere ("ab","c") == ("a","bc").
        let a = ContentId::of_domain_parts("d", &[b"ab", b"c"]);
        let b = ContentId::of_domain_parts("d", &[b"a", b"bc"]);
        assert_ne!(a, b);
    }

    #[test]
    fn shard_id_geht_in_hash_ein() {
        let c = b"gleicher inhalt";
        let x = ContentId::shard_content(&sail_shard::ShardId::new("a", 0), c, "v1");
        let y = ContentId::shard_content(&sail_shard::ShardId::new("b", 0), c, "v1");
        let z = ContentId::shard_content(&sail_shard::ShardId::new("a", 1), c, "v1");
        assert_ne!(x, y);
        assert_ne!(x, z);
        assert_ne!(x, ContentId::shard_content(&sail_shard::ShardId::new("a", 0), c, "v2"));
    }

    #[test]
    fn hex_roundtrip() {
        let id = ContentId::of_bytes(b"abc");
        assert_eq!(ContentId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(ContentId::from_hex(&id.to_hex().to_uppercase()), Some(id));
        assert_eq!(ContentId::from_hex("xyz"), None);
        assert_eq!(ContentId::from_hex(&id.to_hex()[..63]), None);
    }

    #[test]
    fn lane11_fingerprint_uebernahme() {
        let fp = sail_shard::fingerprint("insn ADD", "passes-v1");
        let id = ContentId::from(fp);
        assert_eq!(id.to_hex(), fp.to_hex().to_lowercase());
    }
}
