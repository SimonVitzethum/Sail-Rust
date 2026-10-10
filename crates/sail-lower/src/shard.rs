//! Shard-Verarbeitung mit begrenztem Speicher.
//!
//! RAM-Regel (Lane 09): `sysregs_autogen.sail` (160k Zeilen) muss in
//! konstanter Shard-Größe durchlaufen. Deshalb:
//!
//! * [`lower_stream`] zieht Defs aus einem [`Iterator`] und gibt einen
//!   [`Iterator`] zurück — zu keinem Zeitpunkt liegt der Gesamt-AST im
//!   Speicher (kein Gesamt-Clone, kein Sammel-`Vec`).
//! * Jede Def wird per Ownership in [`anf_lower`](crate::anf_lower)
//!   bewegt und danach gedroppt (frühes Drop); nur das Lowering-Ergebnis
//!   der laufenden Shard (`shard_size` Defs) lebt gleichzeitig.
//! * [`lower_shard`] verarbeitet genau eine Shard; parallel via
//!   `std::thread::scope` (multithreaded, ohne `unsafe`, ohne extra Deps).

use crate::anf::{AnfConfig, AnfError, anf_lower};
use crate::ir::FunDef;
use std::fmt;

/// Konfiguration der Shard-Verarbeitung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardConfig {
    /// Max. Defs pro Shard (= konstante Speicher-Schranke).
    pub shard_size: usize,
    pub anf: AnfConfig,
}

impl Default for ShardConfig {
    fn default() -> Self {
        ShardConfig { shard_size: 64, anf: AnfConfig::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShardStats {
    pub functions: usize,
    pub shards: usize,
    pub temps: usize,
}

impl fmt::Display for ShardStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} functions in {} shards", self.functions, self.shards)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardError {
    Anf(AnfError),
    /// Leere Shard (sollte nicht vorkommen; schützt vor stillen Lücken).
    EmptyShard,
}

impl fmt::Display for ShardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShardError::Anf(e) => write!(f, "{e}"),
            ShardError::EmptyShard => write!(f, "shard: empty shard"),
        }
    }
}

impl std::error::Error for ShardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShardError::Anf(e) => Some(e),
            ShardError::EmptyShard => None,
        }
    }
}

impl From<AnfError> for ShardError {
    fn from(e: AnfError) -> Self {
        ShardError::Anf(e)
    }
}

/// Lowert genau eine Shard (Ownership; Originale werden verbraucht/gedroppt).
///
/// Gibt `(gelowerte_defs, temps_in_shard)` zurück. Parallelisiert pro Def
/// via `std::thread::scope` — jede Def ist unabhängig (nur pro Funktion).
pub fn lower_shard(
    shard: Vec<FunDef>,
    config: &AnfConfig,
) -> Result<(Vec<FunDef>, usize), ShardError> {
    if shard.is_empty() {
        return Err(ShardError::EmptyShard);
    }
    // Parallel pro Def; `scope` erlaubt Ausleihen von `config` ohne `unsafe`.
    // Bei genau einer Def kein Thread-Overhead.
    if shard.len() == 1 {
        let mut shard = shard;
        // `remove(0)` statt `into_iter().next()`: behält Vec-Allokation nicht.
        let def = shard.remove(0);
        drop(shard);
        let before = def.size();
        let lowered = anf_lower(def, config).map_err(ShardError::Anf)?;
        let temps = lowered.size().saturating_sub(before);
        return Ok((vec![lowered], temps));
    }
    let mut out: Vec<Option<Result<FunDef, ShardError>>> = Vec::with_capacity(shard.len());
    out.resize_with(shard.len(), || None);
    let mut shard = shard;
    {
        let out_refs = &mut out;
        std::thread::scope(|s| {
            for (slot, def) in out_refs.iter_mut().zip(shard.drain(..)) {
                let cfg = &*config;
                s.spawn(move || {
                    // `def` (Original) wird hier bewegt und nach Lowering
                    // gedroppt — nur das Ergebnis landet im Slot.
                    *slot = Some(anf_lower(def, cfg).map_err(ShardError::Anf));
                });
            }
        });
    }
    drop(shard); // Drained, aber explizit: keine Originale überleben die Shard.
    let mut lowered = Vec::with_capacity(out.len());
    let mut temps = 0usize;
    for slot in out {
        match slot {
            Some(Ok(def)) => {
                temps += def.body.size();
                lowered.push(def);
            }
            Some(Err(e)) => return Err(e),
            None => panic!("shard: worker produced no result"),
        }
    }
    Ok((lowered, temps))
}

