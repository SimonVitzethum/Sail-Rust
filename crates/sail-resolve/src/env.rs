//! Symbol tables (`Env`) and resolution errors.
//!
//! One global scope for top-level items (Sail top level is
//! order-independent) plus an explicit lexical-scope stack for future
//! locals. Duplicate insertion into the *same* scope is a `distincto`
//! error; shadowing across scopes is allowed.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use sail_ast::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeEntry {
    pub name: String,
    pub span: Span,
    pub is_union: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorEntry {
    /// Constructor name as written.
    pub name: String,
    /// Owning union/enum type name.
    pub owner: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValSpecEntry {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterEntry {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingEntry {
    pub name: String,
    pub span: Span,
}

/// `distincto`/scope error with both definition sites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    pub kind: ResolveErrorKind,
    pub name: String,
    pub first: Span,
    pub second: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveErrorKind {
    DuplicateType,
    DuplicateCtor,
    DuplicateValSpec,
    DuplicateRegister,
    DuplicateMapping,
    /// Constructor collides with a type name (needs a rename downstream).
    /// Kept as a diagnostic, *not* fatal: [`crate::compute_ctor_renames`]
    /// resolves it.
    CtorTypeCollision,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            ResolveErrorKind::DuplicateType => "duplicate type",
            ResolveErrorKind::DuplicateCtor => "duplicate constructor",
            ResolveErrorKind::DuplicateValSpec => "duplicate val spec",
            ResolveErrorKind::DuplicateRegister => "duplicate register",
            ResolveErrorKind::DuplicateMapping => "duplicate mapping",
            ResolveErrorKind::CtorTypeCollision => "constructor collides with type name",
        };
        write!(
            f,
            "{what} '{}' (first at {}, again at {})",
            self.name, self.first, self.second
        )
    }
}

impl std::error::Error for ResolveError {}

/// A single lexical scope: the set of value-namespace names bound in it.
/// Used for future locals; the top-level tables below are the global scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: HashMap<String, Span>,
}

impl Scope {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `name`; `Err(previous_span)` if already bound in this scope.
    pub fn bind(&mut self, name: &str, span: Span) -> Result<(), Span> {
        use std::collections::hash_map::Entry;
        match self.bindings.entry(name.to_owned()) {
            Entry::Occupied(e) => Err(*e.get()),
            Entry::Vacant(e) => {
                e.insert(span);
                Ok(())
            }
        }
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<Span> {
        self.bindings.get(name).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Global symbol tables: one namespace per Sail definition kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub types: HashMap<String, TypeEntry>,
    pub ctors: HashMap<String, CtorEntry>,
    pub valspecs: HashMap<String, ValSpecEntry>,
    pub registers: HashMap<String, RegisterEntry>,
    pub mappings: HashMap<String, MappingEntry>,
    /// Non-fatal diagnostics (e.g. [`ResolveErrorKind::CtorTypeCollision`]).
    pub warnings: Vec<ResolveError>,
    /// Lexical scope stack; element 0 is the global scope mirror.
    scopes: Vec<Scope>,
}

impl Env {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
            ..Self::default()
        }
    }

    // -- scopes ---------------------------------------------------------
    pub fn enter_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn exit_scope(&mut self) {
        // Never pop the global scope.
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Bind a local name in the innermost scope.
    pub fn bind_local(&mut self, name: &str, span: Span) -> Result<(), Span> {
        debug_assert!(!self.scopes.is_empty());
        if self.scopes.is_empty() {
            self.scopes.push(Scope::new());
        }
        self.scopes
            .last_mut()
            .expect("scope stack non-empty")
            .bind(name, span)
    }

    /// Look up innermost-first.
    #[must_use]
    pub fn lookup_local(&self, name: &str) -> Option<Span> {
        self.scopes.iter().rev().find_map(|s| s.lookup(name))
    }

    // -- global tables ---------------------------------------------------
    pub(crate) fn insert_type(&mut self, e: TypeEntry) -> Result<(), ResolveError> {
        let prev = self.types.insert(e.name.clone(), e.clone());
        match prev {
            None => Ok(()),
            Some(p) => {
                self.types.insert(p.name.clone(), p.clone());
                Err(ResolveError {
                    kind: ResolveErrorKind::DuplicateType,
                    name: e.name,
                    first: p.span,
                    second: e.span,
                })
            }
        }
    }

    pub(crate) fn insert_ctor(&mut self, e: CtorEntry) -> Result<(), ResolveError> {
        let prev = self.ctors.insert(e.name.clone(), e.clone());
        match prev {
            None => Ok(()),
            Some(p) => {
                self.ctors.insert(p.name.clone(), p.clone());
                Err(ResolveError {
                    kind: ResolveErrorKind::DuplicateCtor,
                    name: e.name,
                    first: p.span,
                    second: e.span,
                })
            }
        }
    }

    pub(crate) fn insert_valspec(&mut self, e: ValSpecEntry) -> Result<(), ResolveError> {
        let prev = self.valspecs.insert(e.name.clone(), e.clone());
        match prev {
            None => Ok(()),
            Some(p) => {
                self.valspecs.insert(p.name.clone(), p.clone());
                Err(ResolveError {
                    kind: ResolveErrorKind::DuplicateValSpec,
                    name: e.name,
                    first: p.span,
                    second: e.span,
                })
            }
        }
    }

    pub(crate) fn insert_register(&mut self, e: RegisterEntry) -> Result<(), ResolveError> {
        let prev = self.registers.insert(e.name.clone(), e.clone());
        match prev {
            None => Ok(()),
            Some(p) => {
                self.registers.insert(p.name.clone(), p.clone());
                Err(ResolveError {
                    kind: ResolveErrorKind::DuplicateRegister,
                    name: e.name,
                    first: p.span,
                    second: e.span,
                })
            }
        }
    }

    pub(crate) fn insert_mapping(&mut self, e: MappingEntry) -> Result<(), ResolveError> {
        let prev = self.mappings.insert(e.name.clone(), e.clone());
        match prev {
            None => Ok(()),
            Some(p) => {
                self.mappings.insert(p.name.clone(), p.clone());
                Err(ResolveError {
                    kind: ResolveErrorKind::DuplicateMapping,
                    name: e.name,
                    first: p.span,
                    second: e.span,
                })
            }
        }
    }

    // -- queries ----------------------------------------------------------
    #[must_use]
    pub fn lookup_type(&self, name: &str) -> Option<&TypeEntry> {
        self.types.get(name)
    }

    #[must_use]
    pub fn lookup_ctor(&self, name: &str) -> Option<&CtorEntry> {
        self.ctors.get(name)
    }

    #[must_use]
    pub fn lookup_valspec(&self, name: &str) -> Option<&ValSpecEntry> {
        self.valspecs.get(name)
    }

    #[must_use]
    pub fn lookup_register(&self, name: &str) -> Option<&RegisterEntry> {
        self.registers.get(name)
    }

    #[must_use]
    pub fn lookup_mapping(&self, name: &str) -> Option<&MappingEntry> {
        self.mappings.get(name)
    }
}
