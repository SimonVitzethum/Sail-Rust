//! Scope resolution: `Ast` -> [`ResolvedProgram`].
//!
//! Two passes over the top-level definitions (Sail top level is
//! order-independent, so declaration-before-use is *not* required):
//!
//! 1. **Bind**: insert every name of every [`DefKind`] variant into its
//!    namespace table, collecting `distincto` (duplicate) errors. Clause
//!    patterns (`Funcl`/`Mapcl` arms, top-level `let`) bind their pattern
//!    variables in a fresh lexical scope (see [`pat_names`]).
//! 2. **Check**: record ctor-vs-type collisions as non-fatal warnings,
//!    verify `overload` targets and `impl`/scattered-clause/`end` linkage,
//!    and compute [`crate::compute_ctor_renames`].
//!
//! Bridge contract (Lane 63, `sail-parse/src/bridge.rs`): `Val` carries
//! `TypKind::Unknown`, `Fundef` carries one skeleton `Funcl` per node with
//! the real `rec` flag, `Mapdef` carries empty clause lists, scattered and
//! `#42`-meta (`Impl`, `Fixity`, `Overload`, `Default`, `Pragma`) carry
//! real names. All of these resolve fully here; expression/type interiors
//! stay `Unknown`/`Undef` placeholders until Lane 02 (see
//! `RESOLVE_GAPS.md`).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{
    Ast, Def, DefKind, Funcl, Interner, Mapcl, MapclKind, MPExpKind, Name, Pat, PatKind, PExp, PExpKind,
    RecOpt, ScatteredDef, Span, Symbol, TypeDefKind,
};

use crate::env::{
    CtorEntry, Env, FixityEntry, FunctionEntry, ImplEntry, InstantiationEntry, MappingEntry,
    MeasureEntry, OutcomeEntry, OverloadEntry, PragmaEntry, RegisterEntry, ResolveError,
    ResolveErrorKind, ScatteredEntry, ScatteredNamespace, TypeEntry, ValSpecEntry, ValueEntry,
};
use crate::renames::compute_ctor_renames;

/// Resolution output: success flag, symbol tables, diagnostics, renames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedProgram {
    /// Symbol tables (usable even when `errors` is non-empty).
    pub env: Env,
    /// Fatal `distincto`/linkage errors. Empty iff resolution succeeded.
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
    /// `overload` name + (target, span) pairs; targets resolve in pass 2.
    pending_overloads: Vec<(String, Vec<(String, Span)>, Span)>,
    /// `impl` clause targets; linkage resolves in pass 2.
    pending_impls: Vec<(String, Span)>,
    /// Scattered clause targets (namespace, name, span); pass 2.
    pending_clauses: Vec<(ScatteredNamespace, String, Span)>,
    /// `scattered end` names; pass 2.
    pending_ends: Vec<(String, Span)>,
}

