//! Local minimal AST (skeleton).
//!
//! Lane 01 (`sail-ast`) has not landed yet, so this crate defines the
//! smallest definition-level AST that lets the parser preserve all
//! information Lane 11 (sharding) and later lanes need. See
//! `crates/sail-parse/AST_NEEDED.md` for the expected `sail-ast`
//! interface and the migration path.
//!
//! Design (mirrors `DEF_*` in `sail/src/lib/extraction/Ast.ml`):
//! * Every definition keeps its leading keyword span, its header data
//!   (names, fixity, pragma args — cheap, extracted) and its raw source
//!   text (`body_src`). Full expression/type grammars are later lanes;
//!   storing the raw text keeps memory proportional to one definition,
//!   never to the whole model.
//! * Pragmas (`$name ...` / `$name { ... }`) are first-class [`Def`]
//!   nodes, never dropped: `file_start`/`file_end` (auto-wrapped per
//!   file, like OCaml `initial_check.ml`) and `include_start`/`end`
//!   are the shard markers Lane 11 consumes.

use crate::span::Span;

/// A pragma argument: either a rest-of-line string or structured `{ k = v }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PragmaArg {
    /// `$name <rest of line, trimmed>`.
    Line(String),
    /// `$name { k = v, ... }` — raw inside-braces text (structured
    /// key/value parsing is a later refinement).
    Structured(String),
}

/// One Sail2 top-level definition (skeleton).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Def {
    /// `val name : type` (+ optional `forall`/`constraint` prefix kept raw).
    Val { name: String, sig_src: String },
    /// `function [rec] name (params) [-> ret] = body`.
    Fundef { name: String, head_src: String, body_src: String },
    /// `mapping name ... = ...` (header + raw clauses).
    Mapdef { name: String, head_src: String, body_src: String },
    /// `impl name ... = ...` (function clause instance).
    Impl { name: String, head_src: String, body_src: String },
    /// `type name ... = ...` / `struct` / `union` / `enum` / `bitfield` RHS.
    Type { name: String, body_src: String },
    /// `let pat = exp`.
    Let { pat_src: String, body_src: String },
    /// `constraint ...` (kind/typ-side constraint decl, raw).
    Constraint { src: String },
    /// `overload name = { ids }`.
    Overload { name: String, ids: Vec<String> },
    /// `fixity prec level name`, e.g. `infixr 6 ^`.
    Fixity { prec: String, level: String, name: String },
    /// `default Order inc|dec`.
    Default { kind: String, value: String },
    /// `scattered ...` (function/union/mapping/enum spread over files).
    Scattered { head_src: String, body_src: String },
    /// `register name : type [= init]`.
    Register { name: String, type_src: String, init_src: Option<String> },
    /// `measure ...` / loop measures (raw; termination lane refines).
    Measure { src: String },
    /// `outcome ...` / `instantiation ...` (raw).
    Outcome { src: String },
    Instantiation { src: String },
    /// `$name args` — includes `include`, `define`, `ifdef`, ... and the
    /// shard markers `file_start`/`file_end`, `include_start`/`include_end`.
    Pragma { name: String, arg: PragmaArg },
    /// Forward-compatible fallback: recognised starter keyword with raw
    /// text, so new/rare definition forms never lose source.
    Other { keyword: String, src: String },
}

impl Def {
    /// True for the Lane-11 shard markers.
    pub fn is_shard_marker(&self) -> bool {
        matches!(
            self,
            Def::Pragma { name, .. }
                if name == "file_start"
                    || name == "file_end"
                    || name == "include_start"
                    || name == "include_end"
        )
    }

    /// Short kind label (for sharding stats / debugging).
    pub fn kind(&self) -> &'static str {
        match self {
            Def::Val { .. } => "val",
            Def::Fundef { .. } => "function",
            Def::Mapdef { .. } => "mapping",
            Def::Impl { .. } => "impl",
            Def::Type { .. } => "type",
            Def::Let { .. } => "let",
            Def::Constraint { .. } => "constraint",
            Def::Overload { .. } => "overload",
            Def::Fixity { .. } => "fixity",
            Def::Default { .. } => "default",
            Def::Scattered { .. } => "scattered",
            Def::Register { .. } => "register",
            Def::Measure { .. } => "measure",
            Def::Outcome { .. } => "outcome",
            Def::Instantiation { .. } => "instantiation",
            Def::Pragma { .. } => "pragma",
            Def::Other { .. } => "other",
        }
    }
}

/// One parsed file: path + definitions in source order, always wrapped in
/// `file_start`/`file_end` pragma markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub path: String,
    pub defs: Vec<SpannedDef>,
}

/// A definition with its full source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedDef {
    pub def: Def,
    pub span: Span,
}

impl SpannedDef {
    pub fn new(def: Def, span: Span) -> Self {
        SpannedDef { def, span }
    }
}
