//! Warm-Layer (E4): fertige Shards, Prelude/Builtins — per Datei eingeblendet.
//!
//! **Kein raw-mmap trotz `mmap`-Name im E4-Text:** `mmap` braucht `unsafe`
//! fuer den Zeiger-Cast, und dieses Crate steht unter
//! `#![forbid(unsafe_code)]` (E3/E4-bindend). Stattdessen liegt jeder Blob als
//! Datei `<dir>/<content-hex>.blob`; [`WarmLayer::get`] laedt ihn per
//! `fs::read` — der OS-Page-Cache paget selbst ein/aus, das Zugriffsprofil
//! (sequentielles Shard-Streaming, Punktzugriffe, read-mostly) ist dasselbe.
//! Ein spaeterer mmap-Backend kann hinter der gleichen API nachgeruestet
//! werden, ohne Aufrufer zu aendern.
//!
//! Read-mostly-Garantie: Aufrufer erhalten [`std::sync::Arc`]`<[u8]>`
//! (geteilt, unveraenderlich). Es gibt keinen `&mut`-Zugriff auf warme Bytes;
//! Aenderungen laufen immer ueber [`WarmLayer::spill`] (neuer Content-Hash,
//! alter Blob bleibt gueltig — Unveraenderlichkeit wie bei Z3-Memos).

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ContentId;

/// Dateiendung warmer Blobs.
pub const WARM_BLOB_EXT: &str = "blob";
/// Praefix temporaerer Schreibdateien (nie sichtbarer Endzustand).
const TMP_PREFIX: &str = ".tmp-";

/// Fehler des Warm-Layers (eigener Typ statt `io::ErrorKind`-Abhaengigkeit;
/// MSRV-sicher, keine jungen `ErrorKind`-Varianten).
#[derive(Debug)]
pub enum WarmError {
    /// Zugrunde liegender IO-Fehler (Platte/ Rechte/ ...).
    Io(io::Error),
    /// [`WarmLayer::spill`] wuerde das Live-Budget sprengen — erst
    /// `evict`/`evict_all`, dann erneut spillen (explizite Drop-Punkte, E4).
    BudgetExceeded {
        /// Aktuell live gehaltene Bytes.
        live: u64,
        /// Zusaetzlich angefragte Bytes.
        add: u64,
        /// Konfiguriertes Budget.
        max: u64,
    },
    /// Unter `id` liegt bereits anderer Inhalt (Korruption oder
    /// Domaenenfehler — IDs sind Content-Hashes und damit unveraenderlich).
    ContentMismatch {
        /// Betroffene ID.
        id: ContentId,
    },
}

impl fmt::Display for WarmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "WarmLayer-IO: {e}"),
            Self::BudgetExceeded { live, add, max } => write!(
                f,
                "WarmLayer-Budget erschoepft (live {live} + {add} > max {max}; erst evict)"
            ),
            Self::ContentMismatch { id } => write!(
                f,
                "WarmLayer: ID {id} belegt, Inhalt weicht ab (Hash-Kollision oder Domaenenfehler)"
            ),
        }
    }
}

impl std::error::Error for WarmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for WarmError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Warme, dateigestuetzte Ablage fertiger Shards (read-mostly, safe).
///
/// Nur [`WarmLayer::spill`] schreibt; [`WarmLayer::get`]/[`WarmLayer::prefetch`]
/// lesen. Die Live-Menge (`Arc`-Cache) ist explizit begrenzt: Wuerde
/// [`WarmLayer::spill`] *oder* ein Nachladen per [`WarmLayer::get`]/
/// [`WarmLayer::prefetch`] das Budget sprengen, muss der Aufrufer per
/// [`WarmLayer::evict`] oder [`WarmLayer::evict_all`] Platz schaffen — kein
/// implizites Raeumen hinter dem Ruecken der Phasen-Barriere (E4: explizite
/// Drop-Punkte).
#[derive(Debug)]
pub struct WarmLayer {
    dir: PathBuf,
    live: HashMap<ContentId, Arc<[u8]>>,
    live_bytes: u64,
    /// Budget fuer live gehaltene Bytes (`u64::MAX` = unbegrenzt).
    pub max_live_bytes: u64,
}

