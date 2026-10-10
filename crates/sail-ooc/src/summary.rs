//! Cross-Shard-Summaries (E4): Lanes 08/09 duerfen nur Summaries in RAM halten.
//!
//! Alles-auf-einmal-Phaesen (globale Monomorphisierung, programmweite
//! Const-Prop) brechen Out-of-core, wenn sie Bodies laden. Vertrag:
//!
//! * In RAM liegt nur [`SummaryTable`]: ein Eintrag pro Shard, jeder Eintrag
//!   klein ([`MAX_SUMMARY_BYTES`], Default 64 KiB).
//! * Shard-Bodies liegen warm/kalt und werden per [`ContentId`](crate::ContentId)
//!   referenziert ([`SummaryRecord::content`]), nie geladen ausser fuer den
//!   gerade heissen Shard.
//! * Ist ein Shard endgueltig verarbeitet, fliegt seine Summary per
//!   [`SummaryTable::remove`] aus dem RAM.
//!
//! [`ShardSummary`] ist die Schnittstelle, die Lanes 08/09 implementieren;
//! [`ExportedSymbols`] ist ein mitgeliefertes Beispiel (exportierte Namen).

use std::collections::HashMap;
use std::fmt;

use sail_shard::ShardId;

use crate::ContentId;

/// Harte Obergrenze fuer einen Summary-Eintrag (RAM-Schutz).
pub const MAX_SUMMARY_BYTES: usize = 64 * 1024;

/// Fehler beim Einpflegen einer Summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryError {
    /// Payload sprengt [`MAX_SUMMARY_BYTES`] (Bodies gehoeren auf Platte).
    TooLarge {
        /// Shard der abgelehnten Summary.
        shard: ShardId,
        /// Gemessene Payload-Groesse.
        bytes: usize,
    },
    /// Dekodieren gescheitert (korrupte/ fremde Bytes).
    Decode(String),
}

impl fmt::Display for SummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { shard, bytes } => write!(
                f,
                "Shard-Summary von {shard} zu gross ({bytes} > {MAX_SUMMARY_BYTES} Bytes; Bodies gehoeren auf Platte)"
            ),
            Self::Decode(msg) => write!(f, "Shard-Summary unlesbar: {msg}"),
        }
    }
}

impl std::error::Error for SummaryError {}

/// Schnittstelle fuer Cross-Shard-Summaries (Lanes 08/09).
///
/// Implementierungen halten nur aggregierte Fakten (Namen, Typ-Signaturen,
/// Kosten), nie ASTs/Rewrite-State. `encode` muss deterministisch sein
/// (gleiche Fakten → gleiche Bytes), damit [`SummaryRecord`] stabil bleibt.
pub trait ShardSummary: Sized {
    /// Art der Summary (z. B. `"mono/exports-v1"`); trennt Namensraeume.
    fn kind() -> &'static str;
    /// Zugehoeriger Shard.
    fn shard(&self) -> &ShardId;
    /// Content-Hash des zugehoerigen Bodies (warm/kalt referenziert).
    fn content(&self) -> ContentId;
    /// Deterministische Serialisierung (muss unter [`MAX_SUMMARY_BYTES`] bleiben).
    fn encode(&self) -> Vec<u8>;
    /// Gegenstueck zu `encode`.
    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError>;
}

/// RAM-residenter Summary-Eintrag: klein, mit Body-Referenz statt Body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRecord {
    /// Zugehoeriger Shard.
    pub shard: ShardId,
    /// Content-Hash des Bodies (Warm-/Kalt-Ablage).
    pub content: ContentId,
    /// Art der Summary (siehe [`ShardSummary::kind`]).
    pub kind: &'static str,
    /// Serialisierte Fakten (≤ [`MAX_SUMMARY_BYTES`]).
    pub payload: Vec<u8>,
}

