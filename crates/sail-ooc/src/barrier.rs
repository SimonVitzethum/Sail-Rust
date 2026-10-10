//! Phasen-Barrieren (E4): pro Shard `typchecken → serialisieren + verwerfen → nächster`.
//!
//! Nur der aktuelle Shard ist heiss (RAM: Shard-AST, Rewrite-State, heisse
//! Memos). [`PhaseBarrier`] erzwingt das per Typzustand zur Laufzeit: Es gibt
//! zu jedem Zeitpunkt hoechstens einen heissen Shard; der naechste beginnt
//! erst, wenn der aktuelle versiegelt (`seal`: serialisieren + verwerfen) oder
//! explizit verworfen wurde. Die Identitaet eines versiegelten Shards ist
//! immer ein [`ContentId`](crate::ContentId) (Content-Hash), nie ein Zaehler:
//! Die Protokoll-Sequence (`seq`) ist nur ein Log-Index, keine ID.
//!
//! Typischer Treiber:
//!
//! ```text
//! for shard in plan {
//!     let hot = barrier.begin(shard.id.clone())?;   // heiss: typchecken/rewrite
//!     let bytes = typchecken_und_serialisieren(&hot);
//!     let receipt = barrier.seal(hot, &bytes, PASS_VERSION); // kalt: verwerfen
//!     warm.spill(&receipt.content, &bytes)?;                 // warm/kalt ablegen
//!     summaries.insert(...)?;                               // nur Summary in RAM
//! }
//! ```

use std::fmt;
use std::sync::{Arc, Mutex};

use sail_shard::ShardId;

use crate::ContentId;

/// Lebensphase eines Shards im OoC-Durchlauf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardPhase {
    /// In RAM (AST, Rewrite-State, heisse Memos). Hoechstens einer pro Barriere.
    Hot,
    /// Serialisiert + verworfen; nur [`SealReceipt`] + Warm-/Kalt-Ablage bleiben.
    Sealed,
    /// Ohne Versiegelung verworfen (Fehler-/Abbruchpfad, kein Cache-Eintrag).
    Evicted,
}

/// Beleg einer vollzogenen Barriere: Shard ist versiegelt und verworfen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealReceipt {
    /// Welcher Shard versiegelt wurde.
    pub shard: ShardId,
    /// Stabile Inhalts-ID der serialisierten Bytes (Cache-/Warm-Schluessel).
    pub content: ContentId,
    /// Laenge der serialisierten Bytes.
    pub bytes_len: usize,
    /// Log-Index der Versiegelung (Reihenfolge, KEINE Identitaet).
    pub seq: u64,
}

/// Fehler der Barriere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarrierError {
    /// Bereits ein Shard heiss; erst `seal`/`evict`, dann `begin`.
    AlreadyHot {
        /// Der noch heisse Shard.
        current: ShardId,
    },
    /// Token gehoert nicht zur aktuellen heissen Phase (veraltet/ fremd).
    StaleToken {
        /// Shard des Tokens.
        token_shard: ShardId,
    },
}

impl fmt::Display for BarrierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyHot { current } => {
                write!(f, "Phasen-Barriere: Shard {current} noch heiss (erst seal/evict)")
            }
            Self::StaleToken { token_shard } => {
                write!(f, "Phasen-Barriere: Token von Shard {token_shard} ist veraltet")
            }
        }
    }
}

impl std::error::Error for BarrierError {}

/// Geteilter heisser Slot der Barriere (nur in diesem Modul).
///
/// `HotToken` haelt eine Referenz auf denselben Zustand: Faellt das Token
/// ohne `seal`/`evict` weg, kann sein `Drop` den Slot freigeben, ohne
/// `&mut`-Zugriff auf die Barriere zu brauchen. `evicted` lebt hier (nicht in
/// [`PhaseBarrier`]), damit auch der Drop-Pfad zaehlen kann.
#[derive(Debug, Default)]
struct HotState {
    hot: Option<(ShardId, u64)>,
    evicted: u64,
}

/// Besitz-Token der heissen Phase. Nicht klonbar: Genau ein Besitzer kann
/// versiegeln. Faellt das Token ohne `seal`/`evict` weg (`Drop`, z. B.
/// Fehlerpfad mit `?`), gibt der Drop die Barriere frei und zaehlt den Shard
/// als verworfen (`Evicted`, kein Cache-Eintrag): Der naechste Shard kann
/// sofort beginnen.
#[derive(Debug)]
pub struct HotToken {
    shard: ShardId,
    generation: u64,
    state: Arc<Mutex<HotState>>,
    consumed: bool,
}

impl Drop for HotToken {
    /// Abbruchpfad: War das Token noch heiss (weder `seal` noch `evict` haben
    /// es konsumiert) und gehoert der Slot noch ihm, wird die Barriere
    /// freigegeben und der Shard als verworfen gezaehlt. Passt der Slot nicht
    /// mehr zu diesem Token (fremd/veraltet/konsumiert), passiert nichts.
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match &state.hot {
            Some((current, generation))
                if *current == self.shard && *generation == self.generation => {}
            _ => return,
        }
        state.hot = None;
        state.evicted += 1;
    }
}

