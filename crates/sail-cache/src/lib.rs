//! sail-cache — Disk-Cache für Shard-Outputs (Lane 36).
//!
//! Layout: `<root>/<shard-hash>/` mit den Dateien `output` und
//! `fingerprint`. Schreiben erfolgt atomar (Tempfile + Rename + fsync),
//! alte Einträge entsorgt [`DiskCache::gc`] per LRU-Limit (mtime-basiert).
//!
//! Zweitlauf-Garantie: [`DiskCache::store`] schreibt bei unverändertem
//! Fingerprint **nichts** (gibt `Ok(false)` zurück, fasst keine Datei an).
//! Nur [`DiskCache::lookup`] aktualisiert die LRU-Position (durch
//! Neuschreiben der winzigen Fingerprint-Datei).
//!
//! Reines Rust: `#![forbid(unsafe_code)]`, nur `std`, keine
//! Dependencies (low-RAM, offline baubar).
#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Standard-Limit für Einträge, wenn keines angegeben wird.
pub const DEFAULT_MAX_ENTRIES: usize = 128;
/// Datei mit dem kompilierten Shard-Output innerhalb eines Eintrags.
///
/// Lane-127-Namenswahrheit: Diese beiden Dateinamen (`output`,
/// `fingerprint`) plus das Layout `<root>/<shard-hash>/` sind kanonisch
/// fuer DiskCache-Eintraege. Verbraucher (`sail-ooc::MemoSpills`)
/// nutzen sie ausschliesslich ueber die [`DiskCache`]-API (nie eigene
/// Pfade); der Warm-Layer (`sail-ooc::warm`, eigene Domaene, eigene
/// Endung `*.blob`) ist bewusst getrennt dokumentiert.
pub const OUTPUT_FILE: &str = "output";
/// Datei mit dem Fingerprint (Hex) innerhalb eines Eintrags.
///
/// Siehe [`OUTPUT_FILE`]: Teil der kanonischen Eintrags-Namen.
pub const FINGERPRINT_FILE: &str = "fingerprint";
/// Präfix für temporäre Schreibdateien (niemals sichtbarer Endzustand).
///
/// Namenswahrheit auch fuer den Warm-Layer: `sail-ooc::warm` nutzt
/// denselben Wert ueber diesen Re-Export (kein zweites Literal).
pub const TMP_PREFIX: &str = ".tmp-";
/// Maximale Länge einer Shard-ID (Schutz vor absurden Pfaden).
const MAX_ID_LEN: usize = 128;

/// Minimaler Fingerprint über Shard-Input + Pass-Version.
///
/// Lane-127-Wahrheit: Der kanonische Shard-Fingerprint ist
/// `sail_shard::{Fingerprint, fingerprint}` (`sail-shard`, Lane 11:
/// blake3-256 ueber `(pass_version, content)` mit Laengen-Prefix,
/// 64-Hex, `fingerprint_all` parallel). Dieser Typ hier ist das
/// FNV-1a-64-Minimal aus `CACHE_NEEDED.md` (16-Hex, Lane-36-lokal,
/// toolchain-stabil): Test-Vehikel, bis `sail-shard` echte Typen
/// lieferte — was seit Lane 11 der Fall ist.
///
/// Migrationsnotiz (Owner-Folgearbeit): `ShardFingerprint` durch
/// `sail_shard::Fingerprint` ersetzen. Beachten: Der `CACHE_NEEDED.md`-
/// Hinweis "beide sind 16-Hex-FNV, kein Layout-Wechsel" ist ueberholt —
/// blake3 liefert 64-Hex, d. h. der *Inhalt* der `fingerprint`-Datei im
/// Eintrag wechselt (Cache-Invalidierung beim Wechsel, alte Eintraege
/// missen sauber via [`DiskCache::lookup`]-Mismatch). Das Layout
/// `<root>/<shard-hash>/` + Dateinamen
/// ([`OUTPUT_FILE`]/[`FINGERPRINT_FILE`]) bleibt unveraendert, da
/// `shard_hash` aufrufervergeben ist. Uebereinstimmende Eigenschaft
/// beider Hashes (Laengen-Prefix-Trennschaerfe) pinnt
/// `tests/fingerprint_parity.rs` (Code, E3: ungeprueft).
///
/// 16 Hex-Zeichen (FNV-1a 64, deterministisch, stabil über
/// Toolchain-Wechsel hinweg — bewusst kein `DefaultHasher`).
/// Wird durch den Shard-Fingerprint aus Lane 11 ersetzt, sobald
/// `sail-shard` echte Typen liefert (siehe `CACHE_NEEDED.md`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShardFingerprint(String);

