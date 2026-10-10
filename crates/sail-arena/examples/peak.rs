//! Fixture-Runner fuer `ARENA_REPORT.md`: `phased-temps`-Workload.
//!
//! Aufruf:
//! ```sh
//! cargo run -p sail-arena --example peak --release -- before
//! cargo run -p sail-arena --example peak --release -- after
//! ```
//! Je ein Prozess pro Variante, damit `VmHWM` (Prozess-Maximum) der
//! jeweiligen Variante zugeordnet werden kann.

use sail_arena::{ShardArena, ShardSize, live_bytes_estimate, peak_rss_kib};
use sail_ast::GenArena;

const PHASES: usize = 10;
const PER_PHASE: usize = 20_000;

#[derive(Clone, Copy)]
struct Node32([u64; 4]);

fn run_before() -> (usize, u64) {
    let mut arena: GenArena<Node32> = GenArena::new();
    let mut live = 0_usize;
    let mut peak = 0_usize;
    for p in 0..PHASES {
        for i in 0..PER_PHASE {
            arena.insert(Node32([p as u64, i as u64, 0, 0]));
            live += 1;
            peak = peak.max(live);
        }
    }
    // Pruefsumme: beweist, dass alle Werte wirklich leben (kein Wegoptimieren).
    let mut sum = 0_u64;
    for (_, node) in arena.iter() {
        sum = sum.wrapping_add(node.0[0]).wrapping_add(node.0[1]);
    }
    (peak, sum)
}

fn run_after() -> (usize, usize, usize, u64) {
    let mut arena = ShardArena::new(ShardSize::Small);
    for p in 0..PHASES {
        for i in 0..PER_PHASE {
            arena.insert(Node32([p as u64, i as u64, 0, 0]));
        }
        let last = arena.num_shards() - 1;
        for s in 0..last {
            if arena.shard_live(s).unwrap_or(0) > 0 {
                arena.drop_shard(s);
            }
        }
    }
    let mut sum = 0_u64;
    for (_, node) in arena.iter() {
        sum = sum.wrapping_add(node.0[0]).wrapping_add(node.0[1]);
    }
    (arena.peak_live(), arena.live(), arena.num_shards(), sum)
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "after".to_owned());
    let elem = size_of::<Node32>();
    match mode.as_str() {
        "before" => {
            let (peak, sum) = run_before();
            println!("variante=vorher (eine GenArena, Drop erst am Ende)");
            println!("live_peak={peak}");
            println!("bytes_geschaetzt={}", live_bytes_estimate(peak, elem));
            println!("pruefsumme={sum}");
            println!("vmhwm_kib={}", peak_rss_kib().map_or(-1, |v| v as i64));
        }
        "after" => {
            let (peak, live, shards, sum) = run_after();
            println!("variante=nachher (ShardArena, Shard-Drop pro Phase)");
            println!("live_peak={peak}");
            println!("bytes_geschaetzt={}", live_bytes_estimate(peak, elem));
            println!("rest_live={live}");
            println!("shards={shards}");
            println!("pruefsumme={sum}");
            println!("vmhwm_kib={}", peak_rss_kib().map_or(-1, |v| v as i64));
        }
        other => {
            eprintln!("unbekannter Modus: {other} (erwartet: before|after)");
            std::process::exit(2);
        }
    }
}
