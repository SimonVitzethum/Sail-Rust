//! Lane 12 — OoC-Fenster und Profiler-Anschluss im Pipeline-Treiber.
//!
//! Verdrahtet Out-of-core in die Treiber-Skizze (`e2e.rs`): Barriere um die
//! Rewrite-Pässe (höchstens ein heißer Shard), Warm-Layer für fertige Shards,
//! Memo-Spills für Solver-Anfragen, Cross-Shard-Wissen nur als Summaries.
//!
//! # Herkunft — nichts gedoppelt
//!
//! Die kanonische OoC-Implementierung ist `sail-ooc` (Lane 60). Dieser Treiber
//! hält nur die Ein-Shard-Barriere und die Zähler; Storage, Content-Hashes
//! und Schranken kommen aus `sail-ooc` und sind hier verankert (keine
//! Zweit-Literale):
//!
//! | Hook hier | Kanonisch dort |
//! |---|---|
//! | [`OocDriver`] (Ein-Shard-Barriere im Treiber) | `sail_ooc::PhaseBarrier`/`HotToken`/`SealReceipt` (`barrier.rs`) |
//! | [`OocHooks::warm_spill`]/[`OocHooks::warm_fetch`] (Key + Bytes) | `sail_ooc::WarmLayer` (`warm.rs`: `<hex>.blob`, atomar, Budget) |
//! | [`OocHooks::memo_lookup`]/[`OocHooks::memo_store`] (Query-Key + Wert) | `sail_ooc::MemoSpills` über `sail_cache::DiskCache` (`memo.rs`) |
//! | [`OocHooks::summary_insert`] (kleine Payload + Body-Referenz) | `sail_ooc::SummaryTable`/`SummaryRecord` (`summary.rs`, ≤ 64 KiB) |
//! | Content-Keys (`content_key`/`query_key`, opake Strings) | `sail_ooc::ContentId` (blake3, längen-präfixiert, `id.rs`) |
//!
//! Dieses Modul hasht NICHT selbst (kein blake3-Dep, keine Zähler-IDs — E4
//! verlangt stabile Content-Hashes, Zähler würden jeden Lauf den Cache
//! invalidieren). Schlüssel berechnen die Aufrufer über `sail-ooc`
//! (`ContentId`); der Treiber trägt sie nur als opake Strings durch. Die
//! file-backed Bindung an `sail-ooc` erfolgt über `run_with_ooc` in
//! `e2e.rs` — der Treiber ändert sich dadurch nicht.
//!
//! # E4-Regeln im Treiber
//!
//! - Heiß (RAM): genau ein Shard — [`OocDriver::begin_hot`] verweigert einen
//!   zweiten (`AlreadyHot`); weiter geht es erst nach `seal_hot`/`abort_hot`.
//! - Warm: fertige Shards via [`OocHooks::warm_spill`]; Drop-Punkt
//!   [`OocDriver::finish`] (`warm_evict_all`, Lane-37-Policy).
//! - Kalt: Solver-Memos via [`OocDriver::memo_prove`] (Lookup → Rechnen →
//!   Store; run-übergreifend wiederverwendbar, Domäne gehört in den Key).
//! - Cross-Shard: nur [`OocHooks::summary_insert`] (≤ [`MAX_SUMMARY_BYTES`]);
//!   Bodies bleiben auf Platte und werden per `content_key` referenziert.
//!
//! E3: keine Builds/Tests aus Lane-Agents — Tests liegen als Code bei
//! (`#[cfg(test)]`); Verifikation später zentral per Test-Rig.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Anker an `sail_ooc::MAX_SUMMARY_BYTES` (Herkunft s. Modul-Doku):
/// harte Obergrenze je Summary-Eintrag (RAM-Schutz). Bodies gehören auf Platte.
pub const MAX_SUMMARY_BYTES: usize = sail_ooc::MAX_SUMMARY_BYTES;

/// Pass-Version dieses Treiber-Standes. Geht kanonisch in jeden Content-Key
/// ein (`sail_ooc::ContentId::shard_content`); hier nur dokumentiert, weil
/// dieser Treiber noch Stub-Bytes spillt (s. `e2e.rs`).
pub const OOC_PASS_VERSION: &str = "sail-cli-ooc-wire-v1";

/// Domänen-Präfix für Solver-Memo-Keys: kanonisch
/// `sail_ooc::MEMO_DOMAIN_PREFIX` plus Domäne (`"solver/z3-v1"`).
pub const MEMO_DOMAIN_SOLVER_Z3: &str = "ooc/memo-v1/solver/z3-v1";

