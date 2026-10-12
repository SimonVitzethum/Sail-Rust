#![forbid(unsafe_code)]
//! Scattered-clause body elaboration (Lane-37).
//!
//! `check_definition` currently rejects every `DefKind::Scattered` body with
//! `Unsupported` (`canonical_body.rs`). This module elaborates those bodies
//! against the scattered tables instead of rejecting them: it reuses the
//! block-opening / clause-counting / end-linkage rules of `declare_scattered`
//! (`program.rs`) as read-only linkage checks, then returns a [`BodyPlan`]
//! per clause that the body pass executes (function bodies are checked,
//! headers/ends/enum members carry no body work).
//!
//! The module is intentionally decoupled from [`crate::program::Program`]:
//! it only sees a [`ScatteredView`] (open state + clause counts at one
//! source position), so it compiles without the declaration tables and its
//! linkage rules are unit-testable. The Main-Agent owns the wiring
//! (`program.rs` / `canonical_body.rs`, see `INSERT-37` below).
//!
//! # View contract (body time)
//!
//! The driver declares every definition before any body is checked, and a
//! body worker at `position` sees history entries with `position <= own`.
//! Therefore the view passed to [`elaborate`] must be the *body-time* view:
//!
//! - `is_open` reflects openings/ends declared at or before this def.
//! - `clause_count` for a clause's own block *includes this clause's own*
//!   declare entry, so the 0-based clause index is `count - 1`
//!   ([`elaborate`] applies `saturating_sub(1)`).
//! - `closed_at` reports the namespace the `end` declared at exactly
//!   `position` closed (`declare_scattered` pushes the closed entry at the
//!   end's own position). With exactly one open block and no recorded close
//!   (declare-time view) the end target is unambiguous and resolved from the
//!   open set.
//!
//! All linkage diagnostics reuse `declare_scattered`'s messages verbatim so
//! declare-time and body-time failures agree.
//!
//! # INSERT-37 (Main-Agent wiring; this file is otherwise complete)
//!
//! 1. `lib.rs`: add `pub mod scattered_elab;` (already present in this
//!    worktree; keep the line when merging).
//! 2. `program.rs`: the scattered accessors (`scattered`,
//!    `scattered_clause_count`) already exist. Add:
//!
//! ```ignore
//! impl From<crate::scattered_elab::Namespace> for ScatteredNamespace {
//!     fn from(ns: crate::scattered_elab::Namespace) -> Self {
//!         match ns {
//!             crate::scattered_elab::Namespace::Function => Self::Function,
//!             crate::scattered_elab::Namespace::Mapping => Self::Mapping,
//!             crate::scattered_elab::Namespace::Variant => Self::Variant,
//!             crate::scattered_elab::Namespace::Enum => Self::Enum,
//!         }
//!     }
//! }
//!
//! /// Namespace closed by the `scattered end` declared at exactly
//! /// `position`, if any (Lane-37 linkage for the body pass: the end's own
//! /// declare entry is visible at body time and masks the open block).
//! #[must_use]
//! pub fn scattered_closed_at(&self, name: &str, position: usize) -> Option<ScatteredNamespace> {
//!     [
//!         ScatteredNamespace::Function,
//!         ScatteredNamespace::Mapping,
//!         ScatteredNamespace::Variant,
//!         ScatteredNamespace::Enum,
//!     ]
//!     .into_iter()
//!     .find(|ns| {
//!         self.scattered.get(&(*ns, name.to_owned())).is_some_and(|history| {
//!             history
//!                 .iter()
//!                 .rfind(|b| b.position <= position)
//!                 .is_some_and(|b| b.position == position && b.value.closed)
//!         })
//!     })
//! }
//! ```
//!
//! 3. `canonical_body.rs`: remove `DefKind::Scattered(_)` from the
//!    `Unsupported` group and add (anchor: the `match &def.kind` in
//!    `check`, next to the `DefKind::Fundef` arm):
//!
//! ```ignore
//! // INSERT-37A: body-time view adapter (paste near the `Checker` impl, once).
//! struct ProgramScattered<'a> {
//!     program: &'a Program,
//!     position: usize,
//! }
//! impl crate::scattered_elab::ScatteredView for ProgramScattered<'_> {
//!     fn is_open(&self, ns: crate::scattered_elab::Namespace, name: &str) -> bool {
//!         self.program
//!             .scattered(ns.into(), name, self.position)
//!             .is_some_and(|decl| !decl.closed)
//!     }
//!     fn clause_count(&self, ns: crate::scattered_elab::Namespace, name: &str) -> usize {
//!         self.program.scattered_clause_count(ns.into(), name, self.position)
//!     }
//!     fn closed_at(&self, name: &str, position: usize) -> Option<crate::scattered_elab::Namespace> {
//!         // `scattered_closed_at` returns `program::ScatteredNamespace`; map
//!         // back variant-for-variant (same four variants, same order).
//!         self.program.scattered_closed_at(name, position).map(|ns| match ns {
//!             ScatteredNamespace::Function => crate::scattered_elab::Namespace::Function,
//!             ScatteredNamespace::Mapping => crate::scattered_elab::Namespace::Mapping,
//!             ScatteredNamespace::Variant => crate::scattered_elab::Namespace::Variant,
//!             ScatteredNamespace::Enum => crate::scattered_elab::Namespace::Enum,
//!         })
//!     }
//! }
//!
//! // INSERT-37B: in `check`, replacing the `DefKind::Scattered(_)` arm.
//! DefKind::Scattered(scat) => {
//!     use crate::scattered_elab::{BodyPlan, ScatteredView as _};
//!     let _ = ScatteredView::is_open; // trait bound via the adapter below.
//!     let view = ProgramScattered { program, position };
//!     match crate::scattered_elab::elaborate(def, &checker.it, &view, position)? {
//!         BodyPlan::AcceptHeader { .. }
//!         | BodyPlan::AcceptEnumcl { .. }
//!         | BodyPlan::AcceptEnd { .. } => {}
//!         BodyPlan::CheckFuncl { .. } => {
//!             let ScatteredDef::Funcl(funcl) = scat else {
//!                 return Err(unsupported(def.span, "scattered plan/clause mismatch"));
//!             };
//!             // INSERT-37C (follow-up): `checker.function` resolves the
//!             // signature via `program.value`, but scattered functions keep
//!             // their header in the scattered table, not the value table.
//!             // Resolve the header with
//!             // `program.scattered(Function, name, position)` (quant + ret),
//!             // synthesize the argument types with `infer_pattern_type` when
//!             // the header has no annotation (same as inline `Fundef`
//!             // synthesis in `declare`), then run the existing pattern/body
//!             // checking path on `funcl`. Until then this arm keeps the
//!             // precise `Unsupported` below instead of the blanket one.
//!             checker.function(funcl)?;
//!         }
//!         BodyPlan::CheckMapcl { .. } | BodyPlan::CheckUnioncl { .. } => {
//!             return Err(unsupported(
//!                 def.span,
//!                 "scattered mapping/union clause bodies (mapping plans / union payload elaboration pending)",
//!             ))
//!         }
//!     }
//! }
//! ```
//!
//! Until INSERT-37C lands, `CheckFuncl` falls back to the precise
//! `Unsupported` above; every other scattered body is already accepted or
//! carries its exact plan instead of the blanket rejection.

