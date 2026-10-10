#![forbid(unsafe_code)]
//! Generational IDs, SlotMap-style: plain `(index, generation)` values.
//! Stale handles are rejected on lookup; no pointers are stored.

use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// Generational node handle. `Copy + Send + Sync`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    /// Slot index inside [`GenArena`].
    pub index: u32,
    /// Generation bumped on every reuse of the slot.
    pub generation: u32,
}

impl NodeId {
    /// Placeholder for synthesized nodes (tests, desugaring in later lanes).
    pub const DUMMY: NodeId = NodeId {
        index: u32::MAX,
        generation: u32::MAX,
    };

    #[must_use]
    pub fn new(index: u32, generation: u32) -> Self {
        NodeId { index, generation }
    }

    #[must_use]
    pub fn is_dummy(self) -> bool {
        self == NodeId::DUMMY
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Monotonic fresh-ID source for AST construction (no reuse).
/// For reuse-with-validation use [`GenArena`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdGen {
    next: u32,
}

impl IdGen {
    #[must_use]
    pub fn new() -> Self {
        IdGen { next: 0 }
    }

    pub fn fresh(&mut self) -> NodeId {
        let id = NodeId::new(self.next, 0);
        self.next = self.next.saturating_add(1);
        id
    }
}

/// SlotMap-style arena holding `T` behind [`NodeId`]s.
/// `Send + Sync` whenever `T` is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenArena<T> {
    slots: Vec<Slot<T>>,
    /// Recycled slot indices (free list).
    free: Vec<u32>,
    /// Marker so `T` participates in auto-trait inference explicitly.
    #[serde(skip)]
    _marker: PhantomData<T>,
}

impl<T> GenArena<T> {
    #[must_use]
    pub fn new() -> Self {
        GenArena {
            slots: Vec::new(),
            free: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Insert a value, returning a live handle.
    pub fn insert(&mut self, value: T) -> NodeId {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.value.is_none());
            slot.value = Some(value);
            NodeId::new(index, slot.generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            NodeId::new(index, 0)
        }
    }

    /// Remove the live value behind `id`; a stale id returns `None`.
    /// The slot generation is bumped so old handles stay stale.
    pub fn remove(&mut self, id: NodeId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        Some(value)
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&T> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_ref()
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_mut()
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    /// Number of live entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over live `(id, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value.as_ref().map(|v| {
                (
                    NodeId::new(i as u32, slot.generation),
                    v,
                )
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove_reuse_bumps_generation() {
        let mut arena: GenArena<&str> = GenArena::new();
        let a = arena.insert("a");
        let b = arena.insert("b");
        assert_eq!(arena.get(a), Some(&"a"));
        assert_eq!(arena.len(), 2);

        assert_eq!(arena.remove(a), Some("a"));
        assert_eq!(arena.get(a), None); // stale
        assert!(!arena.contains(a));

        let c = arena.insert("c");
        assert_eq!(c.index, a.index); // slot recycled
        assert_ne!(c.generation, a.generation); // generation bumped
        assert_eq!(arena.get(c), Some(&"c"));
        assert_eq!(arena.remove(a), None); // old handle still rejected
        assert_eq!(arena.get(b), Some(&"b"));
    }

    #[test]
    fn unknown_and_dummy_ids_rejected() {
        let mut arena: GenArena<u32> = GenArena::new();
        assert_eq!(arena.get(NodeId::new(7, 0)), None);
        assert_eq!(arena.remove(NodeId::DUMMY), None);
        let id = arena.insert(1);
        *arena.get_mut(id).unwrap() = 2;
        assert_eq!(arena.get(id), Some(&2));
    }

    fn assert_send_sync<T: Send + Sync>() {}
    #[test]
    fn ids_and_arena_are_send_sync() {
        assert_send_sync::<NodeId>();
        assert_send_sync::<GenArena<String>>();
        assert_send_sync::<IdGen>();
    }
}
