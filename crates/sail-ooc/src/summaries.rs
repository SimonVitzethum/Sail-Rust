//! Interprozedurale Pass-Summaries (Lane 74, E4-bindend).
//!
//! Lanes 08/09 brechen Out-of-core, wenn sie Bodies programmweit laden
//! (globaler Callgraph, globale Monomorphisierung, programmweite Const-Prop).
//! Vertrag wie in [`crate::summary`]: In RAM liegt nur
//! [`SummaryTable`](crate::SummaryTable) (ein Eintrag pro Shard,
//! hoechstens [`MAX_SUMMARY_BYTES`](crate::MAX_SUMMARY_BYTES)), Bodies bleiben
//! warm/kalt auf Platte und werden nur per
//! [`ContentId`](crate::ContentId) referenziert.
//!
//! Dieses Modul liefert je interprozeduralem Pass einen Summary-Typ
//! ([`ShardSummary`](crate::ShardSummary)-Impl) plus Export-Schnittstelle
//! (bauen → [`SummaryRecord`](crate::SummaryRecord) → in die Tabelle):
//!
//! | Typ | Pass (gelesen, nicht umgebaut) | Fakten |
//! |---|---|---|
//! | [`CallgraphSummary`] | `sail-effects` (Lane 08: `CallGraph`, `effect_label`) | definierte Funktionen je Shard, direkte Callees, Effektklasse, Loop-Flag |
//! | [`MonoSummary`] | `sail-lower` (Lane 09: `monomorphize`, `MonoConfig`) | angefragte Mono-Instanzen je Shard (`func + Typ-Argumente`), keine Bodies |
//! | [`ConstPropSummary`] | `sail-rewrite` (`const_prop_mutrec`-Stub im Lean-Order) | pro Shard bewiesene Konstanten (`func :: var = value`), Abwesenheit = unbekannt |
//!
//! Absichtlich keine Abhaengigkeit auf die Pass-Crates: Die Treiber mappen in
//! 2–3 Zeilen von der Pass-IR auf reine Daten (Beispiele unten). Fremde
//! Crates bleiben unberuehrt, nichts wird gedoppelt — [`crate::SummaryTable`]
//! wird wiederverwendet.
//!
//! ## Treiber-Skizzen
//!
//! ```text
//! // Lane 08 (sail-effects), pro heissem Shard:
//! let cg = sail_effects::CallGraph::build(&funs);
//! let facts = funs.iter().map(|f| CallFunc {
//!     name: f.name.clone(),
//!     callees: cg.callees_of(&f.name).into_iter().map(str::to_owned).collect(),
//!     effect: EffectClass::from_label(sail_effects::effect_label(f)),
//!     has_loop: sail_effects::has_loop(f),
//! });
//! let summary = CallgraphSummary::new(shard.clone(), receipt.content, facts);
//! export_callgraph(&mut table, &summary)?; // Bodies bleiben auf Platte
//!
//! // Lane 09 (sail-lower), pro heissem Shard:
//! let summary = MonoSummary::new(shard.clone(), receipt.content,
//!     requested.iter().map(|(f, args)| MonoInstance {
//!         func: f.clone(),
//!         type_args: args.iter().map(ToString::to_string).collect(),
//!     }));
//! export_mono(&mut table, &summary)?;
//!
//! // const_prop_mutrec (sail-rewrite), pro heissem Shard:
//! let summary = ConstPropSummary::new(shard.clone(), receipt.content,
//!     proven.map(|(f, v, c)| ConstFact { func: f.into(), var: v.into(), value: c }));
//! export_const_prop(&mut table, &summary)?;
//! ```
//!
//! Verifikation per Test-Rig (spaeter, zentral — E3-Moratorium: keine
//! Builds/Tests in Lane-Agents); Tests liegen als Code bei.

#![forbid(unsafe_code)]

use sail_shard::ShardId;

use crate::{ContentId, ShardSummary, SummaryError, SummaryRecord, SummaryTable};

