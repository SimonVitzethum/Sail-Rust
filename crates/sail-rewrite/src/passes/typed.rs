//! Shared helpers for the canonical (`sail_ast`) rewrite path.
//!
//! Source shapes follow `sail/src/lib/rewrites.ml` (`rewrite_ast_base`): one
//! bottom-up walk reaches every [`Exp`](sail_ast::Exp) and match arm
//! ([`PExp`](sail_ast::PExp)) in the program. Passes reuse
//! [`visit_all`] instead of each hand-rolling definition/expression
//! recursion, so coverage stays in one place.
//!
//! Fresh names follow OCaml `fresh_id`: a per-run counter with a text prefix
//! (`tup__`, ...), skipping every name already present in the program (see
//! [`Fresh`]). Synthesized nodes use [`NodeId::DUMMY`](sail_ast::NodeId) /
//! [`Span::DUMMY`](sail_ast::Span), matching `Node::dummy` for desugared
//! output.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use sail_ast::{
    Ast, Def, DefKind, Exp, ExpKind, Funcl, Interner, LExp, LExpKind, Mapcl, MapclKind, MPExp,
    MPExpKind, Name, Node, NodeId, PExp, PExpKind, Pat, PatKind, ScatteredDef, Span, Symbol,
};

/// Collect every interned name text in `ast`.
///
/// Used to seed [`Fresh`] so generated binders never capture user names.
/// Unresolvable symbols are ignored (they carry no text to collide with).
/// Type-level names (`Constraint`, `NExp`, `Typ`) are intentionally skipped:
/// generated binders are value-level and never occur in type positions.
#[must_use]
pub fn collect_used(ast: &Ast, interner: &dyn Interner) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    let mut name = |n: &Name| {
        if let Some(text) = interner.resolve(n.symbol) {
            used.insert(text.to_owned());
        }
    };
    for def in &ast.defs {
        collect_def_names(def, &mut name);
    }
    used
}

fn collect_def_names(def: &Def, name: &mut impl FnMut(&Name)) {
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            for f in funcls {
                collect_funcl_names(f, name);
            }
        }
        DefKind::Mapdef { name: n, clauses, .. } => {
            name(n);
            for c in clauses {
                collect_mapcl_names(c, name);
            }
        }
        DefKind::Impl(f) => collect_funcl_names(f, name),
        DefKind::Let { pat, exp } => {
            collect_pat_names(pat, name);
            collect_exp_names(exp, name);
        }
        DefKind::Outcome { defs, name: n, .. } => {
            name(n);
            for d in defs {
                collect_def_names(d, name);
            }
        }
        DefKind::Measure { pat, exp, name: n } => {
            name(n);
            collect_pat_names(pat, name);
            collect_exp_names(exp, name);
        }
        DefKind::LoopMeasures { name: n, measures } => {
            name(n);
            for (_, e) in measures {
                collect_exp_names(e, name);
            }
        }
        DefKind::Register(spec) => {
            name(&spec.name);
            if let Some(e) = &spec.init {
                collect_exp_names(e, name);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for s in sets {
                for f in &s.funcls {
                    collect_funcl_names(f, name);
                }
            }
        }
        DefKind::Scattered(s) => match s {
            ScatteredDef::Function { name: n, .. }
            | ScatteredDef::Mapping { name: n, .. }
            | ScatteredDef::Enum(n)
            | ScatteredDef::End(n) => name(n),
            ScatteredDef::Funcl(f) => collect_funcl_names(f, name),
            ScatteredDef::Mapcl { name: n, clause } => {
                name(n);
                collect_mapcl_names(clause, name);
            }
            ScatteredDef::Variant { name: n, .. } | ScatteredDef::Unioncl { name: n, .. } => {
                name(n);
            }
            ScatteredDef::Enumcl { enum_name, member } => {
                name(enum_name);
                name(member);
            }
        },
        DefKind::Val(vs) => name(&vs.name),
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
}

fn collect_funcl_names(f: &Funcl, name: &mut impl FnMut(&Name)) {
    name(&f.name);
    collect_pexp_names(&f.arm, name);
}

fn collect_mapcl_names(c: &Mapcl, name: &mut impl FnMut(&Name)) {
    match &c.kind {
        MapclKind::Bidir { forwards, backwards } => {
            collect_mpexp_names(forwards, name);
            collect_mpexp_names(backwards, name);
        }
        MapclKind::Forwards(p) | MapclKind::Backwards(p) => collect_pexp_names(p, name),
    }
}

