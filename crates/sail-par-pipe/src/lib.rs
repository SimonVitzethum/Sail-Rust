//! Shard-parallele Pipeline-Orchestrierung (Lane 84, Implementierung der
//! Lane-69-Skizze).
//!
//! Führt die drei Compiler-Stufen **Parse → Check → Emit** shard-parallel aus:
//! ein Shard pro Worker, begrenzte Queues (Backpressure), deterministische
//! Merge-Reihenfolge. Noch **kein Treiber-Umbau** fremder Crates: Die Stufen
//! sind injizierte Closures (`Fn(Shard) -> …`), sodass echte
//! `sail-parse`-/`sail-ty`-/`sail-lean`-Treiber später verdrahtet werden,
//! ohne diese Crates umzubauen.
//!
//! ## Architektur
//!
//! ```text
//!  shards (kanonische Ordnung, z. B. ARMv9_SHARD_ORDER)
//!    │  ingress: (order: usize, Shard)
//!    ▼
//!  ┌─────────┐  bounded(cap)  ┌─────────┐  bounded(cap)  ┌────────┐  unbounded  ┌─────────┐
//!  │  PARSE  │ ─────────────▶ │  CHECK  │ ─────────────▶ │  EMIT  │ ──────────▶ │  MERGE  │
//!  │ np Work.│   Backpressure │ nc Work.│   Backpressure │ne Work.│  deadlock-  │ sort by │
//!  └─────────┘                └─────────┘                └────────┘  frei       │  order  │
//!                                                                             └─────────┘
//! ```
//!
//! * **Bounded Queues / Backpressure:** Die Kanäle zwischen den Stufen haben
//!   Kapazität [`PipeConfig::max_inflight`] (Default 8, Lane-11-Konvention).
//!   `send` blockiert bei voller Queue — der Produzent kann nie mehr als
//!   `max_inflight` wartende Shards aufstauen (low-RAM), egal wie viele
//!   Shards es gibt. Entspricht [`sail_shard::Scheduler`] (Lane 11).
//! * **Ergebniskanal unbegrenzt:** Wie im Lane-11-Scheduler ist der letzte
//!   Kanal bewusst unbegrenzt — begrenzt + erst-nach-Produktion-lesen
//!   verklemmt bei mehreren Workern (Produzent blockiert auf voller
//!   Arbeits-, Worker auf voller Ergebnis-Queue).
//! * **Ein Shard pro Worker:** Jede Stufe läuft in einem eigenen
//!   Worker-Pool; ein Worker bearbeitet stets genau einen Shard.
//! * **Deterministischer Merge:** Jede Eingabe trägt ihren Eingangsindex
//!   (`order`); [`merge`](merge/index.html) sortiert vor der Rückgabe nach
//!   `order`. Ausgabe ist bit-identisch unabhängig von Thread-Scheduling.
//! * **Fehlerisolation:** Schlägt eine Stufe für einen Shard fehl, wird der
//!   Fehler als [`StageError`] mit Stufen- und Shard-Angabe nach unten
//!   durchgereicht (Downstream-Stufen überspringen den Shard); alle anderen
//!   Shards laufen normal weiter.
//!
//! ## Varianten
//!
//! * [`run_pipeline`](pipeline/fn.run_pipeline.html): dreistufige Pipeline
//!   (Überlappung: Parse von Shard N+1 läuft neben Check von Shard N).
//! * [`run_shard_parallel`](pipeline/fn.run_shard_parallel.html): ganze Kette
//!   Parse→Check→Emit pro Shard in einem Worker, direkt auf
//!   [`sail_shard::Scheduler::run_with_index`] (Lane 11) aufsetzend.
//!   Einfacher, kein Stufen-Überlapp, gleiche Merge-Garantie.
//!
//! ## Lane-Bezug (nur gelesen, nichts gedoppelt)
//!
//! * Lane 11 (`sail-shard`): Shard-Typ, `Scheduler` (bounded Arbeits-Queue,
//!   `max_inflight`-Default 8), `IncrementalCache` — wiederverwendet, nicht
//!   nachimplementiert.
//! * Lane 43 (Scheduler/Replay, QUEUED): bounded Queues + deterministisches
//!   Replay — diese Pipeline übernimmt das Queue-Prinzip und liefert über
//!   den `order`-Index die deterministische Ordnung, auf der ein späterer
//!   Replay-Modus aufsetzen kann.
//! * Lane 62: im Branch-Stand kein Crate vorhanden (geprüft 10.10.2026) —
//!   kein Bezug.
//!
//! ## E3-Hinweis
//!
//! Unter dem E3-Test-Moratorium wird diese Crate **nicht gebaut und nicht
//! getestet**. Die `#[cfg(test)]`-Module sind Test-Code (Verifikation via
//! Test-Rig, später).
//!
//! ## Verwendungsskizze
//!
//! ```ignore
//! use sail_par_pipe::{PipeConfig, run_pipeline};
//! use sail_shard::shard_files;
//!
//! let shards = shard_files(&files, 400); // Lane 11, kanonische Ordnung
//! let out = run_pipeline(
//!     shards,
//!     |shard| Ok::<_, String>(shard.contents), // parse (Platzhalter)
//!     |parsed| Ok::<_, String>(parsed),        // check (Platzhalter)
//!     |checked| Ok::<_, String>(checked),      // emit (Platzhalter)
//!     PipeConfig::default(),
//! );
//! // `out` ist in Eingangsreihenfolge sortiert; Fehler pro Shard in `output`.
//! ```

#![forbid(unsafe_code)]

pub mod merge;
pub mod pipeline;

pub use merge::{concat_ok, manifest, merge_ordered, merge_ordered_par};
pub use pipeline::{
    PipeConfig, ShardOutput, Stage, StageError, run_pipeline, run_pipeline_streaming, run_shard_parallel,
};
