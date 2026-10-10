//! Scope resolution: `Ast` -> [`ResolvedProgram`].
//!
//! Two passes over the top-level definitions (Sail top level is
//! order-independent, so declaration-before-use is *not* required):
//!
//! 1. **Bind**: insert every type/ctor/valspec/register/mapping name,
//!    collecting `distincto` (duplicate) errors.
//! 2. **Check**: record ctor-vs-type collisions as non-fatal warnings
//!    and compute [`crate::compute_ctor_renames`].

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{Ast, Def, DefKind, Interner, Symbol, TypeDefKind};

use crate::env::{
    CtorEntry, Env, MappingEntry, RegisterEntry, ResolveError, ResolveErrorKind, TypeEntry,
    ValSpecEntry,
};
use crate::renames::compute_ctor_renames;

/// Resolution output: success flag, symbol tables, diagnostics, renames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedProgram {
    /// Symbol tables (usable even when `errors` is non-empty).
    pub env: Env,
    /// Fatal `distincto` errors. Empty iff resolution succeeded.
    pub errors: Vec<ResolveError>,
    /// Non-fatal ctor-vs-type collision notes.
    pub warnings: Vec<ResolveError>,
    /// Pure rename map for colliding union ctors (possibly empty).
    pub ctor_renames: BTreeMap<String, String>,
}

impl ResolvedProgram {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Stateless resolver; construct per program (cheap, low-RAM).
#[derive(Clone, Debug, Default)]
pub struct Resolver {
    env: Env,
    errors: Vec<ResolveError>,
    /// Every ctor binding *attempt* (owner, name), including duplicates
    /// rejected by the env. Drives rename computation so multi-owner
    /// collisions are detected even though the table keeps the winner.
    seen_ctors: Vec<(String, String)>,
}

impl Resolver {
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: Env::new(),
            errors: Vec::new(),
            seen_ctors: Vec::new(),
        }
    }

    /// Resolve a whole program (both passes).
    #[must_use]
    pub fn resolve_program(mut self, ast: &Ast, it: &impl Interner) -> ResolvedProgram {
        for def in &ast.defs {
            self.bind_def(def, it);
        }
        let warnings = self.collision_warnings();
        let ctor_renames = self.renames();
        let Self { env, errors, .. } = self;
        ResolvedProgram {
            env,
            errors,
            warnings,
            ctor_renames,
        }
    }

    /// Resolve and return only the environment, dropping diagnostics.
    /// Prefer [`Self::resolve_program`] when diagnostics matter.
    #[must_use]
    pub fn resolve_env(mut self, ast: &Ast, it: &impl Interner) -> Env {
        for def in &ast.defs {
            self.bind_def(def, it);
        }
        self.env
    }

    fn push_err(&mut self, e: ResolveError) {
        self.errors.push(e);
    }

    fn name_text(it: &impl Interner, sym: Symbol) -> Option<String> {
        it.resolve(sym).map(str::to_owned)
    }

    fn bind_def(&mut self, def: &Def, it: &impl Interner) {
        let span = def.span;
        match &def.kind {
            DefKind::Type(t) => {
                let Some(type_name) = Self::typedef_name(&t.kind, it) else {
                    return;
                };
                let is_union = matches!(t.kind, TypeDefKind::Variant { .. });
                if let Err(e) = self.env.insert_type(TypeEntry {
                    name: type_name.clone(),
                    span: t.span,
                    is_union,
                }) {
                    self.push_err(e);
                }
                for (ctor_name, ctor_span) in Self::typedef_ctors(&t.kind, it) {
                    self.seen_ctors.push((type_name.clone(), ctor_name.clone()));
                    if let Err(e) = self.env.insert_ctor(CtorEntry {
                        name: ctor_name,
                        owner: type_name.clone(),
                        span: ctor_span,
                    }) {
                        self.push_err(e);
                    }
                }
            }
            DefKind::Val(v) => {
                let Some(name) = Self::name_text(it, v.name.symbol) else {
                    return;
                };
                if let Err(e) = self.env.insert_valspec(ValSpecEntry { name, span: v.span }) {
                    self.push_err(e);
                }
            }
            DefKind::Register(r) => {
                let Some(name) = Self::name_text(it, r.name.symbol) else {
                    return;
                };
                if let Err(e) = self.env.insert_register(RegisterEntry { name, span: r.span }) {
                    self.push_err(e);
                }
            }
            DefKind::Mapdef { name, .. } => {
                let Some(name) = Self::name_text(it, name.symbol) else {
                    return;
                };
                if let Err(e) = self.env.insert_mapping(MappingEntry { name, span }) {
                    self.push_err(e);
                }
            }
            // Functions, scattered defs, pragmas, constraints, … carry no
            // resolution-table entry (same as the former `Def::Other`).
            _ => {}
        }
    }

    /// Owning type name of a typedef, if resolvable.
    fn typedef_name(kind: &TypeDefKind, it: &impl Interner) -> Option<String> {
        match kind {
            TypeDefKind::Abbrev { name, .. }
            | TypeDefKind::Record { name, .. }
            | TypeDefKind::Variant { name, .. }
            | TypeDefKind::Enum { name, .. }
            | TypeDefKind::Abstract { name, .. }
            | TypeDefKind::Bitfield { name, .. } => Self::name_text(it, name.symbol),
        }
    }

    /// `(ctor_name, span)` pairs carried by a typedef (empty for
    /// records/abbrevs/abstract/bitfield).
    fn typedef_ctors(kind: &TypeDefKind, it: &impl Interner) -> Vec<(String, sail_ast::Span)> {
        match kind {
            TypeDefKind::Enum { members, .. } => members
                .iter()
                .filter_map(|m| Some((Self::name_text(it, m.symbol)?, m.span)))
                .collect(),
            TypeDefKind::Variant { ctors, .. } => ctors
                .iter()
                .filter_map(|u| Some((Self::name_text(it, u.ctor.symbol)?, u.span)))
                .collect(),
            TypeDefKind::Abbrev { .. }
            | TypeDefKind::Record { .. }
            | TypeDefKind::Abstract { .. }
            | TypeDefKind::Bitfield { .. } => Vec::new(),
        }
    }

    /// One warning per ctor whose name is also a type name.
    fn collision_warnings(&self) -> Vec<ResolveError> {
        let mut out = Vec::new();
        let mut ctors: Vec<&CtorEntry> = self.env.ctors.values().collect();
        ctors.sort_by(|a, b| a.name.cmp(&b.name));
        for c in ctors {
            if let Some(t) = self.env.lookup_type(&c.name) {
                out.push(ResolveError {
                    kind: ResolveErrorKind::CtorTypeCollision,
                    name: c.name.clone(),
                    first: t.span,
                    second: c.span,
                });
            }
        }
        out
    }

    fn renames(&self) -> BTreeMap<String, String> {
        let type_names: BTreeSet<String> = self.env.types.keys().cloned().collect();
        let mut by_owner: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (owner, name) in &self.seen_ctors {
            by_owner
                .entry(owner.as_str())
                .or_default()
                .push(name.clone());
        }
        for v in by_owner.values_mut() {
            v.sort();
        }
        let unions: Vec<(String, Vec<String>)> = by_owner
            .into_iter()
            .map(|(o, v)| (o.to_owned(), v))
            .collect();
        compute_ctor_renames(&type_names, &unions)
    }
}

