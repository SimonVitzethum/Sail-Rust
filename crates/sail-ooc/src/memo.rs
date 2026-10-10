//! Memo-Spills mit Content-Hash-Key (E4, kalt): Z3-Ergebnisse & Co.
//!
//! Unveraenderliche Memo-Werte (z. B. Solver-Antworten) werden unter einem
//! Content-Hash-Key auf Platte gespillt und sind **run-uebergreifend**
//! wiederverwendbar: gleiche Anfrage → gleicher Key → Treffer ohne Rechnen.
//!
//! Backend ist [`sail_cache::DiskCache`] (Lane 36, gelesen/nicht gedoppelt):
//! Der Eintragsname ist der Content-Hash (`<hex>/`), als Fingerprint liegt der
//! Hash der Anfrage (Domaene + Query) — damit findet `lookup` allein aus der
//! Anfrage zurueck, und der Zweitlauf-Schreibschutz von Lane 36
//! (`store` schreibt bei frischem Eintrag nichts) gilt automatisch.
//!
//! Vertrag: Anfrage → Wert muss eine **reine Funktion** sein. Gibt es zu einer
//! Anfrage je einen anderen Wert (Aufruferfehler), gewinnt der erste
//! gespeicherte (Determinismus vor Letzt-Schreiber).

use std::io;
use std::path::Path;

use crate::ContentId;

/// Domaenen-Praefix in allen Memo-Hashes (trennt OoC-Memos von anderen Keys).
pub const MEMO_DOMAIN_PREFIX: &str = "ooc/memo-v1";

/// Content-Hash-verschluesselte Memo-Ablage auf Platte.
#[derive(Debug, Clone)]
pub struct MemoSpills {
    inner: sail_cache::DiskCache,
    domain: String,
}

impl MemoSpills {
    /// Oeffnet die Ablage unter `dir` fuer `domain` (z. B. `"solver/z3-v1"`).
    /// Verschiedene Domaenen teilen sich nie einen Key.
    #[must_use]
    pub fn open(dir: impl AsRef<Path>, domain: impl Into<String>) -> Self {
        Self {
            inner: sail_cache::DiskCache::new(dir),
            domain: domain.into(),
        }
    }

    /// Oeffnet mit LRU-Limit (Eintraege) wie in Lane 36.
    #[must_use]
    pub fn with_max_entries(mut self, n: usize) -> Self {
        self.inner = self.inner.with_max_entries(n);
        self
    }

    /// Domaene dieser Ablage.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Stabile ID einer Anfrage: Domaene + Query-Bytes (run-uebergreifend).
    #[must_use]
    pub fn key(&self, query: &[u8]) -> ContentId {
        ContentId::of_domain_parts(
            &format!("{MEMO_DOMAIN_PREFIX}/{}", self.domain),
            &[query],
        )
    }

    fn entry_name(&self, query: &[u8]) -> String {
        self.key(query).to_hex()
    }

    fn query_fingerprint(&self, query: &[u8]) -> sail_cache::ShardFingerprint {
        sail_cache::fingerprint(query, &format!("{MEMO_DOMAIN_PREFIX}/{}", self.domain))
    }

    /// `true`, wenn die Anfrage kalt vorliegt (fasst nichts an).
    #[must_use]
    pub fn is_fresh(&self, query: &[u8]) -> bool {
        self.inner
            .is_fresh(&self.entry_name(query), &self.query_fingerprint(query))
    }

    /// Holt den gespeicherten Wert zur Anfrage (`None` = Miss).
    /// Aktualisiert die LRU-Position (Lane-36-Verhalten).
    #[must_use]
    pub fn lookup(&self, query: &[u8]) -> Option<Vec<u8>> {
        self.inner
            .lookup(&self.entry_name(query), &self.query_fingerprint(query))
    }

    /// Legt den Wert zur Anfrage ab. `Ok(true)` = neu geschrieben,
    /// `Ok(false)` = bereits frisch (nichts angefasst, Zweitlauf-Skip).
    pub fn store(&self, query: &[u8], value: &[u8]) -> io::Result<bool> {
        self.inner
            .store(&self.entry_name(query), &self.query_fingerprint(query), value)
    }

    /// Raeumt ueber dem LRU-Limit auf; gibt entfernte Eintraege zurueck.
    pub fn gc(&self) -> io::Result<usize> {
        self.inner.gc()
    }

    /// Anzahl gespeicherter Memos.
    pub fn entry_count(&self) -> io::Result<usize> {
        self.inner.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);
    impl TestDir {
        fn new(tag: &str) -> Self {
            let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("sail-ooc-memo-{tag}-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("testdir");
            Self(dir)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const DOMAIN: &str = "solver/z3-test-v1";

    #[test]
    fn key_stabil_domaenen_getrennt() {
        let t = TestDir::new("key");
        let m = MemoSpills::open(&t.0, DOMAIN);
        assert_eq!(m.key(b"anfrage"), m.key(b"anfrage"));
        assert_ne!(m.key(b"anfrage"), m.key(b"andere"));
        let other = MemoSpills::open(&t.0, "andere-domaene");
        assert_ne!(m.key(b"anfrage"), other.key(b"anfrage"));
    }

    #[test]
    fn store_lookup_zweitlauf_skip() {
        let t = TestDir::new("roundtrip");
        let m = MemoSpills::open(&t.0, DOMAIN);
        assert!(!m.is_fresh(b"q"));
        assert!(m.lookup(b"q").is_none());
        assert!(m.store(b"q", b"sat(model)").expect("store"));
        assert!(m.is_fresh(b"q"));
        assert_eq!(m.lookup(b"q").as_deref(), Some(b"sat(model)".as_slice()));
        // Zweitlauf: kein Rewrite.
        assert!(!m.store(b"q", b"sat(model)").expect("zweitlauf"));
        assert_eq!(m.lookup(b"q").as_deref(), Some(b"sat(model)".as_slice()));
    }

    #[test]
    fn run_uebergreifend_wiederverwendbar() {
        let t = TestDir::new("runs");
        let first = MemoSpills::open(&t.0, DOMAIN);
        first.store(b"z3-query", b"unsat").expect("lauf 1");
        drop(first);
        // Neuer Prozess-Lauf, gleiche Domaene: Treffer ohne Rechnen.
        let second = MemoSpills::open(&t.0, DOMAIN);
        assert!(second.is_fresh(b"z3-query"));
        assert_eq!(second.lookup(b"z3-query").as_deref(), Some(b"unsat".as_slice()));
        // Andere Domaene: kein Treffer (kein Key-Leak).
        let foreign = MemoSpills::open(&t.0, "fremd");
        assert!(!foreign.is_fresh(b"z3-query"));
    }

    #[test]
    fn gc_mit_limit() {
        let t = TestDir::new("gc");
        let m = MemoSpills::open(&t.0, DOMAIN).with_max_entries(2);
        for q in [b"a".as_slice(), b"b", b"c"] {
            m.store(q, b"v").expect("store");
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert_eq!(m.entry_count().expect("count"), 2);
        assert!(!m.is_fresh(b"a"));
        assert!(m.is_fresh(b"c"));
    }
}
