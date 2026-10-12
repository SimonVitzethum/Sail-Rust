//! Toplevel let-pattern splitting, matching `rewrite_toplevel_let_patterns`
//! in `sail/src/lib/rewrites.ml`.
//!
//! A toplevel `let pat = exp` whose pattern is not "simple" becomes one
//! base definition plus one projection definition per bound identifier:
//! `let let__N = exp` followed by
//! `let id = let pat = let__N in id` for every `id` in `pat_ids(pat)`.
//! Simple patterns (`P_id`, one `P_typ` over `P_id`, and the `P_var`
//! forms whose binder matches its kind variable, including the `atom`
//! application) pass through unchanged, exactly as in OCaml.
//!
//! What works without a type environment:
//! - the simple-pattern test (purely syntactic; kind-variable identity is
//!   symbol equality, which is text equality on interned names);
//! - `pat_ids` (purely syntactic);
//! - the structural split with fresh, capture-avoiding base names.
//!
//! Documented gaps (need `Env` / elaboration, unavailable on the untyped
//! AST, so they are skipped rather than guessed):
//! - OCaml annotates the base and projection definitions with
//!   `mk_tannot env (typ_of exp)` / `Env.lookup_id id env`. The untyped
//!   AST carries no definition annotations, so dummy nodes are used.
//!   The one exception OCaml itself notes ("we may need the type
//!   information for (e.g.) records"): when the source pattern already
//!   carries a syntactic `P_typ` wrapper, that `Typ` is preserved on the
//!   base binder instead of dropped.
//!
//! Wiring note: this file is deliberately self-contained (only `sail_ast`)
//! so it compiles before the canonical-path helpers land. Once
//! `crate::passes::typed` exists, the local `Fresh`/`collect_used`/`mk_*`
//! helpers below can be replaced by its imports, and the inherent
//! [`ToplevelLetPatterns::run`] gains a trivial `TypedPass` delegate (see
//! `LANE59_INSERT_SPEC.md`).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use sail_ast::{Ast, Def, DefKind, Exp, ExpKind, Interner, Name, Pat, PatKind, Symbol, TypPat};

use sail_ast::TypPatKind;

#[derive(Clone, Copy, Debug, Default)]
pub struct ToplevelLetPatterns;

impl crate::pass::TypedPass for ToplevelLetPatterns {
    fn name(&self) -> &'static str { ToplevelLetPatterns::name(self) }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        ToplevelLetPatterns::run(self, ast, interner)
    }
}

impl ToplevelLetPatterns {
    /// Pass name, matching the OCaml rewrite name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "toplevel_let_patterns"
    }

    /// Run the pass on the whole canonical AST.
    pub fn run(&self, ast: &mut Ast, interner: &mut dyn Interner) {
        // OCaml maps `rewrite_def` over the toplevel definition list only;
        // nested definitions (e.g. inside `Outcome`) pass through untouched.
        let mut fresh = Fresh::new(collect_used(ast, &*interner));
        let mut defs = std::mem::take(&mut ast.defs);
        let mut out = Vec::with_capacity(defs.len());
        for def in defs.drain(..) {
            expand_def(def, &mut out, &mut fresh, interner);
        }
        ast.defs = out;
    }
}

/// Expand one toplevel definition, pushing one or more definitions to `out`.
fn expand_def(def: Def, out: &mut Vec<Def>, fresh: &mut Fresh, interner: &mut dyn Interner) {
    if !matches!(&def.kind, DefKind::Let { .. }) {
        out.push(def);
        return;
    }
    let DefKind::Let { pat, .. } = &def.kind else {
        unreachable!("matched as DefKind::Let above");
    };
    if is_pat_simple(pat, &*interner) {
        out.push(def);
        return;
    }
    let DefKind::Let { pat, exp } = def.kind else {
        unreachable!("matched as DefKind::Let above");
    };
    // Deterministic projection order: bound symbols sorted by name text
    // (OCaml uses `IdSet.elements`, i.e. its own internal id order; the
    // untyped AST has no such order, so text order keeps the pass
    // deterministic for a given interner state).
    let mut ids: Vec<Symbol> = {
        let mut set = BTreeSet::new();
        pat_ids(&pat, &mut set);
        set.into_iter().collect()
    };
    ids.sort_by(|a, b| {
        let ta = interner.resolve(*a).unwrap_or("");
        let tb = interner.resolve(*b).unwrap_or("");
        ta.cmp(tb).then_with(|| a.0.cmp(&b.0))
    });
    let base = mk_name(interner.intern(&fresh.fresh("let__")));
    // OCaml keeps the outer `P_typ` on the base binder when present
    // (`add_pat_typ`); every other shape becomes a plain `P_id`.
    let base_pat = match &pat.kind {
        PatKind::Typ { typ, .. } => mk_pat(PatKind::Typ {
            typ: typ.clone(),
            pat: Box::new(mk_pat(PatKind::Id(base.clone()))),
        }),
        _ => mk_pat(PatKind::Id(base.clone())),
    };
    out.push(Def::dummy(DefKind::Let { pat: base_pat, exp }));
    for sym in ids {
        let id = mk_name(sym);
        let body = mk_exp(ExpKind::Let {
            pat: pat.clone(),
            bind: Box::new(mk_exp(ExpKind::Id(base.clone()))),
            body: Box::new(mk_exp(ExpKind::Id(id.clone()))),
        });
        out.push(Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Id(id)),
            exp: body,
        }));
    }
}