/// Fehler der OoC-Verdrahtung (Treiber-Seite; Storage-Fehler der kanonischen
/// Backends werden als `Io`/`WarmBudget`/… durchgereicht).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OocError {
    /// Bereits ein Shard heiß; erst `seal_hot`/`abort_hot`, dann weiter.
    AlreadyHot {
        /// Der noch heiße Shard.
        current: String,
    },
    /// `seal_hot`/`abort_hot` für einen Shard, der nicht heiß ist.
    StaleToken {
        /// Shard des Aufrufs.
        shard: String,
    },
    /// Warm-Budget erschöpft — erst `warm_evict_all`/`finish`, dann erneut
    /// spillen (explizite Drop-Punkte, E4; kein implizites Räumen).
    WarmBudget {
        /// Live gehaltene Bytes.
        live: u64,
        /// Zusätzlich angefragte Bytes.
        add: u64,
        /// Konfiguriertes Budget.
        max: u64,
    },
    /// Gleiche ID, anderer Inhalt (Korruption/Domänenfehler — IDs sind
    /// Content-Hashes und damit unveränderlich).
    ContentMismatch {
        /// Betroffener Key.
        key: String,
    },
    /// Summary sprengt [`MAX_SUMMARY_BYTES`] (Bodies gehören auf Platte).
    SummaryTooLarge {
        /// Shard der abgelehnten Summary.
        shard: String,
        /// Gemessene Payload-Größe.
        bytes: usize,
    },
    /// Zugrunde liegender IO-/Storage-Fehler des Backends.
    Io(String),
}

impl fmt::Display for OocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyHot { current } => {
                write!(f, "OoC-Barriere: Shard {current} noch heiß (erst seal/abort)")
            }
            Self::StaleToken { shard } => {
                write!(f, "OoC-Barriere: Shard {shard} ist nicht heiß")
            }
            Self::WarmBudget { live, add, max } => write!(
                f,
                "Warm-Budget erschöpft (live {live} + {add} > max {max}; erst evict)"
            ),
            Self::ContentMismatch { key } => write!(
                f,
                "Warm-Layer: Key {key} belegt, Inhalt weicht ab (Hash-Kollision oder Domänenfehler)"
            ),
            Self::SummaryTooLarge { shard, bytes } => write!(
                f,
                "Shard-Summary von {shard} zu groß ({bytes} > {MAX_SUMMARY_BYTES} Bytes; Bodies gehören auf Platte)"
            ),
            Self::Io(msg) => write!(f, "OoC-Storage: {msg}"),
        }
    }
}

impl std::error::Error for OocError {}

/// Zähler des OoC-Durchlaufs (wandern in `e2e::Outcome`, für Verbose-Logs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OocStats {
    /// Versiegelte Shards (`seal_hot` + Spill + Summary).
    pub sealed: u64,
    /// Abgebrochene heiße Phasen (ohne Versiegelung, kein Cache-Eintrag).
    pub evicted: u64,
    /// Warm-Spill-Vorgänge.
    pub warm_spills: u64,
    /// Erfolgreiche Warm-Nachlader (`warm_recall`).
    pub warm_recalls: u64,
    /// Solver-Memo-Treffer (kein Rechnen nötig).
    pub memo_hits: u64,
    /// Solver-Memo-Fehlschläge (berechnet + gespillt).
    pub memo_misses: u64,
    /// Eingepflegte Cross-Shard-Summaries.
    pub summaries: u64,
}

