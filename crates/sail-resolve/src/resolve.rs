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

use crate::env::{
    CtorEntry, Env, MappingEntry, RegisterEntry, ResolveError, ResolveErrorKind, TypeEntry,
    ValSpecEntry,
};
use crate::mini_ast::{Ast, Def};
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
    pub fn resolve_program(mut self, ast: &Ast) -> ResolvedProgram {
        for def in &ast.defs {
            self.bind_def(def);
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
    pub fn resolve_env(mut self, ast: &Ast) -> Env {
        for def in &ast.defs {
            self.bind_def(def);
        }
        self.env
    }

    fn push_err(&mut self, e: ResolveError) {
        self.errors.push(e);
    }

    fn bind_def(&mut self, def: &Def) {
        match def {
            Def::Type(t) => {
                if let Err(e) = self.env.insert_type(TypeEntry {
                    name: t.name.clone(),
                    span: t.span,
                    is_union: t.is_union(),
                }) {
                    self.push_err(e);
                }
                for c in t.ctors() {
                    self.seen_ctors.push((t.name.clone(), c.name.clone()));
                    if let Err(e) = self.env.insert_ctor(CtorEntry {
                        name: c.name.clone(),
                        owner: t.name.clone(),
                        span: c.span,
                    }) {
                        self.push_err(e);
                    }
                }
            }
            Def::ValSpec(v) => {
                if let Err(e) = self.env.insert_valspec(ValSpecEntry {
                    name: v.name.clone(),
                    span: v.span,
                }) {
                    self.push_err(e);
                }
            }
            Def::Register(r) => {
                if let Err(e) = self.env.insert_register(RegisterEntry {
                    name: r.name.clone(),
                    span: r.span,
                }) {
                    self.push_err(e);
                }
            }
            Def::Mapping(m) => {
                if let Err(e) = self.env.insert_mapping(MappingEntry {
                    name: m.name.clone(),
                    span: m.span,
                }) {
                    self.push_err(e);
                }
            }
            Def::Other { .. } => {
                // No table yet; real `sail-ast` nodes (functions,
                // overloads, scattered defs) will extend this arm.
            }
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
pub fn resolve(ast: &Ast) -> ResolvedProgram {
    Resolver::new().resolve_program(ast)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mini_ast::{CtorDecl, MappingDef, RegisterDef, Span, TypeDef, TypeDefKind, ValSpec};

    const S: Span = Span::new(1, 1);

    fn union_def(owner: &str, ctors: &[&str]) -> Def {
        Def::Type(TypeDef::new(
            owner,
            TypeDefKind::Union(
                ctors
                    .iter()
                    .map(|c| CtorDecl::new((*c).to_owned(), S))
                    .collect(),
            ),
            S,
        ))
    }

    #[test]
    fn happy_path_tables_fill() {
        let ast = Ast::new(vec![
            Def::Type(TypeDef::new("U", TypeDefKind::Struct, S)),
            union_def("V", &["A", "B"]),
            Def::ValSpec(ValSpec::new("f", S)),
            Def::Register(RegisterDef::new("x1", S)),
            Def::Mapping(MappingDef::new("enc", S)),
        ]);
        let r = resolve(&ast);
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
        let ast = Ast::new(vec![
            Def::Type(TypeDef::new("U", TypeDefKind::Struct, S)),
            union_def("V", &["U", "W"]),
        ]);
        let r = resolve(&ast);
        assert!(r.is_ok(), "collision is non-fatal: {:?}", r.errors);
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.warnings[0].kind, ResolveErrorKind::CtorTypeCollision);
        assert_eq!(r.ctor_renames.get("U"), Some(&"U_ctor".to_owned()));
        assert!(!r.ctor_renames.contains_key("W"));
    }
}
