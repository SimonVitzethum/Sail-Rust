//! [`ShardArena`]: Tuning-Wrapper um `sail-ast`-`GenArena`s.
//!
//! Kein eigener Slot-Code, keine Pointer, kein `unsafe`: Jeder Shard ist
//! eine vollwertige [`sail_ast::GenArena`]; dieses Modul enthaelt nur
//! Routing (Budget einhalten), Peak-Zaehler und Drop-Punkte.
//!
//! Drop-Punkte (verbindliche Checkliste, siehe Policy-Tests in
//! `tests/policy.rs`):
//!
//! 1. [`ShardArena::drop_shard`]: ganzen Shard auf einmal freigeben.
//!    Alle Handles in diesen Shard werden stale (die `GenArena` wird
//!    ersetzt — alte `(index, generation)`-Paare loesen sich zu `None` auf).
//! 2. [`ShardArena::clear`]: alle Shards freigeben (`live == 0`).
//! 3. `Drop` von `ShardArena` gibt alle Werte frei (implizit ueber die
//!    enthaltenen `GenArena`s; verifiziert per Drop-Zaehler im Policy-Test).

use sail_ast::{GenArena, NodeId};

use crate::sizing::{ShardPlan, ShardSize};

/// Handle in eine [`ShardArena`]: Shard-Nummer plus Generations-ID.
///
/// Beide Teile sind plain data (`Copy`); keine Pointer, keine Lifetimes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    /// Index des Shards zum Zeitpunkt von `insert`.
    pub shard: u32,
    /// Generationale ID innerhalb dieses Shards.
    pub id: NodeId,
}

/// Shard-groessenabhaengig dimensionierte Arena ueber `GenArena`s.
///
/// `T` ist nur der Nutzdaten-Typ; die Policy (`budget`, Zaehler) ist
/// nicht-generisch gehalten (bloat-arm, siehe `BLOAT.md`).
#[derive(Clone, Debug, Default)]
pub struct ShardArena<T> {
    shards: Vec<ShardSlot<T>>,
    per_shard_budget: usize,
    live: usize,
    peak_live: usize,
}

#[derive(Clone, Debug)]
struct ShardSlot<T> {
    arena: GenArena<T>,
    live: usize,
}

impl<T> ShardArena<T> {
    /// Neue Arena fuer die angegebene Shard-Groessenklasse.
    #[must_use]
    pub fn new(size: ShardSize) -> Self {
        ShardArena {
            shards: Vec::new(),
            per_shard_budget: size.per_shard_budget(),
            live: 0,
            peak_live: 0,
        }
    }

    /// Neue Arena nach Reservierungsplan ([`plan_for`]).
    ///
    /// Der Plan dimensioniert die Shard-Anzahl vorab (`Vec`-Kapazitaet);
    /// die `GenArena`s selbst allokieren faul — Reservierung heisst hier:
    /// Routing-Budget plus Shard-Vektor-Groesse, kein `unsafe`, kein
    /// Vorbefuellen mit Dummy-Werten.
    #[must_use]
    pub fn with_plan(plan: &ShardPlan) -> Self {
        ShardArena {
            shards: Vec::with_capacity(plan.num_shards),
            per_shard_budget: plan.per_shard_budget,
            live: 0,
            peak_live: 0,
        }
    }

    /// Aktives Live-Budget pro Shard (Reservierung).
    #[must_use]
    pub fn per_shard_budget(&self) -> usize {
        self.per_shard_budget
    }

    /// Anzahl angelegter Shards (inkl. leerer/gedroppter).
    #[must_use]
    pub fn num_shards(&self) -> usize {
        self.shards.len()
    }

    /// Lebende Eintraege insgesamt.
    #[must_use]
    pub fn live(&self) -> usize {
        self.live
    }

    /// Hoechste je erreichte `live`-Zahl (monoton, Drop setzt sie nie zurueck).
    #[must_use]
    pub fn peak_live(&self) -> usize {
        self.peak_live
    }

    /// Lebende Eintraege in Shard `shard`.
    #[must_use]
    pub fn shard_live(&self, shard: usize) -> Option<usize> {
        self.shards.get(shard).map(|slot| slot.live)
    }

