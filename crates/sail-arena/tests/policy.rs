//! Policy-Tests der Lane 37 (Drop-Punkte-Checkliste, Sizing, Peak-Vergleich).
//!
//! Jede Testfunktion prueft genau einen Punkt der verbindlichen Checkliste
//! aus `src/shard.rs`. Faellt ein Punkt, ist die Lane nicht DONE.

use sail_arena::{ShardArena, ShardPlan, ShardSize, live_bytes_estimate, plan_for};
use sail_ast::{GenArena, NodeId};
use std::cell::Cell;
use std::rc::Rc;

/// Drop-Zaehler: beweist, dass Drop-Punkte Werte wirklich freigeben.
#[derive(Debug, PartialEq, Eq)]
struct DropGuard {
    counter: Rc<Cell<usize>>,
}

impl DropGuard {
    fn new(counter: &Rc<Cell<usize>>) -> Self {
        DropGuard {
            counter: Rc::clone(counter),
        }
    }
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.counter.set(self.counter.get() + 1);
    }
}

fn dropped_counter() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}

/// Sizing-Policy: Plaene decken die Erwartung ab, sind knapp und gueltig.
#[test]
fn sizing_plan_covers_and_is_tight() {
    for total in [0, 1, 1_023, 1_024, 1_025, 4_096, 65_536, 65_537, 1_000_000] {
        let plan: ShardPlan = plan_for(total);
        assert!(plan.num_shards >= 1, "total={total}: kein Shard geplant");
        assert!(
            plan.covers(),
            "total={total}: Plan {plan:?} deckt Erwartung nicht ab"
        );
        assert_eq!(plan.per_shard_budget, plan.size.per_shard_budget());
        if total > 0 {
            let smaller = (plan.num_shards - 1).saturating_mul(plan.per_shard_budget);
            assert!(smaller < total, "total={total}: Plan {plan:?} zu grosszuegig");
        }
    }
}

/// Routing-Invariante: kein Shard haelt je mehr als sein Budget.
#[test]
fn routing_never_exceeds_per_shard_budget() {
    for size in [ShardSize::Small, ShardSize::Medium, ShardSize::Large] {
        let mut arena = ShardArena::new(size);
        for i in 0..3 * size.per_shard_budget() + 7 {
            arena.insert(i as u64);
        }
        for s in 0..arena.num_shards() {
            let live = arena.shard_live(s).expect("Shard muss existieren");
            assert!(
                live <= size.per_shard_budget(),
                "Shard {s} haelt {live}, Budget {}",
                size.per_shard_budget()
            );
        }
        assert_eq!(arena.live(), 3 * size.per_shard_budget() + 7);
    }
}

/// Drop-Punkte-Checkliste (verbindlich):
/// 1. `drop_shard` meldet exakt die freigegebene Zahl.
/// 2. Danach ist jedes Handle in den Shard stale, andere Shards unberuehrt.
/// 3. `peak_live` bleibt als High-Water-Mark erhalten.
/// 4. `clear` leert alles; `Drop` der Arena gibt den Rest frei.
#[test]
fn drop_point_checklist() {
    let counter = dropped_counter();
    let budget = ShardSize::Small.per_shard_budget();
    let mut arena = ShardArena::new(ShardSize::Small);

    // 3 Shards: voll, voll, teilvoll.
    let mut shard0 = Vec::new();
    let mut shard1 = Vec::new();
    let mut shard2 = Vec::new();
    for _ in 0..budget {
        shard0.push(arena.insert(DropGuard::new(&counter)));
    }
    for _ in 0..budget {
        shard1.push(arena.insert(DropGuard::new(&counter)));
    }
    for _ in 0..41 {
        shard2.push(arena.insert(DropGuard::new(&counter)));
    }
    assert_eq!(arena.num_shards(), 3);
    assert_eq!(arena.live(), 2 * budget + 41);
    assert_eq!(arena.peak_live(), 2 * budget + 41);
    assert_eq!(counter.get(), 0);

    // Punkt 1+2: Shard 0 droppen.
    let released = arena.drop_shard(0).expect("Shard 0 existiert");
    assert_eq!(released, budget);
    assert_eq!(counter.get(), budget, "Drop-Punkt muss Werte freigeben");
    for h in &shard0 {
        assert_eq!(arena.get(*h), None, "Handle in gedroppten Shard stale?");
    }
    for h in shard1.iter().chain(shard2.iter()) {
        assert!(arena.get(*h).is_some(), "fremder Shard darf nicht leiden");
    }
    // Punkt 3: Peak bleibt High-Water-Mark.
    assert_eq!(arena.live(), budget + 41);
    assert_eq!(arena.peak_live(), 2 * budget + 41);

    // Erneuter Drop desselben Shards: gueltig, gibt 0 frei.
    assert_eq!(arena.drop_shard(0), Some(0));
    assert_eq!(counter.get(), budget);

    // Ungueltige Shard-Nummer: None, kein Effekt.
    assert_eq!(arena.drop_shard(77), None);
    assert_eq!(counter.get(), budget);

    // Punkt 4a: clear leert alles.
    arena.clear();
    assert!(arena.is_empty());
    assert_eq!(counter.get(), 2 * budget + 41);
    for h in shard1.iter().chain(shard2.iter()) {
        assert_eq!(arena.get(*h), None);
    }
    assert_eq!(arena.peak_live(), 2 * budget + 41);

    // Punkt 4b: Drop der Arena gibt den Rest frei.
    let h = arena.insert(DropGuard::new(&counter));
    assert!(arena.get(h).is_some());
    drop(arena);
    assert_eq!(counter.get(), 2 * budget + 42);
}

