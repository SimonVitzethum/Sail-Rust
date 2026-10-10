//! Minimal local AST stand-ins.
//!
//! These exist only because `sail-ast` (Lane 01) has not landed yet.
//! They cover exactly what name resolution needs: top-level definitions
//! with names, union/enum constructors, and use-sites for scope checks.
//! See `RESOLVE_NEEDED.md` for the mapping onto the future real AST.
//!
//! Locations are the canonical `sail-ast` type (Lane 61 unification of
//! MIGRATION.md #8; previously a line/col-only local copy, removed).

#![forbid(unsafe_code)]

/// Canonical source span (re-exported from `sail-ast`).
pub use sail_ast::{FileId, Span};

/// What a union/enum member carries. Payload types are intentionally
/// opaque strings here: resolution only needs the *names*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorDecl {
    pub name: String,
    pub span: Span,
}

impl CtorDecl {
    #[must_use]
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeDefKind {
    /// `struct Name { ... }` — no constructors.
    Struct,
    /// `enum Name { A | B | ... }`
    Enum(Vec<CtorDecl>),
    /// `union Name { A(...) | B(...) }`
    Union(Vec<CtorDecl>),
    /// `type Name = ...` / abbrev / bitfield-ish alias.
    Abbrev,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDef {
    pub name: String,
    pub kind: TypeDefKind,
    pub span: Span,
}

impl TypeDef {
    #[must_use]
    pub fn new(name: impl Into<String>, kind: TypeDefKind, span: Span) -> Self {
        Self {
            name: name.into(),
            kind,
            span,
        }
    }

    /// Constructor declarations carried by this typedef (empty for
    /// structs/abbrevs).
    #[must_use]
    pub fn ctors(&self) -> &[CtorDecl] {
        match &self.kind {
            TypeDefKind::Enum(c) | TypeDefKind::Union(c) => c,
            TypeDefKind::Struct | TypeDefKind::Abbrev => &[],
        }
    }

    #[must_use]
    pub const fn is_union(&self) -> bool {
        matches!(self.kind, TypeDefKind::Union(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValSpec {
    pub name: String,
    pub span: Span,
}

impl ValSpec {
    #[must_use]
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterDef {
    pub name: String,
    pub span: Span,
}

impl RegisterDef {
    #[must_use]
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingDef {
    pub name: String,
    pub span: Span,
}

impl MappingDef {
    #[must_use]
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

/// A top-level definition. Only the name-carrying subset of Sail `def`
/// needed for resolution; everything else (bodies, types) is out of
/// scope for Lane 03 (see Lane 05 typechecker).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Def {
    Type(TypeDef),
    ValSpec(ValSpec),
    Register(RegisterDef),
    Mapping(MappingDef),
    /// Any other top-level item (function, scattered def, pragma, …).
    /// Carries a name so overload/duplicate checks keep working once
    /// `sail-ast` provides the real node.
    Other {
        name: String,
        span: Span,
    },
}

impl Def {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Type(t) => &t.name,
            Self::ValSpec(v) => &v.name,
            Self::Register(r) => &r.name,
            Self::Mapping(m) => &m.name,
            Self::Other { name, .. } => name,
        }
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Type(t) => t.span,
            Self::ValSpec(v) => v.span,
            Self::Register(r) => r.span,
            Self::Mapping(m) => m.span,
            Self::Other { span, .. } => *span,
        }
    }
}

/// A whole program: flat list of top-level definitions, Sail-style
/// (order-independent at top level).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ast {
    pub defs: Vec<Def>,
}

impl Ast {
    #[must_use]
    pub fn new(defs: Vec<Def>) -> Self {
        Self { defs }
    }
}