/// OoC-Backend-Schnittstelle des Treibers.
///
/// Schlüssel (`content_key`, `query_key`) sind opake Content-Hash-Strings —
/// kanonisch `sail_ooc::ContentId`-Hex (Herkunft s. Modul-Doku). Der Treiber
/// (`OocDriver`) erzwingt die Ein-Shard-Barriere selbst; das Backend hält nur
/// Storage (warm/kalt/Summaries) und muss seinerseits keine Barriere kennen.
pub trait OocHooks {
    /// Öffnet die heiße Phase für `shard` (Backend-Seite, z. B. Buchhaltung).
    fn begin(&mut self, shard: &str) -> Result<(), OocError>;
    /// Versiegelt `shard`; `content_key` identifiziert die serialisierten
    /// Bytes (`bytes_len` nur zur Buchhaltung).
    fn seal(&mut self, shard: &str, content_key: &str, bytes_len: usize) -> Result<(), OocError>;
    /// Verwirft die heiße Phase ohne Versiegelung (Abbruchpfad).
    fn evict(&mut self, shard: &str);
    /// Legt fertige Shard-Bytes warm ab (idempotent bei gleichem Inhalt).
    fn warm_spill(&mut self, content_key: &str, bytes: &[u8]) -> Result<(), OocError>;
    /// Lädt warme Bytes nach (`None` = nie gespillt).
    fn warm_fetch(&mut self, content_key: &str) -> Option<Vec<u8>>;
    /// Gibt alle live gehaltenen Warm-Referenzen frei (Dateien bleiben;
    /// Barrieren-Drop-Punkt, Lane-37-Policy).
    fn warm_evict_all(&mut self);
    /// Kalter Memo-Lookup (`None` = Miss).
    fn memo_lookup(&self, query_key: &str) -> Option<Vec<u8>>;
    /// Kalter Memo-Store (unveränderliche Werte; Erster gewinnt).
    fn memo_store(&mut self, query_key: &str, value: &[u8]);
    /// Pflegt eine Cross-Shard-Summary ein (klein; Body nur per
    /// `content_key` referenziert, nie eingebettet).
    fn summary_insert(
        &mut self,
        shard: &str,
        content_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), OocError>;
    /// Entfernt die Summary (Shard endgültig fertig → RAM frei).
    fn summary_remove(&mut self, shard: &str) -> bool;
}

/// No-Op-Backend: nur Barrierenzählung, kein Storage.
///
/// Damit läuft der Treiberpfad (`e2e::run`) ohne Dateisystem-Effekte; die
/// file-backed Bindung an `sail-ooc` erfolgt über `run_with_ooc`.
#[derive(Debug, Default)]
pub struct NoopHooks;

impl OocHooks for NoopHooks {
    fn begin(&mut self, _shard: &str) -> Result<(), OocError> {
        Ok(())
    }
    fn seal(&mut self, _shard: &str, _content_key: &str, _bytes_len: usize) -> Result<(), OocError> {
        Ok(())
    }
    fn evict(&mut self, _shard: &str) {}
    fn warm_spill(&mut self, _content_key: &str, _bytes: &[u8]) -> Result<(), OocError> {
        Ok(())
    }
    fn warm_fetch(&mut self, _content_key: &str) -> Option<Vec<u8>> {
        None
    }
    fn warm_evict_all(&mut self) {}
    fn memo_lookup(&self, _query_key: &str) -> Option<Vec<u8>> {
        None
    }
    fn memo_store(&mut self, _query_key: &str, _value: &[u8]) {}
    fn summary_insert(
        &mut self,
        _shard: &str,
        _content_key: &str,
        _payload: Vec<u8>,
    ) -> Result<(), OocError> {
        Ok(())
    }
    fn summary_remove(&mut self, _shard: &str) -> bool {
        false
    }
}

/// In-Memory-Testdoppel (E3: Tests als Code, läuft später im Test-Rig):
/// prüft Budget- und Größen-Schranken wie die kanonischen Backends und
/// protokolliert die Hook-Reihenfolge in `events`.
#[derive(Debug)]
pub struct RecordingHooks {
    /// Hook-Protokoll (`"begin:<shard>"`, `"seal:<shard>=<key>"`,
    /// `"spill:<key> <n>B"`, `"summary:<shard>=<key> <n>B"`, `"evict:…"`,
    /// `"evict-all"`, `"memo:<key> <n>B"`, `"fetch:<key> hit|miss"`).
    pub events: Vec<String>,
    /// Warm abgelegte Bytes (Datei-Ersatz; `evict_all` räumt nur live).
    pub warm_bytes: HashMap<String, Vec<u8>>,
    live: HashSet<String>,
    live_bytes: u64,
    /// Budget für live gehaltene Bytes (`u64::MAX` = unbegrenzt).
    pub max_live_bytes: u64,
    memos: HashMap<String, Vec<u8>>,
    summaries: HashMap<String, (String, Vec<u8>)>,
}

impl Default for RecordingHooks {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            warm_bytes: HashMap::new(),
            live: HashSet::new(),
            live_bytes: 0,
            max_live_bytes: u64::MAX,
            memos: HashMap::new(),
            summaries: HashMap::new(),
        }
    }
}

impl RecordingHooks {
    /// Setzt das Live-Budget (Drop-Punkte explizit testen).
    #[must_use]
    pub fn with_max_live_bytes(mut self, n: u64) -> Self {
        self.max_live_bytes = n;
        self
    }