/// Whether `pat` is "simple" in OCaml's sense (`is_pat_simple` in
/// `rewrite_toplevel_let_patterns`): a bare identifier, one `P_typ`
/// wrapper over a bare identifier, or a `P_var` (optionally under one
/// `P_typ`) whose identifier matches its kind variable, including the
/// `atom('n)` application form. Anything else forces the split.
fn is_pat_simple(pat: &Pat, interner: &dyn Interner) -> bool {
    match &pat.kind {
        PatKind::Id(_) => true,
        PatKind::Typ { pat: inner, .. } => match &inner.kind {
            PatKind::Id(_) => true,
            PatKind::Var { pat, typ_pat } => is_simple_var(pat, typ_pat, interner),
            _ => false,
        },
        PatKind::Var { pat, typ_pat } => is_simple_var(pat, typ_pat, interner),
        _ => false,
    }
}

/// `P_var (P_id id, TP_var kid)` with `id == kid`, or the same with
/// `TP_app (atom, [TP_var kid])`.
fn is_simple_var(pat: &Pat, typ_pat: &TypPat, interner: &dyn Interner) -> bool {
    let PatKind::Id(id) = &pat.kind else {
        return false;
    };
    match &typ_pat.kind {
        TypPatKind::Var(kid) => *kid == id.symbol,
        TypPatKind::App { name, args } => {
            if interner.resolve(name.symbol) != Some("atom") {
                return false;
            }
            if args.len() != 1 {
                return false;
            }
            matches!(&args[0].kind, TypPatKind::Var(kid) if *kid == id.symbol)
        }
        TypPatKind::Wild => false,
    }
}

