//! Lane 09: Monomorphisierung + ANF-Lowering, speicherschonend.
//!
//! * Shard für Shard, kein Gesamt-AST-Clone (Streaming-Iterator).
//! * Frühes Droppen: nach dem Lowering wird die Original-Def freigegeben
//!   (Ownership wandert in die Lowering-Funktion, kein Zurückhalten).
//! * `-auto_mono`-Äquivalent mit Split-Limits, `internal_lets`,
//!   `remove_blocks`, Pattern-Exhaustiveness nur pro Funktion.

#![forbid(unsafe_code)]

pub mod anf;
pub mod ir;
pub mod mono;
pub mod shard;

pub use anf::{
    AnfConfig, AnfError, anf_lower, internal_lets, internal_lets_cow, remove_blocks,
    remove_blocks_cow,
};
pub use ir::{BinOp, Expr, FunDef, Lit, Pat, Ty};
pub use mono::{MonoConfig, MonoError, monomorphize};
pub use shard::{ShardConfig, ShardStats, lower_shard, lower_stream};
