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
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ContentId;
use sail_cache::TMP_PREFIX;
// Lane-127-Namenswahrheit: Tmp-Praefix kanonisch aus `sail-cache`
// (dort als `TMP_PREFIX` dokumentiert); kein zweites Literal.

/// Dateiendung warmer Blobs.
pub const WARM_BLOB_EXT: &str = "blob";
// Tmp-Praefix: `TMP_PREFIX` (Import oben, Wahrheit in `sail-cache`).

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
    /// Unveraenderlichkeit (Lane 126): Haelt der Layer `id` bereits live mit
    /// *anderen* Bytes, kommt [`WarmError::ContentMismatch`] — der alte
    /// Ersetzungspfad (`account` schrieb still drueber) ist geschlossen.
    /// Gibt `Err`, wenn das Live-Budget ueberschritten wuerde — dann erst
    /// [`WarmLayer::evict`]/[`WarmLayer::evict_all`] aufrufen.
    pub fn spill(&mut self, id: &ContentId, bytes: &[u8]) -> Result<(), WarmError> {
        if let Some(held) = self.live.get(id) {
            if held.as_ref() == bytes {
                return Ok(());
            }
            // Live anderer Inhalt: Korruption/Domaenenfehler, kein Ersetzen.
            return Err(WarmError::ContentMismatch { id: *id });
        }
        let add = bytes.len() as u64;
        if self.live_bytes.checked_add(add).unwrap_or(u64::MAX) > self.max_live_bytes {
            return Err(WarmError::BudgetExceeded {
                live: self.live_bytes,
                add,
                max: self.max_live_bytes,
            });
        }
        self.store(id, bytes)?;
        self.account(id, Arc::<[u8]>::from(bytes));
        Ok(())
    }

    /// Persist a completed shard without retaining or copying it into RAM.
    /// Shared access allows workers to spill concurrently. Existing blobs are
    /// compared in fixed-size blocks; publication never replaces another
    /// writer's blob. Live-cache budget is independent of disk storage.
    pub fn store(&self, id: &ContentId, bytes: &[u8]) -> Result<(), WarmError> {
        let path = self.blob_path(id);
        match same_file_bytes(&path, bytes) {
            Ok(true) => return Ok(()),
            Ok(false) => return Err(WarmError::ContentMismatch { id: *id }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        std::fs::create_dir_all(&self.dir)?;
        match atomic_write(&path, bytes) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if same_file_bytes(&path, bytes)? { Ok(()) }
                else { Err(WarmError::ContentMismatch { id: *id }) }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Open a disk-backed blob for sequential consumption using a bounded
    /// buffer. No cache entry is created; even blobs larger than the live
    /// budget can be processed with `io::copy` or an incremental decoder.
    pub fn reader(&self, id: &ContentId) -> Result<Option<io::BufReader<std::fs::File>>, WarmError> {
        match std::fs::File::open(self.blob_path(id)) {
            Ok(file) => Ok(Some(io::BufReader::new(file))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
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
        let Some(mut reader) = self.reader(id)? else { return Ok(None) };
        // Reject oversized files BEFORE allocating/reading them. The previous
        // fs::read path could OOM even when it eventually rejected the blob.
        let add = reader.get_ref().metadata()?.len();
        if self.live_bytes.checked_add(add).unwrap_or(u64::MAX) > self.max_live_bytes {
            return Err(WarmError::BudgetExceeded {
                live: self.live_bytes, add, max: self.max_live_bytes,
            });
        }
        let len = usize::try_from(add)
            .map_err(|_| io::Error::other("warm blob exceeds address space"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(io::Error::other)?;
        bytes.resize(len, 0);
        reader.read_exact(&mut bytes)?;
        if reader.read(&mut [0])? != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "warm blob changed while reading").into());
        }
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        self.account(id, Arc::clone(&arc));
        Ok(Some(arc))
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
                self.sub_live(bytes.len() as u64);
                self.debug_check_accounting();
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
            self.sub_live(old.len() as u64);
        }
        self.add_live(arc.len() as u64);
        self.debug_check_accounting();
    }

    /// Zaehlt `n` Live-Bytes dazu. Kein `saturating_add` (Lane 126): Saettigung
    /// wuerde Buchungsfehler still verbergen (Drift); Ueberlauf ist logisch
    /// unmoeglich (Summe real allokierter Blobs) und faellt hier laut auf.
    fn add_live(&mut self, n: u64) {
        self.live_bytes = self
            .live_bytes
            .checked_add(n)
            .expect("live_bytes-Ueberlauf");
    }

    /// Zieht `n` Live-Bytes ab. Kein `saturating_sub` (Lane 126, s. `add_live`):
    /// Abzug ohne Deckung ist ein Buchungsfehler und faellt laut auf.
    fn sub_live(&mut self, n: u64) {
        self.live_bytes = self
            .live_bytes
            .checked_sub(n)
            .expect("live_bytes-Unterlauf");
    }

    /// Prueft `live_bytes == Summe(held)` — nur in Debug-Builds (O(n), faengt
    /// jede Drift zwischen Zaehler und gehaltenen `Arc`s; Release verlaesst
    /// sich auf die geprueften `add_live`/`sub_live`-Pfade).
    fn debug_check_accounting(&self) {
        #[cfg(debug_assertions)]
        {
            let sum: u64 = self.live.values().map(|b| b.len() as u64).sum();
            debug_assert_eq!(
                self.live_bytes, sum,
                "WarmLayer-Buchung gedriftet: Zaehler {}, Summe {sum}",
                self.live_bytes,
            );
        }
    }
}

/// Compare without materializing another full copy of an existing blob.
fn same_file_bytes(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() != bytes.len() as u64 { return Ok(false); }
    let mut buffer = [0u8; 64 * 1024];
    for chunk in bytes.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..chunk.len()])?;
        if &buffer[..chunk.len()] != chunk { return Ok(false); }
    }
    let mut extra = [0];
    Ok(file.read(&mut extra)? == 0)
}