fn collect_mpexp_names(m: &MPExp, name: &mut impl FnMut(&Name)) {
    match &m.kind {
        MPExpKind::Pat(p) => collect_pat_names(p, name),
        MPExpKind::When { pat, guard } => {
            collect_pat_names(pat, name);
            collect_exp_names(guard, name);
        }
    }
}

fn collect_pexp_names(p: &PExp, name: &mut impl FnMut(&Name)) {
    match &p.kind {
        PExpKind::PatExp { pat, body } => {
            collect_pat_names(pat, name);
            collect_exp_names(body, name);
        }
        PExpKind::PatWhen { pat, guard, body } => {
            collect_pat_names(pat, name);
            collect_exp_names(guard, name);
            collect_exp_names(body, name);
        }
    }
}

fn collect_pat_names(p: &Pat, name: &mut impl FnMut(&Name)) {
    match &p.kind {
        PatKind::Id(n)
        | PatKind::VectorSubrange { id: n, .. }
        | PatKind::App { func: n, .. } => name(n),
        PatKind::As { pat, id } => {
            name(id);
            collect_pat_names(pat, name);
        }
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            collect_pat_names(a, name);
            collect_pat_names(b, name);
        }
        PatKind::Not(p) | PatKind::Typ { pat: p, .. } | PatKind::Var { pat: p, .. } => {
            collect_pat_names(p, name);
        }
        PatKind::Tuple(ps)
        | PatKind::Vector(ps)
        | PatKind::VectorConcat(ps)
        | PatKind::List(ps)
        | PatKind::StringAppend(ps) => {
            for q in ps {
                collect_pat_names(q, name);
            }
        }
        PatKind::Struct { name: s, fields, .. } => {
            if let sail_ast::StructName::Id(n) = s {
                name(n);
            }
            for (f, q) in fields {
                name(f);
                collect_pat_names(q, name);
            }
        }
        PatKind::Lit(_) | PatKind::Wild => {}
    }
}

/// Read-only expression walk for [`collect_used`]: every value-level name a
/// generated binder could capture (`E_id`, call targets, `LExp` roots,
/// `for` binders, struct fields are included as harmless over-approximation).
fn collect_exp_names(exp: &Exp, name: &mut impl FnMut(&Name)) {
    match &exp.kind {
        ExpKind::Id(n) | ExpKind::Ref(n) => name(n),
        ExpKind::App { func, args } => {
            name(func);
            for e in args {
                collect_exp_names(e, name);
            }
        }
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                collect_exp_names(e, name);
            }
        }
        ExpKind::Typ { exp: e, .. } => collect_exp_names(e, name),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_exp_names(cond, name);
            collect_exp_names(then_branch, name);
            collect_exp_names(else_branch, name);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                collect_exp_names(m, name);
            }
            collect_exp_names(cond, name);
            collect_exp_names(body, name);
        }
        ExpKind::For { var, from, to, step, body, .. } => {
            name(var);
            collect_exp_names(from, name);
            collect_exp_names(to, name);
            collect_exp_names(step, name);
            collect_exp_names(body, name);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            collect_exp_names(a, name);
            collect_exp_names(b, name);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                name(&f.field);
                collect_exp_names(&f.value, name);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            collect_exp_names(record, name);
            for f in fields {
                name(&f.field);
                collect_exp_names(&f.value, name);
            }
        }
        ExpKind::Field { record, field } => {
            collect_exp_names(record, name);
            name(field);
        }
        ExpKind::Match { scrutinee, arms } => {
            collect_exp_names(scrutinee, name);
            for a in arms {
                collect_pexp_names(a, name);
            }
        }
        ExpKind::Let { bind, body, .. } => {
            collect_exp_names(bind, name);
            collect_exp_names(body, name);
        }
        ExpKind::Assign { lexp, value } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(value, name);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => collect_exp_names(e, name),
        ExpKind::Try { exp: e, arms } => {
            collect_exp_names(e, name);
            for a in arms {
                collect_pexp_names(a, name);
            }
        }
        ExpKind::Assert { cond, msg } => {
            collect_exp_names(cond, name);
            collect_exp_names(msg, name);
        }
        ExpKind::Var { lexp, init, body } => {
            collect_lexp_names(lexp, name);
            collect_exp_names(init, name);
            collect_exp_names(body, name);
        }
        ExpKind::InternalPlet { bind, body, .. } => {
            collect_exp_names(bind, name);
            collect_exp_names(body, name);
        }
        ExpKind::InternalAssume { exp: e, .. } => collect_exp_names(e, name),
        ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
    }
}