// --- Draht-Helfer (deterministisch, little-endian) ---------------------------

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn write_i128(out: &mut Vec<u8>, v: i128) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], SummaryError> {
        if self.rest.len() < n {
            return Err(SummaryError::Decode(format!("Summary unlesbar: {what}")));
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, SummaryError> {
        match self.take(1, "kein Byte") {
            Ok(b) => Ok(b[0]),
            Err(e) => Err(e),
        }
    }

    fn u32(&mut self) -> Result<u32, SummaryError> {
        let b = self.take(4, "kein u32")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i128(&mut self) -> Result<i128, SummaryError> {
        let b = self.take(16, "kein i128")?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(b);
        Ok(i128::from_le_bytes(arr))
    }

    fn str(&mut self) -> Result<String, SummaryError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len, "String abgeschnitten")?;
        let s = std::str::from_utf8(bytes)
            .map_err(|_| SummaryError::Decode("Summary unlesbar: kein UTF-8".into()))?;
        if s.contains('\0') {
            return Err(SummaryError::Decode("Summary unlesbar: NUL".into()));
        }
        Ok(s.to_owned())
    }

    fn end(self) -> Result<(), SummaryError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(SummaryError::Decode("Summary unlesbar: Trailing Bytes".into()))
        }
    }
}

// --- Callgraph (Lane 08) -----------------------------------------------------

/// Effektklasse einer Funktion (Spiegel von `sail-effects::EffectLabel`).
///
/// Absichtlich eigene Mini-Enum (kein Dep auf `sail-effects`): Treiber mappen
/// per [`EffectClass::from_lane08`] bzw. per Match auf `Pure/ExceptM/SailM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectClass {
    /// Keine Register-/Memory-/Nondet-Effekte, kein `throw`.
    Pure,
    /// Darf frueh zurueckkehren/werfen, sonst effektfrei.
    ExceptM,
    /// Darf Register/Memory/Nondeterminismus beruehren (staerkstes Label).
    SailM,
}

impl EffectClass {
    /// Drahtwert (stabil).
    #[must_use]
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Pure => 0,
            Self::ExceptM => 1,
            Self::SailM => 2,
        }
    }

    /// Gegenstueck zu [`Self::as_u8`].
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Pure),
            1 => Some(Self::ExceptM),
            2 => Some(Self::SailM),
            _ => None,
        }
    }

    /// Mapping-Hilfe fuer Lane-08-Treiber: `name` ist der Debug-Name des
    /// Labels (`"Pure"`, `"ExceptM"`, `"SailM"`, wie `format!("{:?}", label)`).
    #[must_use]
    pub fn from_lane08(label_debug: &str) -> Option<Self> {
        match label_debug {
            "Pure" => Some(Self::Pure),
            "ExceptM" => Some(Self::ExceptM),
            "SailM" => Some(Self::SailM),
            _ => None,
        }
    }
}

/// Fakten einer Funktion fuer die Callgraph-Summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFunc {
    /// Funktionsname (shard-lokal definiert).
    pub name: String,
    /// Direkte Callees (Namen; Externs/Primitives ok).
    pub callees: Vec<String>,
    /// Direkte Effektklasse (transitiv loest der Treiber via Tabelle).
    pub effect: EffectClass,
    /// Ob der Body eine Schleife enthaelt (`while`/`for`).
    pub has_loop: bool,
}

/// Cross-Shard-Summary des Callgraph-/Effekt-Passes (Lane 08).
///
/// Haelt nur Namen + aggregierte Fakten, nie Bodies. `encode` sortiert
/// deterministisch (Funktionen nach Name, Callees sortiert/deduped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallgraphSummary {
    /// Zugehoeriger Shard.
    pub shard: ShardId,
    /// Content-Hash des Bodies (warm/kalt referenziert).
    pub content: ContentId,
    /// Fakten je definierter Funktion.
    pub funcs: Vec<CallFunc>,
}

impl CallgraphSummary {
    /// Baut eine Summary; sortiert/deduped Callees deterministisch.
    #[must_use]
    pub fn new(
        shard: ShardId,
        content: ContentId,
        funcs: impl IntoIterator<Item = CallFunc>,
    ) -> Self {
        let mut funcs: Vec<CallFunc> = funcs
            .into_iter()
            .map(|mut f| {
                f.callees.sort();
                f.callees.dedup();
                f
            })
            .collect();
        funcs.sort_by(|a, b| a.name.cmp(&b.name));
        Self { shard, content, funcs }
    }
}