impl ShardFingerprint {
    /// Hex-Darstellung (`[0-9a-f]{16}`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShardFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Fehler beim Parsen: nur exakt 16 Kleinbuchstaben-Hex-Zeichen sind gültig.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseFingerprintError;

impl fmt::Display for ParseFingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid shard fingerprint (expected 16 lowercase hex chars)")
    }
}

impl FromStr for ShardFingerprint {
    type Err = ParseFingerprintError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = s.len() == 16
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if ok {
            Ok(ShardFingerprint(s.to_owned()))
        } else {
            Err(ParseFingerprintError)
        }
    }
}

fn fnv1a64(mut hash: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Berechnet den Fingerprint über `input` (Shard-Bytes) und `pass_version`
/// (ändert sich bei jeder Compiler-Pass-Änderung → invalidiert den Cache).
pub fn fingerprint(input: &[u8], pass_version: &str) -> ShardFingerprint {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h = fnv1a64(h, &(pass_version.len() as u64).to_le_bytes());
    h = fnv1a64(h, pass_version.as_bytes());
    h = fnv1a64(h, &(input.len() as u64).to_le_bytes());
    h = fnv1a64(h, input);
    ShardFingerprint(format!("{h:016x}"))
}

/// Disk-Cache für Shard-Outputs unter `<root>/<shard-hash>/`.
#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
    max_entries: usize,
}

impl DiskCache {
    /// Öffnet (bzw. definiert) den Cache unter `root` mit
    /// [`DEFAULT_MAX_ENTRIES`]. Verzeichnis wird erst bei Bedarf angelegt.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_owned(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    /// Setzt das LRU-Limit (Anzahl Einträge). `0` = keine Retention
    /// (Lookups missen immer; `store` schreibt, `gc` räumt sofort ab).
    pub fn with_max_entries(mut self, n: usize) -> Self {
        self.max_entries = n;
        self
    }

    /// Cache-Wurzel (`target/sail-cache`-artig).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Aktives LRU-Limit.
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    fn entry_dir(&self, shard_hash: &str) -> PathBuf {
        self.root.join(shard_hash)
    }

    /// `true`, wenn der Eintrag existiert, der Fingerprint passt und ein
    /// Output vorliegt. Schreibt nichts, fasst nichts an.
    pub fn is_fresh(&self, shard_hash: &str, expected: &ShardFingerprint) -> bool {
        if check_id(shard_hash).is_err() {
            return false;
        }
        is_fresh_inner(&self.entry_dir(shard_hash), expected)
    }

    /// Liest den gecachten Output bei Fingerprint-Treffer.
    /// Aktualisiert die LRU-Position (best effort, winzige Datei).
    pub fn lookup(&self, shard_hash: &str, expected: &ShardFingerprint) -> Option<Vec<u8>> {
        if check_id(shard_hash).is_err() {
            return None;
        }
        let dir = self.entry_dir(shard_hash);
        let stored = fs::read(dir.join(FINGERPRINT_FILE)).ok()?;
        if stored != expected.as_str().as_bytes() {
            return None;
        }
        let out = fs::read(dir.join(OUTPUT_FILE)).ok()?;
        // LRU-Touch: Fingerprint mit identischem Inhalt neu schreiben
        // (mtime-Bump). Fehler ignorieren — der Treffer zählt trotzdem.
        let _ = atomic_write(&dir, FINGERPRINT_FILE, &stored);
        Some(out)
    }