/// `remove` auf stale Handles (Doppel-Drop-Schutz via Generation).
#[test]
fn stale_removes_are_rejected() {
    let mut arena = ShardArena::new(ShardSize::Small);
    let h = arena.insert(7_u32);
    assert_eq!(arena.remove(h), Some(7));
    assert_eq!(arena.remove(h), None, "zweites remove muss None liefern");
    // Slot-Recycling vergibt hoehere Generation: altes Handle bleibt stale.
    let h2 = arena.insert(8_u32);
    assert_eq!(h.shard, h2.shard, "Slot sollte recycelt werden");
    assert_ne!(h.id, h2.id, "Generation muss sich unterscheiden");
    assert_eq!(arena.remove(h), None);
    assert_eq!(arena.get(h2), Some(&8));
}

/// Unbekannte Handles (falscher Shard, DUMMY-IDs) werden abgewiesen.
#[test]
fn unknown_handles_are_rejected() {
    let arena: ShardArena<u32> = ShardArena::new(ShardSize::Small);
    let bogus = sail_arena::Handle {
        shard: 0,
        id: NodeId::new(0, 0),
    };
    assert_eq!(arena.get(bogus), None);
    let dummy = sail_arena::Handle {
        shard: u32::MAX,
        id: NodeId::DUMMY,
    };
    assert_eq!(arena.get(dummy), None);
}

// ---------------------------------------------------------------------------
// Peak-Vergleich vorher/nachher (Fixture, siehe ARENA_REPORT.md)
// ---------------------------------------------------------------------------

/// 32-Byte-Nutzlast (steht fuer einen AST-Knoten mit Span + IDs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Node32([u64; 4]);

/// Fixture `phased-temps`: phasenspezifische Temporaries (z. B.
/// Lowering-Puffer), die nach Phasenende nicht mehr gebraucht werden.
/// Vorher: eine `GenArena`, Drop erst am Ende. Nachher: `ShardArena`
/// mit Shard-Drop nach jeder Phase.
const PHASES: usize = 10;
const PER_PHASE: usize = 2_000;

fn run_before() -> usize {
    let mut arena: GenArena<Node32> = GenArena::new();
    let mut live = 0_usize;
    let mut peak = 0_usize;
    for p in 0..PHASES {
        for i in 0..PER_PHASE {
            arena.insert(Node32([p as u64, i as u64, 0, 0]));
            live += 1;
            peak = peak.max(live);
        }
        // Vorher: kein phasenspezifischer Drop-Punkt — alles bleibt leben.
    }
    peak
}

fn run_after() -> (usize, usize) {
    let mut arena = ShardArena::new(ShardSize::Small);
    for p in 0..PHASES {
        for i in 0..PER_PHASE {
            arena.insert(Node32([p as u64, i as u64, 0, 0]));
        }
        // Nachher: Drop-Punkt pro Phase — volle Shards der beendeten Phase
        // freigeben, nur der Anbruch-Shard lebt weiter.
        let last = arena.num_shards() - 1;
        for s in 0..last {
            if arena.shard_live(s).unwrap_or(0) > 0 {
                arena.drop_shard(s);
            }
        }
    }
    (arena.peak_live(), arena.live())
}

#[test]
fn tuned_peak_is_below_plain_peak() {
    let before = run_before();
    let (after_peak, after_live) = run_after();
    assert_eq!(before, PHASES * PER_PHASE);
    assert!(
        after_peak < before,
        "Tuning muss Peak senken: nachher={after_peak}, vorher={before}"
    );
    println!(
        "peak-vergleich: vorher live={before} (~{} B), nachher peak={after_peak} (~{} B), rest-live={after_live}",
        live_bytes_estimate(before, size_of::<Node32>()),
        live_bytes_estimate(after_peak, size_of::<Node32>()),
    );
}