impl ShardSummary for CallgraphSummary {
    fn kind() -> &'static str {
        "ooc/callgraph-v1"
    }

    fn shard(&self) -> &ShardId {
        &self.shard
    }

    fn content(&self) -> ContentId {
        self.content
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_u32(&mut out, self.funcs.len() as u32);
        for f in &self.funcs {
            write_str(&mut out, &f.name);
            write_u8(&mut out, f.effect.as_u8());
            write_u8(&mut out, u8::from(f.has_loop));
            write_u32(&mut out, f.callees.len() as u32);
            for c in &f.callees {
                write_str(&mut out, c);
            }
        }
        out
    }

    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError> {
        let mut cur = Cursor::new(bytes);
        let n = cur.u32()? as usize;
        let mut funcs = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let name = cur.str()?;
            let effect = cur.u8()?;
            let has_loop = cur.u8()?;
            let effect = EffectClass::from_u8(effect)
                .ok_or_else(|| SummaryError::Decode("Summary unlesbar: Effekt".into()))?;
            let has_loop = match has_loop {
                0 => false,
                1 => true,
                _ => return Err(SummaryError::Decode("Summary unlesbar: Loop-Flag".into())),
            };
            let m = cur.u32()? as usize;
            let mut callees = Vec::with_capacity(m.min(1024));
            for _ in 0..m {
                callees.push(cur.str()?);
            }
            funcs.push(CallFunc { name, callees, effect, has_loop });
        }
        cur.end()?;
        Ok(Self::new(shard, content, funcs))
    }
}

/// Baut den Tabellen-Eintrag fuer eine Callgraph-Summary (prueft Groesse).
pub fn callgraph_record(summary: &CallgraphSummary) -> Result<SummaryRecord, SummaryError> {
    SummaryRecord::from_summary(summary)
}

/// Export-Schnittstelle Callgraph: Eintrag bauen + in die Tabelle einpflegen.
///
/// Ersetzt aeltere Eintraege desselben Shards. Bodies bleiben auf Platte —
/// nur [`ContentId`] + Fakten landen in RAM.
pub fn export_callgraph(
    table: &mut SummaryTable,
    summary: &CallgraphSummary,
) -> Result<(), SummaryError> {
    table.insert(callgraph_record(summary)?);
    Ok(())
}

// --- Mono (Lane 09) ----------------------------------------------------------

/// Angeforderte Mono-Instanz: polymorphe Funktion + konkrete Typ-Argumente.
///
/// Typ-Argumente als Text (`ToString` von `sail-lower::Ty`, z. B. `"int"`,
/// `"bits(32)"`): Die Summary haelt nur Namen, nie Bodies/IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoInstance {
    /// Polymorphe Funktion (z. B. `"id"`).
    pub func: String,
    /// Konkrete Typ-Argumente (z. B. `["int"]`).
    pub type_args: Vec<String>,
}

/// Cross-Shard-Summary der Monomorphisierung (Lane 09).
///
/// Haelt die angefragten Instanzen (fuer globale Duplikat-Erkennung und
/// Split-Limit-Abgleich), nie spezialisierte Bodies — die liegen warm/kalt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoSummary {
    /// Zugehoeriger Shard.
    pub shard: ShardId,
    /// Content-Hash des Bodies (warm/kalt referenziert).
    pub content: ContentId,
    /// Angeforderte Instanzen.
    pub instances: Vec<MonoInstance>,
}

impl MonoSummary {
    /// Baut eine Summary; sortiert deterministisch (func, dann Typ-Argumente).
    #[must_use]
    pub fn new(
        shard: ShardId,
        content: ContentId,
        instances: impl IntoIterator<Item = MonoInstance>,
    ) -> Self {
        let mut instances: Vec<MonoInstance> = instances.into_iter().collect();
        instances.sort_by(|a, b| {
            a.func.cmp(&b.func).then_with(|| a.type_args.cmp(&b.type_args))
        });
        Self { shard, content, instances }
    }
}