impl SummaryRecord {
    /// Baut einen Eintrag aus einer Summary; prueft die Groessengrenze.
    pub fn from_summary<S: ShardSummary>(summary: &S) -> Result<Self, SummaryError> {
        let payload = summary.encode();
        Self::new(
            summary.shard().clone(),
            summary.content(),
            S::kind(),
            payload,
        )
    }

    /// Baut einen Eintrag aus Rohteilen; prueft die Groessengrenze.
    pub fn new(
        shard: ShardId,
        content: ContentId,
        kind: &'static str,
        payload: Vec<u8>,
    ) -> Result<Self, SummaryError> {
        if payload.len() > MAX_SUMMARY_BYTES {
            return Err(SummaryError::TooLarge {
                shard,
                bytes: payload.len(),
            });
        }
        Ok(Self {
            shard,
            content,
            kind,
            payload,
        })
    }

    /// Payload-Groesse in Bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// `true`, wenn die Payload leer ist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

/// RAM-Tabelle aller Cross-Shard-Summaries (ein Eintrag pro Shard).
#[derive(Debug, Default)]
pub struct SummaryTable {
    entries: HashMap<ShardId, SummaryRecord>,
}

impl SummaryTable {
    /// Leere Tabelle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pflegt eine Summary ein (ersetzt aeltere desselben Shards).
    pub fn insert(&mut self, record: SummaryRecord) {
        self.entries.insert(record.shard.clone(), record);
    }

    /// Holt die Summary eines Shards (reine RAM-Auskunft).
    #[must_use]
    pub fn get(&self, shard: &ShardId) -> Option<&SummaryRecord> {
        self.entries.get(shard)
    }

    /// Entfernt die Summary (Shard endgueltig fertig → RAM frei).
    /// Gibt `true` zurueck, wenn ein Eintrag vorhanden war.
    pub fn remove(&mut self, shard: &ShardId) -> bool {
        self.entries.remove(shard).is_some()
    }

    /// Anzahl gehaltener Summaries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true`, wenn keine Summary gehalten wird.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Alle Shards mit Summary (unsortiert).
    #[must_use]
    pub fn shards(&self) -> Vec<ShardId> {
        self.entries.keys().cloned().collect()
    }

    /// Gesamt-Payload in Bytes (RAM-Budget-Kontrolle).
    #[must_use]
    pub fn payload_bytes(&self) -> usize {
        self.entries.values().map(SummaryRecord::len).sum()
    }
}

/// Beispiel-Summary: exportierte Symbolnamen eines Shards.
///
/// Drahtformat (deterministisch, little-endian): `u32 n`, dann pro Name
/// `u32 len + bytes`. Namen duerfen kein `\0` enthalten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedSymbols {
    /// Zugehoeriger Shard.
    pub shard: ShardId,
    /// Content-Hash des Bodies.
    pub content: ContentId,
    /// Exportierte Namen (unsortiert ok; `encode` sortiert deterministisch).
    pub symbols: Vec<String>,
}