/// All identifiers bound by `pat`, mirroring `pat_ids` in
/// `sail/src/lib/ast_util.ml`.
fn pat_ids(pat: &Pat, acc: &mut BTreeSet<Symbol>) {
    match &pat.kind {
        PatKind::Lit(_) | PatKind::Wild => {}
        PatKind::Id(name) | PatKind::VectorSubrange { id: name, .. } => {
            acc.insert(name.symbol);
        }
        PatKind::As { pat, id } => {
            acc.insert(id.symbol);
            pat_ids(pat, acc);
        }
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            pat_ids(a, acc);
            pat_ids(b, acc);
        }
        PatKind::Not(p) | PatKind::Var { pat: p, .. } | PatKind::Typ { pat: p, .. } => {
            pat_ids(p, acc);
        }
        PatKind::App { args, .. }
        | PatKind::Tuple(args)
        | PatKind::Vector(args)
        | PatKind::VectorConcat(args)
        | PatKind::List(args)
        | PatKind::StringAppend(args) => {
            for q in args {
                pat_ids(q, acc);
            }
        }
        PatKind::Struct { fields, .. } => {
            for (_, q) in fields {
                pat_ids(q, acc);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Local synthesis helpers (mirror `crate::passes::typed` until it lands; see
// the wiring note above).
// ---------------------------------------------------------------------------

/// Make a synthesized expression (dummy id/span, as OCaml desugared output).
fn mk_exp(kind: ExpKind) -> Exp {
    sail_ast::Node::dummy(kind)
}

/// Make a synthesized pattern.
fn mk_pat(kind: PatKind) -> Pat {
    sail_ast::Node::dummy(kind)
}

/// Make a synthesized identifier.
fn mk_name(symbol: Symbol) -> Name {
    Name::dummy(symbol)
}

/// Per-run fresh-name generator (`let__0`, `let__1`, ...), seeded with
/// [`collect_used`]; every result is inserted into the used set, so
/// repeated calls never collide with each other or with program names.
#[derive(Clone, Debug)]
struct Fresh {
    used: BTreeSet<String>,
    next: usize,
}

impl Fresh {
    fn new(used: BTreeSet<String>) -> Self {
        Self { used, next: 0 }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        loop {
            let cand = format!("{prefix}{}", self.next);
            self.next += 1;
            if self.used.insert(cand.clone()) {
                return cand;
            }
        }
    }
}

/// Every value-level name text in `ast`, so generated binders never capture
/// user names. Covers all definition, expression, lvalue, and pattern
/// forms of the untyped AST.
fn collect_used(ast: &Ast, interner: &dyn Interner) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for def in &ast.defs {
        collect_def_names(def, interner, &mut used);
    }
    used
}

fn push_name(name: &Name, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    if let Some(text) = interner.resolve(name.symbol) {
        used.insert(text.to_owned());
    }
}

fn collect_def_names(def: &Def, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            for f in funcls {
                push_name(&f.name, interner, used);
                collect_pexp_names(&f.arm, interner, used);
            }
        }
        DefKind::Mapdef { name, clauses, .. } => {
            push_name(name, interner, used);
            for c in clauses {
                match &c.kind {
                    sail_ast::MapclKind::Bidir {
                        forwards,
                        backwards,
                    } => {
                        collect_mpexp_names(forwards, interner, used);
                        collect_mpexp_names(backwards, interner, used);
                    }
                    sail_ast::MapclKind::Forwards(p) | sail_ast::MapclKind::Backwards(p) => {
                        collect_pexp_names(p, interner, used);
                    }
                }
            }
        }
        DefKind::Impl(f) => {
            push_name(&f.name, interner, used);
            collect_pexp_names(&f.arm, interner, used);
        }
        DefKind::Let { pat, exp } => {
            collect_pat_names(pat, interner, used);
            collect_exp_names(exp, interner, used);
        }
        DefKind::Outcome { defs, name, .. } => {
            push_name(name, interner, used);
            for d in defs {
                collect_def_names(d, interner, used);
            }
        }
        DefKind::Measure { pat, exp, name } => {
            push_name(name, interner, used);
            collect_pat_names(pat, interner, used);
            collect_exp_names(exp, interner, used);
        }
        DefKind::LoopMeasures { name, measures } => {
            push_name(name, interner, used);
            for (_, e) in measures {
                collect_exp_names(e, interner, used);
            }
        }
        DefKind::Register(spec) => {
            push_name(&spec.name, interner, used);
            if let Some(e) = &spec.init {
                collect_exp_names(e, interner, used);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for s in sets {
                for f in &s.funcls {
                    push_name(&f.name, interner, used);
                    collect_pexp_names(&f.arm, interner, used);
                }
            }
        }
        DefKind::Scattered(s) => match s {
            sail_ast::ScatteredDef::Function { name, .. }
            | sail_ast::ScatteredDef::Mapping { name, .. }
            | sail_ast::ScatteredDef::Enum(name)
            | sail_ast::ScatteredDef::End(name) => push_name(name, interner, used),
            sail_ast::ScatteredDef::Funcl(f) => {
                push_name(&f.name, interner, used);
                collect_pexp_names(&f.arm, interner, used);
            }
            sail_ast::ScatteredDef::Mapcl { name, clause } => {
                push_name(name, interner, used);
                match &clause.kind {
                    sail_ast::MapclKind::Bidir {
                        forwards,
                        backwards,
                    } => {
                        collect_mpexp_names(forwards, interner, used);
                        collect_mpexp_names(backwards, interner, used);
                    }
                    sail_ast::MapclKind::Forwards(p) | sail_ast::MapclKind::Backwards(p) => {
                        collect_pexp_names(p, interner, used);
                    }
                }
            }
            sail_ast::ScatteredDef::Variant { name, .. }
            | sail_ast::ScatteredDef::Unioncl { name, .. } => push_name(name, interner, used),
            sail_ast::ScatteredDef::Enumcl { enum_name, member } => {
                push_name(enum_name, interner, used);
                push_name(member, interner, used);
            }
        },
        DefKind::Val(vs) => push_name(&vs.name, interner, used),
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
}

fn collect_mpexp_names(m: &sail_ast::MPExp, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &m.kind {
        sail_ast::MPExpKind::Pat(p) => collect_pat_names(p, interner, used),
        sail_ast::MPExpKind::When { pat, guard } => {
            collect_pat_names(pat, interner, used);
            collect_exp_names(guard, interner, used);
        }
    }
}

fn collect_pexp_names(p: &sail_ast::PExp, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &p.kind {
        sail_ast::PExpKind::PatExp { pat, body } => {
            collect_pat_names(pat, interner, used);
            collect_exp_names(body, interner, used);
        }
        sail_ast::PExpKind::PatWhen { pat, guard, body } => {
            collect_pat_names(pat, interner, used);
            collect_exp_names(guard, interner, used);
            collect_exp_names(body, interner, used);
        }
    }
}

fn collect_pat_names(pat: &Pat, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &pat.kind {
        PatKind::Id(n) | PatKind::VectorSubrange { id: n, .. } | PatKind::App { func: n, .. } => {
            push_name(n, interner, used);
        }
        PatKind::As { pat, id } => {
            push_name(id, interner, used);
            collect_pat_names(pat, interner, used);
        }
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            collect_pat_names(a, interner, used);
            collect_pat_names(b, interner, used);
        }
        PatKind::Not(p) | PatKind::Typ { pat: p, .. } | PatKind::Var { pat: p, .. } => {
            collect_pat_names(p, interner, used);
        }
        PatKind::Tuple(ps)
        | PatKind::Vector(ps)
        | PatKind::VectorConcat(ps)
        | PatKind::List(ps)
        | PatKind::StringAppend(ps) => {
            for q in ps {
                collect_pat_names(q, interner, used);
            }
        }
        PatKind::Struct { name, fields, .. } => {
            if let sail_ast::StructName::Id(n) = name {
                push_name(n, interner, used);
            }
            for (f, q) in fields {
                push_name(f, interner, used);
                collect_pat_names(q, interner, used);
            }
        }
        PatKind::Lit(_) | PatKind::Wild => {}
    }
}

fn collect_exp_names(exp: &Exp, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &exp.kind {
        ExpKind::Id(n) | ExpKind::Ref(n) => push_name(n, interner, used),
        ExpKind::App { func, args } => {
            push_name(func, interner, used);
            for e in args {
                collect_exp_names(e, interner, used);
            }
        }
        ExpKind::Block(es) | ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            for e in es {
                collect_exp_names(e, interner, used);
            }
        }
        ExpKind::Typ { exp: e, .. } => collect_exp_names(e, interner, used),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_exp_names(cond, interner, used);
            collect_exp_names(then_branch, interner, used);
            collect_exp_names(else_branch, interner, used);
        }
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            if let Some(m) = measure {
                collect_exp_names(m, interner, used);
            }
            collect_exp_names(cond, interner, used);
            collect_exp_names(body, interner, used);
        }
        ExpKind::For {
            var,
            from,
            to,
            step,
            body,
            ..
        } => {
            push_name(var, interner, used);
            collect_exp_names(from, interner, used);
            collect_exp_names(to, interner, used);
            collect_exp_names(step, interner, used);
            collect_exp_names(body, interner, used);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            collect_exp_names(a, interner, used);
            collect_exp_names(b, interner, used);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                push_name(&f.field, interner, used);
                collect_exp_names(&f.value, interner, used);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            collect_exp_names(record, interner, used);
            for f in fields {
                push_name(&f.field, interner, used);
                collect_exp_names(&f.value, interner, used);
            }
        }
        ExpKind::Field { record, field } => {
            collect_exp_names(record, interner, used);
            push_name(field, interner, used);
        }
        ExpKind::Match { scrutinee, arms } => {
            collect_exp_names(scrutinee, interner, used);
            for a in arms {
                collect_pexp_names(a, interner, used);
            }
        }
        ExpKind::Let { bind, body, .. } => {
            collect_exp_names(bind, interner, used);
            collect_exp_names(body, interner, used);
        }
        ExpKind::Assign { lexp, value } => {
            collect_lexp_names(lexp, interner, used);
            collect_exp_names(value, interner, used);
        }
        ExpKind::Return(e) | ExpKind::Exit(e) | ExpKind::Throw(e) | ExpKind::InternalReturn(e) => {
            collect_exp_names(e, interner, used)
        }
        ExpKind::Try { exp: e, arms } => {
            collect_exp_names(e, interner, used);
            for a in arms {
                collect_pexp_names(a, interner, used);
            }
        }
        ExpKind::Assert { cond, msg } => {
            collect_exp_names(cond, interner, used);
            collect_exp_names(msg, interner, used);
        }
        ExpKind::Var { lexp, init, body } => {
            collect_lexp_names(lexp, interner, used);
            collect_exp_names(init, interner, used);
            collect_exp_names(body, interner, used);
        }
        ExpKind::InternalPlet { bind, body, .. } => {
            collect_exp_names(bind, interner, used);
            collect_exp_names(body, interner, used);
        }
        ExpKind::InternalAssume { exp: e, .. } => collect_exp_names(e, interner, used),
        ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
    }
}