    /// Legt den Output ab.
    ///
    /// * `Ok(false)` — Eintrag frisch: **nichts geschrieben**, keine Datei
    ///   angefasst (Zweitlauf-Skip).
    /// * `Ok(true)` — neu geschrieben (atomar), danach `gc()` gelaufen.
    pub fn store(
        &self,
        shard_hash: &str,
        fp: &ShardFingerprint,
        output: &[u8],
    ) -> io::Result<bool> {
        check_id(shard_hash)?;
        let dir = self.entry_dir(shard_hash);
        if is_fresh_inner(&dir, fp) {
            return Ok(false);
        }
        fs::create_dir_all(&dir)?;
        atomic_write(&dir, OUTPUT_FILE, output)?;
        atomic_write(&dir, FINGERPRINT_FILE, fp.as_str().as_bytes())?;
        self.gc()?;
        Ok(true)
    }

    /// Entsorgt Einträge über dem LRU-Limit (älteste zuerst).
    /// Gibt die Anzahl entfernter Einträge zurück.
    pub fn gc(&self) -> io::Result<usize> {
        // list_entries: neueste zuerst → skip behält die neuesten.
        let entries = self.list_entries()?;
        let mut removed = 0usize;
        for (_, name) in entries.iter().skip(self.max_entries) {
            if fs::remove_dir_all(self.root.join(name)).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Anzahl gecachter Einträge.
    pub fn entry_count(&self) -> io::Result<usize> {
        Ok(self.list_entries()?.len())
    }

    /// `(mtime, name)` aller gültigen Einträge, neueste zuerst.
    fn list_entries(&self) -> io::Result<Vec<(SystemTime, String)>> {
        let read = match fs::read_dir(&self.root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            r => r?,
        };
        let mut entries = Vec::new();
        for ent in read {
            let ent = ent?;
            if !ent.file_type()?.is_dir() {
                continue;
            }
            let name = ent.file_name().to_string_lossy().into_owned();
            if check_id(&name).is_err() {
                continue;
            }
            entries.push((newest_mtime(&ent.path()), name));
        }
        // Neueste zuerst; bei Gleichstand Name für Determinismus.
        entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        Ok(entries)
    }
}

/// Prüft, ob `shard_hash` eine sichere Pfad-Komponente ist.
fn check_id(shard_hash: &str) -> io::Result<()> {
    let bad = shard_hash.is_empty()
        || shard_hash.len() > MAX_ID_LEN
        || shard_hash == "."
        || shard_hash == ".."
        || !shard_hash
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    if bad {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid shard id: {shard_hash:?}"),
        ))
    } else {
        Ok(())
    }
}

fn is_fresh_inner(dir: &Path, expected: &ShardFingerprint) -> bool {
    match fs::read(dir.join(FINGERPRINT_FILE)) {
        Ok(stored) if stored == expected.as_str().as_bytes() => {}
        _ => return false,
    }
    dir.join(OUTPUT_FILE).is_file()
}