impl ShardSummary for MonoSummary {
    fn kind() -> &'static str {
        "ooc/mono-v1"
    }

    fn shard(&self) -> &ShardId {
        &self.shard
    }

    fn content(&self) -> ContentId {
        self.content
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_u32(&mut out, self.instances.len() as u32);
        for inst in &self.instances {
            write_str(&mut out, &inst.func);
            write_u32(&mut out, inst.type_args.len() as u32);
            for arg in &inst.type_args {
                write_str(&mut out, arg);
            }
        }
        out
    }

    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError> {
        let mut cur = Cursor::new(bytes);
        let n = cur.u32()? as usize;
        let mut instances = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let func = cur.str()?;
            let m = cur.u32()? as usize;
            let mut type_args = Vec::with_capacity(m.min(16));
            for _ in 0..m {
                type_args.push(cur.str()?);
            }
            instances.push(MonoInstance { func, type_args });
        }
        cur.end()?;
        Ok(Self::new(shard, content, instances))
    }
}

/// Baut den Tabellen-Eintrag fuer eine Mono-Summary (prueft Groesse).
pub fn mono_record(summary: &MonoSummary) -> Result<SummaryRecord, SummaryError> {
    SummaryRecord::from_summary(summary)
}

/// Export-Schnittstelle Mono: Eintrag bauen + in die Tabelle einpflegen.
pub fn export_mono(table: &mut SummaryTable, summary: &MonoSummary) -> Result<(), SummaryError> {
    table.insert(mono_record(summary)?);
    Ok(())
}

// --- Const-Prop (sail-rewrite, const_prop_mutrec) -----------------------------

/// Bewiesener Konstantenwert (nur bekannte Fakten werden gehalten;
/// Abwesenheit = unbekannt/nicht-konstant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstValue {
    /// Ganzzahlige Konstante.
    Int(i128),
    /// Boolesche Konstante.
    Bool(bool),
    /// Bitvektor-Literal als Text (z. B. `"0b101"`).
    Bits(String),
}

impl ConstValue {
    fn tag(&self) -> u8 {
        match self {
            Self::Int(_) => 1,
            Self::Bool(_) => 2,
            Self::Bits(_) => 3,
        }
    }
}

/// Bewiesenes Konstanten-Faktum: `func :: var = value`.
///
/// Nur intraprozedural bewiesene, exportierte Fakten (z. B. `mutrec`-Fixpunkt-
/// Export); der Fixpunkt selbst iteriert shard-weise ueber diese Tabelle,
/// Bodies bleiben auf Platte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstFact {
    /// Funktion, in der das Faktum gilt.
    pub func: String,
    /// Variable mit konstantem Wert.
    pub var: String,
    /// Bewiesener Wert.
    pub value: ConstValue,
}

/// Cross-Shard-Summary der Const-Prop (`const_prop_mutrec`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstPropSummary {
    /// Zugehoeriger Shard.
    pub shard: ShardId,
    /// Content-Hash des Bodies (warm/kalt referenziert).
    pub content: ContentId,
    /// Bewiesene Fakten.
    pub facts: Vec<ConstFact>,
}

impl ConstPropSummary {
    /// Baut eine Summary; sortiert deterministisch (func, var).
    #[must_use]
    pub fn new(
        shard: ShardId,
        content: ContentId,
        facts: impl IntoIterator<Item = ConstFact>,
    ) -> Self {
        let mut facts: Vec<ConstFact> = facts.into_iter().collect();
        facts.sort_by(|a, b| a.func.cmp(&b.func).then_with(|| a.var.cmp(&b.var)));
        Self { shard, content, facts }
    }
}