fn collect_lexp_names(lexp: &LExp, name: &mut impl FnMut(&Name)) {
    match &lexp.kind {
        LExpKind::Id(n) | LExpKind::Typ { id: n, .. } => name(n),
        LExpKind::Deref(e) => collect_exp_names(e, name),
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => {
            for p in parts {
                collect_lexp_names(p, name);
            }
        }
        LExpKind::Vector { lexp: inner, index } => {
            collect_lexp_names(inner, name);
            collect_exp_names(index, name);
        }
        LExpKind::VectorRange { lexp: inner, lo, hi } => {
            collect_lexp_names(inner, name);
            collect_exp_names(lo, name);
            collect_exp_names(hi, name);
        }
        LExpKind::Field { lexp: inner, field } => {
            collect_lexp_names(inner, name);
            name(field);
        }
    }
}

/// Per-run fresh-name generator (`tup__0`, `tup__1`, ...).
///
/// Seeded with [`collect_used`]; every result is inserted into the used set,
/// so repeated calls never collide with each other or with program names.
#[derive(Clone, Debug)]
pub struct Fresh {
    used: BTreeSet<String>,
    next: usize,
}

impl Fresh {
    /// Seed from the program's existing names.
    #[must_use]
    pub fn new(used: BTreeSet<String>) -> Self {
        Self { used, next: 0 }
    }

    /// Generate a fresh `prefix{n}` name.
    pub fn fresh(&mut self, prefix: &str) -> String {
        loop {
            let cand = format!("{prefix}{}", self.next);
            self.next += 1;
            if self.used.insert(cand.clone()) {
                return cand;
            }
        }
    }
}

/// Make a synthesized identifier.
#[must_use]
pub fn mk_name(symbol: Symbol) -> Name {
    Name::dummy(symbol)
}

/// Make a synthesized expression.
#[must_use]
pub fn mk_exp(kind: ExpKind) -> Exp {
    Node::dummy(kind)
}

/// Make a synthesized pattern.
#[must_use]
pub fn mk_pat(kind: PatKind) -> Pat {
    Node::dummy(kind)
}

/// Make a synthesized lvalue.
#[must_use]
pub fn mk_lexp(kind: LExpKind) -> LExp {
    Node::dummy(kind)
}

/// Make a synthesized match arm without guard.
#[must_use]
pub fn mk_arm(pat: Pat, body: Exp) -> PExp {
    PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind: PExpKind::PatExp { pat, body },
    }
}

/// Bottom-up walk over every expression and match arm in `ast`.
///
/// `visit_exp` runs after the node's children (OCaml `fold_exp` order), so a
/// single traversal suffices for passes that only look downward. `visit_pexp`
/// runs on each arm before descending into its guard/body. Either callback may
/// mutate in place; passes needing a global fixpoint (OCaml `go` in
/// `rewrite_tuple_assignments`) loop the whole walk until it reports no
/// change.
pub fn visit_all(
    ast: &mut Ast,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    for def in &mut ast.defs {
        visit_def(def, visit_exp, visit_pexp);
    }
}