/// Neueste mtime aus Verzeichnis + Inhaltsdateien (LRU-Stempel).
fn newest_mtime(dir: &Path) -> SystemTime {
    let mut best = SystemTime::UNIX_EPOCH;
    for p in [dir, &dir.join(OUTPUT_FILE), &dir.join(FINGERPRINT_FILE)] {
        if let Ok(md) = fs::metadata(p) {
            if let Ok(m) = md.modified() {
                if m > best {
                    best = m;
                }
            }
        }
    }
    best
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Schreibt `bytes` atomar nach `dir/name`: Tempfile im selben Verzeichnis
/// (gleicher Mount → Rename ist atomar) + fsync + Rename. Reste werden bei
/// Fehlern best effort entfernt.
fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        "{}{}-{n}-{nanos}-{name}",
        TMP_PREFIX,
        std::process::id()
    ));
    let res = (|| -> io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dir.join(name))?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
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
            let dir = std::env::temp_dir()
                .join(format!("sail-cache-test-{}-{n}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("testdir anlegen");
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sleep_ms(ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    /// (Dateiname, Bytes, mtime) aller Dateien im Eintragsverzeichnis.
    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>, SystemTime)> {
        let mut out = Vec::new();
        for ent in fs::read_dir(dir).expect("read_dir") {
            let ent = ent.expect("dir entry");
            let p = ent.path();
            if p.is_file() {
                let m = fs::metadata(&p)
                    .expect("metadata")
                    .modified()
                    .expect("mtime");
                out.push((
                    ent.file_name().to_string_lossy().into_owned(),
                    fs::read(&p).expect("read"),
                    m,
                ));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Keine Tempfile-Reste in einem Eintragsverzeichnis.
    fn assert_no_tmps(dir: &Path) {
        for ent in fs::read_dir(dir).expect("read_dir") {
            let name = ent.expect("dir entry").file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with(TMP_PREFIX),
                "tempfile-Rest gefunden: {name}"
            );
        }
    }

    #[test]
    fn fingerprint_stabil_und_sensitiv() {
        let a = fingerprint(b"shard-bytes", "pass-v1");
        let b = fingerprint(b"shard-bytes", "pass-v1");
        assert_eq!(a, b);
        assert_ne!(a, fingerprint(b"shard-bytes-x", "pass-v1"));
        assert_ne!(a, fingerprint(b"shard-bytes", "pass-v2"));
        // Format: 16 Kleinbuchstaben-Hex.
        assert_eq!(a.as_str().len(), 16);
        assert!(a.as_str().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            a,
            a.as_str().parse::<ShardFingerprint>().expect("roundtrip")
        );
        assert!("ABCDEF1234567890".parse::<ShardFingerprint>().is_err());
        assert!("xyz".parse::<ShardFingerprint>().is_err());
    }

    #[test]
    fn store_lookup_roundtrip() {
        let t = TestDir::new("roundtrip");
        let cache = DiskCache::new(t.path());
        let fp = fingerprint(b"input", "v1");
        assert!(!cache.is_fresh("shard1", &fp));
        assert!(cache.lookup("shard1", &fp).is_none());
        assert!(cache.store("shard1", &fp, b"compiled").expect("store"));
        assert!(cache.is_fresh("shard1", &fp));
        assert_eq!(
            cache.lookup("shard1", &fp).as_deref(),
            Some(b"compiled".as_slice())
        );
        assert_eq!(cache.entry_count().expect("count"), 1);
        assert_no_tmps(&t.path().join("shard1"));
    }

    #[test]
    fn zweitlauf_ohne_aenderung_schreibt_nichts() {
        let t = TestDir::new("zweitlauf");
        let cache = DiskCache::new(t.path());
        let fp = fingerprint(b"stabiler-input", "v1");
        assert!(cache.store("shard9", &fp, b"output-v1").expect("erstlauf"));

        let dir = t.path().join("shard9");
        let before = snapshot(&dir);
        assert_eq!(before.len(), 2); // genau output + fingerprint

        // Zweitlauf: gleiche Eingabe (anderer Output-Puffer, dürfte nie landen).
        assert!(!cache
            .store("shard9", &fp, b"output-NEU-darf-nie-landen")
            .expect("zweitlauf"));
        // Auch reines Fresh-Prüfen schreibt nichts.
        assert!(cache.is_fresh("shard9", &fp));

        let after = snapshot(&dir);
        assert_eq!(before, after, "Zweitlauf hat Dateien angefasst");
        // Alter Output unangetastet, kein Tempfile-Rest.
        assert_eq!(
            cache.lookup("shard9", &fp).as_deref(),
            Some(b"output-v1".as_slice())
        );
        assert_no_tmps(&dir);
    }

    #[test]
    fn geaenderter_input_schreibt_neu() {
        let t = TestDir::new("rewrite");
        let cache = DiskCache::new(t.path());
        let fp1 = fingerprint(b"alt", "v1");
        let fp2 = fingerprint(b"neu", "v1");
        assert!(cache.store("s", &fp1, b"o1").expect("s1"));
        assert!(!cache.store("s", &fp1, b"o1").expect("s1-dup"));
        assert!(cache.store("s", &fp2, b"o2").expect("s2"));
        assert_eq!(cache.lookup("s", &fp2).as_deref(), Some(b"o2".as_slice()));
        assert!(cache.lookup("s", &fp1).is_none());
        assert!(!cache.is_fresh("s", &fp1));
    }

    #[test]
    fn lookup_miss_faelle() {
        let t = TestDir::new("miss");
        let cache = DiskCache::new(t.path());
        let fp = fingerprint(b"x", "v1");
        let other = fingerprint(b"y", "v1");
        assert!(cache.lookup("nix", &fp).is_none());
        assert!(cache.store("s", &fp, b"o").expect("store"));
        // Falscher Fingerprint → Miss.
        assert!(cache.lookup("s", &other).is_none());
        // Korrupte Fingerprint-Datei → Miss + nicht fresh.
        fs::write(t.path().join("s").join(FINGERPRINT_FILE), b"mull").expect("korrupt");
        assert!(!cache.is_fresh("s", &fp));
        assert!(cache.lookup("s", &fp).is_none());
        // Fehlender Output → Miss + nicht fresh.
        assert!(cache.store("s", &fp, b"o").expect("re-store"));
        fs::remove_file(t.path().join("s").join(OUTPUT_FILE)).expect("output weg");
        assert!(!cache.is_fresh("s", &fp));
        assert!(cache.lookup("s", &fp).is_none());
        // Kaputter Eintrag erholt sich per store.
        assert!(cache.store("s", &fp, b"o-neu").expect("heal"));
        assert_eq!(cache.lookup("s", &fp).as_deref(), Some(b"o-neu".as_slice()));
    }

    #[test]
    fn gc_lru_limit_mit_touch() {
        let t = TestDir::new("lru");
        let cache = DiskCache::new(t.path()).with_max_entries(3);
        for name in ["e1", "e2", "e3", "e4"] {
            let fp = fingerprint(name.as_bytes(), "v1");
            assert!(cache.store(name, &fp, b"o").expect("store"));
            sleep_ms(15);
        }
        // e1 (ältester) entsorgt, Rest da.
        assert_eq!(cache.entry_count().expect("count"), 3);
        let fp1 = fingerprint(b"e1", "v1");
        assert!(!cache.is_fresh("e1", &fp1));
        assert!(cache.is_fresh("e4", &fingerprint(b"e4", "v1")));

        // lookup(e2) bumpt e2 → nächster Store entsorgt e3 statt e2.
        let fp2 = fingerprint(b"e2", "v1");
        assert!(cache.lookup("e2", &fp2).is_some());
        sleep_ms(15);
        let fp5 = fingerprint(b"e5", "v1");
        assert!(cache.store("e5", &fp5, b"o").expect("store5"));
        assert_eq!(cache.entry_count().expect("count2"), 3);
        assert!(cache.is_fresh("e2", &fp2), "e2 wurde trotz Touch entsorgt");
        assert!(
            !cache.is_fresh("e3", &fingerprint(b"e3", "v1")),
            "e3 hätte gehen müssen"
        );
        assert!(cache.is_fresh("e5", &fp5));

        // Explizites gc ohne Überhang entfernt nichts.
        assert_eq!(cache.gc().expect("gc"), 0);
    }

    #[test]
    fn ungueltige_ids_abgewiesen() {
        let t = TestDir::new("ids");
        let cache = DiskCache::new(t.path());
        let fp = fingerprint(b"x", "v1");
        for bad in ["", ".", "..", "a/b", "a\\b", "../x", "a:b", "mit leer"] {
            assert!(cache.store(bad, &fp, b"o").is_err(), "store({bad:?})");
            assert!(cache.lookup(bad, &fp).is_none());
            assert!(!cache.is_fresh(bad, &fp));
        }
        assert_eq!(cache.entry_count().expect("count"), 0);
    }

    #[test]
    fn limit_null_keine_retention() {
        let t = TestDir::new("zero");
        let cache = DiskCache::new(t.path()).with_max_entries(0);
        let fp = fingerprint(b"x", "v1");
        assert!(cache.store("s", &fp, b"o").expect("store"));
        assert_eq!(cache.entry_count().expect("count"), 0);
        assert!(cache.lookup("s", &fp).is_none());
    }
}