fn collect_lexp_names(lexp: &sail_ast::LExp, interner: &dyn Interner, used: &mut BTreeSet<String>) {
    match &lexp.kind {
        sail_ast::LExpKind::Id(n) | sail_ast::LExpKind::Typ { id: n, .. } => {
            push_name(n, interner, used);
        }
        sail_ast::LExpKind::Deref(e) => collect_exp_names(e, interner, used),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for p in parts {
                collect_lexp_names(p, interner, used);
            }
        }
        sail_ast::LExpKind::Vector { lexp: inner, index } => {
            collect_lexp_names(inner, interner, used);
            collect_exp_names(index, interner, used);
        }
        sail_ast::LExpKind::VectorRange {
            lexp: inner,
            lo,
            hi,
        } => {
            collect_lexp_names(inner, interner, used);
            collect_exp_names(lo, interner, used);
            collect_exp_names(hi, interner, used);
        }
        sail_ast::LExpKind::Field { lexp: inner, field } => {
            collect_lexp_names(inner, interner, used);
            push_name(field, interner, used);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::SimpleInterner;

    fn run(defs: Vec<Def>, it: &mut SimpleInterner) -> Ast {
        let mut ast = Ast::new(defs);
        ToplevelLetPatterns.run(&mut ast, it);
        ast
    }

    fn toplevel(pat: Pat, exp: Exp) -> Def {
        Def::dummy(DefKind::Let { pat, exp })
    }

    fn text(it: &SimpleInterner, name: &Name) -> String {
        it.resolve(name.symbol).unwrap_or("?").to_owned()
    }

    fn def_pat_names(ast: &Ast, it: &SimpleInterner) -> Vec<String> {
        ast.defs
            .iter()
            .map(|d| match &d.kind {
                DefKind::Let { pat, .. } => match &pat.kind {
                    PatKind::Id(n) => text(it, n),
                    PatKind::Typ { pat: inner, .. } => match &inner.kind {
                        PatKind::Id(n) => format!("typ({})", text(it, n)),
                        other => format!("typ({other:?})"),
                    },
                    other => format!("{other:?}"),
                },
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn simple_patterns_pass_through() {
        let mut it = SimpleInterner::new();
        let sx = it.intern("x");
        let sy = it.intern("y");
        let sn = it.intern("n");
        let sm = it.intern("m");
        let se = it.intern("e");
        // Capture-free: kind-variable identity is plain symbol equality.
        let var = |s: Symbol| {
            mk_pat(PatKind::Var {
                pat: Box::new(mk_pat(PatKind::Id(mk_name(s)))),
                typ_pat: sail_ast::Node::dummy(TypPatKind::Var(s)),
            })
        };
        let pats = vec![
            mk_pat(PatKind::Id(mk_name(sx))),
            mk_pat(PatKind::Typ {
                typ: sail_ast::Node::dummy(sail_ast::TypKind::Unknown),
                pat: Box::new(mk_pat(PatKind::Id(mk_name(sy)))),
            }),
            var(sn),
            mk_pat(PatKind::Typ {
                typ: sail_ast::Node::dummy(sail_ast::TypKind::Unknown),
                pat: Box::new(var(sm)),
            }),
        ];
        for pat in pats {
            let snapshot = pat.clone();
            assert!(is_pat_simple(&pat, &it));
            let ast = run(
                vec![toplevel(pat, mk_exp(ExpKind::Id(mk_name(se))))],
                &mut it,
            );
            assert_eq!(ast.defs.len(), 1);
            let DefKind::Let { pat: got, .. } = &ast.defs[0].kind else {
                panic!("def shape")
            };
            assert_eq!(got, &snapshot);
        }
    }

    #[test]
    fn atom_var_pattern_is_simple_only_for_matching_binder() {
        let mut it = SimpleInterner::new();
        let sn = it.intern("n");
        let sm = it.intern("m");
        let satom = it.intern("atom");
        let atom = |inner: Symbol, kid: Symbol| {
            mk_pat(PatKind::Var {
                pat: Box::new(mk_pat(PatKind::Id(mk_name(inner)))),
                typ_pat: sail_ast::Node::dummy(TypPatKind::App {
                    name: mk_name(satom),
                    args: vec![sail_ast::Node::dummy(TypPatKind::Var(kid))],
                }),
            })
        };
        let yes = atom(sn, sn);
        let no = atom(sn, sm);
        assert!(is_pat_simple(&yes, &it));
        assert!(!is_pat_simple(&no, &it));
    }

    #[test]
    fn tuple_pattern_splits_into_base_and_projections() {
        let mut it = SimpleInterner::new();
        let a = mk_name(it.intern("a"));
        let b = mk_name(it.intern("b"));
        let e = mk_exp(ExpKind::Id(mk_name(it.intern("e"))));
        let pat = mk_pat(PatKind::Tuple(vec![
            mk_pat(PatKind::Id(a)),
            mk_pat(PatKind::Id(b)),
        ]));
        let ast = run(vec![toplevel(pat, e)], &mut it);
        assert_eq!(ast.defs.len(), 3);
        // Base definition first, then projections in name order.
        assert_eq!(def_pat_names(&ast, &it), vec!["let__0", "a", "b"]);
        for d in ast.defs.iter().skip(1) {
            let DefKind::Let { pat, exp } = &d.kind else {
                panic!("def shape")
            };
            assert!(matches!(&pat.kind, PatKind::Id(_)));
            let ExpKind::Let {
                pat: inner_pat,
                bind,
                body,
            } = &exp.kind
            else {
                panic!("expected projection let, got {:?}", exp.kind)
            };
            assert!(matches!(&inner_pat.kind, PatKind::Tuple(_)));
            assert!(matches!(&bind.kind, ExpKind::Id(n) if text(&it, n) == "let__0"));
            assert!(matches!(&body.kind, ExpKind::Id(_)));
        }
    }

    #[test]
    fn fresh_base_avoids_capture_and_typ_wrapper_survives_on_base() {
        let mut it = SimpleInterner::new();
        // Occupy `let__0` so the base must become `let__1`.
        let clash = mk_exp(ExpKind::Id(mk_name(it.intern("let__0"))));
        let wild = mk_pat(PatKind::Wild);
        let typ = sail_ast::Node::dummy(sail_ast::TypKind::Id(mk_name(it.intern("u"))));
        let pat = mk_pat(PatKind::Typ {
            typ,
            pat: Box::new(wild),
        });
        let ast = run(
            vec![
                toplevel(mk_pat(PatKind::Wild), clash),
                toplevel(pat, mk_exp(ExpKind::Undef)),
            ],
            &mut it,
        );
        // `let _ = let__0` has no bound ids: only the renamed base survives.
        // The second def keeps its syntactic `P_typ` on the base binder.
        assert_eq!(def_pat_names(&ast, &it), vec!["let__1", "typ(let__2)"]);
    }

    #[test]
    fn non_let_defs_pass_through_and_pass_is_idempotent() {
        let mut it = SimpleInterner::new();
        let pragma = Def::dummy(DefKind::Pragma {
            key: "k".to_string(),
            payload: "v".to_string(),
        });
        let pat = mk_pat(PatKind::Tuple(vec![mk_pat(PatKind::Id(mk_name(
            it.intern("a"),
        )))]));
        let mut ast = run(
            vec![
                pragma,
                toplevel(pat, mk_exp(ExpKind::Id(mk_name(it.intern("e"))))),
            ],
            &mut it,
        );
        assert!(matches!(ast.defs[0].kind, DefKind::Pragma { .. }));
        let snapshot = ast.clone();
        ToplevelLetPatterns.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot, "pass must be idempotent");
    }
}
