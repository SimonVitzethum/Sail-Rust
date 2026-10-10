#![forbid(unsafe_code)]
//! Sail2 core AST: pure data, no parser / typechecker / printer.

mod id;
mod interner;
mod span;

pub use id::{GenArena, IdGen, NodeId};
pub use interner::{Interner, SimpleInterner, Symbol};
pub use span::{FileId, Span};
