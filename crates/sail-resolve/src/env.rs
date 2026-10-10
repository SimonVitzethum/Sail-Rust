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
    //
    // RAM diet (Lane 68): all inserts move the entry in (`entry` API, one
    // key clone for the map slot). The old code cloned key *and* value on
    // every insert and re-inserted the previous entry on the duplicate
    // path; now the duplicate path only reads the stored span.

    pub(crate) fn insert_type(&mut self, e: TypeEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.types.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateType,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_ctor(&mut self, e: CtorEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.ctors.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateCtor,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_valspec(&mut self, e: ValSpecEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.valspecs.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateValSpec,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_register(&mut self, e: RegisterEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.registers.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateRegister,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_mapping(&mut self, e: MappingEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.mappings.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateMapping,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{FileId, Span};

    fn span_at(start: u32) -> Span {
        Span::new(FileId(0), start, start + 1)
    }

    #[test]
    fn insert_type_duplicate_keeps_first_and_reports_both_spans() {
        let mut env = Env::new();
        let first = span_at(0);
        let second = span_at(10);
        assert!(
            env.insert_type(TypeEntry {
                name: "T".to_string(),
                span: first,
                is_union: false,
            })
            .is_ok()
        );
        let err = env
            .insert_type(TypeEntry {
                name: "T".to_string(),
                span: second,
                is_union: true,
            })
            .unwrap_err();
        assert_eq!(err.kind, ResolveErrorKind::DuplicateType);
        assert_eq!(err.name, "T");
        assert_eq!(err.first, first);
        assert_eq!(err.second, second);
        // The winner stays the first entry (restore-on-duplicate).
        assert_eq!(
            env.lookup_type("T").map(|e| (e.span, e.is_union)),
            Some((first, false))
        );
    }

    #[test]
    fn insert_tables_accept_distinct_names() {
        let mut env = Env::new();
        let span = span_at(0);
        assert!(
            env.insert_ctor(CtorEntry {
                name: "A".to_string(),
                owner: "U".to_string(),
                span,
            })
            .is_ok()
        );
        assert!(
            env.insert_valspec(ValSpecEntry { name: "f".to_string(), span })
                .is_ok()
        );
        assert!(
            env.insert_register(RegisterEntry { name: "x".to_string(), span })
                .is_ok()
        );
        assert!(
            env.insert_mapping(MappingEntry { name: "m".to_string(), span })
                .is_ok()
        );
        assert_eq!(env.lookup_ctor("A").map(|c| c.owner.as_str()), Some("U"));
        assert!(env.lookup_valspec("f").is_some());
        assert!(env.lookup_register("x").is_some());
        assert!(env.lookup_mapping("m").is_some());
    }

    #[test]
    fn insert_ctor_duplicate_reports_without_clobbering() {
        let mut env = Env::new();
        let first = span_at(1);
        let second = span_at(2);
        assert!(
            env.insert_ctor(CtorEntry {
                name: "A".to_string(),
                owner: "U".to_string(),
                span: first,
            })
            .is_ok()
        );
        let err = env
            .insert_ctor(CtorEntry {
                name: "A".to_string(),
                owner: "V".to_string(),
                span: second,
            })
            .unwrap_err();
        assert_eq!(err.kind, ResolveErrorKind::DuplicateCtor);
        assert_eq!(err.first, first);
        assert_eq!(err.second, second);
        assert_eq!(
            env.lookup_ctor("A").map(|c| c.owner.as_str()),
            Some("U")
        );
    }
}
