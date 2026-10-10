//! Out-of-core-Geruests (Lane 60, E4-bindend).
//!
//! Nur Heisses liegt in RAM (aktueller Shard-AST, aktiver Rewrite-State,
//! heisse Memos). Fertige Shards/Prelude/Builtins liegen warm
//! (dateigestuetzt, read-mostly, safe — kein raw-mmap wegen
//! `#![forbid(unsafe_code)]`, der OS-Page-Cache paget selbst). Memo-Spills und
//! inkrementelle Artefakte liegen kalt im Disk-Cache unter Content-Hash-Keys
//! (run-uebergreifend). Cross-Shard-Wissen tragen nur Summaries in RAM
//! (Lanes 08/09).
//!
//! Bausteine:
//!
//! * [`ContentId`]: stabile Content-Hash-ID statt Zaehler (E4-Voraussetzung;
//!   sonst invalidiert jeder Lauf den Cache).
//! * [`PhaseBarrier`]/[`HotToken`]/[`SealReceipt`]: Phasen-Barrieren —
//!   `typchecken → serialisieren + verwerfen → naechster`, hoechstens ein
//!   heisser Shard.
//! * [`WarmLayer`]: warmer Layer fuer fertige Shards (Datei-Backend,
//!   `Arc<[u8]>`-Reads, explizites Budget + Drop-Punkte).
//! * [`MemoSpills`]: kalte Memo-Spills ueber [`sail_cache::DiskCache`]
//!   (Lane 36), Keys sind Content-Hashes.
//! * [`SummaryTable`]/[`ShardSummary`]/[`ExportedSymbols`]:
//!   Cross-Shard-Summaries fuer Lanes 08/09 (Bodies bleiben auf Platte).
//! * [`CallgraphSummary`]/[`MonoSummary`]/[`ConstPropSummary`]
//!   (+ `export_*`): Summary-Typen + Export-Schnittstelle je
//!   interprozeduralem Pass (Lane 74; Bodies auf Platte).
//!
//! Auf Lane 11 ([`sail_shard`]) und Lane 36 ([`sail_cache`]) wird
//! aufgesetzt (gelesen, nicht gedoppelt); Lane 37 liefert die
//! Arena-Drop-Policy, an deren Drop-Punkten [`WarmLayer::evict_all`] und
//! [`SummaryTable::remove`] haengen.
//!
//! Verifikation per Test-Rig (spaeter, zentral — E3-Moratorium: keine
//! Builds/Tests in Lane-Agents); Tests liegen als Code bei.

#![forbid(unsafe_code)]

mod barrier;
mod id;
mod memo;
mod summary;
mod summaries;
mod warm;

pub use barrier::{BarrierError, HotToken, PhaseBarrier, SealReceipt, ShardPhase};
pub use id::ContentId;
pub use memo::{MEMO_DOMAIN_PREFIX, MemoSpills};
pub use summary::{
    ExportedSymbols, MAX_SUMMARY_BYTES, ShardSummary, SummaryError, SummaryRecord, SummaryTable,
};
pub use summaries::{
    CallFunc, CallgraphSummary, ConstFact, ConstPropSummary, ConstValue, EffectClass,
    MonoInstance, MonoSummary, callgraph_record, const_prop_record, export_callgraph,
    export_const_prop, export_mono, mono_record,
};
pub use warm::{WARM_BLOB_EXT, WarmError, WarmLayer};
