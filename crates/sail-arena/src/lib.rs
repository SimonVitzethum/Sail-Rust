//! sail-arena — Tuning-Layer ueber [`sail_ast::GenArena`] (Lane 37).
//!
//! Reines Safe-Rust (`#![forbid(unsafe_code)]`): Es gibt hier **keine**
//! eigene Arena-Implementierung und keinen `unsafe`-Code. Alle Slots leben
//! in `sail-ast`-`GenArena`s (reine Index-Arena, `(index, generation)`);
//! dieses Crate legt nur eine **Sizing-/Drop-Policy** darum:
//!
//! - [`sizing`]: shard-groessenabhaengige Reservierung
//!   ([`sizing::ShardSize`], [`sizing::ShardPlan`]).
//! - [`shard`]: [`shard::ShardArena`], die Eintraege so auf Teil-Arenen
//!   verteilt, dass kein Shard sein Budget ueberschreitet, und ganze Shards
//!   an definierten Drop-Punkten auf einmal freigibt.
//! - [`peak`]: Peak-Messung (live-Set + optional echte Peak-RSS via
//!   `/proc/self/status`, nur std, nur Linux).
//!
//! Messergebnisse stehen in `ARENA_REPORT.md`, Hinweise zur Binaergroesse
//! (`cargo bloat`) in `BLOAT.md`.

#![forbid(unsafe_code)]

pub mod peak;
pub mod shard;
pub mod sizing;

pub use peak::{live_bytes_estimate, peak_rss_kib};
pub use shard::{Handle, ShardArena};
pub use sizing::{ShardPlan, ShardSize, plan_for};
