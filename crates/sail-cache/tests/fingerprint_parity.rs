#![forbid(unsafe_code)]
//! Lane-127-Paritaet Fingerprint (Code, E3: NICHT ausfuehren).
//!
//! Wahrheit: `sail_shard::{Fingerprint, fingerprint}` (blake3-256,
//! 64-Hex). `sail_cache::{ShardFingerprint, fingerprint}` ist das
//! FNV-1a-64-Minimal (16-Hex) aus `CACHE_NEEDED.md`. Beide teilen die
//! Laengen-Prefix-Trennschaerfe (`("ab","c") != ("a","bc")`), beide sind
//! stabil und versions-sensitiv; Formate (16 vs 64 Hex) sind bewusst
//! verschieden (Migration invalidiert alte `fingerprint`-Dateien via
//! Lookup-Mismatch, Layout bleibt).

use sail_cache::{fingerprint as cache_fingerprint, ShardFingerprint};
use sail_shard::{fingerprint as shard_fingerprint, Fingerprint};

/// Beide Hashes sind stabil, versions- und inhalts-sensitiv.
#[test]
fn both_stable_and_sensitive() {
    assert_eq!(cache_fingerprint(b"shard-bytes", "pass-v1"), cache_fingerprint(b"shard-bytes", "pass-v1"));
    assert_ne!(cache_fingerprint(b"shard-bytes", "pass-v1"), cache_fingerprint(b"shard-bytes-x", "pass-v1"));
    assert_ne!(cache_fingerprint(b"shard-bytes", "pass-v1"), cache_fingerprint(b"shard-bytes", "pass-v2"));
    assert_eq!(shard_fingerprint("shard-bytes", "pass-v1"), shard_fingerprint("shard-bytes", "pass-v1"));
    assert_ne!(shard_fingerprint("shard-bytes", "pass-v1"), shard_fingerprint("shard-bytes-x", "pass-v1"));
    assert_ne!(shard_fingerprint("shard-bytes", "pass-v1"), shard_fingerprint("shard-bytes", "pass-v2"));
}

/// Beide trennen `("ab","c")` von `("a","bc")` (Laengen-Prefix).
#[test]
fn both_have_length_prefix_separation() {
    // FNV-16-Hex-Seite (parsebar, Kleinbuchstaben).
    let a = cache_fingerprint(b"ab", "c");
    let b = cache_fingerprint(b"a", "bc");
    assert_ne!(a, b);
    assert_eq!(a.as_str().len(), 16);
    assert_eq!(a, a.as_str().parse::<ShardFingerprint>().expect("roundtrip"));
    // blake3-64-Hex-Seite (roundtrip ueber `from_hex`).
    let x = shard_fingerprint("ab", "c");
    let y = shard_fingerprint("a", "bc");
    assert_ne!(x, y);
    assert_eq!(x.to_hex().len(), 64);
    assert_eq!(Fingerprint::from_hex(&x.to_hex()), Some(x));
    // Formate sind verschieden: Migration invalidiert alte Dateien.
    assert_ne!(a.as_str().len(), x.to_hex().len());
}