/// Streaming-Lowering über einen Def-Iterator.
///
/// Zieht `shard_size` Defs, lowert die Shard, yieldet Ergebnisse einzeln —
/// Peak-Speicher ist `O(shard_size)`, nie `O(gesamt)`. Bricht beim ersten
/// Fehler ab (harter Fehler mit Funktions-Spans aus [`anf_lower`]).
pub fn lower_stream<I>(
    defs: I,
    config: ShardConfig,
) -> impl Iterator<Item = Result<FunDef, ShardError>>
where
    I: Iterator<Item = FunDef>,
{
    LowerStream { defs, config, buf: Vec::new().into_iter(), stats: ShardStats::default() }
}

struct LowerStream<I> {
    defs: I,
    config: ShardConfig,
    buf: std::vec::IntoIter<FunDef>,
    stats: ShardStats,
}

impl<I> LowerStream<I>
where
    I: Iterator<Item = FunDef>,
{
    /// Zieht die nächste Shard aus `defs` (exakt `shard_size`, letzte kürzer).
    fn fill(&mut self) -> Result<bool, ShardError> {
        let mut shard = Vec::with_capacity(self.config.shard_size);
        for def in self.defs.by_ref().take(self.config.shard_size) {
            shard.push(def);
        }
        if shard.is_empty() {
            return Ok(false);
        }
        self.stats.shards += 1;
        self.stats.functions += shard.len();
        let (lowered, temps) = lower_shard(shard, &self.config.anf)?;
        self.stats.temps += temps;
        self.buf = lowered.into_iter();
        Ok(true)
    }
}

impl<I> Iterator for LowerStream<I>
where
    I: Iterator<Item = FunDef>,
{
    type Item = Result<FunDef, ShardError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(def) = self.buf.next() {
                return Some(Ok(def));
            }
            match self.fill() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Expr, Lit, Ty};

    fn simple(i: i128) -> FunDef {
        FunDef::new(
            format!("f{i}"),
            vec![],
            vec![],
            Ty::Named("int".to_string()),
            Expr::Block(vec![Expr::Lit(Lit::Int(i))], Box::new(Expr::Var("x".to_string()))),
        )
    }

    fn has_block(e: &Expr) -> bool {
        match e {
            Expr::Block(_, _) => true,
            Expr::Let(_, v, b) => has_block(v) || has_block(b),
            Expr::BinOp(_, l, r) => has_block(l) || has_block(r),
            Expr::Call(_, a) => a.iter().any(has_block),
            Expr::If(c, t, e) => has_block(c) || has_block(t) || has_block(e),
            Expr::Match(s, arms) => {
                has_block(s) || arms.iter().any(|(_, e)| has_block(e))
            }
            _ => false,
        }
    }

    #[test]
    fn stream_preserves_all_defs_without_collecting() {
        // Generator statt Vec: beweist, dass kein Gesamt-Clone nötig ist.
        let defs = (0..200).map(simple);
        let cfg = ShardConfig { shard_size: 16, anf: AnfConfig::default() };
        let out: Vec<_> = lower_stream(defs, cfg).collect::<Result<_, _>>().unwrap();
        assert_eq!(out.len(), 200);
        assert!(out.iter().all(|d| !has_block(&d.body)));
    }

    #[test]
    fn stream_shard_boundaries_exact() {
        // 65 Defs bei shard_size 64 => 2 Shards, nichts verloren.
        let defs = (0..65).map(simple);
        let cfg = ShardConfig { shard_size: 64, anf: AnfConfig::default() };
        let names: Vec<String> =
            lower_stream(defs, cfg).map(|r| r.unwrap().name).collect();
        assert_eq!(names.len(), 65);
        assert_eq!(names[0], "f0");
        assert_eq!(names[64], "f64");
    }

    #[test]
    fn stream_error_aborts_with_span() {
        let mut defs = vec![simple(0)];
        defs.push(FunDef::new(
            "bad",
            vec![],
            vec![],
            Ty::Named("int".to_string()),
            Expr::Match(
                Box::new(Expr::Var("x".to_string())),
                vec![(crate::ir::Pat::Lit(Lit::Int(0)), Expr::Lit(Lit::Int(1)))],
            ),
        ));
        let cfg = ShardConfig { shard_size: 1, anf: AnfConfig::default() };
        let results: Vec<_> = lower_stream(defs.into_iter(), cfg).collect();
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ShardError::Anf(_))));
    }

    #[test]
    fn lower_shard_rejects_empty() {
        assert_eq!(
            lower_shard(vec![], &AnfConfig::default()),
            Err(ShardError::EmptyShard)
        );
    }

    #[test]
    fn lower_shard_single_def_no_block_left() {
        let (out, _) = lower_shard(vec![simple(1)], &AnfConfig::default()).unwrap();
        assert_eq!(out.len(), 1);
        assert!(!has_block(&out[0].body));
    }

    #[test]
    fn lower_shard_parallel_many() {
        let shard: Vec<_> = (0..32).map(simple).collect();
        let (out, _) = lower_shard(shard, &AnfConfig::default()).unwrap();
        assert_eq!(out.len(), 32);
    }
}