impl HotToken {
    /// Shard, fuer den die heisse Phase laeuft.
    #[must_use]
    pub fn shard(&self) -> &ShardId {
        &self.shard
    }
}

/// Erzwingt die OoC-Phasenfolge. Genau eine heisse Phase zur Zeit.
///
/// Der heisse Slot ist mit den ausgegebenen [`HotToken`]s geteilt: Wird ein
/// Token ohne `seal`/`evict` fallengelassen, gibt sein `Drop` die Barriere
/// frei (Abbruchpfad, zaehlt als `Evicted`).
#[derive(Debug, Default)]
pub struct PhaseBarrier {
    state: Arc<Mutex<HotState>>,
    generation: u64,
    sealed: Vec<SealReceipt>,
}

impl PhaseBarrier {
    /// Sperrt den geteilten Zustand (toleriert Vergiftung: Haltet der
    /// kritische Abschnitt nie Anwendercode, ist der Zustand konsistent).
    fn lock_state(&self) -> std::sync::MutexGuard<'_, HotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Neue, leere Barriere (nichts heiss).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Beginnt die heisse Phase fuer `shard`. Fehler, wenn noch einer heiss ist.
    pub fn begin(&mut self, shard: ShardId) -> Result<HotToken, BarrierError> {
        let mut state = self.lock_state();
        if let Some((current, _)) = &state.hot {
            return Err(BarrierError::AlreadyHot {
                current: current.clone(),
            });
        }
        self.generation += 1;
        let generation = self.generation;
        state.hot = Some((shard.clone(), generation));
        Ok(HotToken {
            shard,
            generation,
            state: Arc::clone(&self.state),
            consumed: false,
        })
    }

    /// Versiegelt die heisse Phase: `serialized` ist die kanonische
    /// Serialisierung des Shard-Ergebnisses; der Beleg traegt dessen
    /// Content-Hash. Danach ist der Shard verworfen (RAM frei).
    pub fn seal(
        &mut self,
        mut token: HotToken,
        serialized: &[u8],
        pass_version: &str,
    ) -> Result<SealReceipt, BarrierError> {
        {
            let mut state = token.state.lock().unwrap_or_else(|e| e.into_inner());
            match &state.hot {
                Some((current, generation))
                    if *current == token.shard && *generation == token.generation => {}
                _ => {
                    return Err(BarrierError::StaleToken {
                        token_shard: token.shard.clone(),
                    });
                }
            }
            state.hot = None;
        }
        token.consumed = true;
        let receipt = SealReceipt {
            content: ContentId::shard_content(&token.shard, serialized, pass_version),
            shard: token.shard.clone(),
            bytes_len: serialized.len(),
            seq: self.sealed.len() as u64,
        };
        self.sealed.push(receipt.clone());
        Ok(receipt)
    }

    /// Verwirft die heisse Phase ohne Versiegelung (Abbruchpfad; kein
    /// Cache-Eintrag, keine Wiederverwendbarkeit).
    pub fn evict(&mut self, mut token: HotToken) -> Result<ShardPhase, BarrierError> {
        {
            let mut state = token.state.lock().unwrap_or_else(|e| e.into_inner());
            match &state.hot {
                Some((current, generation))
                    if *current == token.shard && *generation == token.generation => {}
                _ => {
                    return Err(BarrierError::StaleToken {
                        token_shard: token.shard.clone(),
                    });
                }
            }
            state.hot = None;
            state.evicted += 1;
        }
        token.consumed = true;
        Ok(ShardPhase::Evicted)
    }

    /// Aktuell heisser Shard (genau einer oder keiner).
    #[must_use]
    pub fn hot(&self) -> Option<ShardId> {
        self.lock_state().hot.as_ref().map(|(id, _)| id.clone())
    }

    /// `true`, wenn gerade ein Shard heiss ist.
    #[must_use]
    pub fn is_hot(&self) -> bool {
        self.lock_state().hot.is_some()
    }

    /// Alle Versiegelungs-Belege in Reihenfolge.
    #[must_use]
    pub fn sealed(&self) -> &[SealReceipt] {
        &self.sealed
    }

    /// Anzahl versiegelter Shards.
    #[must_use]
    pub fn sealed_count(&self) -> usize {
        self.sealed.len()
    }

