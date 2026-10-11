//! OoC-Driver-Schleife (Lane 83, E4-bindend).
//!
//! Verdrahtet die Bausteine aus Lane 60/62/74 — [`PhaseBarrier`](crate::PhaseBarrier)
//! (Barriere), [`WarmLayer`](crate::WarmLayer) (warm), [`MemoSpills`](crate::MemoSpills)
//! (kalt), [`SummaryTable`](crate::SummaryTable) (Cross-Shard, Lane-74-Summaries
//! referenziert, nicht gedoppelt) — zu einer fahrtuechtigen Schleife:
//!
//! ```text
//! Shard holen → Phase (heiss) → sealen → spillen → weiter
//! ```
//!
//! Nur der aktuelle Shard ist heiss (RAM: Shard-AST, Rewrite-State, heisse
//! Memos). Alles andere liegt warm (Datei, OS-Page-Cache) oder kalt
//! (Disk-Cache, Content-Hash-Key). Cross-Shard-Wissen tragen nur Summaries.
//!
//! # Zustaende (Diagramm s. `ARCHITECTURE.md`)
//!
//! ```text
//! Idle ── holen/begin ──▶ Hot(Fetch|Typecheck|Rewrite) ── seal ──▶ Sealed
//!   ▲                          │                                │
//!   │                     abort/evict                      spill/summary
//!   │                          │                                │
//!   └──────────────────────────┴────────────── weiter ◀─────────┘
//! Idle ── finish (evict_all) ──▶ Finished
//! ```
//!
//! # Peak-Budgets (Lane-14-Format, deklariert — keine Messlaeufe, E3)
//!
//! Je Phase eine Konstante ([`PHASE_BUDGETS`], [`check_phase_budget`]); das
//! Format spiegelt `sail-perf` (`BUDGETS`/`check_budget()`), ohne neue
//! Abhaengigkeit. Werte sind deklariert (32-GiB-Maschine ohne Swap, kleinstes
//! Modellbudget 4 GiB); Messung spaeter zentral per Test-Rig.
//!
//! # Abgrenzung zu Lane 62
//!
//! `sail-cli::ooc::OocDriver` (Lane 62) ist der Hook-Treiber im
//! Pipeline-Treiber (generisch ueber `OocHooks`, Stub-Keys). Diese Schleife
//! hier bindet die echten `sail-ooc`-Backends (echte [`ContentId`](crate::ContentId)s,
//! Datei-Warm-Layer, `sail-cache`-Memos). Sobald `sail-ooc` dort ein
//! `impl OocHooks` liefert, laufen beide ueber denselben Vertrag
//! (Ein-Shard-Barriere, Seal-vor-Spill, Summaries ≤ 64 KiB).
//!
//! E3: keine Builds/Tests in Lane-Agents — Tests liegen als Code bei
//! (`#[cfg(test)]`); Verifikation spaeter zentral per Test-Rig.

use std::fmt;
use std::path::Path;

use sail_shard::ShardId;

use crate::{
    BarrierError, ContentId, HotToken, MemoSpills, PhaseBarrier, SealReceipt, SummaryError,
    SummaryRecord, SummaryTable, WarmError, WarmLayer,
};

/// Pass-Version dieser Treiberschleife. Geht in jeden Shard-Content-Key ein
/// ([`ContentId::shard_content`]); bei Pass-Aenderung invalideren alte
/// Warm-/Kalt-Eintraege von selbst (kein stilles Mischen).
pub const OOC_DRIVER_PASS_VERSION: &str = "sail-ooc-driver-v1";

/// 1 MiB in KiB (Budget-Einheit wie GNU-time `Maximum resident set size`).
pub const KIB_PER_MIB: u64 = 1024;

/// Treiber-Phase mit heissem Shard (genau eine zur Zeit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriverPhase {
    /// Shard-Bytes holen (Datei → RAM, noch kein AST).
    Fetch,
    /// Shard typchecken (Shard-AST heiss).
    Typecheck,
    /// Rewrite-Paesse (Rewrite-State heiss).
    Rewrite,
    /// Serialisieren + verwerfen (Barrieren-`seal`).
    Seal,
    /// Warm spillen (fertiger Shard → Datei).
    Spill,
    /// Summary einpflegen (nur Fakten in RAM, Body per [`ContentId`](crate::ContentId)).
    Summary,
}