/// Convenience wrapper: [`Resolver::resolve_program`] in one call.
#[must_use]
pub fn resolve(ast: &Ast, it: &impl Interner) -> ResolvedProgram {
    Resolver::new().resolve_program(ast, it)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{
        DecSpec, Mapcl, Name, Node, NodeId, Span, TAnnotOpt, Typ, TypKind, TypSchm, TypeUnion,
        ValSpec,
    };

    fn interner() -> sail_ast::SimpleInterner {
        sail_ast::SimpleInterner::new()
    }

    fn nm(it: &mut sail_ast::SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn unknown_typ() -> Typ {
        Node::dummy(TypKind::Unknown)
    }

    fn struct_def(it: &mut sail_ast::SimpleInterner, name: &str) -> Def {
        Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Record {
            name: nm(it, name),
            quant: Vec::new(),
            fields: Vec::new(),
            wf: false,
        })))
    }

    fn union_def(it: &mut sail_ast::SimpleInterner, owner: &str, ctors: &[&str]) -> Def {
        Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Variant {
            name: nm(it, owner),
            quant: Vec::new(),
            ctors: ctors
                .iter()
                .map(|c| TypeUnion {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    typ: unknown_typ(),
                    ctor: nm(it, c),
                })
                .collect(),
            wf: false,
        })))
    }

    fn valspec_def(it: &mut sail_ast::SimpleInterner, name: &str) -> Def {
        Node::dummy(DefKind::Val(ValSpec {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, name),
            typschm: TypSchm { quant: Vec::new(), typ: unknown_typ() },
            extern_opt: None,
        }))
    }

    fn register_def(it: &mut sail_ast::SimpleInterner, name: &str) -> Def {
        Node::dummy(DefKind::Register(DecSpec {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            typ: unknown_typ(),
            name: nm(it, name),
            init: None,
        }))
    }

    fn mapping_def(it: &mut sail_ast::SimpleInterner, name: &str) -> Def {
        Node::dummy(DefKind::Mapdef {
            name: nm(it, name),
            annot: TAnnotOpt::None,
            clauses: Vec::<Mapcl>::new(),
        })
    }

    #[test]
    fn happy_path_tables_fill() {
        let mut it = interner();
        let ast = Ast::new(vec![
            struct_def(&mut it, "U"),
            union_def(&mut it, "V", &["A", "B"]),
            valspec_def(&mut it, "f"),
            register_def(&mut it, "x1"),
            mapping_def(&mut it, "enc"),
        ]);
        let r = resolve(&ast, &it);
        assert!(r.is_ok(), "errors: {:?}", r.errors);
        assert!(r.warnings.is_empty());
        assert!(r.ctor_renames.is_empty());
        assert!(r.env.lookup_type("U").is_some());
        assert_eq!(r.env.lookup_ctor("A").map(|c| c.owner.as_str()), Some("V"));
        assert!(r.env.lookup_valspec("f").is_some());
        assert!(r.env.lookup_register("x1").is_some());
        assert!(r.env.lookup_mapping("enc").is_some());
    }

    #[test]
    fn union_ctor_vs_typename_collision() {
        // `union V { U }` + `struct U`: ctor `U` collides with type `U`.
        let mut it = interner();
        let ast = Ast::new(vec![struct_def(&mut it, "U"), union_def(&mut it, "V", &["U", "W"])]);
        let r = resolve(&ast, &it);
        assert!(r.is_ok(), "collision is non-fatal: {:?}", r.errors);
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.warnings[0].kind, ResolveErrorKind::CtorTypeCollision);
        assert_eq!(r.ctor_renames.get("U"), Some(&"U_ctor".to_owned()));
        assert!(!r.ctor_renames.contains_key("W"));
    }
}
