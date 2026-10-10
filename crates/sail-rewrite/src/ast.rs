//! Minimal shard-local AST surface for rewrite passes.
//!
//! This is intentionally small: passes operate on one [`Shard`] (a chunk of
//! [`Def`]s) at a time so they stay shard-local, allocation-light, and
//! trivially parallelisable with `rayon`. It will grow / be replaced by the
//! real `sail-ast` once that lane lands; pass logic must only rely on what is
//! defined here.

#![forbid(unsafe_code)]

/// One rewrite unit: a small chunk of definitions.
///
/// Shards are the parallelism + low-RAM granularity: the pipeline processes
/// each shard independently, so peak memory is `O(shard)` per thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shard {
    /// Stable shard index (used for deterministic reassembly).
    pub id: usize,
    /// Definitions contained in this shard.
    pub defs: Vec<Def>,
}

impl Shard {
    /// Create a shard from an id and its definitions.
    #[must_use]
    pub fn new(id: usize, defs: Vec<Def>) -> Self {
        Self { id, defs }
    }

    /// Split definitions into `n` shards of at most `chunk_size` defs.
    ///
    /// Pure function, no global state. Used to keep per-thread RAM bounded.
    #[must_use]
    pub fn chunk(defs: Vec<Def>, chunk_size: usize) -> Vec<Shard> {
        let chunk_size = chunk_size.max(1);
        defs.chunks(chunk_size)
            .enumerate()
            .map(|(id, c)| Shard::new(id, c.to_vec()))
            .collect()
    }

    /// Number of definitions in this shard.
    #[must_use]
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Whether this shard holds no definitions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// A single top-level Sail definition (function, valspec, register, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    /// Definition name (function name, register name, ...).
    pub name: String,
    /// What kind of definition this is.
    pub kind: DefKind,
    /// Body statements of the definition.
    pub body: Vec<Stmt>,
}

impl Def {
    /// Create a definition.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: DefKind, body: Vec<Stmt>) -> Self {
        Self {
            name: name.into(),
            kind,
            body,
        }
    }
}

/// Coarse definition kind. Enough for passes to decide applicability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefKind {
    /// Executable function / mapping body.
    Function,
    /// `val` spec (signature without body).
    Valspec,
    /// Register / config declaration.
    Register,
    /// Type / synonym declaration.
    Type,
    /// Anything else (pragma, import, ...). Passes leave these alone.
    Other,
}

/// Minimal statement model covering what the implemented passes need.
///
/// Variants carry plain strings (variable / expression text) so passes stay
/// independent of a full Sail expression AST for now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// Plain assignment `lhs = rhs;`.
    Assign {
        /// Left-hand side (variable or pattern text).
        lhs: String,
        /// Right-hand side expression text.
        rhs: String,
    },
    /// Tuple destructuring assignment `(a, b, ..) = rhs;`.
    TupleAssign {
        /// Destructured targets in order.
        targets: Vec<String>,
        /// Right-hand side expression text.
        rhs: String,
    },
    /// `let x : ty = undefined;` placeholder (OCaml `undefined` rewrite input).
    Undefined {
        /// Variable being given an undefined value.
        target: String,
        /// Declared type text, used to pick the default value.
        ty: String,
    },
    /// Any other statement / expression; passes preserve it verbatim.
    Expr(String),
    /// `return e;`.
    Return(String),
}