impl DriverPhase {
    /// Kurzname (fuer Logs/Timings im Lane-14-Format).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Typecheck => "typecheck",
            Self::Rewrite => "rewrite",
            Self::Seal => "seal",
            Self::Spill => "spill",
            Self::Summary => "summary",
        }
    }

    /// Deklariertes Peak-Budget dieser Phase (KiB). Alle Phasen halten hoechstens
    /// EINEN Shard heiss; die Summe bleibt mit Abstand unter dem kleinsten
    /// Modellbudget (4 GiB, Lane 14). Deklariert, ungemessen (E3-Moratorium).
    #[must_use]
    pub fn budget_kib(self) -> u64 {
        match self {
            Self::Fetch => PEAK_FETCH_KIB,
            Self::Typecheck => PEAK_TYPECHECK_KIB,
            Self::Rewrite => PEAK_REWRITE_KIB,
            Self::Seal => PEAK_SEAL_KIB,
            Self::Spill => PEAK_SPILL_KIB,
            Self::Summary => PEAK_SUMMARY_KIB,
        }
    }
}

impl fmt::Display for DriverPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Deklarierte Peak-Budgets je Phase (KiB). Lane-14-Format als Konstanten,
/// keine Messlaeufe (E3): Die Werte begrenzen den heissen Shard + Rewrite-State;
/// Ueberschreitung heisst Shard verkleinern oder Pass zurueckweisen (Lane 07/09).
pub const PEAK_FETCH_KIB: u64 = 64 * KIB_PER_MIB;
/// Shard-AST heiss (heissester Posten neben Rewrite).
pub const PEAK_TYPECHECK_KIB: u64 = 512 * KIB_PER_MIB;
/// Rewrite-State heiss (heissester Posten neben AST).
pub const PEAK_REWRITE_KIB: u64 = 512 * KIB_PER_MIB;
/// Serialisierung des versiegelten Shards (kurzlebig).
pub const PEAK_SEAL_KIB: u64 = 128 * KIB_PER_MIB;
/// Warm-Spill (live gehaltene Bytes; Drop-Punkt `finish`).
pub const PEAK_SPILL_KIB: u64 = 256 * KIB_PER_MIB;
/// Summary-Tabelle (Einzeleintrag ≤ 64 KiB, s. [`crate::MAX_SUMMARY_BYTES`]).
pub const PEAK_SUMMARY_KIB: u64 = 4 * KIB_PER_MIB;

/// Budget-Eintrag je Phase (Spiegel von `sail-perf::Budget`, Lane-14-Format).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseBudget {
    /// Phasenname (`fetch`, `typecheck`, …).
    pub phase: &'static str,
    /// Limit in KiB.
    pub limit_kib: u64,
}

impl PhaseBudget {
    /// Prueft `used_kib` gegen das Limit; `Err` traegt die Lane-14-Anweisung
    /// (Shard verkleinern oder Pass zurueckweisen).
    pub fn check(self, used_kib: u64) -> Result<(), String> {
        if used_kib <= self.limit_kib {
            Ok(())
        } else {
            Err(format!(
                "phase budget exceeded: phase={} used={} KiB > limit={} KiB; shard verkleinern oder Pass in Lane 07/09 zurueckweisen",
                self.phase, used_kib, self.limit_kib,
            ))
        }
    }
}

/// Alle Phasenbudgets in Schleifenreihenfolge (Lane-14-`BUDGETS`-Format).
pub const PHASE_BUDGETS: [PhaseBudget; 6] = [
    PhaseBudget { phase: "fetch", limit_kib: PEAK_FETCH_KIB },
    PhaseBudget { phase: "typecheck", limit_kib: PEAK_TYPECHECK_KIB },
    PhaseBudget { phase: "rewrite", limit_kib: PEAK_REWRITE_KIB },
    PhaseBudget { phase: "seal", limit_kib: PEAK_SEAL_KIB },
    PhaseBudget { phase: "spill", limit_kib: PEAK_SPILL_KIB },
    PhaseBudget { phase: "summary", limit_kib: PEAK_SUMMARY_KIB },
];

/// Prueft `used_bytes` gegen das Budget von `phase` (auf KiB aufgerundet).
pub fn check_phase_budget(phase: DriverPhase, used_bytes: usize) -> Result<(), String> {
    let used_kib = used_bytes.div_ceil(1024) as u64;
    PhaseBudget { phase: phase.name(), limit_kib: phase.budget_kib() }.check(used_kib)
}

/// Treiber-Zustand (fuer Diagramm + Assertions; die Barriere selbst haelt den
/// Besitz-Token [`HotToken`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverState {
    /// Nichts heiss; naechster Shard kann geholt werden.
    Idle,
    /// Genau ein Shard heiss in `phase`.
    Hot {
        /// Heisser Shard.
        shard: ShardId,
        /// Laufende Phase.
        phase: DriverPhase,
    },
    /// Alle Shards durch; Warm-Referenzen freigegeben.
    Finished,
}

