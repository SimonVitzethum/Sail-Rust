//! `sail-sysregs-shard` (Lane 31): `sysregs_autogen.sail` als isolierter Shard.
//!
//! Strategie nur fuer diese Datei:
//! * Die Datei besteht ausschliesslich aus `val`/`function`-Paaren mit
//!   Namensgleichheit (2815 Paare, 160302 Zeilen) — verifiziert per Skript.
//! * Streaming-Split in Top-Level-Definitionen, Gruppierung in Bloecke zu
//!   konstant vielen Defs ([`ShardConfig::defs_per_block`], Standard 64 =
//!   32 Paare). Zu jedem Zeitpunkt lebt hoechstens ein Block im Speicher.
//! * Pro Block: Paarungs-Pruefung + FNV-1a-Fingerprint (Lane 11), danach
//!   wird der Quelltext verworfen. Kein Gesamt-Dokument, kein Full-Build,
//!   kein Lake.
//!
//! Reines Rust: `#![forbid(unsafe_code)]`, nur `std`-Abhaengigkeiten.

#![forbid(unsafe_code)]

pub mod block;
pub mod rss;
pub mod run;
pub mod splitter;

pub use block::{def_head, fnv1a64, parse_block, BlockError, ParsedBlock};
pub use run::{run_shard, BlockStat, ShardConfig, ShardReport};
pub use splitter::{Block, BlockSplitter, DefSplitter, RawDef, SplitError};