impl ShardSummary for ConstPropSummary {
    fn kind() -> &'static str {
        "ooc/constprop-v1"
    }

    fn shard(&self) -> &ShardId {
        &self.shard
    }

    fn content(&self) -> ContentId {
        self.content
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_u32(&mut out, self.facts.len() as u32);
        for fact in &self.facts {
            write_str(&mut out, &fact.func);
            write_str(&mut out, &fact.var);
            write_u8(&mut out, fact.value.tag());
            match &fact.value {
                ConstValue::Int(v) => write_i128(&mut out, *v),
                ConstValue::Bool(b) => write_u8(&mut out, u8::from(*b)),
                ConstValue::Bits(s) => write_str(&mut out, s),
            }
        }
        out
    }

    fn decode(shard: ShardId, content: ContentId, bytes: &[u8]) -> Result<Self, SummaryError> {
        let mut cur = Cursor::new(bytes);
        let n = cur.u32()? as usize;
        let mut facts = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let func = cur.str()?;
            let var = cur.str()?;
            let value = match cur.u8()? {
                1 => ConstValue::Int(cur.i128()?),
                2 => match cur.u8()? {
                    0 => ConstValue::Bool(false),
                    1 => ConstValue::Bool(true),
                    _ => {
                        return Err(SummaryError::Decode(
                            "Summary unlesbar: Bool".into(),
                        ));
                    }
                },
                3 => ConstValue::Bits(cur.str()?),
                _ => {
                    return Err(SummaryError::Decode(
                        "Summary unlesbar: Const-Tag".into(),
                    ));
                }
            };
            facts.push(ConstFact { func, var, value });
        }
        cur.end()?;
        Ok(Self::new(shard, content, facts))
    }
}

/// Baut den Tabellen-Eintrag fuer eine Const-Prop-Summary (prueft Groesse).
pub fn const_prop_record(summary: &ConstPropSummary) -> Result<SummaryRecord, SummaryError> {
    SummaryRecord::from_summary(summary)
}