/// Fehler der Treiberschleife (eine Variante je Backend + Budget).
#[derive(Debug)]
pub enum DriverError {
    /// Barriere verletzt (zweiter heisser Shard / veraltetes Token).
    Barrier(BarrierError),
    /// Warm-Budget erschoepft oder Content-Mismatch (erst `evict`/`finish`).
    Warm(WarmError),
    /// Summary zu gross (Bodies gehoeren auf Platte).
    Summary(SummaryError),
    /// Phasen-Budget ueberschritten (Shard verkleinern / Pass zurueckweisen).
    PhaseBudget {
        /// Betroffene Phase.
        phase: DriverPhase,
        /// Gemessene Bytes.
        used_bytes: usize,
        /// Limit in KiB.
        limit_kib: u64,
    },
    /// Storage-IO (Memo-Spills legen nur ab; Lesefehler fallen hierher).
    Io(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Barrier(e) => write!(f, "OoC-Driver Barriere: {e}"),
            Self::Warm(e) => write!(f, "OoC-Driver warm: {e}"),
            Self::Summary(e) => write!(f, "OoC-Driver summary: {e}"),
            Self::PhaseBudget { phase, used_bytes, limit_kib } => write!(
                f,
                "OoC-Driver Budget: Phase {phase} braucht {used_bytes} B (> {limit_kib} KiB); shard verkleinern oder Pass in Lane 07/09 zurueckweisen"
            ),
            Self::Io(msg) => write!(f, "OoC-Driver IO: {msg}"),
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Barrier(e) => Some(e),
            Self::Warm(e) => Some(e),
            Self::Summary(e) => Some(e),
            Self::PhaseBudget { .. } | Self::Io(_) => None,
        }
    }
}

impl From<BarrierError> for DriverError {
    fn from(e: BarrierError) -> Self {
        Self::Barrier(e)
    }
}

impl From<WarmError> for DriverError {
    fn from(e: WarmError) -> Self {
        Self::Warm(e)
    }
}

impl From<SummaryError> for DriverError {
    fn from(e: SummaryError) -> Self {
        Self::Summary(e)
    }
}

/// Zaehler des Treiberdurchlaufs (wandern spaeter ins Timings-Log, Lane 14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriverStats {
    /// Versiegelte Shards (seal + spill + summary).
    pub sealed: u64,
    /// Abgebrochene heisse Phasen (kein Cache-Eintrag).
    pub evicted: u64,
    /// Warm-Spill-Vorgaenge.
    pub warm_spills: u64,
    /// Solver-Memo-Treffer (kein Rechnen noetig).
    pub memo_hits: u64,
    /// Solver-Memo-Fehlschlaege (berechnet + gespillt).
    pub memo_misses: u64,
    /// Eingepflegte Cross-Shard-Summaries.
    pub summaries: u64,
    /// Fehlgeschlagene Memo-Stores (Wert wurde trotzdem zurueckgegeben;
    /// s. [`OocDriverLoop::memo_prove`]. Hält fest, dass nichts geschluckt
    /// wurde — die gepruefte Variante ist [`OocDriverLoop::memo_prove_result`]).
    pub memo_store_errors: u64,
}

/// OoC-Treiberschleife: genau ein heisser Shard, Rest warm/kalt.
///
/// Besitz-Modell: Der heisse [`HotToken`] lebt im Treiber (`hot`); solange er
/// lebt, weist `begin` jeden zweiten Shard ab (`AlreadyHot`). `seal` konsumiert
/// das Token (Shard verworfen, RAM frei), danach erst spillen/summarisieren.
/// `Drop` ohne `seal` gibt die Barriere frei und zaehlt `evicted` (Abbruchpfad).
#[derive(Debug)]
pub struct OocDriverLoop {
    barrier: PhaseBarrier,
    warm: WarmLayer,
    memo: MemoSpills,
    summaries: SummaryTable,
    hot: Option<HotToken>,
    hot_phase: Option<DriverPhase>,
    finished: bool,
    pass_version: String,
    stats: DriverStats,
}

impl OocDriverLoop {
    /// Oeffnet die Schleife: `warm_dir` (fertige Shards), `memo_dir` + `domain`
    /// (kalte Solver-Memos, z. B. `"solver/z3-v1"`), `pass_version` (geht in
    /// jeden Content-Key ein; Default [`OOC_DRIVER_PASS_VERSION`]).
    #[must_use]
    pub fn open(
        warm_dir: impl AsRef<Path>,
        memo_dir: impl AsRef<Path>,
        memo_domain: impl Into<String>,
        pass_version: impl Into<String>,
    ) -> Self {
        Self {
            barrier: PhaseBarrier::new(),
            warm: WarmLayer::open(warm_dir),
            memo: MemoSpills::open(memo_dir, memo_domain),
            summaries: SummaryTable::new(),
            hot: None,
            hot_phase: None,
            finished: false,
            pass_version: pass_version.into(),
            stats: DriverStats::default(),
        }
    }