use crate::error::{TypeError, TypeErrorKind, TypeResult};
use sail_ast::{Def, DefKind, Interner, Name, ScatteredDef, SimpleInterner, Span, Symbol};

/// Namespace of a `scattered` block (OCaml `parse_ast` `scattered_def_aux`).
/// Functions, mappings, unions and enums live in separate namespaces, so a
/// `function f` and a `mapping f` may coexist. Mirrors
/// `program::ScatteredNamespace` variant-for-variant (see `INSERT-37`); this
/// copy keeps the elaboration testable without the declaration tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    Function,
    Mapping,
    Variant,
    Enum,
}

impl Namespace {
    /// Deterministic iteration order (matches `declare_scattered`'s end scan).
    pub const ALL: [Namespace; 4] = [
        Namespace::Function,
        Namespace::Mapping,
        Namespace::Variant,
        Namespace::Enum,
    ];

    /// Deterministic key prefix for reports and clause summaries.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Mapping => "mapping",
            Self::Variant => "union",
            Self::Enum => "enum",
        }
    }
}

/// Namespace of a scattered opening or clause item. `End` closes whichever
/// single namespace is open (resolved by [`elaborate`]), so it maps to
/// `None`.
#[must_use]
pub fn namespace_of(scat: &ScatteredDef) -> Option<Namespace> {
    match scat {
        ScatteredDef::Function { .. } | ScatteredDef::Funcl(_) => Some(Namespace::Function),
        ScatteredDef::Mapping { .. } | ScatteredDef::Mapcl { .. } => Some(Namespace::Mapping),
        ScatteredDef::Variant { .. } | ScatteredDef::Unioncl { .. } => Some(Namespace::Variant),
        ScatteredDef::Enum(_) | ScatteredDef::Enumcl { .. } => Some(Namespace::Enum),
        ScatteredDef::End(_) => None,
    }
}

