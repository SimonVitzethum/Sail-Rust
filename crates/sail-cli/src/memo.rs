//! Bounded `--memo-z3`-Aequivalent (Lane 38).
//!
//! OCaml-Sail memoiziert Z3-Aufrufe unbeschraenkt (RAM-Risiko, vgl. Lane 14).
//! Diese CLI nutzt stattdessen einen strikt begrenzten In-Memory-Cache
//! (harte Schranke an Eintragszahl und Gesamtbytes, FIFO-Eviction).
//!
//! Die Disk-Persistenz (`--memo-z3-path`) folgt mit Lane 06 (Z3-Bridge);
//! bis dahin ist der Cache prozesslokal und bleibt per Konstruktion klein.
#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};

/// Konfiguration des begrenzten Memo-Caches.
#[derive(Debug, Clone)]
pub struct BoundedMemoConfig {
    pub enabled: bool,
    pub path: String,
    pub max_entries: usize,
    pub max_bytes: u64,
}

impl BoundedMemoConfig {
    pub fn new(enabled: bool, path: String, max_entries: usize, max_bytes: u64) -> Self {
        assert!(max_entries > 0, "memo cache bound must be > 0");
        Self {
            enabled,
            path,
            max_entries,
            max_bytes,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            max_entries: 1,
            max_bytes: 1,
        }
    }
}

/// Strikt begrenzter FIFO-Memo-Cache (Schluessel/Wert als Bytes).
#[derive(Debug, Default)]
pub struct BoundedMemoCache {
    config: Option<BoundedMemoConfig>,
    entries: HashMap<Vec<u8>, Vec<u8>>,
    order: VecDeque<Vec<u8>>,
    bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl BoundedMemoCache {
    pub fn new(config: BoundedMemoConfig) -> Self {
        Self {
            config: Some(config),
            ..Self::default()
        }
    }

    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.enabled)
    }

    pub fn config(&self) -> Option<&BoundedMemoConfig> {
        self.config.as_ref()
    }

    /// `(eintraege, hits, misses, evictions)` fuer Verbose-/Profiling-Logs.
    pub fn stats(&self) -> (usize, u64, u64, u64) {
        (self.len(), self.hits, self.misses, self.evictions)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&mut self, key: &[u8]) -> Option<&[u8]> {
        if !self.is_enabled() {
            return None;
        }
        match self.entries.get(key) {
            Some(v) => {
                self.hits += 1;
                Some(v.as_slice())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        if !config.enabled {
            return;
        }
        // Einzelner Eintrag groesser als das Budget: nie cachen.
        let size = (key.len() as u64) + (value.len() as u64);
        if size > config.max_bytes {
            return;
        }
        if self.entries.contains_key(&key) {
            return; // kein Update-Pfad noetig fuer Memo-Semantik
        }
        while self.entries.len() >= config.max_entries || self.bytes + size > config.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes -= (oldest.len() as u64) + (removed.len() as u64);
                self.evictions += 1;
            }
        }
        self.order.push_back(key.clone());
        self.bytes += size;
        self.entries.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(entries: usize, bytes: u64) -> BoundedMemoConfig {
        BoundedMemoConfig::new(true, "sail_smt_cache".to_string(), entries, bytes)
    }

    #[test]
    fn disabled_cache_never_stores() {
        let mut c = BoundedMemoCache::disabled();
        assert!(!c.is_enabled());
        c.insert(b"k".to_vec(), b"v".to_vec());
        assert_eq!(c.len(), 0);
        assert_eq!(c.get(b"k"), None);
    }

    #[test]
    fn entry_bound_is_enforced_fifo() {
        let mut c = BoundedMemoCache::new(cfg(2, u64::MAX));
        c.insert(b"a".to_vec(), b"1".to_vec());
        c.insert(b"b".to_vec(), b"2".to_vec());
        c.insert(b"c".to_vec(), b"3".to_vec());
        assert_eq!(c.len(), 2);
        assert!(c.evictions >= 1);
        // Aeltester Eintrag ("a") wurde verdrängt.
        assert_eq!(c.get(b"a"), None);
        assert_eq!(c.get(b"c"), Some(b"3".as_slice()));
    }

    #[test]
    fn byte_bound_is_enforced() {
        let mut c = BoundedMemoCache::new(cfg(usize::MAX, 4));
        c.insert(b"ab".to_vec(), b"cd".to_vec()); // 4 Bytes: passt genau
        assert_eq!(c.len(), 1);
        c.insert(b"ef".to_vec(), b"gh".to_vec()); // wuerde 8 Bytes: Eviction
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(b"ab"), None);
        assert_eq!(c.get(b"ef"), Some(b"gh".as_slice()));
    }

    #[test]
    fn oversized_single_entry_is_rejected() {
        let mut c = BoundedMemoCache::new(cfg(usize::MAX, 2));
        c.insert(b"toolongkey".to_vec(), b"v".to_vec());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn hits_and_misses_are_counted() {
        let mut c = BoundedMemoCache::new(cfg(8, u64::MAX));
        c.insert(b"k".to_vec(), b"v".to_vec());
        assert_eq!(c.get(b"k"), Some(b"v".as_slice()));
        assert_eq!(c.get(b"missing"), None);
        assert_eq!((c.hits, c.misses), (1, 1));
    }
}
