#![forbid(unsafe_code)]
//! Concurrency-interface mirror V1/V2 (`sail-archsem`, Lane 29).
//!
//! # Single-Master-Regel (bindend, `ARCHITECTURE.md` E1)
//!
//! Lean ist Master (mit Beweisen), Rust ist Spiegel: diese Crate definiert
//! **keine eigene Speichermodell-Semantik**. Jede Abbildung hier spiegelt
//! genau eine Stelle im Master:
//!
//! - `v2::Arch` spiegelt `class Arch` in
//!   `lean-sail/Sail/ArchSem.lean:8-59` (Feld-für-Feld, gleiche Reihenfolge).
//! - `v2::MemRequest` / `v2::SailMemRequest::to_archsem` spiegelt
//!   `MemRequest` / `Mem_request.toArchSem` (`ArchSem.lean:66-97`).
//! - `v2::InstructionEffect` + `v2::RetDesc` spiegelt
//!   `InstructionEffect` / `InstructionEffect.ret` (`ArchSem.lean:157-192`).
//! - `v2::SequentialState` spiegelt `SequentialState` +
//!   `interpretInstructionEffect` (`ArchSemSequential.lean:17-79`).
//! - `v1` spiegelt `Sail.ConcurrencyInterfaceV1`
//!   (`ConcurrencyInterfaceV1.lean:27-91`, sequentielle Auswertung `:195-290`).
//! - `instantiate::{V1Params,V2Instance}::emit` spiegelt
//!   `doc_instantiations_v1/v2` in
//!   `sail/src/sail_lean_backend/pretty_print_lean.ml:1524-1605`.
//!
//! Kein Multicore-Emulator: nur Instanz-Gen (Multicore bleibt Isla/RMEM-Sache).
//! Jede Abweichung vom Master ist per Definition ein Fehler und wird per
//! Differentialtest in `src/tests.rs` abgesichert (transkribierte
//! Referenzvektoren mit Quellenangabe). Drift blockiert den Merge.

pub mod instantiate;
pub mod v1;
pub mod v2;

#[cfg(test)]
mod tests;