    /// Aktueller Zustand (Diagramm-Sicht).
    #[must_use]
    pub fn state(&self) -> DriverState {
        match (&self.hot, &self.hot_phase) {
            (Some(token), Some(phase)) => DriverState::Hot {
                shard: token.shard().clone(),
                phase: *phase,
            },
            _ => {
                if self.finished {
                    DriverState::Finished
                } else {
                    DriverState::Idle
                }
            }
        }
    }

    /// `true`, wenn gerade ein Shard heiss ist.
    #[must_use]
    pub fn is_hot(&self) -> bool {
        self.hot.is_some()
    }

    /// Zaehler des Durchlaufs.
    #[must_use]
    pub fn stats(&self) -> &DriverStats {
        &self.stats
    }

    /// RAM-residente Summaries (ein Eintrag pro Shard, Bodies nur referenziert).
    #[must_use]
    pub fn summaries(&self) -> &SummaryTable {
        &self.summaries
    }

    /// Warm-Layer (Datei-Ablage fertiger Shards).
    #[must_use]
    pub fn warm(&self) -> &WarmLayer {
        &self.warm
    }

    /// Kalte Memo-Ablage (Solver-Verdicts, run-uebergreifend).
    #[must_use]
    pub fn memo(&self) -> &MemoSpills {
        &self.memo
    }

    /// Ein Shard vollstaendig: holen → Phase → sealen → spillen → Summary.
    ///
    /// * `fetch` holt die Shard-Bytes (kalt → heiss, Phase [`DriverPhase::Fetch`]).
    /// * `run_phase` faehrt Typcheck + Rewrite auf den geholten Bytes
    ///   (Phasen [`DriverPhase::Typecheck`]/[`DriverPhase::Rewrite`]); gibt die
    ///   kanonische Serialisierung zurueck.
    /// * `summarize` baut aus Beleg + serialisierten Bytes genau einen
    ///   [`SummaryRecord`] (nur Fakten; Lane-74-Typen wie Callgraph/Mono/
    ///   ConstProp passen hier, ohne dass der Treiber sie kennt).
    ///
    /// Reihenfolge ist fix (Seal-vor-Spill): Schlaegt der Spill/die Summary
    /// fehl, ist die Barriere trotzdem frei (versiegelt vor dem Spill).
    ///
    /// Leck-Freiheit (Lane 126): Schlaegt ein Schritt *nach* `begin` fehl
    /// (Budget, Spill, Summary, Seal-Deckel), wird die heisse Phase per
    /// `evict` verworfen (`evicted` gezaehlt) — die Barriere bleibt nie mit
    /// heissem Token liegen, der naechste Shard kann sofort beginnen.
    pub fn run_shard(
        &mut self,
        shard: ShardId,
        fetch: impl FnOnce() -> Vec<u8>,
        run_phase: impl FnOnce(&[u8]) -> Vec<u8>,
        summarize: impl FnOnce(&ShardId, ContentId, &[u8]) -> Result<SummaryRecord, SummaryError>,
    ) -> Result<SealReceipt, DriverError> {
        if let Some(token) = self.hot.as_ref() {
            return Err(DriverError::Barrier(BarrierError::AlreadyHot {
                current: token.shard().clone(),
            }));
        }
        // holen (kalt → heiss)
        self.hot_phase = Some(DriverPhase::Fetch);
        let fetched = fetch();
        self.check_bytes(DriverPhase::Fetch, fetched.len())?;
        // heisse Phase oeffnen
        let token = self.barrier.begin(shard.clone())?;
        self.hot = Some(token);
        self.hot_phase = Some(DriverPhase::Typecheck);
        let res = self.run_shard_hot(&shard, fetched, run_phase, summarize);
        if res.is_err() {
            // Fehlerpfad nach `begin`: Barriere nicht lecken lassen.
            self.fail_hot();
        }
        res
    }