impl ShardSummary for ExportedSymbols {
    fn kind() -> &'static str {
        "ooc/exports-v1"
    }

    fn shard(&self) -> &ShardId {
        &self.shard
    }

    fn content(&self) -> ContentId {
        self.content
    }

    fn encode(&self) -> Vec<u8> {
        let mut names = self.symbols.clone();
        names.sort();
        names.dedup();
        let mut out = Vec::new();
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        for name in &names {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        out
    }

    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError> {
        let err = |msg: &str| SummaryError::Decode(msg.into());
        let n_bytes = bytes.get(..4).ok_or_else(|| err("kein Count"))?;
        let n = u32::from_le_bytes([n_bytes[0], n_bytes[1], n_bytes[2], n_bytes[3]]) as usize;
        let mut rest = &bytes[4..];
        let mut symbols = Vec::new();
        for _ in 0..n {
            let l_bytes = rest.get(..4).ok_or_else(|| err("keine Laenge"))?;
            let len =
                u32::from_le_bytes([l_bytes[0], l_bytes[1], l_bytes[2], l_bytes[3]]) as usize;
            let end = len.checked_add(4).ok_or_else(|| err("Laenge ueberlaeuft"))?;
            let name_bytes = rest.get(4..end).ok_or_else(|| err("Name abgeschnitten"))?;
            let name = std::str::from_utf8(name_bytes).map_err(|_| err("Name kein UTF-8"))?;
            if name.contains('\0') {
                return Err(err("Name enthaelt NUL"));
            }
            symbols.push(name.to_owned());
            rest = &rest[end..];
        }
        if !rest.is_empty() {
            return Err(err("Trailing Bytes"));
        }
        Ok(Self { shard, content, symbols })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(content_byte: u8) -> ExportedSymbols {
        ExportedSymbols {
            shard: ShardId::new("instrs32", 0),
            content: ContentId::of_bytes(&[content_byte]),
            symbols: vec!["execute".into(), "decode".into()],
        }
    }

    #[test]
    fn record_rundreise_ueber_trait() {
        let s = sample(7);
        let record = SummaryRecord::from_summary(&s).expect("klein genug");
        assert_eq!(record.kind, ExportedSymbols::kind());
        let back = ExportedSymbols::decode(record.shard.clone(), record.content, &record.payload)
            .expect("decode");
        // encode sortiert/deduped deterministisch.
        assert_eq!(back.symbols, vec!["decode".to_owned(), "execute".to_owned()]);
        // Determinismus: gleiche Fakten → gleiche Bytes.
        assert_eq!(record.payload, SummaryRecord::from_summary(&s).expect("neu").payload);
    }

    #[test]
    fn zu_grosse_summary_wird_abgewiesen() {
        let big = ExportedSymbols {
            shard: ShardId::new("riesig", 0),
            content: ContentId::of_bytes(b"x"),
            symbols: vec!["y".repeat(MAX_SUMMARY_BYTES)],
        };
        let err = SummaryRecord::from_summary(&big).expect_err("zu gross");
        assert!(matches!(err, SummaryError::TooLarge { .. }));
        assert!(err.to_string().contains("auf Platte"));
    }

    #[test]
    fn tabelle_insert_remove_budget() {
        let mut table = SummaryTable::new();
        assert!(table.is_empty());
        let a = SummaryRecord::from_summary(&sample(1)).expect("a");
        let bytes_a = a.len();
        table.insert(a);
        let mut b_sym = sample(2);
        b_sym.shard = ShardId::new("sve", 0);
        let b = SummaryRecord::from_summary(&b_sym).expect("b");
        let bytes_b = b.len();
        table.insert(b);
        assert_eq!(table.len(), 2);
        assert_eq!(table.payload_bytes(), bytes_a + bytes_b);
        assert!(table.get(&ShardId::new("sve", 0)).is_some());
        assert!(table.remove(&ShardId::new("sve", 0)));
        assert!(!table.remove(&ShardId::new("sve", 0)));
        assert_eq!(table.len(), 1);
        // Bodies stehen nie in der Tabelle — nur Content-Referenzen.
        let kept = table.get(&ShardId::new("instrs32", 0)).expect("kept");
        assert_eq!(kept.content, ContentId::of_bytes(&[1]));
    }

    #[test]
    fn decode_lehnt_muill_ab() {
        let shard = ShardId::new("a", 0);
        let content = ContentId::of_bytes(b"c");
        assert!(ExportedSymbols::decode(shard.clone(), content, b"").is_err());
        assert!(ExportedSymbols::decode(shard.clone(), content, &[1, 0, 0]).is_err());
        // Count 1, aber kein Name.
        assert!(ExportedSymbols::decode(shard.clone(), content, &[1, 0, 0, 0]).is_err());
        // Trailing Bytes.
        let mut ok = sample(9).encode();
        ok.push(0xff);
        assert!(ExportedSymbols::decode(shard, content, &ok).is_err());
    }
}