    /// Live gehaltene Bytes (RAM-Sicht).
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.live_bytes
    }

    /// Anzahl gehaltener Summaries (RAM-Sicht, ein Eintrag pro Shard).
    #[must_use]
    pub fn summary_count(&self) -> usize {
        self.summaries.len()
    }

    fn event(&mut self, e: String) {
        self.events.push(e);
    }
}

impl OocHooks for RecordingHooks {
    fn begin(&mut self, shard: &str) -> Result<(), OocError> {
        self.event(format!("begin:{shard}"));
        Ok(())
    }

    fn seal(&mut self, shard: &str, content_key: &str, bytes_len: usize) -> Result<(), OocError> {
        self.event(format!("seal:{shard}={content_key} {bytes_len}B"));
        Ok(())
    }

    fn evict(&mut self, shard: &str) {
        self.event(format!("evict:{shard}"));
    }

    fn warm_spill(&mut self, content_key: &str, bytes: &[u8]) -> Result<(), OocError> {
        if let Some(held) = self.warm_bytes.get(content_key) {
            if held.as_slice() == bytes {
                if self.live.insert(content_key.to_string()) {
                    self.live_bytes = self.live_bytes.saturating_add(bytes.len() as u64);
                }
                self.event(format!("spill:{content_key} known"));
                return Ok(());
            }
            return Err(OocError::ContentMismatch {
                key: content_key.to_string(),
            });
        }
        let add = bytes.len() as u64;
        if self.live_bytes.saturating_add(add) > self.max_live_bytes {
            return Err(OocError::WarmBudget {
                live: self.live_bytes,
                add,
                max: self.max_live_bytes,
            });
        }
        self.warm_bytes.insert(content_key.to_string(), bytes.to_vec());
        self.live.insert(content_key.to_string());
        self.live_bytes = self.live_bytes.saturating_add(add);
        self.event(format!("spill:{content_key} {}B", bytes.len()));
        Ok(())
    }

    fn warm_fetch(&mut self, content_key: &str) -> Option<Vec<u8>> {
        match self.warm_bytes.get(content_key).cloned() {
            Some(bytes) => {
                if self.live.insert(content_key.to_string()) {
                    self.live_bytes = self.live_bytes.saturating_add(bytes.len() as u64);
                }
                self.event(format!("fetch:{content_key} hit"));
                Some(bytes)
            }
            None => {
                self.event(format!("fetch:{content_key} miss"));
                None
            }
        }
    }

    fn warm_evict_all(&mut self) {
        self.live.clear();
        self.live_bytes = 0;
        self.event("evict-all".to_string());
    }

    fn memo_lookup(&self, query_key: &str) -> Option<Vec<u8>> {
        self.memos.get(query_key).cloned()
    }

    fn memo_store(&mut self, query_key: &str, value: &[u8]) {
        self.event(format!("memo:{query_key} {}B", value.len()));
        self.memos.insert(query_key.to_string(), value.to_vec());
    }

    fn summary_insert(
        &mut self,
        shard: &str,
        content_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), OocError> {
        if payload.len() > MAX_SUMMARY_BYTES {
            return Err(OocError::SummaryTooLarge {
                shard: shard.to_string(),
                bytes: payload.len(),
            });
        }
        self.event(format!("summary:{shard}={content_key} {}B", payload.len()));
        self.summaries
            .insert(shard.to_string(), (content_key.to_string(), payload));
        Ok(())
    }

    fn summary_remove(&mut self, shard: &str) -> bool {
        let had = self.summaries.remove(shard).is_some();
        self.event(format!("summary-rm:{shard} {had}"));
        had
    }
}

/// OoC-Treiber: erzwingt die Ein-Shard-Barriere um Rewrite-Pässe und fächert
/// auf Warm-Spills, Memo-Spills und Summaries auf.
///
/// Der Treiber hält die Barriere selbst (Typzustand zur Laufzeit: `hot` ist
/// höchstens ein Shard) — unabhängig vom Backend, sodass auch die künftige
/// `sail-ooc`-Bindung (`PhaseBarrier` auf Storage-Seite) denselben Vertrag
/// sieht. Generisch über `H: OocHooks`, damit Tests ohne Dateisystem laufen.
#[derive(Debug)]
pub struct OocDriver<H> {
    hooks: H,
    hot: Option<String>,
    stats: OocStats,
}

impl<H: OocHooks> OocDriver<H> {
    /// Neuer Treiber über `hooks` (nichts heiß).
    #[must_use]
    pub fn new(hooks: H) -> Self {
        Self {
            hooks,
            hot: None,
            stats: OocStats::default(),
        }
    }