/// Read-only view of the scattered tables at one source position (see the
/// module-level view contract). The Main-Agent implements this for the body
/// pass over `Program` (INSERT-37A); tests use a fake.
pub trait ScatteredView {
    /// Whether `(namespace, name)` has an unclosed block at this position.
    fn is_open(&self, namespace: Namespace, name: &str) -> bool;
    /// Clauses recorded for `(namespace, name)` at this position. At body
    /// time this includes the clause's own declare entry (see contract).
    fn clause_count(&self, namespace: Namespace, name: &str) -> usize;
    /// Namespace closed by the `end` declared at exactly `position`, if any.
    fn closed_at(&self, name: &str, position: usize) -> Option<Namespace>;
    /// Namespaces with an unclosed block for `name`, in [`Namespace::ALL`]
    /// order.
    fn open_namespaces(&self, name: &str) -> Vec<Namespace> {
        Namespace::ALL
            .iter()
            .copied()
            .filter(|namespace| self.is_open(*namespace, name))
            .collect()
    }
}

/// What the body pass must do with one `DefKind::Scattered` definition.
/// Linkage (open / clause / end) is already validated by [`elaborate`];
/// only the `Check*` variants carry body work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyPlan {
    /// Check one scattered function clause like a `Fundef` clause
    /// (INSERT-37B/37C). `index` is the 0-based clause position.
    CheckFuncl { name: String, index: usize },
    /// Check one scattered mapping clause bidirectionally (plan only;
    /// mapping body elaboration pending, INSERT-37B keeps the precise
    /// `Unsupported` for this variant).
    CheckMapcl { name: String, index: usize },
    /// Check one scattered union constructor payload (plan only; union
    /// payload elaboration pending, same fallback as `CheckMapcl`).
    CheckUnioncl { name: String, index: usize },
    /// Scattered enum member: no body, linkage only.
    AcceptEnumcl { name: String, index: usize },
    /// Block opening: no body work; the declare-time header checks stand.
    AcceptHeader { namespace: Namespace, name: String },
    /// `end`: closes the single open block; linkage only.
    AcceptEnd { namespace: Namespace, name: String },
}

fn quant(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::QuantError(message.into()),
    }
}

fn unsupported(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported(message.into()),
    }
}

