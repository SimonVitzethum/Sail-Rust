//! Datei-Sharding + Inkrementalitaet (Lane 11).
//!
//! * [`Shard`]: Dateigrenzen aus `file_start`/`file_end`-Pragmas + Extra-Splits
//!   fuer grosse Instr-Dateien (`instrs32`/`instrs64`/SVE/SME, `v8_base`,
//!   `sysregs_autogen`). Namenloser Text wird synthetisch (`preamble` fuehrend,
//!   `postamble[#n]` danach); Shard-IDs sind eindeutig.
//! * [`Fingerprint`]: stabiler Hash ueber Shard-Inhalt + Pass-Version.
//! * [`Scheduler`]/[`SchedulerConfig`]: paralleler Scheduler mit begrenzter
//!   Arbeits-Queue (bounded, low-RAM) und konfigurierbarem `max_inflight`
//!   (Default 8); Ergebniskanal bewusst unbegrenzt (deadlock-frei).
//!   Streaming laeuft wahlweise im Count-Fenster (`run_streaming`) oder im
//!   Byte-Budget (`run_streaming_weighed` + `max_bytes`, Lane 22): Credits
//!   zaehlen dann Eingabe-Bytes statt Definitionen; ein einzelnes Item groesser
//!   als das Budget laeuft allein im leeren Fenster (RAM-Schranke: das einzelne
//!   Item muss selbst in RAM passen).
//! * [`IncrementalCache`]: Skip-bei-unveraendert-Logik, inkl. textuellem
//!   Cache-Format (`serialize`/`deserialize`) fuer prozessuebergreifende Laeufe.
//!
//! ## Lane-127-Begriffswahrheit (Shard)
//!
//! `Shard`/`ShardId`/`ShardKind` hier sind die Datei-Shards (atomare
//! Inkrementalitaets-Einheit). Andere `Shard`-Namen im Workspace sind
//! eigene Begriffe, keine Duplikate: `sail-rewrite::Shard` ist die
//! Rewrite-Einheit (Def-Chunk, Umbenennung `RewriteUnit` als
//! Owner-Folgearbeit Lane 07), `sail-arena::{ShardArena, ShardPlan}`
//! ist Arena-Sharding, `sail-sysregs-shard::{ShardReport, ShardConfig}`
//! der Runner-Bericht. Fingerprint-Wahrheit ist [`Fingerprint`]
//! (blake3-256); das FNV-16-Minimal in `sail-cache` (`ShardFingerprint`)
//! migriert hierher (Migrationsnotiz dort).
//!
//! # ARMv9-Shardliste (dokumentiert, vgl. Done-Kriterium)
//!
//! Die Funktion [`armv9_shard_plan`] nennt die erwarteten Shards des
//! ARMv9-Modells in kanonischer Reihenfolge. Namen sind stabil und bilden die
//! `ShardId`-Basis:
//!
//! ```text
//! preamble, v8_base, instrs32, instrs32_extra..., instrs64, instrs64_extra...,
//! sve, sme, sysregs, sysregs_autogen (+chunks), decode, execute, misc
//! ```
//! Details siehe [`ARMV9_SHARD_ORDER`].

#![forbid(unsafe_code)]

mod fingerprint;
mod incremental;
mod scheduler;
mod shard;

pub use fingerprint::Fingerprint;
pub use fingerprint::fingerprint;
pub use fingerprint::fingerprint_all;
pub use fingerprint::fingerprint_bytes;
pub use incremental::IncrementalCache;
pub use incremental::ShardPlan;
pub use scheduler::Scheduler;
pub use scheduler::SchedulerConfig;
pub use shard::ARMV9_SHARD_ORDER;
pub use shard::Shard;
pub use shard::ShardId;
pub use shard::ShardKind;
pub use shard::armv9_shard_plan;
pub use shard::extra_split;
pub use shard::is_large_instr_file;
pub use shard::shard_files;
pub use shard::split_by_file_markers;
