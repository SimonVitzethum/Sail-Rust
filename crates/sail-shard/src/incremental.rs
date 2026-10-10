//! Skip-bei-unveraendert-Logik: nur geaenderte Shards neu bauen.

use crate::{Fingerprint, ShardId};
use std::collections::HashMap;

/// Plan pro Shard: ueberspringen oder neu bauen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardPlan {
    /// Fingerprint unveraendert: Shard kann uebersprungen werden.
    Skipped,
    /// Neu oder geaendert (oder Pass-Version geaendert): neu bauen.
    Dirty,
}

impl ShardPlan {
    /// `true`, wenn der Shard neu gebaut werden muss.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        *self == ShardPlan::Dirty
    }
}

/// Inkremental-Cache: merkt sich pro Shard den letzten Fingerprint.
///
/// Typischer Ablauf pro Lauf:
/// 1. Fingerprints aller Shards berechnen ([`crate::fingerprint`]).
/// 2. [`IncrementalCache::plan`] aufrufen -> nur `Dirty`-Shards bauen
///    (parallel via [`crate::Scheduler`]).
/// 3. [`IncrementalCache::mark_built`] fuer erfolgreich gebaute Shards.
/// 4. Optional [`IncrementalCache::retain_only`] zum Aufraeumen entfernter Shards.
#[derive(Debug, Clone, Default)]
pub struct IncrementalCache {
    entries: HashMap<ShardId, Fingerprint>,
}

impl IncrementalCache {
    /// Leerer Cache (Erstlauf: alles `Dirty`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Anzahl gespeicherter Shard-Fingerprints.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true`, wenn der Cache leer ist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Gespeicherten Fingerprint eines Shards abfragen.
    #[must_use]
    pub fn get(&self, id: &ShardId) -> Option<Fingerprint> {
        self.entries.get(id).copied()
    }

    /// Prueft einen einzelnen Shard: `false` = unveraendert, kann geskippt werden.
    #[must_use]
    pub fn needs_rebuild(&self, id: &ShardId, current: Fingerprint) -> bool {
        self.entries.get(id) != Some(&current)
    }

    /// Erstellt den Bauplan fuer alle Shards in der gegebenen Reihenfolge.
    ///
    /// Rueckgabe: `(ShardId, aktueller Fingerprint, Plan)` pro Shard.
    /// Der Cache wird dabei NICHT veraendert; erst [`IncrementalCache::mark_built`]
    /// uebernimmt Ergebnisse (so bleiben fehlgeschlagene Builds `Dirty`).
    pub fn plan(&self, fingerprints: &[(ShardId, Fingerprint)]) -> Vec<(ShardId, Fingerprint, ShardPlan)> {
        fingerprints
            .iter()
            .map(|(id, fp)| {
                let plan = if self.needs_rebuild(id, *fp) {
                    ShardPlan::Dirty
                } else {
                    ShardPlan::Skipped
                };
                (id.clone(), *fp, plan)
            })
            .collect()
    }

    /// Uebernimmt einen erfolgreich gebauten Shard in den Cache.
    pub fn mark_built(&mut self, id: ShardId, fp: Fingerprint) {
        self.entries.insert(id, fp);
    }

    /// Uebernimmt viele erfolgreich gebaute Shards auf einmal.
    pub fn mark_many_built(&mut self, built: &[(ShardId, Fingerprint)]) {
        for (id, fp) in built {
            self.entries.insert(id.clone(), *fp);
        }
    }

    /// Entfernt alle Shards ausser den genannten (z. B. geloeschte Shards).
    pub fn retain_only(&mut self, live: &[ShardId]) {
        self.entries.retain(|id, _| live.contains(id));
    }