fn text(it: &SimpleInterner, symbol: Symbol, span: Span) -> TypeResult<String> {
    it.resolve(symbol)
        .map(str::to_owned)
        .ok_or_else(|| TypeError::unbound_var(span, format!("symbol {symbol}")))
}

/// Elaborate one `DefKind::Scattered` definition: validate its linkage
/// against the body-time view (block-opening / clause-counting / end rules
/// of `declare_scattered`, messages verbatim) and return the body plan
/// instead of rejecting the body with `Unsupported`.
pub fn elaborate<V: ScatteredView>(
    def: &Def,
    it: &SimpleInterner,
    view: &V,
    position: usize,
) -> TypeResult<BodyPlan> {
    let DefKind::Scattered(scat) = &def.kind else {
        return Err(unsupported(def.span, "not a scattered definition"));
    };
    match scat {
        ScatteredDef::Function { name, .. } => {
            open_header(view, Namespace::Function, name, it)
        }
        ScatteredDef::Mapping { name, .. } => open_header(view, Namespace::Mapping, name, it),
        ScatteredDef::Variant { name, .. } => open_header(view, Namespace::Variant, name, it),
        ScatteredDef::Enum(name) => open_header(view, Namespace::Enum, name, it),
        ScatteredDef::Funcl(funcl) => clause_body(
            view,
            Namespace::Function,
            &funcl.name,
            funcl.span,
            it,
            |name, index| BodyPlan::CheckFuncl { name, index },
        ),
        ScatteredDef::Mapcl { name, .. } => clause_body(
            view,
            Namespace::Mapping,
            name,
            def.span,
            it,
            |name, index| BodyPlan::CheckMapcl { name, index },
        ),
        ScatteredDef::Unioncl { name, .. } => clause_body(
            view,
            Namespace::Variant,
            name,
            def.span,
            it,
            |name, index| BodyPlan::CheckUnioncl { name, index },
        ),
        ScatteredDef::Enumcl { enum_name, .. } => clause_body(
            view,
            Namespace::Enum,
            enum_name,
            def.span,
            it,
            |name, index| BodyPlan::AcceptEnumcl { name, index },
        ),
        ScatteredDef::End(name) => end_linkage(view, name, it, position),
    }
}

/// Block opening: reject re-opening an unclosed block of the same
/// namespace (OCaml rejects it during scattered elaboration); otherwise
/// the header carries no body work.
fn open_header(
    view: &impl ScatteredView,
    namespace: Namespace,
    name: &Name,
    it: &SimpleInterner,
) -> TypeResult<BodyPlan> {
    let name_text = text(it, name.symbol, name.span)?;
    if view.is_open(namespace, &name_text) {
        return Err(quant(
            name.span,
            format!(
                "{} '{name_text}' is already openly scattered",
                namespace.prefix()
            ),
        ));
    }
    Ok(BodyPlan::AcceptHeader {
        namespace,
        name: name_text,
    })
}

/// Clause linkage: the block must be open; the plan carries the 0-based
/// clause index. The body-time count includes this clause's own declare
/// entry, hence `saturating_sub(1)` (see the module-level contract).
fn clause_body(
    view: &impl ScatteredView,
    namespace: Namespace,
    name: &Name,
    span: Span,
    it: &SimpleInterner,
    plan: impl FnOnce(String, usize) -> BodyPlan,
) -> TypeResult<BodyPlan> {
    let name_text = text(it, name.symbol, name.span)?;
    if !view.is_open(namespace, &name_text) {
        return Err(quant(
            span,
            format!(
                "{} clause for '{name_text}' has no open scattered block",
                namespace.prefix()
            ),
        ));
    }
    let index = view.clause_count(namespace, &name_text).saturating_sub(1);
    Ok(plan(name_text, index))
}

