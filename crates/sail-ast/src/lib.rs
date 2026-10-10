#![forbid(unsafe_code)]
//! Sail2 core AST: pure data, no parser / typechecker / printer.
//!
//! Conventions for other lanes:
//! - Every major node is [`Node<K>`] carrying a generational [`NodeId`]
//!   plus a [`Span`]. No pointers, no arenas inside the AST itself.
//! - Names are interned [`Symbol`]s; see [`Interner`] / [`SimpleInterner`].
//!   Literal payloads ([`LitKind`]) are interned too (Lane 71);
//!   resolve them through the caller's interner.
//! - [`Ast`] is the snapshot root with a JSON roundtrip
//!   ([`Ast::to_json`] / [`Ast::from_json`]).
//! - All types are `Clone + Send + Sync` and `serde`-serializable.

mod id;
mod interner;
mod span;

pub mod ast;
pub mod json;

pub use id::{GenArena, IdGen, NodeId};
pub use interner::{Interner, SimpleInterner, Symbol};
pub use span::{FileId, Span};

pub use ast::{
    Ast, Constraint, ConstraintKind, DecSpec, Def, DefKind, Exp, ExpKind, Extern, Fixity, Funcl,
    FunclSet, FExp, IndexRange, IndexRangeKind, KOpt, Kind, LExp, LExpKind, Lit, LitKind, Loop,
    Mapcl, MapclKind, Name, NExp, NExpKind, Node, Order, PExp, PExpKind, Pat, PatKind, QuantItem,
    RecOpt, ScatteredDef, Subst, StructName, TAnnotOpt, Typ, TypArg, TypArgKind, TypKind, TypPat,
    TypPatKind, TypSchm, TypeDef, TypeDefKind, TypeUnion, ValSpec, Value,
};