fn visit_def(
    def: &mut Def,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    match &mut def.kind {
        DefKind::Fundef { funcls, .. } => {
            for f in funcls {
                visit_funcl(f, visit_exp, visit_pexp);
            }
        }
        DefKind::Mapdef { clauses, .. } => {
            for c in clauses {
                visit_mapcl(c, visit_exp, visit_pexp);
            }
        }
        DefKind::Impl(f) => visit_funcl(f, visit_exp, visit_pexp),
        DefKind::Let { exp, .. } => visit_exp_node(exp, visit_exp, visit_pexp),
        DefKind::Outcome { defs, .. } => {
            for d in defs {
                visit_def(d, visit_exp, visit_pexp);
            }
        }
        DefKind::Measure { exp, .. } => visit_exp_node(exp, visit_exp, visit_pexp),
        DefKind::LoopMeasures { measures, .. } => {
            for (_, e) in measures {
                visit_exp_node(e, visit_exp, visit_pexp);
            }
        }
        DefKind::Register(spec) => {
            if let Some(e) = &mut spec.init {
                visit_exp_node(e, visit_exp, visit_pexp);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for s in sets {
                for f in &mut s.funcls {
                    visit_funcl(f, visit_exp, visit_pexp);
                }
            }
        }
        DefKind::Scattered(s) => match s {
            ScatteredDef::Funcl(f) => visit_funcl(f, visit_exp, visit_pexp),
            ScatteredDef::Mapcl { clause, .. } => visit_mapcl(clause, visit_exp, visit_pexp),
            ScatteredDef::Function { .. }
            | ScatteredDef::Variant { .. }
            | ScatteredDef::Unioncl { .. }
            | ScatteredDef::Mapping { .. }
            | ScatteredDef::Enum(_)
            | ScatteredDef::Enumcl { .. }
            | ScatteredDef::End(_) => {}
        },
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Val(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
}

fn visit_funcl(
    f: &mut Funcl,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    visit_pexp_node(&mut f.arm, visit_exp, visit_pexp);
}

fn visit_mapcl(
    c: &mut Mapcl,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    match &mut c.kind {
        MapclKind::Bidir { forwards, backwards } => {
            visit_mpexp(forwards, visit_exp, visit_pexp);
            visit_mpexp(backwards, visit_exp, visit_pexp);
        }
        MapclKind::Forwards(p) | MapclKind::Backwards(p) => {
            visit_pexp_node(p, visit_exp, visit_pexp);
        }
    }
}

fn visit_mpexp(
    m: &mut MPExp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    if let MPExpKind::When { guard, .. } = &mut m.kind {
        visit_exp_node(guard, visit_exp, visit_pexp);
    }
}

fn visit_pexp_node(
    p: &mut PExp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    visit_pexp(p);
    match &mut p.kind {
        PExpKind::PatExp { body, .. } => visit_exp_node(body, visit_exp, visit_pexp),
        PExpKind::PatWhen { guard, body, .. } => {
            visit_exp_node(guard, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
    }
}

fn visit_exp_node(
    exp: &mut Exp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    visit_exp_children(exp, visit_exp, visit_pexp);
    visit_exp(exp);
}

fn visit_lexp_children(
    lexp: &mut LExp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    match &mut lexp.kind {
        LExpKind::Deref(e) => visit_exp_node(e, visit_exp, visit_pexp),
        LExpKind::Vector { lexp: inner, index } => {
            visit_lexp_children(inner, visit_exp, visit_pexp);
            visit_exp_node(index, visit_exp, visit_pexp);
        }
        LExpKind::VectorRange { lexp: inner, lo, hi } => {
            visit_lexp_children(inner, visit_exp, visit_pexp);
            visit_exp_node(lo, visit_exp, visit_pexp);
            visit_exp_node(hi, visit_exp, visit_pexp);
        }
        LExpKind::Field { lexp: inner, .. } => visit_lexp_children(inner, visit_exp, visit_pexp),
        LExpKind::Id(_)
        | LExpKind::Typ { .. }
        | LExpKind::Tuple(_)
        | LExpKind::VectorConcat(_) => {
            visit_lexp_shallow(lexp, visit_exp, visit_pexp);
        }
    }
}

/// Nested lexps inside tuple/concat targets hold no expressions in `sail_ast`
/// except through `Vector`/`VectorRange`/`Deref`, but tuple elements and
/// concat parts recurse structurally so future expression-carrying targets
/// stay covered.
fn visit_lexp_shallow(
    lexp: &mut LExp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    match &mut lexp.kind {
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => {
            for p in parts {
                visit_lexp_children(p, visit_exp, visit_pexp);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_lines)]
fn visit_exp_children(
    exp: &mut Exp,
    visit_exp: &mut impl FnMut(&mut Exp),
    visit_pexp: &mut impl FnMut(&mut PExp),
) {
    match &mut exp.kind {
        ExpKind::Block(es)
        | ExpKind::Tuple(es)
        | ExpKind::Vector(es)
        | ExpKind::List(es) => {
            for e in es {
                visit_exp_node(e, visit_exp, visit_pexp);
            }
        }
        ExpKind::Typ { exp: e, .. } => visit_exp_node(e, visit_exp, visit_pexp),
        ExpKind::App { args, .. } => {
            for e in args {
                visit_exp_node(e, visit_exp, visit_pexp);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            visit_exp_node(cond, visit_exp, visit_pexp);
            visit_exp_node(then_branch, visit_exp, visit_pexp);
            visit_exp_node(else_branch, visit_exp, visit_pexp);
        }
        ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(m) = measure {
                visit_exp_node(m, visit_exp, visit_pexp);
            }
            visit_exp_node(cond, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
        ExpKind::For {
            from, to, step, body, ..
        } => {
            visit_exp_node(from, visit_exp, visit_pexp);
            visit_exp_node(to, visit_exp, visit_pexp);
            visit_exp_node(step, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            visit_exp_node(a, visit_exp, visit_pexp);
            visit_exp_node(b, visit_exp, visit_pexp);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                visit_exp_node(&mut f.value, visit_exp, visit_pexp);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            visit_exp_node(record, visit_exp, visit_pexp);
            for f in fields {
                visit_exp_node(&mut f.value, visit_exp, visit_pexp);
            }
        }
        ExpKind::Field { record, .. } => visit_exp_node(record, visit_exp, visit_pexp),
        ExpKind::Match { scrutinee, arms } => {
            visit_exp_node(scrutinee, visit_exp, visit_pexp);
            for a in arms {
                visit_pexp_node(a, visit_exp, visit_pexp);
            }
        }
        ExpKind::Let { bind, body, .. } => {
            visit_exp_node(bind, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
        ExpKind::Assign { lexp, value } => {
            visit_lexp_children(lexp, visit_exp, visit_pexp);
            visit_exp_node(value, visit_exp, visit_pexp);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => visit_exp_node(e, visit_exp, visit_pexp),
        ExpKind::Try { exp: e, arms } => {
            visit_exp_node(e, visit_exp, visit_pexp);
            for a in arms {
                visit_pexp_node(a, visit_exp, visit_pexp);
            }
        }
        ExpKind::Assert { cond, msg } => {
            visit_exp_node(cond, visit_exp, visit_pexp);
            visit_exp_node(msg, visit_exp, visit_pexp);
        }
        ExpKind::Var { lexp, init, body } => {
            visit_lexp_children(lexp, visit_exp, visit_pexp);
            visit_exp_node(init, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
        ExpKind::InternalPlet { bind, body, .. } => {
            visit_exp_node(bind, visit_exp, visit_pexp);
            visit_exp_node(body, visit_exp, visit_pexp);
        }
        ExpKind::InternalAssume { exp: e, .. } => visit_exp_node(e, visit_exp, visit_pexp),
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
    }
}

/// Whether `pat` contains a [`PatKind::StringAppend`] anywhere.
///
/// Mirrors the `has_string_append_pattern` test in OCaml
/// `rewrite_ast_pat_string_append`.
#[must_use]
pub fn pat_has_string_append(pat: &Pat) -> bool {
    match &pat.kind {
        PatKind::StringAppend(_) => true,
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            pat_has_string_append(a) || pat_has_string_append(b)
        }
        PatKind::Not(p) | PatKind::Typ { pat: p, .. } | PatKind::Var { pat: p, .. } => {
            pat_has_string_append(p)
        }
        PatKind::As { pat: p, .. } => pat_has_string_append(p),
        PatKind::App { args, .. }
        | PatKind::Vector(args)
        | PatKind::VectorConcat(args)
        | PatKind::Tuple(args)
        | PatKind::List(args) => args.iter().any(pat_has_string_append),
        PatKind::Struct { fields, .. } => fields.iter().any(|(_, p)| pat_has_string_append(p)),
        PatKind::Lit(_)
        | PatKind::Wild
        | PatKind::Id(_)
        | PatKind::VectorSubrange { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{PExpKind, SimpleInterner};

    fn interner() -> SimpleInterner {
        SimpleInterner::new()
    }

    #[test]
    fn fresh_skips_used_names() {
        let used: BTreeSet<String> = ["tup__0".to_owned(), "x".to_owned()].into_iter().collect();
        let mut fresh = Fresh::new(used);
        assert_eq!(fresh.fresh("tup__"), "tup__1");
        assert_eq!(fresh.fresh("tup__"), "tup__2");
    }

    #[test]
    fn walker_reaches_nested_match_arms() {
        let mut it = interner();
        let x = it.intern("x");
        let scrutinee = mk_exp(ExpKind::Id(mk_name(x)));
        let body = mk_exp(ExpKind::Undef);
        let arm = mk_arm(mk_pat(PatKind::Wild), body);
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Let {
            pat: mk_pat(PatKind::Wild),
            exp: mk_exp(ExpKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![arm],
            }),
        })]);
        let mut exps = 0;
        let mut pexps = 0;
        visit_all(&mut ast, &mut |_| exps += 1, &mut |_| pexps += 1);
        // scrutinee + Undef body + match node itself.
        assert_eq!(exps, 3);
        assert_eq!(pexps, 1);
        let DefKind::Let { exp, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        let ExpKind::Match { arms, .. } = &exp.kind else {
            panic!("exp shape")
        };
        let PExpKind::PatExp { body, .. } = &arms[0].kind else {
            panic!("arm shape")
        };
        assert!(matches!(body.kind, ExpKind::Undef));
    }

    #[test]
    fn string_append_detector() {
        assert!(pat_has_string_append(&mk_pat(PatKind::StringAppend(vec![
            mk_pat(PatKind::Wild)
        ]))));
        assert!(!pat_has_string_append(&mk_pat(PatKind::Wild)));
    }
}