    /// Heisser Abschnitt von [`OocDriverLoop::run_shard`] (nur mit gesetztem
    /// `hot` aufrufen). Fehler ueberlaesst der Aufrufer [`OocDriverLoop::fail_hot`].
    fn run_shard_hot(
        &mut self,
        shard: &ShardId,
        fetched: Vec<u8>,
        run_phase: impl FnOnce(&[u8]) -> Vec<u8>,
        summarize: impl FnOnce(&ShardId, ContentId, &[u8]) -> Result<SummaryRecord, SummaryError>,
    ) -> Result<SealReceipt, DriverError> {
        self.check_bytes(DriverPhase::Typecheck, fetched.len())?;
        // Typcheck + Rewrite (nur dieser Shard in RAM)
        self.hot_phase = Some(DriverPhase::Rewrite);
        let produced = run_phase(&fetched);
        drop(fetched);
        self.check_bytes(DriverPhase::Rewrite, produced.len())?;
        // sealen (serialisieren + verwerfen, RAM frei)
        self.hot_phase = Some(DriverPhase::Seal);
        self.check_bytes(DriverPhase::Seal, produced.len())?;
        let token = self.hot.take().expect("heiss nach begin");
        self.hot_phase = None;
        let receipt = match self.barrier.seal(token, &produced, &self.pass_version) {
            Ok(receipt) => receipt,
            Err(e @ BarrierError::SealedFull { .. }) => {
                // Beleg-Deckel: `seal` hat das Token wie ein `evict` konsumiert
                // (Barriere frei, Shard als verworfen gezaehlt) — nur Treiber-
                // Zaehler nachziehen, dann Fehler durchreichen.
                self.stats.evicted += 1;
                return Err(e.into());
            }
            Err(e) => {
                // Veraltetes/fremdes Token (Desync, unerreichbar im Normalpfad):
                // weder Barriere noch Zaehler angefasst — Fehler durchreichen,
                // `fail_hot` des Aufrufers findet kein heisses Token mehr.
                return Err(e.into());
            }
        };
        // spillen (warm ablegen)
        self.check_bytes(DriverPhase::Spill, produced.len())?;
        self.warm.store(&receipt.content, &produced)?;
        self.stats.warm_spills += 1;
        // Summary (nur Fakten in RAM)
        self.check_bytes(DriverPhase::Summary, produced.len())?;
        let record = summarize(shard, receipt.content, &produced)?;
        self.summaries.insert(record).map_err(DriverError::Summary)?;
        self.stats.summaries += 1;
        self.stats.sealed += 1;
        Ok(receipt)
    }

    /// Verwirft eine liegengebliebene heisse Phase (Fehlerpfad nach `begin`,
    /// [`OocDriverLoop::finish`] mit heissem Shard): Barriere per `evict` frei,
    /// `evicted` gezaehlt. Idempotent (nichts heiss → nichts zu tun).
    fn fail_hot(&mut self) {
        if let Some(token) = self.hot.take() {
            self.hot_phase = None;
            if self.barrier.evict(token).is_ok() {
                self.stats.evicted += 1;
            }
        }
    }

    /// Ganzer Plan: jeder Shard durch [`OocDriverLoop::run_shard`], der Reihe nach
    /// (Shard-Streaming, kein wilder Random-Access). Bricht beim ersten Fehler
    /// ab (Barriere bleibt frei, s. Seal-vor-Spill); danach `finish`.
    pub fn run_all(
        &mut self,
        plan: &[ShardId],
        fetch_shard: impl Fn(&ShardId) -> Vec<u8>,
        run_phase: impl Fn(&ShardId, &[u8]) -> Vec<u8>,
        summarize: impl Fn(&ShardId, ContentId, &[u8]) -> Result<SummaryRecord, SummaryError>,
    ) -> Result<Vec<SealReceipt>, DriverError> {
        let mut receipts = Vec::with_capacity(plan.len());
        for shard in plan {
            receipts.push(self.run_shard(
                shard.clone(),
                || fetch_shard(shard),
                |bytes| run_phase(shard, bytes),
                |s, c, b| summarize(s, c, b),
            )?);
        }
        Ok(receipts)
    }