    /// Anzahl verworfener (nicht versiegelter) Shards (inklusive per
    /// `Drop` fallengelassener Token, siehe [`HotToken`]).
    #[must_use]
    pub fn evicted_count(&self) -> u64 {
        self.lock_state().evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "ooc-test-passes-v1";

    fn barrier_with_seal(id: &str, bytes: &[u8]) -> (PhaseBarrier, SealReceipt) {
        let mut b = PhaseBarrier::new();
        let hot = b.begin(ShardId::new(id, 0)).expect("begin");
        let receipt = b.seal(hot, bytes, PASS).expect("seal");
        (b, receipt)
    }

    #[test]
    fn genau_ein_heisser_shard() {
        let mut b = PhaseBarrier::new();
        assert!(!b.is_hot());
        let hot = b.begin(ShardId::new("a", 0)).expect("erstes begin");
        assert_eq!(hot.shard(), &ShardId::new("a", 0));
        assert!(b.is_hot());
        assert_eq!(b.hot(), Some(ShardId::new("a", 0)));
        let err = b.begin(ShardId::new("b", 0)).expect_err("zweites begin");
        assert_eq!(
            err,
            BarrierError::AlreadyHot {
                current: ShardId::new("a", 0)
            }
        );
        b.seal(hot, b"a-bytes", PASS).expect("seal");
        assert!(!b.is_hot());
        // Danach darf der naechste beginnen (typchecken → verwerfen → naechster).
        let hot2 = b.begin(ShardId::new("b", 0)).expect("begin nach seal");
        b.seal(hot2, b"b-bytes", PASS).expect("seal2");
        assert_eq!(b.sealed_count(), 2);
    }

    #[test]
    fn beleg_traegt_content_hash_keinen_zaehler() {
        let (_, r1) = barrier_with_seal("instrs32", b"bytes");
        let (_, r2) = barrier_with_seal("instrs32", b"bytes");
        // Gleiche Eingabe → gleiche ID (run-uebergreifend stabil).
        assert_eq!(r1.content, r2.content);
        assert_eq!(r1.bytes_len, 5);
        // seq ist nur Log-Index, ab 0 pro Barriere.
        assert_eq!(r1.seq, 0);
        let mut b = PhaseBarrier::new();
        let h1 = b.begin(ShardId::new("x", 0)).expect("b1");
        let rc1 = b.seal(h1, b"eins", PASS).expect("s1");
        let h2 = b.begin(ShardId::new("y", 0)).expect("b2");
        let rc2 = b.seal(h2, b"zwei", PASS).expect("s2");
        assert_eq!((rc1.seq, rc2.seq), (0, 1));
        assert_ne!(rc1.content, rc2.content);
    }

    #[test]
    fn evict_ohne_versiegelung() {
        let mut b = PhaseBarrier::new();
        let hot = b.begin(ShardId::new("a", 0)).expect("begin");
        b.seal(hot, b"a", PASS).expect("seal");
        // Doppeltes Seal mit verbrauchtem Token ist per Move unmoeglich
        // (kompiliert nur einmal). Abbruchpfad: evict ohne Versiegelung.
        let hot2 = b.begin(ShardId::new("b", 0)).expect("begin2");
        b.evict(hot2).expect("evict");
        assert_eq!(b.evicted_count(), 1);
        assert_eq!(b.sealed_count(), 1);
    }

    #[test]
    fn evict_gibt_barriere_frei() {
        let mut b = PhaseBarrier::new();
        let first = b.begin(ShardId::new("a", 0)).expect("begin");
        // Zweites begin scheitert, solange heiss.
        assert!(b.begin(ShardId::new("b", 0)).is_err());
        b.evict(first).expect("evict");
        // Nach evict ist die Barriere frei.
        let next = b.begin(ShardId::new("b", 0)).expect("begin nach evict");
        let receipt = b.seal(next, b"b", PASS).expect("seal");
        assert_eq!(receipt.shard, ShardId::new("b", 0));
    }

    #[test]
    fn drop_ohne_seal_gibt_barriere_frei() {
        let mut b = PhaseBarrier::new();
        {
            let _hot = b.begin(ShardId::new("a", 0)).expect("begin");
            assert!(b.is_hot());
            // `_hot` faellt hier ohne seal/evict weg (Abbruchpfad, z. B. `?`).
        }
        assert!(!b.is_hot());
        assert_eq!(b.hot(), None);
        assert_eq!(b.evicted_count(), 1);
        assert_eq!(b.sealed_count(), 0);
        // Die Barriere ist sofort wieder frei.
        let next = b.begin(ShardId::new("b", 0)).expect("begin nach drop");
        b.seal(next, b"b", PASS).expect("seal");
        assert_eq!(b.sealed_count(), 1);
        assert_eq!(b.evicted_count(), 1);
    }

    #[test]
    fn verbrauchtes_token_zaehlt_drop_nicht_doppelt() {
        let mut b = PhaseBarrier::new();
        let hot = b.begin(ShardId::new("a", 0)).expect("begin");
        b.seal(hot, b"a", PASS).expect("seal");
        // `hot` war konsumiert: Sein Drop darf nichts mehr zaehlen.
        assert_eq!(b.evicted_count(), 0);
        let hot2 = b.begin(ShardId::new("b", 0)).expect("begin2");
        b.evict(hot2).expect("evict");
        assert_eq!(b.evicted_count(), 1);
    }
}
