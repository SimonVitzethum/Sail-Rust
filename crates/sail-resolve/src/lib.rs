//! `sail-resolve`: scope resolution and symbol tables for Sail.
//!
//! Input is (currently) the local [`mini_ast`] until `sail-ast` (Lane 01)
//! lands; see `RESOLVE_NEEDED.md`. Output is a [`ResolvedProgram`]
//! containing symbol tables plus the pure
//! [`compute_ctor_renames`](renames::compute_ctor_renames) map.
//!
//! Design notes:
//! - `#![forbid(unsafe_code)]`, std only, no allocator tricks.
//! - All tables are plain `HashMap`s of owned `String`s; `Send + Sync`.
//! - `compute_ctor_renames` is a pure function: deterministic, no I/O.

#![forbid(unsafe_code)]

pub mod env;
pub mod mini_ast;
pub mod renames;
pub mod resolve;

pub use env::{CtorEntry, Env, MappingEntry, RegisterEntry, ResolveError, TypeEntry, ValSpecEntry};
pub use mini_ast::{Ast, Def, MappingDef, RegisterDef, TypeDef, TypeDefKind, ValSpec};
pub use renames::compute_ctor_renames;
pub use resolve::{ResolvedProgram, Resolver};