    /// Backend-Einsicht (z. B. Assertions im Test-Rig).
    #[must_use]
    pub fn hooks(&self) -> &H {
        &self.hooks
    }

    /// Backend-Zugriff (z. B. `summary_remove` am Lebensende eines Shards).
    pub fn hooks_mut(&mut self) -> &mut H {
        &mut self.hooks
    }

    /// Zähler des Durchlaufs.
    #[must_use]
    pub fn stats(&self) -> &OocStats {
        &self.stats
    }

    /// Aktuell heißer Shard (genau einer oder keiner).
    #[must_use]
    pub fn hot(&self) -> Option<&str> {
        self.hot.as_deref()
    }

    /// Gibt das Backend zurück (nach `finish` inspizierbar).
    #[must_use]
    pub fn into_hooks(self) -> H {
        self.hooks
    }

    /// Öffnet die heiße Phase für `shard`. Fehler, wenn noch einer heiß ist.
    pub fn begin_hot(&mut self, shard: &str) -> Result<(), OocError> {
        if let Some(current) = &self.hot {
            return Err(OocError::AlreadyHot {
                current: current.clone(),
            });
        }
        self.hooks.begin(shard)?;
        self.hot = Some(shard.to_string());
        Ok(())
    }

    /// Versiegelt die heiße Phase: `content_key` ist der stabile Content-Hash
    /// der serialisierten Bytes, `bytes_len` deren Länge. Danach ist der
    /// Shard verworfen (RAM frei).
    pub fn seal_hot(
        &mut self,
        shard: &str,
        content_key: &str,
        bytes_len: usize,
    ) -> Result<(), OocError> {
        match &self.hot {
            Some(current) if current == shard => {}
            _ => {
                return Err(OocError::StaleToken {
                    shard: shard.to_string(),
                });
            }
        }
        self.hooks.seal(shard, content_key, bytes_len)?;
        self.hot = None;
        Ok(())
    }

    /// Verwirft die heiße Phase ohne Versiegelung (Abbruchpfad; kein
    /// Cache-Eintrag, keine Wiederverwendbarkeit).
    pub fn abort_hot(&mut self, shard: &str) -> Result<(), OocError> {
        match &self.hot {
            Some(current) if current == shard => {}
            _ => {
                return Err(OocError::StaleToken {
                    shard: shard.to_string(),
                });
            }
        }
        self.hooks.evict(shard);
        self.hot = None;
        self.stats.evicted += 1;
        Ok(())
    }

    /// Ein Shard vollständig: Barriere → `rewrite` (heiß, nur dieser Shard in
    /// RAM) → Seal → Warm-Spill → Summary. Die Größen-Schranke wird VOR der
    /// Barriere geprüft, damit ein zu großer Eintrag keinen Slot belegt.
    pub fn run_shard(
        &mut self,
        shard: &str,
        content_key: &str,
        rewrite: impl FnOnce() -> Vec<u8>,
        summary_payload: Vec<u8>,
    ) -> Result<(), OocError> {
        if summary_payload.len() > MAX_SUMMARY_BYTES {
            return Err(OocError::SummaryTooLarge {
                shard: shard.to_string(),
                bytes: summary_payload.len(),
            });
        }
        self.begin_hot(shard)?;
        let bytes = rewrite();
        let bytes_len = bytes.len();
        // Seal-Fehler lassen die Barriere nicht lecken (Lane 126): heisse
        // Phase verwerfen (`evicted`), dann Fehler durchreichen.
        if let Err(e) = self.seal_hot(shard, content_key, bytes_len) {
            let _ = self.abort_hot(shard);
            return Err(e);
        }
        self.hooks.warm_spill(content_key, &bytes)?;
        self.hooks
            .summary_insert(shard, content_key, summary_payload)?;
        self.stats.sealed += 1;
        self.stats.warm_spills += 1;
        self.stats.summaries += 1;
        Ok(())
    }

