//! `sail-resolve`: scope resolution and symbol tables for Sail.
//!
//! Input is the canonical [`sail_ast::Ast`] (Lane 61 unification,
//! MIGRATION.md #9–#16; the former `mini_ast` stand-ins are removed).
//! Output is a [`ResolvedProgram`] containing symbol tables plus the pure
//! [`compute_ctor_renames`](renames::compute_ctor_renames) map.
//!
//! Design notes:
//! - `#![forbid(unsafe_code)]`, std + `sail-ast` only, no allocator tricks.
//! - All tables are plain `HashMap`s of owned `String`s; `Send + Sync`.
//! - `compute_ctor_renames` is a pure function: deterministic, no I/O.
//! - Names arrive interned (`Symbol`); they are resolved through the
//!   caller's [`Interner`](sail_ast::Interner) into table keys. A def
//!   whose name does not resolve is skipped (cannot happen with a
//!   consistent interner).

#![forbid(unsafe_code)]

pub mod env;
pub mod renames;
pub mod resolve;

pub use env::{CtorEntry, Env, MappingEntry, RegisterEntry, ResolveError, TypeEntry, ValSpecEntry};
pub use renames::compute_ctor_renames;
pub use resolve::{ResolvedProgram, Resolver};