/// Export-Schnittstelle Const-Prop: Eintrag bauen + in die Tabelle einpflegen.
pub fn export_const_prop(
    table: &mut SummaryTable,
    summary: &ConstPropSummary,
) -> Result<(), SummaryError> {
    table.insert(const_prop_record(summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_SUMMARY_BYTES;

    fn shard(name: &str) -> ShardId {
        ShardId::new(name, 0)
    }

    fn content(b: &[u8]) -> ContentId {
        ContentId::of_bytes(b)
    }

    #[test]
    fn callgraph_rundreise_und_determinismus() {
        let a = CallgraphSummary::new(
            shard("instrs32"),
            content(b"a"),
            vec![
                CallFunc {
                    name: "execute".into(),
                    callees: vec!["decode".into(), "decode".into()],
                    effect: EffectClass::SailM,
                    has_loop: true,
                },
                CallFunc {
                    name: "decode".into(),
                    callees: vec![],
                    effect: EffectClass::Pure,
                    has_loop: false,
                },
            ],
        );
        // Unsortierte Eingabe, doppelte Callees → kanonische Ordnung.
        let b = CallgraphSummary::new(
            shard("instrs32"),
            content(b"a"),
            vec![
                CallFunc {
                    name: "decode".into(),
                    callees: vec![],
                    effect: EffectClass::Pure,
                    has_loop: false,
                },
                CallFunc {
                    name: "execute".into(),
                    callees: vec!["decode".into()],
                    effect: EffectClass::SailM,
                    has_loop: true,
                },
            ],
        );
        assert_eq!(a.encode(), b.encode());
        let record = callgraph_record(&a).expect("klein genug");
        assert_eq!(record.kind, CallgraphSummary::kind());
        let back =
            CallgraphSummary::decode(record.shard.clone(), record.content, &record.payload)
                .expect("decode");
        assert_eq!(back.funcs.len(), 2);
        assert_eq!(back.funcs[0].name, "decode");
        assert_eq!(back.funcs[1].callees, vec!["decode".to_owned()]);
        // Effekt-Mapping-Hilfe.
        assert_eq!(EffectClass::from_lane08("SailM"), Some(EffectClass::SailM));
        assert_eq!(EffectClass::from_lane08("Unbekannt"), None);
    }

    #[test]
    fn mono_rundreise_ohne_bodies() {
        let s = MonoSummary::new(
            shard("v8_base"),
            content(b"m"),
            vec![
                MonoInstance { func: "id".into(), type_args: vec!["bool".into()] },
                MonoInstance { func: "id".into(), type_args: vec!["int".into()] },
            ],
        );
        let record = mono_record(&s).expect("klein genug");
        assert_eq!(record.kind, MonoSummary::kind());
        let back =
            MonoSummary::decode(record.shard.clone(), record.content, &record.payload)
                .expect("decode");
        // Sortiert: bool < int.
        assert_eq!(back.instances[0].type_args, vec!["bool".to_owned()]);
        assert_eq!(back.instances[1].type_args, vec!["int".to_owned()]);
        // Nur Referenzen, keine Bodies.
        assert_eq!(record.content, content(b"m"));
    }

    #[test]
    fn const_prop_rundreise() {
        let s = ConstPropSummary::new(
            shard("sve"),
            content(b"c"),
            vec![
                ConstFact {
                    func: "f".into(),
                    var: "n".into(),
                    value: ConstValue::Int(-42),
                },
                ConstFact {
                    func: "f".into(),
                    var: "b".into(),
                    value: ConstValue::Bool(true),
                },
                ConstFact {
                    func: "g".into(),
                    var: "v".into(),
                    value: ConstValue::Bits("0b101".into()),
                },
            ],
        );
        let record = const_prop_record(&s).expect("klein genug");
        assert_eq!(record.kind, ConstPropSummary::kind());
        let back =
            ConstPropSummary::decode(record.shard.clone(), record.content, &record.payload)
                .expect("decode");
        assert_eq!(back.facts.len(), 3);
        assert!(back.facts.contains(&ConstFact {
            func: "f".into(),
            var: "n".into(),
            value: ConstValue::Int(-42),
        }));
    }

    #[test]
    fn export_pflegt_ein_und_ersetzt() {
        let mut table = SummaryTable::new();
        let first = CallgraphSummary::new(
            shard("a"),
            content(b"1"),
            vec![CallFunc {
                name: "f".into(),
                callees: vec![],
                effect: EffectClass::Pure,
                has_loop: false,
            }],
        );
        export_callgraph(&mut table, &first).expect("export");
        let mono = MonoSummary::new(
            shard("a"),
            content(b"2"),
            vec![MonoInstance { func: "id".into(), type_args: vec!["int".into()] }],
        );
        // Gleicher Shard, andere Summary-Art: ersetzt (ein Eintrag pro Shard).
        export_mono(&mut table, &mono).expect("export");
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(&shard("a")).expect("da").kind, MonoSummary::kind());
        let cp = ConstPropSummary::new(shard("b"), content(b"3"), vec![]);
        export_const_prop(&mut table, &cp).expect("export");
        assert_eq!(table.len(), 2);
        assert!(table.remove(&shard("b")));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn zu_grosse_summaries_werden_abgewiesen() {
        let big = MonoSummary::new(
            shard("riesig"),
            content(b"x"),
            vec![MonoInstance {
                func: "f".into(),
                type_args: vec!["y".repeat(MAX_SUMMARY_BYTES)],
            }],
        );
        let err = mono_record(&big).expect_err("zu gross");
        assert!(matches!(err, SummaryError::TooLarge { .. }));
        let big_cg = CallgraphSummary::new(
            shard("riesig"),
            content(b"x"),
            vec![CallFunc {
                name: "y".repeat(MAX_SUMMARY_BYTES),
                callees: vec![],
                effect: EffectClass::Pure,
                has_loop: false,
            }],
        );
        assert!(callgraph_record(&big_cg).is_err());
        let big_cp = ConstPropSummary::new(
            shard("riesig"),
            content(b"x"),
            vec![ConstFact {
                func: "f".into(),
                var: "v".into(),
                value: ConstValue::Bits("z".repeat(MAX_SUMMARY_BYTES)),
            }],
        );
        assert!(const_prop_record(&big_cp).is_err());
    }

    #[test]
    fn decode_lehnt_muill_ab() {
        let id = shard("a");
        let c = content(b"c");
        assert!(CallgraphSummary::decode(id.clone(), c, b"").is_err());
        assert!(MonoSummary::decode(id.clone(), c, b"\xff\xff").is_err());
        assert!(ConstPropSummary::decode(id.clone(), c, &[1, 0, 0, 0, 9]).is_err());
        // Unbekannter Effekt-Tag.
        let mut ok = CallgraphSummary::new(
            id.clone(),
            c,
            vec![CallFunc {
                name: "f".into(),
                callees: vec![],
                effect: EffectClass::Pure,
                has_loop: false,
            }],
        )
        .encode();
        // funcs=1, name="f", effekt-Byte auf 9 drehen: offset 4+ (4+1)=9.
        ok[9] = 9;
        assert!(CallgraphSummary::decode(id, c, &ok).is_err());
    }
}