    /// Memo-Spills für Solver (z. B. Z3-Verdicts): Treffer ohne Rechnen,
    /// sonst `compute` + Store. `query_key` trägt die Domäne
    /// (s. [`MEMO_DOMAIN_SOLVER_Z3`); Vertrag: Anfrage → Wert ist rein —
    /// bei Miss gewinnt der erstgespeicherte Wert (Determinismus).
    ///
    /// Kein Singleflight (Lane 126, Thundering Herd): Gleichzeitige gleiche
    /// Anfragen rechnen parallel, der Erstling gewinnt; Aufrufer mit teurem
    /// `compute` deduplizieren gleiche Flüge extern. Der Hook-Store ist
    /// backend-seitig infallibel; die file-backed Bindung (`sail-ooc`)
    /// zaehlt/meldet Store-Fehler dort (s. `OocDriverLoop::memo_prove_result`).
    pub fn memo_prove(
        &mut self,
        query_key: &str,
        compute: impl FnOnce() -> Vec<u8>,
    ) -> Vec<u8> {
        if let Some(hit) = self.hooks.memo_lookup(query_key) {
            self.stats.memo_hits += 1;
            return hit;
        }
        self.stats.memo_misses += 1;
        let value = compute();
        self.hooks.memo_store(query_key, &value);
        value
    }

    /// Warme Bytes nachladen (Shard-Streaming/Punktzugriff, kein wilder
    /// Random-Access). `None`, wenn nie gespillt.
    pub fn warm_recall(&mut self, content_key: &str) -> Option<Vec<u8>> {
        let hit = self.hooks.warm_fetch(content_key);
        if hit.is_some() {
            self.stats.warm_recalls += 1;
        }
        hit
    }

    /// Drop-Punkt am Phasenende (Lane-37-Policy): gibt alle live gehaltenen
    /// Warm-Referenzen frei; Dateien/Memos/Summaries bleiben bestehen.
    ///
    /// Heisser Shard (Lane 126): Ist beim Aufruf noch ein Shard heiss, wird er
    /// wie ein Abbruch verworfen (`evicted` gezaehlt, kein Cache-Eintrag) —
    /// `finish` leckt die Barriere nie.
    pub fn finish(&mut self) {
        if let Some(shard) = self.hot.take() {
            self.hooks.evict(&shard);
            self.stats.evicted += 1;
        }
        self.hooks.warm_evict_all();
    }
}

/// Begrenztes Definitions-Fenster (Lane 12): höchstens `max_in_flight`
/// materialisierte Definitionen und höchstens `max_live_bytes` live
/// gehaltene Blob-Bytes. Wer das Budget sprengen würde, bekommt
/// [`OocError::WarmBudget`] und muss erst einen expliziten Drop-Punkt
/// (`release`/`release_all`) erreichen — kein implizites Räumen.
#[derive(Debug)]
pub struct OocWindow {
    in_flight: usize,
    max_in_flight: usize,
    live_bytes: u64,
    max_live_bytes: u64,
    /// Explizite Drop-Punkte (verbrauchte Fenster-Slots).
    pub evictions: u64,
}

impl OocWindow {
    /// Neues Fenster. `max_in_flight == 0` oder ein Null-Budget lehnt jede
    /// Aufnahme ab (Aufrufer müssen dann erst Kapazität schaffen).
    #[must_use]
    pub fn new(max_in_flight: usize, max_live_bytes: u64) -> Self {
        Self {
            in_flight: 0,
            max_in_flight,
            live_bytes: 0,
            max_live_bytes,
            evictions: 0,
        }
    }

    /// Belegt einen Fenster-Slot für `bytes` Blob-Bytes.
    pub fn acquire(&mut self, bytes: u64) -> Result<(), OocError> {
        if self.in_flight >= self.max_in_flight {
            return Err(OocError::WarmBudget {
                live: self.in_flight as u64,
                add: 1,
                max: self.max_in_flight as u64,
            });
        }
        if self.live_bytes.saturating_add(bytes) > self.max_live_bytes {
            return Err(OocError::WarmBudget {
                live: self.live_bytes,
                add: bytes,
                max: self.max_live_bytes,
            });
        }
        self.in_flight += 1;
        self.live_bytes = self.live_bytes.saturating_add(bytes);
        Ok(())
    }

    /// Gibt einen Slot mit `bytes` Blob-Bytes frei (verarbeitete Definition
    /// verwerfen — Bodies bleiben auf Platte, nur Summaries in RAM).
    pub fn release(&mut self, bytes: u64) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.live_bytes = self.live_bytes.saturating_sub(bytes);
        self.evictions += 1;
    }

    /// Drop-Punkt am Phasenende: alle Slots frei, Zähler bleiben (Dateien,
    /// Memos und Summaries bestehen weiter).
    pub fn release_all(&mut self) {
        self.in_flight = 0;
        self.live_bytes = 0;
    }

    /// Belegte Slots.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Live gehaltene Blob-Bytes.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.live_bytes
    }
}