/// End linkage: close the single open block of that name across
/// namespaces. `closed_at` resolves the body-time case (the end's own
/// declare entry already closed exactly one namespace); with exactly one
/// open block and no recorded close the target is unambiguous.
fn end_linkage(
    view: &impl ScatteredView,
    name: &Name,
    it: &SimpleInterner,
    position: usize,
) -> TypeResult<BodyPlan> {
    let name_text = text(it, name.symbol, name.span)?;
    if let Some(namespace) = view.closed_at(&name_text, position) {
        return Ok(BodyPlan::AcceptEnd {
            namespace,
            name: name_text,
        });
    }
    let open = view.open_namespaces(&name_text);
    match open.len() {
        0 => Err(quant(
            name.span,
            format!("scattered end '{name_text}' has no open block"),
        )),
        1 => Ok(BodyPlan::AcceptEnd {
            namespace: open[0],
            name: name_text,
        }),
        _ => Err(unsupported(
            name.span,
            "ambiguous scattered end across namespaces requires elaboration-order stack",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Node, NodeId, PExp, PExpKind, PatKind};
    use std::collections::BTreeMap;

    struct Fake {
        open: BTreeMap<(Namespace, String), bool>,
        counts: BTreeMap<(Namespace, String), usize>,
        closed: BTreeMap<String, Namespace>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                open: BTreeMap::new(),
                counts: BTreeMap::new(),
                closed: BTreeMap::new(),
            }
        }

        fn with_open(mut self, namespace: Namespace, name: &str) -> Self {
            self.open
                .insert((namespace, name.to_owned()), true);
            self
        }

        fn with_closed(mut self, namespace: Namespace, name: &str) -> Self {
            self.open
                .insert((namespace, name.to_owned()), false);
            self
        }

        fn with_count(mut self, namespace: Namespace, name: &str, count: usize) -> Self {
            self.counts
                .insert((namespace, name.to_owned()), count);
            self
        }

        fn with_closed_at(mut self, name: &str, namespace: Namespace) -> Self {
            self.closed.insert(name.to_owned(), namespace);
            self
        }
    }

    impl ScatteredView for Fake {
        fn is_open(&self, namespace: Namespace, name: &str) -> bool {
            self.open
                .get(&(namespace, name.to_owned()))
                .copied()
                .unwrap_or(false)
        }

        fn clause_count(&self, namespace: Namespace, name: &str) -> usize {
            self.counts
                .get(&(namespace, name.to_owned()))
                .copied()
                .unwrap_or(0)
        }

        fn closed_at(&self, name: &str, _position: usize) -> Option<Namespace> {
            self.closed.get(name).copied()
        }
    }

    fn interner() -> SimpleInterner {
        SimpleInterner::new()
    }

    fn name(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn funcl(it: &mut SimpleInterner, fname: &str) -> sail_ast::Funcl {
        sail_ast::Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: name(it, fname),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body: Node::dummy(sail_ast::ExpKind::Undef),
                },
            },
        }
    }

    fn def(kind: ScatteredDef) -> Def {
        Node::dummy(DefKind::Scattered(kind))
    }

    #[test]
    fn prefixes_match_declare_scattered() {
        assert_eq!(Namespace::Function.prefix(), "function");
        assert_eq!(Namespace::Mapping.prefix(), "mapping");
        assert_eq!(Namespace::Variant.prefix(), "union");
        assert_eq!(Namespace::Enum.prefix(), "enum");
    }

    #[test]
    fn namespace_of_covers_all_variants() {
        let mut it = interner();
        let fname = name(&mut it, "f");
        assert_eq!(
            namespace_of(&ScatteredDef::Function {
                name: fname.clone(),
                annot: sail_ast::TAnnotOpt::None,
            }),
            Some(Namespace::Function)
        );
        assert_eq!(
            namespace_of(&ScatteredDef::Funcl(funcl(&mut it, "f"))),
            Some(Namespace::Function)
        );
        assert_eq!(
            namespace_of(&ScatteredDef::Mapping {
                name: fname.clone(),
                annot: sail_ast::TAnnotOpt::None,
            }),
            Some(Namespace::Mapping)
        );
        assert_eq!(
            namespace_of(&ScatteredDef::Variant {
                name: fname.clone(),
                quant: Vec::new(),
            }),
            Some(Namespace::Variant)
        );
        assert_eq!(
            namespace_of(&ScatteredDef::Enum(fname.clone())),
            Some(Namespace::Enum)
        );
        assert_eq!(namespace_of(&ScatteredDef::End(fname)), None);
    }

    #[test]
    fn open_accepts_header_and_rejects_reopen() {
        let mut it = interner();
        let open = def(ScatteredDef::Function {
            name: name(&mut it, "f"),
            annot: sail_ast::TAnnotOpt::None,
        });
        let it = it;
        assert_eq!(
            elaborate(&open, &it, &Fake::new(), 0),
            Ok(BodyPlan::AcceptHeader {
                namespace: Namespace::Function,
                name: "f".to_owned(),
            })
        );
        let err = elaborate(
            &open,
            &it,
            &Fake::new().with_open(Namespace::Function, "f"),
            1,
        )
        .unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::QuantError(_)),
            "unexpected: {err:?}"
        );
        // Same name in another namespace stays openable.
        assert!(elaborate(
            &open,
            &it,
            &Fake::new().with_open(Namespace::Mapping, "f"),
            1,
        )
        .is_ok());
    }

    #[test]
    fn funcl_checks_body_with_zero_based_index() {
        let mut it = interner();
        let clause = def(ScatteredDef::Funcl(funcl(&mut it, "f")));
        let it = it;
        // Body-time count includes the clause's own declare entry.
        let view = Fake::new()
            .with_open(Namespace::Function, "f")
            .with_count(Namespace::Function, "f", 3);
        assert_eq!(
            elaborate(&clause, &it, &view, 7),
            Ok(BodyPlan::CheckFuncl {
                name: "f".to_owned(),
                index: 2,
            })
        );
    }

    #[test]
    fn orphan_clauses_are_rejected_in_every_namespace() {
        let mut it = interner();
        let funcl = def(ScatteredDef::Funcl(funcl(&mut it, "ghost")));
        let mapcl = def(ScatteredDef::Mapcl {
            name: name(&mut it, "ghost"),
            clause: sail_ast::Mapcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: sail_ast::MapclKind::Forwards(PExp {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    kind: PExpKind::PatExp {
                        pat: Node::dummy(PatKind::Wild),
                        body: Node::dummy(sail_ast::ExpKind::Undef),
                    },
                }),
            },
        });
        let unioncl = def(ScatteredDef::Unioncl {
            name: name(&mut it, "ghost"),
            union: sail_ast::TypeUnion {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                typ: Node::dummy(sail_ast::TypKind::Unknown),
                ctor: name(&mut it, "C"),
            },
        });
        let enumcl = def(ScatteredDef::Enumcl {
            enum_name: name(&mut it, "ghost"),
            member: name(&mut it, "A"),
        });
        let it = it;
        let view = Fake::new();
        for clause in [&funcl, &mapcl, &unioncl, &enumcl] {
            let err = elaborate(clause, &it, &view, 0).unwrap_err();
            assert!(
                matches!(err.kind, TypeErrorKind::QuantError(_)),
                "unexpected: {err:?}"
            );
        }
    }

    #[test]
    fn clause_kinds_carry_distinct_body_plans() {
        let mut it = interner();
        let mapcl = def(ScatteredDef::Mapcl {
            name: name(&mut it, "m"),
            clause: sail_ast::Mapcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: sail_ast::MapclKind::Forwards(PExp {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    kind: PExpKind::PatExp {
                        pat: Node::dummy(PatKind::Wild),
                        body: Node::dummy(sail_ast::ExpKind::Undef),
                    },
                }),
            },
        });
        let unioncl = def(ScatteredDef::Unioncl {
            name: name(&mut it, "u"),
            union: sail_ast::TypeUnion {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                typ: Node::dummy(sail_ast::TypKind::Unknown),
                ctor: name(&mut it, "C"),
            },
        });
        let enumcl = def(ScatteredDef::Enumcl {
            enum_name: name(&mut it, "e"),
            member: name(&mut it, "A"),
        });
        let it = it;
        let view = Fake::new()
            .with_open(Namespace::Mapping, "m")
            .with_count(Namespace::Mapping, "m", 1)
            .with_open(Namespace::Variant, "u")
            .with_count(Namespace::Variant, "u", 2)
            .with_open(Namespace::Enum, "e")
            .with_count(Namespace::Enum, "e", 1);
        assert_eq!(
            elaborate(&mapcl, &it, &view, 0),
            Ok(BodyPlan::CheckMapcl {
                name: "m".to_owned(),
                index: 0,
            })
        );
        assert_eq!(
            elaborate(&unioncl, &it, &view, 0),
            Ok(BodyPlan::CheckUnioncl {
                name: "u".to_owned(),
                index: 1,
            })
        );
        assert_eq!(
            elaborate(&enumcl, &it, &view, 0),
            Ok(BodyPlan::AcceptEnumcl {
                name: "e".to_owned(),
                index: 0,
            })
        );
    }

    #[test]
    fn end_resolves_recorded_close_and_rejects_orphans() {
        let mut it = interner();
        let end = def(ScatteredDef::End(name(&mut it, "f")));
        let it = it;
        // Body-time view: the end's own declare entry closed Function/f.
        let view = Fake::new()
            .with_closed(Namespace::Function, "f")
            .with_closed_at("f", Namespace::Function)
            .with_open(Namespace::Mapping, "f");
        assert_eq!(
            elaborate(&end, &it, &view, 3),
            Ok(BodyPlan::AcceptEnd {
                namespace: Namespace::Function,
                name: "f".to_owned(),
            })
        );
        // No open block anywhere: same message as `declare_scattered`.
        let err = elaborate(&end, &it, &Fake::new(), 0).unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::QuantError(_)),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn end_with_single_open_block_resolves_without_recorded_close() {
        let mut it = interner();
        let end = def(ScatteredDef::End(name(&mut it, "f")));
        let it = it;
        // Declare-time view shape (no close recorded yet): unambiguous.
        let view = Fake::new().with_open(Namespace::Enum, "f");
        assert_eq!(
            elaborate(&end, &it, &view, 0),
            Ok(BodyPlan::AcceptEnd {
                namespace: Namespace::Enum,
                name: "f".to_owned(),
            })
        );
    }

    #[test]
    fn ambiguous_end_is_unsupported() {
        let mut it = interner();
        let end = def(ScatteredDef::End(name(&mut it, "f")));
        let it = it;
        // Two namespaces open, no recorded close: same `Unsupported` as
        // `declare_scattered`'s ambiguous end.
        let view = Fake::new()
            .with_open(Namespace::Function, "f")
            .with_open(Namespace::Mapping, "f");
        let err = elaborate(&end, &it, &view, 0).unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::Unsupported(_)),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn non_scattered_def_is_unsupported() {
        let it = interner();
        let def = Node::dummy(DefKind::Pragma {
            key: String::new(),
            payload: String::new(),
        });
        let err = elaborate(&def, &it, &Fake::new(), 0).unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::Unsupported(_)),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn unresolvable_clause_name_is_unbound() {
        let it = interner();
        let clause = def(ScatteredDef::Funcl(sail_ast::Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: Name::dummy(Symbol::DUMMY),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body: Node::dummy(sail_ast::ExpKind::Undef),
                },
            },
        }));
        let err = elaborate(&clause, &it, &Fake::new(), 0).unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::UnboundVar(_)),
            "unexpected: {err:?}"
        );
    }
}