    /// Wert hinter `handle` lesen; stale/fremde Handles geben `None`.
    #[must_use]
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let slot = self.shards.get(handle.shard as usize)?;
        slot.arena.get(handle.id)
    }

    /// Wert einfuegen. Liefert ein [`Handle`].
    ///
    /// Routing: in den juengsten Shard, solange dessen Live-Budget nicht
    /// ausgeschoepft ist; sonst wird ein neuer Shard eroeffnet. Dadurch
    /// haelt kein Shard mehr als [`ShardArena::per_shard_budget`] lebende
    /// Eintraege (Policy-Invariante, getestet).
    pub fn insert(&mut self, value: T) -> Handle {
        let needs_new = match self.shards.last() {
            None => true,
            Some(slot) => slot.live >= self.per_shard_budget,
        };
        if needs_new {
            self.shards.push(ShardSlot {
                arena: GenArena::new(),
                live: 0,
            });
        }
        let shard = self.shards.len() - 1;
        let slot = &mut self.shards[shard];
        let id = slot.arena.insert(value);
        slot.live += 1;
        self.live += 1;
        self.peak_live = self.peak_live.max(self.live);
        Handle {
            shard: shard as u32,
            id,
        }
    }

    /// Einzelnen Eintrag entfernen; stale Handles liefern `None`.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let slot = self.shards.get_mut(handle.shard as usize)?;
        let value = slot.arena.remove(handle.id)?;
        slot.live -= 1;
        self.live -= 1;
        Some(value)
    }

    /// **Drop-Punkt 1:** ganzen Shard freigeben.
    ///
    /// Gibt die Anzahl freigegebener lebender Eintraege zurueck.
    /// Danach loest kein Handle in diesen Shard mehr auf (`None`),
    /// alle anderen Shards bleiben unberuehrt. Eine gueltige, aber
    /// ungenutzte Shard-Nummer liefert `None`.
    pub fn drop_shard(&mut self, shard: usize) -> Option<usize> {
        let slot = self.shards.get_mut(shard)?;
        let released = slot.live;
        // GenArena ersetzen: alle alten Handles werden stale, alle Werte
        // laufen durch Drop. Kein unsafe, keine manuellen Pointer.
        slot.arena = GenArena::new();
        slot.live = 0;
        self.live -= released;
        Some(released)
    }

    /// **Drop-Punkt 2:** alle Shards freigeben; danach `live == 0`.
    pub fn clear(&mut self) {
        for slot in &mut self.shards {
            slot.arena = GenArena::new();
            slot.live = 0;
        }
        self.live = 0;
    }

    /// `true`, wenn kein lebender Eintrag existiert.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Ueber alle lebenden `(Handle, &T)`-Paare iterieren (Shard-Reihenfolge).
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.shards.iter().enumerate().flat_map(|(s, slot)| {
            slot.arena.iter().map(move |(id, value)| {
                (
                    Handle {
                        shard: s as u32,
                        id,
                    },
                    value,
                )
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_arena_reports_zero() {
        let arena: ShardArena<u32> = ShardArena::new(ShardSize::Small);
        assert!(arena.is_empty());
        assert_eq!(arena.live(), 0);
        assert_eq!(arena.peak_live(), 0);
        assert_eq!(arena.num_shards(), 0);
    }

    #[test]
    fn insert_get_remove_roundtrip() {
        let mut arena = ShardArena::new(ShardSize::Small);
        let h = arena.insert(41_u32);
        assert_eq!(arena.get(h), Some(&41));
        assert_eq!(arena.remove(h), Some(41));
        assert_eq!(arena.get(h), None);
        assert!(arena.is_empty());
    }

    #[test]
    fn unknown_handles_are_rejected() {
        let mut arena = ShardArena::<u32>::new(ShardSize::Small);
        let bogus = Handle {
            shard: 99,
            id: NodeId::new(0, 0),
        };
        assert_eq!(arena.get(bogus), None);
        assert_eq!(arena.remove(bogus), None);
        assert_eq!(arena.drop_shard(99), None);
    }
}