/// Profiler-Anschluss (Lane 12): flache Zähler für `-dprofile`-Zeilen und
/// Verbose-Logs. Solver-Felder bleiben 0, solange `sail-ty` keine
/// kanonischen Beweiszähler publiziert; OOC-Felder sind echte
/// Durchlauf-Beobachtungen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileCounters {
    /// Z3-Beweisanfragen (heute 0 — kanonische Zähler noch nicht verdrahtet).
    pub smt_calls: u64,
    /// Z3-Wandzeit in Sekunden (heute 0.0 — s. `smt_calls`).
    pub smt_seconds: f64,
    /// Gestreamte Definitionen (OOC-Fenster).
    pub ooc_streamed: u64,
    /// Gestreamte Blob-Bytes (OOC-Fenster).
    pub ooc_streamed_bytes: u64,
    /// Explizite Fenster-Freigaben.
    pub ooc_evicted: u64,
}

impl ProfileCounters {
    /// Baut Zähler aus einem Treiber-Durchlauf (`smt_*` bleiben 0, s. oben).
    #[must_use]
    pub fn from_ooc(stats: &OocStats) -> Self {
        Self {
            smt_calls: 0,
            smt_seconds: 0.0,
            ooc_streamed: stats.sealed + stats.evicted,
            ooc_streamed_bytes: 0,
            ooc_evicted: stats.evicted,
        }
    }