impl Resolver {
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: Env::new(),
            errors: Vec::new(),
            seen_ctors: Vec::new(),
            pending_overloads: Vec::new(),
            pending_impls: Vec::new(),
            pending_clauses: Vec::new(),
            pending_ends: Vec::new(),
        }
    }

    /// Resolve a whole program (both passes).
    #[must_use]
    pub fn resolve_program(mut self, ast: &Ast, it: &impl Interner) -> ResolvedProgram {
        for def in &ast.defs {
            self.add_definition(def, it);
        }
        self.finish()
    }

    /// Bind a definition and retain only exported names/linkage summaries.
    /// The definition and its worker-local interner can be dropped afterwards;
    /// symbol IDs from different shards never enter the global environment.
    pub fn add_definition(&mut self, def: &Def, it: &impl Interner) {
        self.bind_def(def, it);
    }

    /// Resolve cross-definition linkage after all declarations were streamed.
    /// This shares the same checks as `resolve_program`.
    pub fn finish(mut self) -> ResolvedProgram {
        self.check_pass();
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
        self.check_pass();
        self.env
    }

    fn push_err(&mut self, e: ResolveError) {
        self.errors.push(e);
    }

    fn name_text(it: &impl Interner, sym: Symbol) -> Option<String> {
        it.resolve(sym).map(str::to_owned)
    }

    fn name_of(it: &impl Interner, name: &Name) -> Option<String> {
        Self::name_text(it, name.symbol)
    }

    // -- pass 1: bind ----------------------------------------------------
    #[allow(clippy::too_many_lines)]
    fn bind_def(&mut self, def: &Def, it: &impl Interner) {
        let span = def.span;
        match &def.kind {
            DefKind::Type(t) => self.bind_typedef(t, it),
            DefKind::Val(v) => {
                let Some(name) = Self::name_of(it, &v.name) else {
                    return;
                };
                if let Err(e) = self.env.insert_valspec(ValSpecEntry {
                    name,
                    span: v.span,
                    has_extern: v.extern_opt.is_some(),
                }) {
                    self.push_err(e);
                }
            }
            DefKind::Register(r) => {
                let Some(name) = Self::name_of(it, &r.name) else {
                    return;
                };
                if let Err(e) = self.env.insert_register(RegisterEntry { name, span: r.span }) {
                    self.push_err(e);
                }
            }
            DefKind::Mapdef { name, clauses, .. } => {
                let Some(map_name) = Self::name_of(it, name) else {
                    return;
                };
                if let Err(e) = self.env.insert_mapping(MappingEntry {
                    name: map_name,
                    span,
                }) {
                    self.push_err(e);
                }
                for clause in clauses {
                    self.bind_mapcl(clause, it);
                }
            }
            DefKind::Fundef { rec, funcls, .. } => {
                self.bind_funcls(None, rec_is_rec(rec), funcls, span, it);
            }
            DefKind::Impl(funcl) => {
                let Some(name) = Self::name_of(it, &funcl.name) else {
                    return;
                };
                self.pending_impls.push((name.clone(), funcl.span));
                self.env.impls.push(ImplEntry { name, span: funcl.span });
                self.bind_funcl_arm(&funcl.arm, it);
            }
            DefKind::Let { pat, .. } => {
                // Top-level `let` binds pattern variables globally.
                for (name, ps) in pat_names(pat, it) {
                    if let Err(e) = self.env.insert_value(ValueEntry {
                        name: name.clone(),
                        span: ps,
                    }) {
                        self.push_err(e);
                    } else {
                        // Mirror into the global scope; a duplicate here
                        // cannot happen past the table check above.
                        let _ = self.env.bind_local(&name, ps);
                    }
                }
            }
            DefKind::Constraint(c) => {
                let _ = c;
                self.env.constraints.push(span);
            }
            DefKind::Outcome { name, .. } => {
                let Some(name) = Self::name_of(it, name) else {
                    return;
                };
                if let Err(e) = self.env.insert_outcome(OutcomeEntry { name, span }) {
                    self.push_err(e);
                }
            }
            DefKind::Instantiation { name, .. } => {
                let Some(name) = Self::name_of(it, name) else {
                    return;
                };
                if let Err(e) = self.env.insert_instantiation(InstantiationEntry { name, span })
                {
                    self.push_err(e);
                }
            }
            DefKind::Fixity { fixity, level, id } => {
                let Some(name) = Self::name_of(it, id) else {
                    return;
                };
                if let Err(e) = self.env.insert_fixity(FixityEntry {
                    name,
                    fixity: *fixity,
                    level: level.clone(),
                    span,
                }) {
                    self.push_err(e);
                }
            }
            DefKind::Overload { name, overloads } => {
                let Some(over_name) = Self::name_of(it, name) else {
                    return;
                };
                let mut targets = Vec::with_capacity(overloads.len());
                let mut target_names = Vec::with_capacity(overloads.len());
                for o in overloads {
                    if let Some(t) = Self::name_of(it, o) {
                        targets.push((t.clone(), o.span));
                        target_names.push(t);
                    }
                }
                self.env.insert_overload(OverloadEntry {
                    name: over_name.clone(),
                    targets: target_names,
                    span,
                });
                self.pending_overloads.push((over_name, targets, span));
            }
            DefKind::Default(order) => {
                if let Some((_, first)) = self.env.default {
                    self.push_err(ResolveError {
                        kind: ResolveErrorKind::DuplicateDefault,
                        name: "default".to_owned(),
                        first,
                        second: span,
                    });
                } else {
                    self.env.default = Some((*order, span));
                }
            }
            DefKind::Scattered(s) => self.bind_scattered(s, span, it),
            DefKind::Measure { name, pat, .. } => {
                let Some(name) = Self::name_of(it, name) else {
                    return;
                };
                if let Err(e) = self.env.insert_measure(MeasureEntry { name, span }) {
                    self.push_err(e);
                }
                // The measure pattern binds like a clause arm (fresh scope).
                self.env.enter_scope();
                self.bind_pat_list(&pat_names(pat, it));
                self.env.exit_scope();
            }
            DefKind::LoopMeasures { name, .. } => {
                let Some(name) = Self::name_of(it, name) else {
                    return;
                };
                if let Err(e) = self.env.insert_measure(MeasureEntry { name, span }) {
                    self.push_err(e);
                }
            }
            DefKind::InternalMutrec(groups) => {
                self.env.mutrec_groups += u32::try_from(groups.len()).unwrap_or(u32::MAX);
                for g in groups {
                    self.bind_funcls(None, rec_is_rec(&g.rec), &g.funcls, span, it);
                }
            }
            DefKind::Pragma { key, payload } => {
                self.env.pragmas.push(PragmaEntry {
                    key: key.clone(),
                    payload: payload.clone(),
                    span,
                });
            }
        }
    }

    fn bind_typedef(&mut self, t: &sail_ast::TypeDef, it: &impl Interner) {
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

    /// Bind a `Fundef`/`InternalMutrec` clause group. Clauses sharing one
    /// `DefKind::Fundef` usually name one function, but `and`-chains
    /// (OCaml) may group several: bind per distinct name, count clauses
    /// per name, and scope each arm separately.
    fn bind_funcls(
        &mut self,
        _annot_owner: Option<&str>,
        is_rec: bool,
        funcls: &[Funcl],
        def_span: Span,
        it: &impl Interner,
    ) {
        // (name, first_span, clause_count, order)
        let mut grouped: Vec<(String, Span, u32)> = Vec::new();
        for f in funcls {
            let Some(name) = Self::name_of(it, &f.name) else {
                continue;
            };
            if let Some(g) = grouped.iter_mut().find(|(n, _, _)| *n == name) {
                g.2 += 1;
            } else {
                grouped.push((name, f.span, 1));
            }
        }
        for (name, first_span, clauses) in grouped {
            let span = if first_span == Span::DUMMY { def_span } else { first_span };
            if let Err(e) = self.env.insert_function(FunctionEntry {
                name,
                span,
                is_rec,
                clauses,
            }) {
                self.push_err(e);
            }
        }
        for f in funcls {
            self.bind_funcl_arm(&f.arm, it);
        }
    }

    /// Bind one function-clause arm pattern in a fresh lexical scope.
    fn bind_funcl_arm(&mut self, arm: &PExp, it: &impl Interner) {
        self.env.enter_scope();
        match &arm.kind {
            PExpKind::PatExp { pat, .. } => self.bind_pat_list(&pat_names(pat, it)),
            PExpKind::PatWhen { pat, .. } => self.bind_pat_list(&pat_names(pat, it)),
        }
        self.env.exit_scope();
    }

    fn bind_mapcl(&mut self, clause: &Mapcl, it: &impl Interner) {
        match &clause.kind {
            MapclKind::Bidir { forwards, backwards } => {
                for side in [forwards, backwards] {
                    let (MPExpKind::Pat(pat) | MPExpKind::When { pat, .. }) = &side.kind;
                    self.env.enter_scope();
                    self.bind_pat_list(&pat_names(pat, it));
                    self.env.exit_scope();
                }
            }
            MapclKind::Forwards(arm) | MapclKind::Backwards(arm) =>
                self.bind_funcl_arm(arm, it),
        }
    }

    fn bind_pat_list(&mut self, names: &[(String, Span)]) {
        for (name, span) in names {
            if let Err(first) = self.env.bind_local(name, *span) {
                self.push_err(ResolveError {
                    kind: ResolveErrorKind::DuplicateValue,
                    name: name.clone(),
                    first,
                    second: *span,
                });
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn bind_scattered(&mut self, s: &ScatteredDef, span: Span, it: &impl Interner) {
        match s {
            ScatteredDef::Function { name, .. } => {
                self.bind_scattered_open(ScatteredNamespace::Function, name, span, it);
            }
            ScatteredDef::Mapping { name, .. } => {
                self.bind_scattered_open(ScatteredNamespace::Mapping, name, span, it);
            }
            ScatteredDef::Variant { name, .. } => {
                self.bind_scattered_open(ScatteredNamespace::Variant, name, span, it);
            }
            ScatteredDef::Enum(name) => {
                self.bind_scattered_open(ScatteredNamespace::Enum, name, span, it);
            }
            ScatteredDef::Funcl(f) => {
                let Some(name) = Self::name_of(it, &f.name) else {
                    return;
                };
                self.pending_clauses
                    .push((ScatteredNamespace::Function, name, f.span));
                self.bind_funcl_arm(&f.arm, it);
            }
            ScatteredDef::Mapcl { name, clause } => {
                let Some(name) = Self::name_of(it, name) else {
                    return;
                };
                self.pending_clauses
                    .push((ScatteredNamespace::Mapping, name, clause.span));
                self.bind_mapcl(clause, it);
            }
            ScatteredDef::Unioncl { name, union } => {
                let Some(owner) = Self::name_of(it, name) else {
                    return;
                };
                if let Some(ctor) = Self::name_of(it, &union.ctor) {
                    self.seen_ctors.push((owner.clone(), ctor.clone()));
                    if let Err(e) = self.env.insert_ctor(CtorEntry {
                        name: ctor,
                        owner: owner.clone(),
                        span: union.span,
                    }) {
                        self.push_err(e);
                    }
                }
                self.pending_clauses
                    .push((ScatteredNamespace::Variant, owner, union.span));
            }
            ScatteredDef::Enumcl { enum_name, member } => {
                let Some(owner) = Self::name_of(it, enum_name) else {
                    return;
                };
                if let Some(ctor) = Self::name_of(it, member) {
                    self.seen_ctors.push((owner.clone(), ctor.clone()));
                    if let Err(e) = self.env.insert_ctor(CtorEntry {
                        name: ctor,
                        owner: owner.clone(),
                        span: member.span,
                    }) {
                        self.push_err(e);
                    }
                }
                self.pending_clauses
                    .push((ScatteredNamespace::Enum, owner, member.span));
            }
            ScatteredDef::End(name) => {
                let Some(end_name) = Self::name_of(it, name) else {
                    return;
                };
                self.pending_ends.push((end_name, name.span));
            }
        }
    }

    fn bind_scattered_open(
        &mut self,
        namespace: ScatteredNamespace,
        name: &Name,
        _span: Span,
        it: &impl Interner,
    ) {
        let Some(text) = Self::name_of(it, name) else {
            return;
        };
        if let Err(e) = self.env.insert_scattered(
            namespace,
            ScatteredEntry {
                name: text,
                namespace,
                span: name.span,
                closed: false,
            },
        ) {
            self.push_err(e);
        }
    }

    // -- pass 2: check ---------------------------------------------------
    fn check_pass(&mut self) {
        // Overload targets must name a `val` or a function (order-free:
        // the whole program is bound by now).
        for (over_name, targets, over_span) in std::mem::take(&mut self.pending_overloads) {
            for (target, target_span) in targets {
                if self.env.lookup_valspec(&target).is_none()
                    && self.env.lookup_function(&target).is_none()
                    && self.env.lookup_overload(&target).is_none()
                    && self.env.lookup_ctor(&target).is_none()
                {
                    self.push_err(ResolveError {
                        kind: ResolveErrorKind::UnknownOverloadTarget,
                        name: target,
                        first: over_span,
                        second: target_span,
                    });
                }
            }
            let _ = over_name;
        }
        // `impl` clauses attach to an open scattered function.
        for (name, span) in std::mem::take(&mut self.pending_impls) {
            if let Some(open) = self.env.scattered_functions.get(&name) {
                let _ = open;
            } else {
                let fallback = self.env.lookup_function(&name).map(|f| f.span);
                self.push_err(ResolveError {
                    kind: ResolveErrorKind::DanglingImpl,
                    name,
                    first: fallback.unwrap_or(span),
                    second: span,
                });
            }
        }
        // Scattered clauses attach to an open head in their namespace.
        for (namespace, name, span) in std::mem::take(&mut self.pending_clauses) {
            if self.env.lookup_scattered(namespace, &name).is_none() {
                self.push_err(ResolveError {
                    kind: ResolveErrorKind::DanglingScatteredClause,
                    name,
                    first: span,
                    second: span,
                });
            }
        }
        // `end` closes an open head in any scattered namespace.
        for (name, span) in std::mem::take(&mut self.pending_ends) {
            let mut found = false;
            for table in [
                &mut self.env.scattered_functions,
                &mut self.env.scattered_mappings,
                &mut self.env.scattered_variants,
                &mut self.env.scattered_enums,
            ] {
                if let Some(entry) = table.get_mut(&name) {
                    entry.closed = true;
                    found = true;
                }
            }
            if !found {
                self.push_err(ResolveError {
                    kind: ResolveErrorKind::DanglingScatteredEnd,
                    name,
                    first: span,
                    second: span,
                });
            }
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

fn rec_is_rec(rec: &RecOpt) -> bool {
    !matches!(rec, RecOpt::NonRec)
}

/// Pattern-bound value names in source order: `(name, span)`.
///
/// Only value-namespace binders are collected (`PatKind::Id`, `As`, the
/// binder of `VectorSubrange`, and everything nested inside composite
/// patterns). Struct/enum *references* (`StructName::Id`, `App.func`)
/// and type-level binders (`TypPat::Var`) are not value bindings and are
/// skipped. Expression interiors (`ExpKind::Let/Var/For`, guards, bodies)
/// are *not* walked: bodies are `Undef` placeholders until the Lane-02
/// grammar lands (see `RESOLVE_GAPS.md`).
pub fn pat_names(pat: &Pat, it: &impl Interner) -> Vec<(String, Span)> {
    fn walk(pat: &Pat, it: &impl Interner, out: &mut Vec<(String, Span)>) {
        match &pat.kind {
            PatKind::Id(name) => {
                if let Some(text) = it.resolve(name.symbol).map(str::to_owned) {
                    out.push((text, name.span));
                }
            }
            PatKind::As { pat, id } => {
                if let Some(text) = it.resolve(id.symbol).map(str::to_owned) {
                    out.push((text, id.span));
                }
                walk(pat, it, out);
            }
            PatKind::VectorSubrange { id, .. } => {
                if let Some(text) = it.resolve(id.symbol).map(str::to_owned) {
                    out.push((text, id.span));
                }
            }
            PatKind::Or(a, b) | PatKind::Cons(a, b) => {
                walk(a, it, out);
                walk(b, it, out);
            }
            PatKind::Not(inner) | PatKind::Typ { pat: inner, .. } | PatKind::Var { pat: inner, .. } => {
                walk(inner, it, out);
            }
            PatKind::App { args, .. }
            | PatKind::Vector(args)
            | PatKind::VectorConcat(args)
            | PatKind::Tuple(args)
            | PatKind::List(args)
            | PatKind::StringAppend(args) => {
                for a in args {
                    walk(a, it, out);
                }
            }
            PatKind::Struct { fields, .. } => {
                for (_, fp) in fields {
                    walk(fp, it, out);
                }
            }
            PatKind::Lit(_) | PatKind::Wild => {}
        }
    }
    let mut out = Vec::new();
    walk(pat, it, &mut out);
    out
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
