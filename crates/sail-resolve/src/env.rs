//! Symbol tables (`Env`) and resolution errors.
//!
//! One global scope per Sail definition namespace (Sail top level is
//! order-independent) plus an explicit lexical-scope stack for clause
//! locals. Duplicate insertion into the *same* namespace is a `distincto`
//! error; shadowing across lexical scopes is allowed.
//!
//! Namespaces (Lane 78): types, ctors, valspecs, functions, registers,
//! mappings, values (top-level `let`), overloads, fixities, outcomes,
//! instantiations, measures, scattered opens (per scattered namespace),
//! pragmas (repeatable, no duplicate check), impls/clauses/ends (pending
//! vectors checked in the second pass by [`crate::resolve::Resolver`]).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use sail_ast::{Fixity, Order, Span};

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
    /// Whether the `val` carries an `extern` block (Bridge Lane 63
    /// always yields `None`; real extern bindings arrive with Lane 02).
    pub has_extern: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionEntry {
    pub name: String,
    pub span: Span,
    pub is_rec: bool,
    pub clauses: u32,
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

/// Top-level `let`-bound value name (pattern-bound at `DefKind::Let`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueEntry {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverloadEntry {
    pub name: String,
    pub targets: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScatteredNamespace {
    Function,
    Mapping,
    Variant,
    Enum,
}

/// An open `scattered` head (`Function`/`Mapping`/`Variant`/`Enum`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScatteredEntry {
    pub name: String,
    pub namespace: ScatteredNamespace,
    pub span: Span,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixityEntry {
    pub name: String,
    pub fixity: Fixity,
    pub level: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeEntry {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantiationEntry {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasureEntry {
    pub name: String,
    pub span: Span,
}

/// Repeatable `DEF_pragma` (incl. Lane-11 `file_start`/`file_end` markers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PragmaEntry {
    pub key: String,
    pub payload: String,
    pub span: Span,
}

/// A `DEF_impl` clause (target check runs in the second pass).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplEntry {
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
    DuplicateFunction,
    DuplicateRegister,
    DuplicateMapping,
    DuplicateValue,
    DuplicateOverload,
    DuplicateFixity,
    DuplicateScattered,
    DuplicateOutcome,
    DuplicateInstantiation,
    DuplicateMeasure,
    DuplicateDefault,
    /// Constructor collides with a type name (needs a rename downstream).
    /// Kept as a diagnostic, *not* fatal: [`crate::compute_ctor_renames`]
    /// resolves it.
    CtorTypeCollision,
    /// `overload` target names neither a `val` nor a function.
    UnknownOverloadTarget,
    /// `impl` without a matching open `scattered function`.
    DanglingImpl,
    /// Scattered clause (`Funcl`/`Mapcl`/`Unioncl`/`Enumcl`) without a
    /// matching open head in its namespace.
    DanglingScatteredClause,
    /// `scattered end` without a matching open head.
    DanglingScatteredEnd,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            ResolveErrorKind::DuplicateType => "duplicate type",
            ResolveErrorKind::DuplicateCtor => "duplicate constructor",
            ResolveErrorKind::DuplicateValSpec => "duplicate val spec",
            ResolveErrorKind::DuplicateFunction => "duplicate function",
            ResolveErrorKind::DuplicateRegister => "duplicate register",
            ResolveErrorKind::DuplicateMapping => "duplicate mapping",
            ResolveErrorKind::DuplicateValue => "duplicate value",
            ResolveErrorKind::DuplicateOverload => "duplicate overload",
            ResolveErrorKind::DuplicateFixity => "duplicate fixity",
            ResolveErrorKind::DuplicateScattered => "duplicate scattered definition",
            ResolveErrorKind::DuplicateOutcome => "duplicate outcome",
            ResolveErrorKind::DuplicateInstantiation => "duplicate instantiation",
            ResolveErrorKind::DuplicateMeasure => "duplicate measure",
            ResolveErrorKind::DuplicateDefault => "duplicate default",
            ResolveErrorKind::CtorTypeCollision => "constructor collides with type name",
            ResolveErrorKind::UnknownOverloadTarget => "unknown overload target",
            ResolveErrorKind::DanglingImpl => "impl without scattered function",
            ResolveErrorKind::DanglingScatteredClause => "scattered clause without open head",
            ResolveErrorKind::DanglingScatteredEnd => "scattered end without open head",
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
/// Clause patterns bind here; the top-level tables below are the global scope.
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
    pub functions: HashMap<String, FunctionEntry>,
    pub registers: HashMap<String, RegisterEntry>,
    pub mappings: HashMap<String, MappingEntry>,
    pub values: HashMap<String, ValueEntry>,
    pub overloads: HashMap<String, OverloadEntry>,
    pub fixities: HashMap<String, FixityEntry>,
    pub outcomes: HashMap<String, OutcomeEntry>,
    pub instantiations: HashMap<String, InstantiationEntry>,
    pub measures: HashMap<String, MeasureEntry>,
    pub scattered_functions: HashMap<String, ScatteredEntry>,
    pub scattered_mappings: HashMap<String, ScatteredEntry>,
    pub scattered_variants: HashMap<String, ScatteredEntry>,
    pub scattered_enums: HashMap<String, ScatteredEntry>,
    /// Repeatable pragmas in source order (never deduplicated).
    pub pragmas: Vec<PragmaEntry>,
    /// Accepted `impl` clauses in source order.
    pub impls: Vec<ImplEntry>,
    /// Bare `DEF_constraint` spans (no namespace, counted only).
    pub constraints: Vec<Span>,
    /// The single `DEF_default`, if present.
    pub default: Option<(Order, Span)>,
    /// `DEF_internal_mutrec` groups seen (functions land in `functions`).
    pub mutrec_groups: u32,
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
    // key clone for the map slot). The duplicate path only reads the
    // stored span.

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

    pub(crate) fn insert_function(&mut self, e: FunctionEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.functions.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateFunction,
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

    pub(crate) fn insert_value(&mut self, e: ValueEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.values.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateValue,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_overload(&mut self, e: OverloadEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.overloads.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateOverload,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_scattered(
        &mut self,
        namespace: ScatteredNamespace,
        e: ScatteredEntry,
    ) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        let table = match namespace {
            ScatteredNamespace::Function => &mut self.scattered_functions,
            ScatteredNamespace::Mapping => &mut self.scattered_mappings,
            ScatteredNamespace::Variant => &mut self.scattered_variants,
            ScatteredNamespace::Enum => &mut self.scattered_enums,
        };
        match table.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateScattered,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_fixity(&mut self, e: FixityEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.fixities.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateFixity,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_outcome(&mut self, e: OutcomeEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.outcomes.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateOutcome,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_instantiation(
        &mut self,
        e: InstantiationEntry,
    ) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.instantiations.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateInstantiation,
                name: e.name,
                first: o.get().span,
                second: e.span,
            }),
        }
    }

    pub(crate) fn insert_measure(&mut self, e: MeasureEntry) -> Result<(), ResolveError> {
        use std::collections::hash_map::Entry;
        match self.measures.entry(e.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(e);
                Ok(())
            }
            Entry::Occupied(o) => Err(ResolveError {
                kind: ResolveErrorKind::DuplicateMeasure,
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
    pub fn lookup_function(&self, name: &str) -> Option<&FunctionEntry> {
        self.functions.get(name)
    }

    #[must_use]
    pub fn lookup_register(&self, name: &str) -> Option<&RegisterEntry> {
        self.registers.get(name)
    }

    #[must_use]
    pub fn lookup_mapping(&self, name: &str) -> Option<&MappingEntry> {
        self.mappings.get(name)
    }

    #[must_use]
    pub fn lookup_value(&self, name: &str) -> Option<&ValueEntry> {
        self.values.get(name)
    }

    #[must_use]
    pub fn lookup_overload(&self, name: &str) -> Option<&OverloadEntry> {
        self.overloads.get(name)
    }

    #[must_use]
    pub fn lookup_fixity(&self, name: &str) -> Option<&FixityEntry> {
        self.fixities.get(name)
    }

    #[must_use]
    pub fn lookup_outcome(&self, name: &str) -> Option<&OutcomeEntry> {
        self.outcomes.get(name)
    }

    #[must_use]
    pub fn lookup_instantiation(&self, name: &str) -> Option<&InstantiationEntry> {
        self.instantiations.get(name)
    }

    #[must_use]
    pub fn lookup_measure(&self, name: &str) -> Option<&MeasureEntry> {
        self.measures.get(name)
    }

    #[must_use]
    pub fn lookup_scattered(
        &self,
        namespace: ScatteredNamespace,
        name: &str,
    ) -> Option<&ScatteredEntry> {
        match namespace {
            ScatteredNamespace::Function => self.scattered_functions.get(name),
            ScatteredNamespace::Mapping => self.scattered_mappings.get(name),
            ScatteredNamespace::Variant => self.scattered_variants.get(name),
            ScatteredNamespace::Enum => self.scattered_enums.get(name),
        }
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
            env.insert_valspec(ValSpecEntry { name: "f".to_string(), span, has_extern: false })
                .is_ok()
        );
        assert!(
            env.insert_function(FunctionEntry {
                name: "g".to_string(),
                span,
                is_rec: false,
                clauses: 1,
            })
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
        assert!(env.lookup_function("g").is_some());
        assert!(env.lookup_register("x".to_string().as_str()).is_some());
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

    #[test]
    fn insert_scattered_namespaces_stay_separate() {
        let mut env = Env::new();
        let span = span_at(0);
        for ns in [
            ScatteredNamespace::Function,
            ScatteredNamespace::Mapping,
            ScatteredNamespace::Variant,
            ScatteredNamespace::Enum,
        ] {
            assert!(
                env.insert_scattered(
                    ns,
                    ScatteredEntry {
                        name: "s".to_string(),
                        namespace: ns,
                        span,
                        closed: false,
                    }
                )
                .is_ok()
            );
        }
        let err = env
            .insert_scattered(
                ScatteredNamespace::Function,
                ScatteredEntry {
                    name: "s".to_string(),
                    namespace: ScatteredNamespace::Function,
                    span,
                    closed: false,
                },
            )
            .unwrap_err();
        assert_eq!(err.kind, ResolveErrorKind::DuplicateScattered);
    }
}