impl WarmLayer {
    /// Oeffnet (bzw. definiert) den Layer unter `dir` mit unbegrenztem
    /// Live-Budget. Das Verzeichnis entsteht erst bei Bedarf.
    #[must_use]
    pub fn open(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_owned(),
            live: HashMap::new(),
            live_bytes: 0,
            max_live_bytes: u64::MAX,
        }
    }

    /// Setzt das Live-Budget (Bytes, die als `Arc` gehalten werden duerfen).
    #[must_use]
    pub fn with_max_live_bytes(mut self, n: u64) -> Self {
        self.max_live_bytes = n;
        self
    }

    /// Ablageverzeichnis.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Anzahl live gehaltener Blobs.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Live gehaltene Bytes gesamt.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.live_bytes
    }

    /// Pfad eines Blobs (reine Funktion der ID; Datei muss nicht existieren).
    #[must_use]
    pub fn blob_path(&self, id: &ContentId) -> PathBuf {
        self.dir.join(format!("{id}.{WARM_BLOB_EXT}"))
    }

    /// Legt `bytes` unter `id` ab (atomar: Tempfile + Rename) und haelt sie
    /// live. Idempotent: Existiert die ID bereits mit gleichen Bytes, wird
    /// nichts geschrieben.
    ///
    /// Gibt `Err`, wenn das Live-Budget ueberschritten wuerde — dann erst
    /// [`WarmLayer::evict`]/[`WarmLayer::evict_all`] aufrufen.
    pub fn spill(&mut self, id: &ContentId, bytes: &[u8]) -> Result<(), WarmError> {
        if let Some(held) = self.live.get(id) {
            if held.as_ref() == bytes {
                return Ok(());
            }
        }
        let add = bytes.len() as u64;
        if self.live_bytes.saturating_add(add) > self.max_live_bytes {
            return Err(WarmError::BudgetExceeded {
                live: self.live_bytes,
                add,
                max: self.max_live_bytes,
            });
        }
        let path = self.blob_path(id);
        if path.is_file() {
            let on_disk = std::fs::read(&path)?;
            if on_disk.as_slice() == bytes {
                self.account(id, Arc::<[u8]>::from(bytes));
                return Ok(());
            }
            return Err(WarmError::ContentMismatch { id: *id });
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&path, bytes)?;
        self.account(id, Arc::<[u8]>::from(bytes));
        Ok(())
    }

    /// Holt den Blob zu `id` (geteilt, read-only). Laedt bei Bedarf von Platte
    /// (Page-Cache) und haelt ihn danach live. `None`, wenn nie gespillt.
    ///
    /// Prueft wie [`WarmLayer::spill`] das Live-Budget: Wuerde das Nachladen
    /// das Budget sprengen, kommt `Err` (erst [`WarmLayer::evict`]/
    /// [`WarmLayer::evict_all`], dann erneut holen) — sonst waere
    /// `max_live_bytes` ueber Nachladen umgehbar. Die Datei bleibt liegen.
    pub fn get(&mut self, id: &ContentId) -> Result<Option<Arc<[u8]>>, WarmError> {
        if let Some(held) = self.live.get(id) {
            return Ok(Some(Arc::clone(held)));
        }
        let path = self.blob_path(id);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let add = bytes.len() as u64;
                if self.live_bytes.saturating_add(add) > self.max_live_bytes {
                    return Err(WarmError::BudgetExceeded {
                        live: self.live_bytes,
                        add,
                        max: self.max_live_bytes,
                    });
                }
                let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
                self.live_bytes = self.live_bytes.saturating_add(arc.len() as u64);
                self.live.insert(*id, Arc::clone(&arc));
                Ok(Some(arc))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(WarmError::Io(e)),
        }
    }

    /// Streaming-Hilfe: laedt `ids` der Reihe nach (Shard-Streaming, kein
    /// wilder Random-Access). Gibt die Anzahl geladener Blobs zurueck;
    /// unbekannte IDs werden uebersprungen. Das Live-Budget gilt wie in
    /// [`WarmLayer::get`]: Passt ein Blob nicht mehr, bricht der Lauf mit
    /// `Err` ab (bereits geladene bleiben live).
    pub fn prefetch(&mut self, ids: &[ContentId]) -> Result<usize, WarmError> {
        let mut loaded = 0usize;
        for id in ids {
            if self.get(id)?.is_some() {
                loaded += 1;
            }
        }
        Ok(loaded)
    }

    /// Gibt die Live-Referenz auf `id` frei (Datei bleibt; OS paget aus).
    /// Gibt `true` zurueck, wenn etwas live gehalten wurde.
    pub fn evict(&mut self, id: &ContentId) -> bool {
        match self.live.remove(id) {
            Some(bytes) => {
                self.live_bytes = self.live_bytes.saturating_sub(bytes.len() as u64);
                true
            }
            None => false,
        }
    }

    /// Gibt alle Live-Referenzen frei (Barrieren-Drop-Punkt; Dateien bleiben).
    pub fn evict_all(&mut self) {
        self.live.clear();
        self.live_bytes = 0;
    }

    /// `true`, wenn `id` gerade live gehalten wird (reine Auskunft, kein Laden).
    #[must_use]
    pub fn is_live(&self, id: &ContentId) -> bool {
        self.live.contains_key(id)
    }

    fn account(&mut self, id: &ContentId, arc: Arc<[u8]>) {
        if let Some(old) = self.live.insert(*id, arc.clone()) {
            self.live_bytes = self.live_bytes.saturating_sub(old.len() as u64);
        }
        self.live_bytes = self.live_bytes.saturating_add(arc.len() as u64);
    }
}