    /// Verwirft die heisse Phase ohne Versiegelung (Abbruchpfad; kein
    /// Cache-Eintrag). Gibt `false` zurueck, wenn nichts heiss war.
    pub fn abort_hot(&mut self) -> Result<bool, DriverError> {
        match self.hot.take() {
            Some(token) => {
                self.hot_phase = None;
                self.barrier.evict(token)?;
                self.stats.evicted += 1;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Kalte Solver-Memos (z. B. Z3-Verdicts): Treffer ohne Rechnen, sonst
    /// `compute` + Spill (run-uebergreifend wiederverwendbar). Die Domaene
    /// steckt in der Ablage ([`OocDriverLoop::open`]); Vertrag: Anfrage → Wert
    /// ist rein (Erster gewinnt, Determinismus).
    ///
    /// Kein Singleflight (Lane 126, Thundering Herd): Gleichzeitige gleiche
    /// Anfragen rechnen parallel, der Erstling gewinnt; Aufrufer mit teurem
    /// `compute` deduplizieren gleiche Fluege extern.
    ///
    /// Store-Fehler (Lane 126): Diese Variante gibt den Wert trotzdem zurueck
    /// (Cache ist Best-Effort), zaehlt aber
    /// [`DriverStats::memo_store_errors`] — nichts wird still geschluckt.
    /// Wer den Fehler braucht, nutzt [`OocDriverLoop::memo_prove_result`].
    pub fn memo_prove(
        &mut self,
        query: &[u8],
        compute: impl FnOnce() -> Vec<u8>,
    ) -> Vec<u8> {
        if let Some(hit) = self.memo.lookup(query) {
            self.stats.memo_hits += 1;
            return hit;
        }
        self.stats.memo_misses += 1;
        let value = compute();
        if self.memo.store(query, &value).is_err() {
            self.stats.memo_store_errors += 1;
        }
        value
    }

    /// Wie [`OocDriverLoop::memo_prove`], gibt aber Store-Fehler als
    /// [`DriverError::Io`] zurueck (Lane 126: nichts schlucken). Treffer und
    /// berechnete Werte sind identisch zur infalliblen Variante.
    pub fn memo_prove_result(
        &mut self,
        query: &[u8],
        compute: impl FnOnce() -> Vec<u8>,
    ) -> Result<Vec<u8>, DriverError> {
        if let Some(hit) = self.memo.lookup(query) {
            self.stats.memo_hits += 1;
            return Ok(hit);
        }
        self.stats.memo_misses += 1;
        let value = compute();
        self.memo
            .store(query, &value)
            .map_err(|e| DriverError::Io(format!("memo-store: {e}")))?;
        Ok(value)
    }

    /// Drop-Punkt am Phasenende (Lane-37-Policy): gibt alle live gehaltenen
    /// Warm-Referenzen frei (Dateien/Memos/Summaries bleiben). Danach
    /// [`DriverState::Finished`] (bis der naechste Shard beginnt — die Schleife
    /// bleibt wiederverwendbar).
    ///
    /// Heisser Shard (Lane 126): Ist beim Aufruf noch ein Shard heiss, wird er
    /// wie ein Abbruch verworfen (`evicted` gezaehlt, kein Cache-Eintrag) —
    /// `finish` leckt die Barriere nie.
    pub fn finish(&mut self) {
        self.fail_hot();
        self.warm.evict_all();
        self.finished = true;
    }

    fn check_bytes(&self, phase: DriverPhase, bytes: usize) -> Result<(), DriverError> {
        if check_phase_budget(phase, bytes).is_err() {
            return Err(DriverError::PhaseBudget {
                phase,
                used_bytes: bytes,
                limit_kib: phase.budget_kib(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirs {
        warm: PathBuf,
        memo: PathBuf,
    }

    impl TestDirs {
        fn new(tag: &str) -> Self {
            let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let base =
                std::env::temp_dir().join(format!("sail-ooc-driver-{tag}-{n}-{}", std::process::id()));
            let warm = base.join("warm");
            let memo = base.join("memo");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&warm).expect("warmdir");
            std::fs::create_dir_all(&memo).expect("memodir");
            Self { warm, memo }
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            let _ = self.warm.parent().map(std::fs::remove_dir_all);
        }
    }

    fn driver(dirs: &TestDirs) -> OocDriverLoop {
        OocDriverLoop::open(&dirs.warm, &dirs.memo, "solver/z3-v1", OOC_DRIVER_PASS_VERSION)
    }

    fn tiny_summary(shard: &ShardId, content: ContentId, bytes: &[u8]) -> Result<SummaryRecord, SummaryError> {
        SummaryRecord::new(shard.clone(), content, "ooc/driver-test-v1", bytes.to_vec())
    }

    #[test]
    fn schleife_holt_sealt_spillt_und_geht_weiter() {
        let dirs = TestDirs::new("loop");
        let mut d = driver(&dirs);
        assert_eq!(d.state(), DriverState::Idle);
        let plan = vec![ShardId::new("a", 0), ShardId::new("b", 0)];
        let receipts = d
            .run_all(
                &plan,
                |s| format!("src:{}\n", s.name).into_bytes(),
                |_s, fetched| fetched.to_vec(),
                tiny_summary,
            )
            .expect("plan");
        assert_eq!(receipts.len(), 2);
        // Gleiche Eingabe → stabiler Content-Key; verschiedene Shards → verschieden.
        assert_ne!(receipts[0].content, receipts[1].content);
        assert_eq!((receipts[0].seq, receipts[1].seq), (0, 1));
        assert_eq!(d.state(), DriverState::Idle);
        assert_eq!(d.stats().sealed, 2);
        assert_eq!(d.stats().warm_spills, 2);
        assert_eq!(d.stats().summaries, 2);
        assert_eq!(d.summaries().len(), 2);
        // Bodies liegen warm, nicht in RAM: Tabelle traegt nur Referenzen.
        let kept = d.summaries().get(&ShardId::new("a", 0)).expect("summary a");
        assert_eq!(kept.content, receipts[0].content);
        d.finish();
        assert_eq!(d.state(), DriverState::Finished);
        assert_eq!(d.warm().live_bytes(), 0);
    }

    #[test]
    fn zweiter_heisser_shard_wird_abgewiesen() {
        let dirs = TestDirs::new("hot");
        let mut d = driver(&dirs);
        // Erster Shard bleibt heiss, weil die Phase blockiert (Fehlerpfad-Test
        // ueber abort): run_shard mit Summary-Fehler versiegelt trotzdem.
        let big = vec![0u8; crate::MAX_SUMMARY_BYTES + 1];
        let err = d
            .run_shard(
                ShardId::new("riesig", 0),
                || b"bytes".to_vec(),
                |b| b.to_vec(),
                |s, c, _| SummaryRecord::new(s.clone(), c, "ooc/driver-test-v1", big.clone()),
            )
            .expect_err("zu grosse Summary");
        assert!(matches!(err, DriverError::Summary(_)));
        // Barriere ist frei (Seal war vor Spill/Summary).
        assert_eq!(d.state(), DriverState::Idle);
        assert_eq!(d.stats().sealed, 0);
        // Danach laeuft der naechste normal weiter.
        d.run_shard(
            ShardId::new("klein", 0),
            || b"x".to_vec(),
            |b| b.to_vec(),
            tiny_summary,
        )
        .expect("weiter");
        assert_eq!(d.stats().sealed, 1);
    }

    /// Lane 126: Fehler *nach* `begin` lassen die Barriere frei (kein Leck).
    /// Rig-Hinweis: allokiert absichtlich ueber dem Rewrite-Budget (512 MiB),
    /// laeuft nur im zentralen Test-Rig mit RAM (E3: hier nur Code).
    #[test]
    fn fehler_nach_begin_leckt_die_barriere_nicht() {
        let dirs = TestDirs::new("hot-leak");
        let mut d = driver(&dirs);
        let huge = (PEAK_REWRITE_KIB as usize + 1) * 1024;
        let err = d
            .run_shard(
                ShardId::new("riesig", 0),
                || b"klein".to_vec(),
                |_| vec![0u8; huge],
                tiny_summary,
            )
            .expect_err("Rewrite-Budget");
        assert!(matches!(err, DriverError::PhaseBudget { .. }));
        // Heisse Phase verworfen statt geleckt: Barriere frei, evicted gezaehlt.
        assert_eq!(d.state(), DriverState::Idle);
        assert_eq!(d.stats().evicted, 1);
        assert_eq!(d.stats().sealed, 0);
        // Danach laeuft der naechste normal weiter.
        d.run_shard(
            ShardId::new("klein", 0),
            || b"x".to_vec(),
            |b| b.to_vec(),
            tiny_summary,
        )
        .expect("weiter");
        assert_eq!(d.stats().sealed, 1);
    }

    #[test]
    fn finish_mit_heissem_shard_verwirft_statt_zu_lecken() {
        let dirs = TestDirs::new("finish-hot");
        let mut d = driver(&dirs);
        // Heiss machen (direkt ueber die Barriere, wie ein unterbrochener Lauf).
        let token = d.barrier.begin(ShardId::new("a", 0)).expect("begin");
        d.hot = Some(token);
        d.hot_phase = Some(DriverPhase::Rewrite);
        d.finish();
        assert_eq!(d.stats().evicted, 1);
        assert_eq!(d.stats().sealed, 0);
        assert_eq!(d.state(), DriverState::Finished);
        // Barriere frei: naechster Shard nach finish (wiederverwendbar).
        let r = d
            .run_shard(
                ShardId::new("b", 0),
                || b"w".to_vec(),
                |b| b.to_vec(),
                tiny_summary,
            )
            .expect("nach finish weiter");
        assert_eq!(r.seq, 0);
    }

    #[test]
    fn memo_store_fehler_wird_gezaehlt_nicht_geschluckt() {
        let dirs = TestDirs::new("memo-err");
        // Memo-Verzeichnis durch eine Datei blockieren: jeder Store scheitert.
        let blocker = dirs.memo.clone();
        let _ = std::fs::remove_dir_all(&blocker);
        std::fs::write(&blocker, b"blockiert").expect("blocker");
        let mut d = OocDriverLoop::open(&dirs.warm, &blocker, "solver/z3-v1", OOC_DRIVER_PASS_VERSION);
        // Infallible Variante: Wert trotzdem zurueck, Fehler gezaehlt.
        let v = d.memo_prove(b"q", || b"sat".to_vec());
        assert_eq!(v, b"sat");
        assert_eq!(d.stats().memo_store_errors, 1);
        // Gepruefte Variante: Fehler kommt als Io zurueck.
        let err = d.memo_prove_result(b"q2", || b"unsat".to_vec()).expect_err("store");
        assert!(matches!(err, DriverError::Io(_)));
        assert_eq!(d.stats().memo_store_errors, 1);
        // Treffer-Pfad bleibt fehlerfrei (nichts gezaehlt): flottender Speicher
        // ist leer, also erneut Miss — aber kein Panic, kein Hang.
        assert_eq!(d.stats().memo_misses, 2);
    }

    #[test]
    fn abort_verwirft_ohne_cache_eintrag() {        let dirs = TestDirs::new("abort");
        let mut d = driver(&dirs);
        assert_eq!(d.abort_hot().expect("nichts heiss"), false);
        // Heiss machen, dann abbrechen: kein Seal, kein Spill, kein Summary.
        let token_shard = ShardId::new("a", 0);
        let token = d.barrier.begin(token_shard.clone()).expect("begin");
        d.hot = Some(token);
        d.hot_phase = Some(DriverPhase::Rewrite);
        assert!(matches!(
            d.state(),
            DriverState::Hot { phase: DriverPhase::Rewrite, .. }
        ));
        assert!(d.abort_hot().expect("abort"));
        assert_eq!(d.state(), DriverState::Idle);
        assert_eq!(d.stats().evicted, 1);
        assert_eq!(d.stats().sealed, 0);
    }

    #[test]
    fn disk_spills_do_not_accumulate_in_the_live_cache() {
        let dirs = TestDirs::new("budget");
        let mut d = driver(&dirs);
        d.warm.max_live_bytes = 0;
        for i in 0..32 {
            let receipt = d.run_shard(
                ShardId::new("a", i),
                || vec![42; 1024],
                |b| b.to_vec(),
                tiny_summary,
            ).expect("disk storage needs no live cache budget");
            assert!(d.warm.reader(&receipt.content).unwrap().is_some());
            assert_eq!(d.warm.live_bytes(), 0);
            assert_eq!(d.state(), DriverState::Idle);
        }
        assert_eq!(d.stats().sealed, 32);
    }

    #[test]
    fn memo_prove_trifft_beim_zweiten_mal_ohne_rechnen() {
        let dirs = TestDirs::new("memo");
        let mut d = driver(&dirs);
        let mut calls = 0u32;
        let v1 = d.memo_prove(b"(atom x>0)", || {
            calls += 1;
            b"sat".to_vec()
        });
        let v2 = d.memo_prove(b"(atom x>0)", || {
            calls += 1;
            b"anders".to_vec()
        });
        assert_eq!(calls, 1);
        assert_eq!(v1, b"sat");
        assert_eq!(v2, b"sat");
        assert_eq!((d.stats().memo_hits, d.stats().memo_misses), (1, 1));
        // Run-uebergreifend: neue Schleife, gleiche Domaene/Verzeichnis.
        let mut d2 = driver(&dirs);
        assert_eq!(d2.memo_prove(b"(atom x>0)", || b"neu".to_vec()), b"sat");
        assert_eq!(d2.stats().memo_hits, 1);
    }

    #[test]
    fn phasen_budgets_im_lane14_format_und_zu_gross_wird_abgewiesen() {
        assert_eq!(PHASE_BUDGETS.len(), 6);
        assert!(PHASE_BUDGETS.iter().all(|b| b.limit_kib > 0));
        // Einzeleintrag-Schranke bleibt die harte Summary-Grenze.
        assert!(PEAK_SUMMARY_KIB * 1024 > crate::MAX_SUMMARY_BYTES as u64);
        // check_phase_budget: kleine Nutzung ok, riesige fliegt.
        assert!(check_phase_budget(DriverPhase::Fetch, 16).is_ok());
        let huge = (PEAK_FETCH_KIB as usize + 1) * 1024;
        assert!(check_phase_budget(DriverPhase::Fetch, huge).is_err());
        // Treiber prueft vor der Barriere: zu grosse Fetch belegt keinen Slot.
        let dirs = TestDirs::new("phase");
        let mut d = driver(&dirs);
        let err = d
            .run_shard(
                ShardId::new("a", 0),
                || vec![0u8; huge],
                |b| b.to_vec(),
                tiny_summary,
            )
            .expect_err("Fetch-Budget");
        assert!(matches!(err, DriverError::PhaseBudget { .. }));
        assert_eq!(d.state(), DriverState::Idle);
        assert_eq!(d.stats().sealed, 0);
    }

    #[test]
    fn zustandsfolge_idle_heiss_idle_finished() {
        let dirs = TestDirs::new("states");
        let mut d = driver(&dirs);
        assert_eq!(d.state(), DriverState::Idle);
        d.run_shard(
            ShardId::new("a", 0),
            || b"q".to_vec(),
            |b| b.to_vec(),
            tiny_summary,
        )
        .expect("run");
        // Nach seal+spill+summary ist die Schleife wieder Idle (weiter-faehig).
        assert_eq!(d.state(), DriverState::Idle);
        d.finish();
        assert_eq!(d.state(), DriverState::Finished);
        // Wiederverwendbar: naechster Shard nach finish.
        let r = d
            .run_shard(
                ShardId::new("b", 0),
                || b"w".to_vec(),
                |b| b.to_vec(),
                tiny_summary,
            )
            .expect("nach finish weiter");
        assert_eq!(r.seq, 1);
    }
}