/// Publish an fsynced temporary file atomically without replacing an existing
/// content ID. A hard link on the same filesystem provides create-if-absent.
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
        std::fs::hard_link(&tmp, path)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&tmp);
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
    fn live_ersetzung_wird_abgewiesen_statt_still_ersetzt() {
        let t = TestDir::new("replace");
        let mut warm = WarmLayer::open(t.path());
        let id = id_of(b"original");
        warm.spill(&id, b"original").expect("spill");
        assert_eq!(warm.live_bytes(), 8);
        // Gleicher Key, andere Bytes, Eintrag noch live: kein stilles
        // Ersetzen (Unveraenderlichkeit), Zaehler unveraendert.
        let err = warm.spill(&id, b"manipuliert!").expect_err("mismatch");
        assert!(
            matches!(err, WarmError::ContentMismatch { .. }),
            "unerwartet: {err}"
        );
        assert_eq!(warm.live_bytes(), 8);
        let got = warm.get(&id).expect("get").expect("da");
        assert_eq!(got.as_ref(), b"original");
        // Plattenstand bleibt der alte (nichts ueberschrieben).
        let on_disk = std::fs::read(warm.blob_path(&id)).expect("platte");
        assert_eq!(on_disk.as_slice(), b"original");
    }

    #[test]
    fn buchung_bleibt_exakt_kein_drift() {
        let t = TestDir::new("accounting");
        let mut warm = WarmLayer::open(t.path());
        let a = id_of(b"aaaa");
        let b = id_of(b"bb");
        warm.spill(&a, b"aaaa").expect("spill a");
        warm.spill(&b, b"bb").expect("spill b");
        assert_eq!(warm.live_bytes(), 6);
        // Idempotenter Zweitspill zaehlt nicht doppelt.
        warm.spill(&a, b"aaaa").expect("respin");
        assert_eq!(warm.live_bytes(), 6);
        warm.evict(&a);
        assert_eq!(warm.live_bytes(), 2);
        // Nachladen zaehlt exakt dazu; Budgetvergleich ohne Saettigung.
        warm.evict(&b);
        assert_eq!(warm.live_bytes(), 0);
        let got = warm.get(&a).expect("reload").expect("datei bleibt");
        assert_eq!(got.as_ref(), b"aaaa");
        assert_eq!(warm.live_bytes(), 4);
        warm.evict_all();
        assert_eq!(warm.live_bytes(), 0);
        assert_eq!(warm.live_count(), 0);
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
    fn tmp_prefix_teilt_diskcache_namenwahrheit() {
        // Lane 127: kein zweites Literal — der Import oben ist die
        // kanonische `sail_cache::TMP_PREFIX`-Konstante.
        assert_eq!(TMP_PREFIX, sail_cache::TMP_PREFIX);
        assert_eq!(TMP_PREFIX, ".tmp-");
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
    #[test]
    fn rejects_large_sparse_blob_before_reading_it() {
        let t = TestDir::new("sparse");
        let mut warm = WarmLayer::open(t.path()).with_max_live_bytes(1024);
        std::fs::create_dir_all(t.path()).unwrap();
        let id = id_of(b"large");
        let file = std::fs::File::create(warm.blob_path(&id)).unwrap();
        file.set_len(512 * 1024 * 1024).unwrap();
        assert!(matches!(warm.get(&id), Err(WarmError::BudgetExceeded { .. })));
        assert_eq!(warm.live_bytes(), 0);
    }

    #[test]
    fn disk_storage_and_streaming_need_no_live_cache() {
        let t = TestDir::new("disk-only");
        let warm = WarmLayer::open(t.path()).with_max_live_bytes(0);
        let bytes = vec![123; 1024 * 1024];
        let id = id_of(&bytes);
        warm.store(&id, &bytes).unwrap();
        warm.store(&id, &bytes).unwrap();
        assert_eq!(warm.live_bytes(), 0);
        let mut reader = warm.reader(&id).unwrap().unwrap();
        assert_eq!(io::copy(&mut reader, &mut io::sink()).unwrap(), bytes.len() as u64);
        assert!(matches!(warm.store(&id, b"other"), Err(WarmError::ContentMismatch { .. })));
    }

    #[test]
    fn concurrent_publication_cannot_overwrite_an_existing_id() {
        let t = TestDir::new("concurrent");
        let warm = WarmLayer::open(t.path());
        let id = id_of(b"shared");
        let barrier = std::sync::Barrier::new(8);
        let outcomes = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|i| {
                let warm = &warm;
                let barrier = &barrier;
                scope.spawn(move || {
                    let bytes = vec![i as u8; 256 * 1024];
                    barrier.wait();
                    (i as u8, warm.store(&id, &bytes))
                })
            }).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect::<Vec<_>>()
        });
        let winners: Vec<_> = outcomes.iter().filter(|(_, r)| r.is_ok()).collect();
        assert_eq!(winners.len(), 1);
        for (_, r) in &outcomes {
            assert!(r.is_ok() || matches!(r, Err(WarmError::ContentMismatch { .. })));
        }
        assert!(std::fs::read(warm.blob_path(&id)).unwrap().iter()
            .all(|&b| b == winners[0].0));
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 1, "temporary files leaked");
    }

}