/// Schreibt `bytes` atomar nach `path`: Tempfile im selben Verzeichnis
/// (gleicher Mount → Rename ist atomar) + fsync + Rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!("{TMP_PREFIX}{}-{n}", std::process::id()));
    let res = (|| -> io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);
    impl TestDir {
        fn new(tag: &str) -> Self {
            let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("sail-ooc-warm-{tag}-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn id_of(bytes: &[u8]) -> ContentId {
        ContentId::of_domain_parts("ooc/test-warm-v1", &[bytes])
    }

    #[test]
    fn spill_get_roundtrip() {
        let t = TestDir::new("roundtrip");
        let mut warm = WarmLayer::open(t.path());
        let id = id_of(b"fertiger shard");
        assert!(warm.get(&id).expect("get").is_none());
        warm.spill(&id, b"fertiger shard").expect("spill");
        assert!(warm.is_live(&id));
        let got = warm.get(&id).expect("get").expect("da");
        assert_eq!(got.as_ref(), b"fertiger shard");
        // Zweitspill identisch: idempotent, kein Fehler.
        warm.spill(&id, b"fertiger shard").expect("respin");
    }

    #[test]
    fn evict_gibt_ram_frei_datei_bleibt() {
        let t = TestDir::new("evict");
        let mut warm = WarmLayer::open(t.path());
        let id = id_of(b"prelude-blob");
        warm.spill(&id, b"prelude-blob").expect("spill");
        assert!(warm.evict(&id));
        assert!(!warm.is_live(&id));
        assert_eq!(warm.live_bytes(), 0);
        assert!(!warm.evict(&id));
        // Nachladen von Platte geht (OS-Page-Cache-Pfad).
        let got = warm.get(&id).expect("reload").expect("datei bleibt");
        assert_eq!(got.as_ref(), b"prelude-blob");
        warm.evict_all();
        assert_eq!(warm.live_count(), 0);
    }

    #[test]
    fn budget_wird_explizit_abgewiesen() {
        let t = TestDir::new("budget");
        let mut warm = WarmLayer::open(t.path()).with_max_live_bytes(4);
        let a = id_of(b"aaaa");
        warm.spill(&a, b"aaaa").expect("passt");
        let err = warm.spill(&id_of(b"bb"), b"bb").expect_err("Budget");
        assert!(
            matches!(err, WarmError::BudgetExceeded { .. }),
            "unerwartet: {err}"
        );
        // Nach Evict passt es wieder.
        warm.evict(&a);
        warm.spill(&id_of(b"bb"), b"bb").expect("nach evict");
    }

    #[test]
    fn fremder_inhalt_unter_gleicher_id_wird_abgewiesen() {
        let t = TestDir::new("mismatch");
        let mut warm = WarmLayer::open(t.path());
        let id = id_of(b"original");
        warm.spill(&id, b"original").expect("spill");
        warm.evict(&id);
        // Datei direkt austauschen (simuliert Korruption), dann spillt der
        // Layer nicht stillschweigend drueber: get liefert Plattenstand.
        std::fs::write(warm.blob_path(&id), b"manipuliert").expect("manipulieren");
        let err = warm.spill(&id, b"anderer inhalt").expect_err("mismatch");
        assert!(
            matches!(err, WarmError::ContentMismatch { .. }),
            "unerwartet: {err}"
        );
    }

    #[test]
    fn prefetch_laedt_der_reihe_nach() {
        let t = TestDir::new("prefetch");
        let mut warm = WarmLayer::open(t.path());
        let ids: Vec<ContentId> = (0u8..5).map(|i| id_of(&[i])).collect();
        for (i, id) in ids.iter().enumerate() {
            warm.spill(id, &[i as u8]).expect("spill");
        }
        warm.evict_all();
        let n = warm.prefetch(&ids).expect("prefetch");
        assert_eq!(n, 5);
        assert_eq!(warm.live_count(), 5);
        // Unbekannte ID wird uebersprungen.
        let mut with_unknown = ids.clone();
        with_unknown.push(id_of(b"nie gespillt"));
        warm.evict_all();
        assert_eq!(warm.prefetch(&with_unknown).expect("mit unbekannt"), 5);
    }

    #[test]
    fn get_prueft_budget_wie_spill() {
        let t = TestDir::new("get-budget");
        let mut warm = WarmLayer::open(t.path()).with_max_live_bytes(4);
        let a = id_of(b"aaaa");
        warm.spill(&a, b"aaaa").expect("spill");
        assert_eq!(warm.live_bytes(), 4);
        warm.evict(&a);
        // Datei liegt noch auf Platte; live ist ein kleinerer Blob.
        let c = id_of(b"cc");
        warm.spill(&c, b"cc").expect("spill");
        // Nachladen von `a` (4) ueber live 2 hinaus wuerde das Budget (4)
        // sprengen — wie bei `spill` abgewiesen, Datei bleibt liegen.
        let err = warm.get(&a).expect_err("Budget");
        assert!(
            matches!(err, WarmError::BudgetExceeded { .. }),
            "unerwartet: {err}"
        );
        assert!(!warm.is_live(&a));
        assert_eq!(warm.live_bytes(), 2);
        // Nach Evict passt es wieder.
        warm.evict(&c);
        let got = warm.get(&a).expect("reload").expect("datei bleibt");
        assert_eq!(got.as_ref(), b"aaaa");
    }

    #[test]
    fn prefetch_prueft_budget() {
        let t = TestDir::new("prefetch-budget");
        let mut warm = WarmLayer::open(t.path());
        let ids: Vec<ContentId> = (0u8..3).map(|i| id_of(&[i])).collect();
        for (i, id) in ids.iter().enumerate() {
            warm.spill(id, &[i as u8]).expect("spill");
        }
        warm.evict_all();
        warm.max_live_bytes = 1;
        let err = warm.prefetch(&ids).expect_err("Budget");
        assert!(
            matches!(err, WarmError::BudgetExceeded { .. }),
            "unerwartet: {err}"
        );
        // Genau der erste Blob wurde noch geladen, dann abgebrochen.
        assert_eq!(warm.live_count(), 1);
        assert_eq!(warm.live_bytes(), 1);
    }
}
