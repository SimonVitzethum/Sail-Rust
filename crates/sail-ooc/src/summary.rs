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

/// Harter Deckel fuer gehaltene Tabellen-Eintraege (RAM-Schutz, Lane 126):
/// [`SummaryTable::insert`] weist darueber hinaus ab ([`SummaryError::TooMany`]),
/// statt still zu wachsen. Wer mehr Shards streamt, entfernt fertige per
/// [`SummaryTable::remove`] (ein Eintrag pro Shard; Ersetzen desselben Shards
/// ist immer erlaubt und zaehlt nicht gegen den Deckel).
pub const MAX_SUMMARY_ENTRIES: usize = 65_536;

/// Harter Deckel fuer Draht-Counts beim Dekodieren (DoS-Schutz, Lane 126):
/// Kein gueltiger Eintrag (≤ [`MAX_SUMMARY_BYTES`]) traegt mehr Zaehler —
/// kleinster Eintrag (leerer Name: 4 B Laengen-Prefix) passt hoechstens
/// 16_384-mal hinein. Groessere Counts sind korrupt/fremd.
pub(crate) const MAX_DECODE_ENTRIES: usize = 16_384;

/// Prueft `n` gegen das u32-Drahtformat (Lane 126, Anti-Trunkierung): `as u32`
/// wuerde oberhalb `u32::MAX` still kappen und korrupte Belege erzeugen.
/// `Err` traegt die beanstandete Groesse (Aufrufer mappen auf
/// [`SummaryError::TooLarge`]).
pub(crate) fn check_u32_len(n: usize) -> Result<u32, usize> {
    u32::try_from(n).map_err(|_| n)
}

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
    /// Tabellen-Deckel ([`MAX_SUMMARY_ENTRIES`]) erreicht (RAM-Schutz; erst
    /// fertige Shards per [`SummaryTable::remove`] austragen).
    TooMany {
        /// Gehaltene Eintraege.
        entries: usize,
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
            Self::TooMany { entries } => write!(
                f,
                "Summary-Tabelle voll ({entries} Eintraege; erst fertige Shards austragen)"
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
    ///
    /// Faellt bei logisch riesigen Eingaben (`> u32::MAX` Namen/Eintraege)
    /// auf [`ShardSummary::try_encode`] zurueck (gepruefter Pfad); direkte
    /// `encode`-Aufrufer mit solchen Eingaben bekommen einen Panic statt
    /// still gekappter Laengen (Anti-Trunkierung, Lane 126).
    fn encode(&self) -> Vec<u8>;
    /// Gepruefte Serialisierung: wie `encode`, meldet u32-Ueberlauf aber als
    /// [`SummaryError::TooLarge`] statt zu kappen. [`SummaryRecord::from_summary`]
    /// nutzt diesen Pfad. Default: `Ok(self.encode())` (fuer externe Impls mit
    /// kleinen, statisch begrenzten Fakten).
    fn try_encode(&self) -> Result<Vec<u8>, SummaryError> {
        Ok(self.encode())
    }
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
    /// Nutzt den geprueften Pfad ([`ShardSummary::try_encode`]): u32-Ueberlauf
    /// wird [`SummaryError::TooLarge`], nie eine gekappte Laenge.
    pub fn from_summary<S: ShardSummary>(summary: &S) -> Result<Self, SummaryError> {
        let payload = summary.try_encode()?;
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
    ///
    /// Deckel: Haelt die Tabelle bereits [`MAX_SUMMARY_ENTRIES`] *fremde*
    /// Shards, kommt [`SummaryError::TooMany`] (RAM-Schutz) — erst fertige
    /// Shards per [`SummaryTable::remove`] austragen. Ersetzen desselben
    /// Shards ist immer erlaubt.
    pub fn insert(&mut self, record: SummaryRecord) -> Result<(), SummaryError> {
        if !self.entries.contains_key(&record.shard) && self.entries.len() >= MAX_SUMMARY_ENTRIES {
            return Err(SummaryError::TooMany {
                entries: self.entries.len(),
            });
        }
        self.entries.insert(record.shard.clone(), record);
        Ok(())
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

    /// Alle Shards mit Summary, deterministisch sortiert (Name, Chunk).
    /// Die Tabelle nutzt eine `HashMap` (unsortierte Iteration); ohne
    /// Sortierung waere jede Auflistung laufabhaengig (Lane 126: Ordnung).
    #[must_use]
    pub fn shards(&self) -> Vec<ShardId> {
        let mut out: Vec<ShardId> = self.entries.keys().cloned().collect();
        out.sort();
        out
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
        self.try_encode().expect("ExportedSymbols passt ins u32-Drahtformat")
    }

    fn try_encode(&self) -> Result<Vec<u8>, SummaryError> {
        let too_large = |bytes: usize| SummaryError::TooLarge {
            shard: self.shard.clone(),
            bytes,
        };
        let mut names = self.symbols.clone();
        names.sort();
        names.dedup();
        let mut out = Vec::new();
        let n = check_u32_len(names.len()).map_err(too_large)?;
        out.extend_from_slice(&n.to_le_bytes());
        for name in &names {
            let len = check_u32_len(name.len()).map_err(too_large)?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        Ok(out)
    }

    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError> {
        let err = |msg: &str| SummaryError::Decode(msg.into());
        if bytes.len() > MAX_SUMMARY_BYTES {
            return Err(err("Eintrag ueber Deckel"));
        }
        let n_bytes = bytes.get(..4).ok_or_else(|| err("kein Count"))?;
        let n = u32::from_le_bytes([n_bytes[0], n_bytes[1], n_bytes[2], n_bytes[3]]);
        if n as usize > MAX_DECODE_ENTRIES {
            return Err(err("Count ueber Deckel"));
        }
        let n = n as usize;
        let mut rest = &bytes[4..];
        let mut symbols = Vec::new();
        for _ in 0..n {
            let l_bytes = rest.get(..4).ok_or_else(|| err("keine Laenge"))?;
            let len =
                u32::from_le_bytes([l_bytes[0], l_bytes[1], l_bytes[2], l_bytes[3]]) as usize;
            if len > MAX_SUMMARY_BYTES {
                return Err(err("Laenge ueber Deckel"));
            }
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
        table.insert(a).expect("insert");
        let mut b_sym = sample(2);
        b_sym.shard = ShardId::new("sve", 0);
        let b = SummaryRecord::from_summary(&b_sym).expect("b");
        let bytes_b = b.len();
        table.insert(b).expect("insert");
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
    fn tabelle_deckel_und_sortierte_ordnung() {
        assert!(MAX_SUMMARY_ENTRIES > 0);
        assert!(MAX_DECODE_ENTRIES > 0);
        // Ordnung: unsortiert eingepflegt, sortiert aufgelistet (deterministisch).
        let mut table = SummaryTable::new();
        for name in ["sve", "a", "instrs32", "m"] {
            let mut s = sample(1);
            s.shard = ShardId::new(name, 0);
            table.insert(SummaryRecord::from_summary(&s).expect("klein")).expect("insert");
        }
        let names: Vec<String> = table.shards().iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["a", "instrs32", "m", "sve"]);
        // Deckel: Ersetzen desselben Shards bleibt erlaubt (kein TooMany).
        let mut s = sample(9);
        s.shard = ShardId::new("a", 0);
        table.insert(SummaryRecord::from_summary(&s).expect("ersatz")).expect("ersatz");
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn decode_caps_weisen_riesen_counts_ab() {
        let shard = ShardId::new("a", 0);
        let content = ContentId::of_bytes(b"c");
        // Count ueber Deckel (kleine Bytes, riesiger Count).
        let mut huge_count = Vec::new();
        huge_count.extend_from_slice(&(MAX_DECODE_ENTRIES as u32 + 1).to_le_bytes());
        assert!(ExportedSymbols::decode(shard.clone(), content, &huge_count).is_err());
        // Gesamtbytes ueber Deckel.
        let big = vec![0u8; MAX_SUMMARY_BYTES + 1];
        assert!(ExportedSymbols::decode(shard.clone(), content, &big).is_err());
        // Einzellaenge ueber Deckel (Count 1, len = u32::MAX).
        let huge_len = vec![1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        assert!(ExportedSymbols::decode(shard.clone(), content, &huge_len).is_err());
        // Gepruefter Pfad meldet wie ungepruefter dieselben Bytes.
        let s = sample(3);
        assert_eq!(s.try_encode().expect("klein"), s.encode());
        // u32-Pruefung: Ueberlauf wird erkannt statt gekappt.
        assert!(check_u32_len(u32::MAX as usize).is_ok());
        assert_eq!(check_u32_len(u32::MAX as usize + 1), Err(u32::MAX as usize + 1));
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