    /// Stabile `key=value`-Felder für Profiler-/Verbose-Zeilen.
    #[must_use]
    pub fn log_fields(&self) -> String {
        format!(
            "smt_calls={} smt_s={:.6} ooc_streamed={} ooc_streamed_bytes={} ooc_evicted={}",
            self.smt_calls, self.smt_seconds, self.ooc_streamed, self.ooc_streamed_bytes,
            self.ooc_evicted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> OocDriver<RecordingHooks> {
        OocDriver::new(RecordingHooks::default())
    }

    #[test]
    fn barriere_laesst_nur_einen_heissen_shard_zu() {
        let mut d = driver();
        d.begin_hot("a").expect("erstes begin");
        assert_eq!(d.hot(), Some("a"));
        let err = d.begin_hot("b").expect_err("zweites begin");
        assert_eq!(
            err,
            OocError::AlreadyHot {
                current: "a".to_string()
            }
        );
        d.seal_hot("a", "key-a", 3).expect("seal");
        assert_eq!(d.hot(), None);
        // Danach darf der nächste beginnen (typchecken → verwerfen → nächster).
        d.begin_hot("b").expect("begin nach seal");
        d.seal_hot("b", "key-b", 1).expect("seal2");
        assert_eq!(d.hooks().events.iter().filter(|e| e.starts_with("begin:")).count(), 2);
    }

    #[test]
    fn stale_seal_und_abort_werden_abgewiesen() {
        let mut d = driver();
        assert_eq!(
            d.seal_hot("x", "k", 0).expect_err("seal ohne begin"),
            OocError::StaleToken {
                shard: "x".to_string()
            }
        );
        assert!(d.abort_hot("x").is_err());
        d.begin_hot("a").expect("begin");
        // Fremder Shard kann weder sealen noch aborten.
        assert!(d.seal_hot("b", "k", 0).is_err());
        assert!(d.abort_hot("b").is_err());
        d.abort_hot("a").expect("abort");
        assert_eq!(d.stats().evicted, 1);
        assert_eq!(d.hot(), None);
        // Nach Abort ist die Barriere frei.
        d.begin_hot("b").expect("begin nach abort");
    }

    #[test]
    fn run_shard_reihenfolge_und_summary_schranke() {
        let mut d = driver();
        d.run_shard("a", "key-a", || b"bytes".to_vec(), b"exp".to_vec())
            .expect("run");
        let ev = &d.hooks().events;
        let pos = |prefix: &str| ev.iter().position(|e| e.starts_with(prefix)).expect(prefix);
        assert!(pos("begin:a") < pos("seal:a="));
        assert!(pos("seal:a=") < pos("spill:key-a"));
        assert!(pos("spill:key-a") < pos("summary:a="));
        assert_eq!(d.stats().sealed, 1);
        // Zu große Summary belegt keinen Barrierenslot (Prüfung zuerst).
        let big = vec![0u8; MAX_SUMMARY_BYTES + 1];
        let err = d
            .run_shard("riesig", "key-r", || vec![], big)
            .expect_err("zu groß");
        assert!(matches!(err, OocError::SummaryTooLarge { .. }));
        assert_eq!(d.hot(), None);
        assert_eq!(d.stats().sealed, 1);
    }

    #[test]
    fn memo_prove_trifft_beim_zweiten_mal() {
        let mut d = driver();
        let mut calls = 0u32;
        let q = "ooc/memo-v1/solver/z3-v1:(atom x>0)";
        let v1 = d.memo_prove(q, || {
            calls += 1;
            b"sat".to_vec()
        });
        let v2 = d.memo_prove(q, || {
            calls += 1;
            b"anders".to_vec()
        });
        assert_eq!(calls, 1);
        assert_eq!(v1, b"sat");
        assert_eq!(v2, b"sat");
        assert_eq!((d.stats().memo_hits, d.stats().memo_misses), (1, 1));
    }

    #[test]
    fn warm_budget_mit_drop_punkt_freigeben() {
        let mut d = OocDriver::new(RecordingHooks::default().with_max_live_bytes(2));
        d.run_shard("a", "key-a", || b"aa".to_vec(), b"s".to_vec())
            .expect("passt");
        let err = d
            .run_shard("b", "key-b", || b"bb".to_vec(), b"s".to_vec())
            .expect_err("Budget");
        assert!(matches!(err, OocError::WarmBudget { .. }));
        // Barriere ist frei (seal war vor dem Spill), aber nichts live.
        assert_eq!(d.hot(), None);
        d.finish(); // Drop-Punkt: live Referenzen frei, Dateien bleiben.
        assert_eq!(d.hooks().live_bytes(), 0);
        d.run_shard("b", "key-b", || b"bb".to_vec(), b"s".to_vec())
            .expect("nach evict");
        // Nachladen von Platte geht (Datei blieb).
        d.finish();
        assert_eq!(d.warm_recall("key-a").as_deref(), Some(b"aa".as_slice()));
    }

    #[test]
    fn fremder_inhalt_unter_gleichem_key_wird_abgewiesen() {
        let mut d = driver();
        d.hooks_mut().warm_spill("k", b"original").expect("spill");
        let err = d
            .hooks_mut()
            .warm_spill("k", b"anderer inhalt")
            .expect_err("mismatch");
        assert!(matches!(err, OocError::ContentMismatch { .. }));
    }

    #[test]
    fn finish_mit_heissem_shard_verwirft_statt_zu_lecken() {
        let mut d = driver();
        d.begin_hot("a").expect("begin");
        d.finish();
        // Abbruch gezaehlt, Barriere frei, Backend benachrichtigt.
        assert_eq!(d.stats().evicted, 1);
        assert_eq!(d.hot(), None);
        assert!(d.hooks().events.iter().any(|e| e == "evict:a"));
        assert!(d.hooks().events.iter().any(|e| e == "evict-all"));
        // Danach laeuft der naechste normal weiter.
        d.run_shard("b", "key-b", || b"bb".to_vec(), b"s".to_vec())
            .expect("nach finish weiter");
        assert_eq!(d.stats().sealed, 1);
    }

    #[test]
    fn schranken_sind_an_sail_ooc_verankert() {
        assert_eq!(MAX_SUMMARY_BYTES, sail_ooc::MAX_SUMMARY_BYTES);
        assert!(
            MEMO_DOMAIN_SOLVER_Z3.starts_with(sail_ooc::MEMO_DOMAIN_PREFIX),
            "Memo-Domäne muss unter dem kanonischen Präfix liegen"
        );
    }

    #[test]
    fn fenster_verweigert_ueber_budget_und_zaehlt_drop_punkte() {
        let mut w = OocWindow::new(1, 4);
        w.acquire(4).expect("passt genau");
        assert_eq!((w.in_flight(), w.live_bytes()), (1, 4));
        assert!(matches!(w.acquire(1), Err(OocError::WarmBudget { .. })));
        w.release(4);
        assert_eq!((w.in_flight(), w.live_bytes()), (0, 0));
        assert_eq!(w.evictions, 1);
        w.acquire(2).expect("nach release");
        w.release_all();
        assert_eq!((w.in_flight(), w.live_bytes()), (0, 0));
    }

    #[test]
    fn profiler_zaehler_sind_stabil_parsbar() {
        let c = ProfileCounters::from_ooc(&OocStats {
            sealed: 3,
            evicted: 1,
            ..OocStats::default()
        });
        assert_eq!(c.smt_calls, 0);
        assert_eq!((c.ooc_streamed, c.ooc_evicted), (4, 1));
        let fields = c.log_fields();
        assert!(fields.contains("smt_calls=0"), "{fields}");
        assert!(fields.contains("ooc_streamed=4"), "{fields}");
        assert!(fields.contains("ooc_evicted=1"), "{fields}");
    }
}