    /// Serialisiert den Cache in ein stabiles Textformat (eine Zeile pro Shard,
    /// nach `(name, chunk)` sortiert), damit ein Treiber ihn zwischen Laeufen
    /// als Datei ablegen kann. Format:
    ///
    /// ```text
    /// sail-shard-cache-v1
    /// <name>\t<chunk>\t<hex-fingerprint>
    /// ...
    /// ```
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut ids: Vec<&ShardId> = self.entries.keys().collect();
        ids.sort();
        let mut out = String::from("sail-shard-cache-v1\n");
        for id in ids {
            let fp = self.entries.get(id).expect("key from keys()");
            out.push_str(&id.name);
            out.push('\t');
            out.push_str(&id.chunk.to_string());
            out.push('\t');
            out.push_str(&fp.to_hex());
            out.push('\n');
        }
        out
    }

    /// Gegenstueck zu [`IncrementalCache::serialize`]. Unbekannte Kopfzeilen,
    /// falsche Feldzahlen, unparsebare Chunks/Hashes oder Steuerzeichen in
    /// Namen werden als `Err(String)` gemeldet; ein leerer String ist kein
    /// gueltiger Cache.
    pub fn deserialize(s: &str) -> Result<Self, String> {
        let mut lines = s.lines();
        match lines.next() {
            Some("sail-shard-cache-v1") => {}
            other => return Err(format!("falsche Cache-Kopfzeile: {other:?}")),
        }
        let mut cache = Self::new();
        for (n, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let (name, rest) = line.split_once('\t').ok_or_else(|| format!("Zeile {}: kein Tab", n + 2))?;
            let (chunk, hex) = rest.split_once('\t').ok_or_else(|| format!("Zeile {}: nur ein Tab", n + 2))?;
            if name.is_empty() || name.contains(['\n', '\r']) {
                return Err(format!("Zeile {}: ungueltiger Name", n + 2));
            }
            let chunk: u32 = chunk.parse().map_err(|_| format!("Zeile {}: ungueltiger Chunk", n + 2))?;
            let fp = Fingerprint::from_hex(hex).ok_or_else(|| format!("Zeile {}: ungueltiger Fingerprint", n + 2))?;
            cache.entries.insert(ShardId::new(name.to_string(), chunk), fp);
        }
        Ok(cache)
    }

    /// Anteil geskippter Shards im letzten Plan (0.0..=1.0), `None` bei leerem Plan.
    #[must_use]
    pub fn skip_ratio(planned: &[(ShardId, Fingerprint, ShardPlan)]) -> Option<f64> {
        if planned.is_empty() {
            return None;
        }
        let skipped = planned.iter().filter(|(_, _, p)| *p == ShardPlan::Skipped).count();
        Some(skipped as f64 / planned.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint;

    const PASS: &str = "lane11-passes-v1";

    fn fp(content: &str) -> Fingerprint {
        fingerprint(content, PASS)
    }

    #[test]
    fn first_run_everything_dirty() {
        let cache = IncrementalCache::new();
        let fps = vec![
            (ShardId::new("instrs32", 0), fp("a")),
            (ShardId::new("sve", 0), fp("b")),
        ];
        let plan = cache.plan(&fps);
        assert!(plan.iter().all(|(_, _, p)| *p == ShardPlan::Dirty));
    }

    #[test]
    fn second_run_without_changes_all_skipped() {
        // Done-Kriterium: Zweitlauf ohne Aenderung = alles skipped.
        let mut cache = IncrementalCache::new();
        let fps = vec![
            (ShardId::new("instrs32", 0), fp("a")),
            (ShardId::new("instrs64", 0), fp("b")),
            (ShardId::new("sve", 0), fp("c")),
        ];
        let first = cache.plan(&fps);
        assert!(first.iter().all(|(_, _, p)| p.is_dirty()));
        let built: Vec<_> = first.iter().map(|(id, f, _)| (id.clone(), *f)).collect();
        cache.mark_many_built(&built);

        let second = cache.plan(&fps);
        assert!(!second.is_empty());
        assert!(second.iter().all(|(_, _, p)| *p == ShardPlan::Skipped));
        assert_eq!(IncrementalCache::skip_ratio(&second), Some(1.0));
    }

    #[test]
    fn changed_shard_is_dirty_others_skipped() {
        let mut cache = IncrementalCache::new();
        let v1 = [
            (ShardId::new("instrs32", 0), fp("a")),
            (ShardId::new("sve", 0), fp("b")),
        ];
        let built: Vec<_> = v1.iter().map(|(id, f)| (id.clone(), *f)).collect();
        cache.mark_many_built(&built);

        let v2 = vec![
            (ShardId::new("instrs32", 0), fp("a CHANGED")),
            (ShardId::new("sve", 0), fp("b")),
        ];
        let plan = cache.plan(&v2);
        assert_eq!(plan[0].2, ShardPlan::Dirty);
        assert_eq!(plan[1].2, ShardPlan::Skipped);
    }

    #[test]
    fn pass_version_bump_dirties_everything() {
        let mut cache = IncrementalCache::new();
        let old: Vec<_> = ["x", "y"]
            .iter()
            .enumerate()
            .map(|(i, c)| (ShardId::new(format!("s{i}"), 0), fingerprint(c, "passes-v1")))
            .collect();
        let built: Vec<_> = old.iter().map(|(id, f)| (id.clone(), *f)).collect();
        cache.mark_many_built(&built);

        let new: Vec<_> = ["x", "y"]
            .iter()
            .enumerate()
            .map(|(i, c)| (ShardId::new(format!("s{i}"), 0), fingerprint(c, "passes-v2")))
            .collect();
        let plan = cache.plan(&new);
        assert!(plan.iter().all(|(_, _, p)| p.is_dirty()));
    }

    #[test]
    fn failed_build_stays_dirty() {
        let cache = IncrementalCache::new();
        let fps = vec![(ShardId::new("a", 0), fp("a"))];
        // Nur geplant, nie mark_built -> naechster Plan wieder Dirty.
        let _ = cache.plan(&fps);
        let again = cache.plan(&fps);
        assert_eq!(again[0].2, ShardPlan::Dirty);
    }

    #[test]
    fn serialize_roundtrip_preserves_plan() {
        let mut cache = IncrementalCache::new();
        cache.mark_built(ShardId::new("instrs32", 0), fp("a"));
        cache.mark_built(ShardId::new("sve", 0), fp("b"));
        let text = cache.serialize();
        assert!(text.starts_with("sail-shard-cache-v1\n"));
        let restored = IncrementalCache::deserialize(&text).expect("roundtrip");
        assert_eq!(restored.len(), 2);
        let fps = vec![
            (ShardId::new("instrs32", 0), fp("a")),
            (ShardId::new("sve", 0), fp("b")),
        ];
        let plan = restored.plan(&fps);
        assert!(plan.iter().all(|(_, _, p)| *p == ShardPlan::Skipped));
    }

    #[test]
    fn deserialize_rejects_garbage() {
        assert!(IncrementalCache::deserialize("").is_err());
        assert!(IncrementalCache::deserialize("falscher-header\n").is_err());
        assert!(IncrementalCache::deserialize("sail-shard-cache-v1\nnurzweifelder").is_err());
        assert!(IncrementalCache::deserialize("sail-shard-cache-v1\na\tb\tc").is_err());
    }

    #[test]
    fn retain_only_prunes_removed_shards() {
        let mut cache = IncrementalCache::new();
        cache.mark_built(ShardId::new("a", 0), fp("a"));
        cache.mark_built(ShardId::new("gone", 0), fp("g"));
        cache.retain_only(&[ShardId::new("a", 0)]);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&ShardId::new("a", 0)).is_some());
    }
}
