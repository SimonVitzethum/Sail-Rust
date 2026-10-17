//! Elaboration output: closed `Typ` snapshots of the checker's type views
//! and their attachment to AST nodes (`Annotation::typ`).
//!
//! The Lean backend's rewrites and printer read `typ_of exp` the way the
//! OCaml pipeline reads `tannot`. Snapshots are expressed in the definition's
//! interner (extended with any new names), with call substitutions applied;
//! opened existential witnesses stay as free variables.

use std::collections::HashMap;

use super::*;

/// Addresses of recorded nodes (identity only, never dereferenced).
#[derive(Default)]
pub(super) struct Recorder {
    pub exps: HashMap<usize, Type>,
    pub pats: HashMap<usize, Type>,
    pub lexps: HashMap<usize, Type>,
    /// Assignments that declare their target (`x = e` introducing `x`).
    pub intros: std::collections::HashSet<usize>,
    /// Overloaded calls: the member function each resolved to.
    pub callees: HashMap<usize, String>,
}

impl Checker<'_> {
    pub(super) fn record_exp(&mut self, exp: &Exp, typ: &Type) {
        if let Some(recorder) = &mut self.recorder {
            recorder.exps.insert(exp as *const Exp as usize, typ.clone());
        }
    }

    pub(super) fn record_lexp(&mut self, lexp: &LExp, typ: &Type) {
        if let Some(recorder) = &mut self.recorder {
            recorder.lexps.insert(lexp as *const LExp as usize, typ.clone());
        }
    }

    /// An application of an overloaded name resolved to `callee`.
    pub(super) fn record_call(&mut self, exp: &Exp, _callee: &Type) {
        if self.recorder.is_none() {
            return;
        }
        let ExpKind::App { func, .. } = &exp.kind else {
            return;
        };
        let Ok(text) = self.name(func) else {
            return;
        };
        let Some(picked) = self.picked.borrow().clone() else {
            if std::env::var_os("SAIL_RS_DEBUG_CALLS").is_some() {
                eprintln!("record_call: `{text}` has no picked candidate");
            }
            return;
        };
        if std::env::var_os("SAIL_RS_DEBUG_CALLS").is_some() {
            eprintln!("record_call: `{text}` picked");
        }
        // Overloaded names resolve to the member function the call picked;
        // the parser's `vector_*#` primitives resolve through their overload.
        let arity = match &exp.kind {
            ExpKind::App { args, .. } => args.len(),
            _ => 0,
        };
        let text = crate::vector_prims::resolve_app(text, arity).map_or(text, |r| r.overload);
        let Ok(Some(members)) = self.program.overload_members(text, self.position) else {
            return;
        };
        if let Some((member, _)) = members.iter().find(|(_, scheme)| Arc::ptr_eq(scheme, &picked)) {
            if std::env::var_os("SAIL_RS_DEBUG_CALLS").is_some() {
                eprintln!("record_call: `{text}` -> `{member}` at {:x}", exp as *const Exp as usize);
            }
            if let Some(recorder) = &mut self.recorder {
                recorder.callees.insert(exp as *const Exp as usize, member.clone());
            }
        }
    }

    /// A call whose overload set has exactly one member.
    pub(super) fn record_single(&mut self, exp: &Exp, candidates: &[Type]) {
        if self.recorder.is_none() || candidates.len() != 1 {
            return;
        }
        *self.picked.borrow_mut() = Some(candidates[0].root.clone());
        self.record_call(exp, &candidates[0]);
    }

    pub(super) fn record_intro(&mut self, exp: &Exp) {
        if let Some(recorder) = &mut self.recorder {
            recorder.intros.insert(exp as *const Exp as usize);
        }
    }

    pub(super) fn record_pat(&mut self, pat: &Pat, typ: &Type) {
        if let Some(recorder) = &mut self.recorder {
            recorder.pats.insert(pat as *const Pat as usize, typ.clone());
        }
    }
}

struct Snap<'o> {
    out: &'o mut SimpleInterner,
    fuel: u32,
}

fn name_in(out: &mut SimpleInterner, from: &SimpleInterner, name: &Name) -> Name {
    let text = Interner::resolve(from, name.symbol).unwrap_or("?").to_owned();
    Name {
        symbol: out.intern(&text),
        ..name.clone()
    }
}

fn symbol_in(out: &mut SimpleInterner, from: &SimpleInterner, symbol: Symbol) -> Symbol {
    let text = Interner::resolve(from, symbol).unwrap_or("?").to_owned();
    out.intern(&text)
}

impl Snap<'_> {
    /// Snapshot a type view.
    fn typ(&mut self, t: &Type) -> Typ {
        let t = t.resolved();
        let node = t.node().clone();
        self.typ_node(&node, &t, &[])
    }

    fn typ_node(&mut self, node: &Typ, owner: &Type, shadow: &[Symbol]) -> Typ {
        let from: &SimpleInterner = &owner.root.interner;
        let kind = match &node.kind {
            TypKind::Unknown => TypKind::Unknown,
            TypKind::Id(name) => TypKind::Id(name_in(self.out, from, name)),
            TypKind::Var(symbol) => {
                if !shadow.contains(symbol) {
                    if let Some(bound) = owner.substitutions.get_type(*symbol) {
                        if self.fuel > 0 {
                            self.fuel -= 1;
                            let bound = bound.clone();
                            return self.typ(&bound);
                        }
                    }
                }
                TypKind::Var(symbol_in(self.out, from, *symbol))
            }
            TypKind::Tuple(items) => {
                TypKind::Tuple(items.iter().map(|i| self.typ_node(i, owner, shadow)).collect())
            }
            TypKind::Fn { args, ret } => TypKind::Fn {
                args: args.iter().map(|a| self.typ_node(a, owner, shadow)).collect(),
                ret: Box::new(self.typ_node(ret, owner, shadow)),
            },
            TypKind::Bidir { lhs, rhs } => TypKind::Bidir {
                lhs: Box::new(self.typ_node(lhs, owner, shadow)),
                rhs: Box::new(self.typ_node(rhs, owner, shadow)),
            },
            TypKind::App { name, args } => TypKind::App {
                name: name_in(self.out, from, name),
                args: args.iter().map(|a| self.arg(a, owner, shadow)).collect(),
            },
            TypKind::Exist {
                binders,
                constraint,
                typ,
            } => {
                let mut inner_shadow = shadow.to_vec();
                inner_shadow.extend(binders.iter().map(|b| b.kid));
                let new_binders: Vec<KOpt> = binders
                    .iter()
                    .map(|b| KOpt {
                        kid: symbol_in(self.out, from, b.kid),
                        ..*b
                    })
                    .collect();
                TypKind::Exist {
                    binders: new_binders,
                    constraint: Box::new(self.constraint(constraint, owner, &inner_shadow)),
                    typ: Box::new(self.typ_node(typ, owner, &inner_shadow)),
                }
            }
        };
        Node { kind, ..node.clone() }
    }

    fn arg(&mut self, a: &TypArg, owner: &Type, shadow: &[Symbol]) -> TypArg {
        let kind = match &a.kind {
            TypArgKind::Typ(t) => TypArgKind::Typ(self.typ_node(t, owner, shadow)),
            TypArgKind::NExp(n) => TypArgKind::NExp(self.nexp(n, owner, shadow)),
            TypArgKind::Bool(c) => TypArgKind::Bool(Box::new(self.constraint(c, owner, shadow))),
        };
        Node { kind, ..a.clone() }
    }

    fn nexp(&mut self, n: &NExp, owner: &Type, shadow: &[Symbol]) -> NExp {
        let from: &SimpleInterner = &owner.root.interner;
        let kind = match &n.kind {
            NExpKind::Id(name) => NExpKind::Id(name_in(self.out, from, name)),
            NExpKind::Constant(c) => NExpKind::Constant(c.clone()),
            NExpKind::Var(symbol) => {
                if !shadow.contains(symbol) && self.fuel > 0 {
                    if let Some(bound) = owner.substitutions.get_scalar(*symbol) {
                        let bound = bound.clone();
                        if let TypKind::App { args, .. } = &bound.owner.node().kind {
                            if let Some(TypArg {
                                kind: TypArgKind::NExp(inner),
                                ..
                            }) = args.get(bound.index)
                            {
                                self.fuel -= 1;
                                let inner = inner.clone();
                                return self.nexp(&inner, &bound.owner, &[]);
                            }
                        }
                    }
                }
                NExpKind::Var(symbol_in(self.out, from, *symbol))
            }
            NExpKind::App { func, args } => NExpKind::App {
                func: name_in(self.out, from, func),
                args: args.iter().map(|a| self.nexp(a, owner, shadow)).collect(),
            },
            NExpKind::If {
                cond,
                then_branch,
                else_branch,
            } => NExpKind::If {
                cond: Box::new(self.constraint(cond, owner, shadow)),
                then_branch: Box::new(self.nexp(then_branch, owner, shadow)),
                else_branch: Box::new(self.nexp(else_branch, owner, shadow)),
            },
            NExpKind::Times(a, b) => {
                NExpKind::Times(Box::new(self.nexp(a, owner, shadow)), Box::new(self.nexp(b, owner, shadow)))
            }
            NExpKind::Sum(a, b) => {
                NExpKind::Sum(Box::new(self.nexp(a, owner, shadow)), Box::new(self.nexp(b, owner, shadow)))
            }
            NExpKind::Minus(a, b) => {
                NExpKind::Minus(Box::new(self.nexp(a, owner, shadow)), Box::new(self.nexp(b, owner, shadow)))
            }
            NExpKind::Exp(a) => NExpKind::Exp(Box::new(self.nexp(a, owner, shadow))),
            NExpKind::Neg(a) => NExpKind::Neg(Box::new(self.nexp(a, owner, shadow))),
        };
        Node { kind, ..n.clone() }
    }

    fn constraint(&mut self, c: &Constraint, owner: &Type, shadow: &[Symbol]) -> Constraint {
        let from: &SimpleInterner = &owner.root.interner;
        let kind = match &c.kind {
            ConstraintKind::Equal(a, b) => ConstraintKind::Equal(
                Box::new(self.arg(a, owner, shadow)),
                Box::new(self.arg(b, owner, shadow)),
            ),
            ConstraintKind::NotEqual(a, b) => ConstraintKind::NotEqual(
                Box::new(self.arg(a, owner, shadow)),
                Box::new(self.arg(b, owner, shadow)),
            ),
            ConstraintKind::Ge(a, b) => ConstraintKind::Ge(self.nexp(a, owner, shadow), self.nexp(b, owner, shadow)),
            ConstraintKind::Gt(a, b) => ConstraintKind::Gt(self.nexp(a, owner, shadow), self.nexp(b, owner, shadow)),
            ConstraintKind::Le(a, b) => ConstraintKind::Le(self.nexp(a, owner, shadow), self.nexp(b, owner, shadow)),
            ConstraintKind::Lt(a, b) => ConstraintKind::Lt(self.nexp(a, owner, shadow), self.nexp(b, owner, shadow)),
            ConstraintKind::Set { nexp, members } => ConstraintKind::Set {
                nexp: self.nexp(nexp, owner, shadow),
                members: members.clone(),
            },
            ConstraintKind::And(a, b) => ConstraintKind::And(
                Box::new(self.constraint(a, owner, shadow)),
                Box::new(self.constraint(b, owner, shadow)),
            ),
            ConstraintKind::Or(a, b) => ConstraintKind::Or(
                Box::new(self.constraint(a, owner, shadow)),
                Box::new(self.constraint(b, owner, shadow)),
            ),
            ConstraintKind::Not(a) => ConstraintKind::Not(Box::new(self.constraint(a, owner, shadow))),
            ConstraintKind::App { func, args } => ConstraintKind::App {
                func: name_in(self.out, from, func),
                args: args.iter().map(|a| self.arg(a, owner, shadow)).collect(),
            },
            ConstraintKind::Id(name) => ConstraintKind::Id(name_in(self.out, from, name)),
            ConstraintKind::Var(symbol) => ConstraintKind::Var(symbol_in(self.out, from, *symbol)),
            ConstraintKind::True => ConstraintKind::True,
            ConstraintKind::False => ConstraintKind::False,
        };
        Node { kind, ..c.clone() }
    }
}

impl Checker<'_> {
    pub(super) fn snapshot_typ(&self, typ: &Type, out: &mut SimpleInterner) -> Typ {
        Snap { out, fuel: 4096 }.typ(typ)
    }
}

/// Visit every expression and pattern of a definition mutably.
pub(super) trait Visit {
    fn exp(&mut self, exp: &mut Exp);
    fn pat(&mut self, pat: &mut Pat);
    fn lexp(&mut self, lexp: &mut LExp);
}

pub(super) fn walk_def_mut(def: &mut Def, v: &mut dyn Visit) {
    fn pat(p: &mut Pat, v: &mut dyn Visit) {
        v.pat(p);
        match &mut p.kind {
            PatKind::As { pat: inner, .. } | PatKind::Typ { pat: inner, .. } | PatKind::Var { pat: inner, .. } => {
                pat(inner, v);
            }
            PatKind::Or(a, b) | PatKind::Cons(a, b) => {
                pat(a, v);
                pat(b, v);
            }
            PatKind::Not(a) => pat(a, v),
            PatKind::App { args, .. } => args.iter_mut().for_each(|x| pat(x, v)),
            PatKind::Vector(ps) | PatKind::VectorConcat(ps) | PatKind::Tuple(ps) | PatKind::List(ps)
            | PatKind::StringAppend(ps) => ps.iter_mut().for_each(|x| pat(x, v)),
            PatKind::Struct { fields, .. } => fields.iter_mut().for_each(|(_, x)| pat(x, v)),
            PatKind::Lit(_) | PatKind::Wild | PatKind::Id(_) | PatKind::VectorSubrange { .. } => {}
        }
    }
    fn lexp(l: &mut LExp, v: &mut dyn Visit) {
        v.lexp(l);
        match &mut l.kind {
            LExpKind::Id(_) | LExpKind::Typ { .. } => {}
            LExpKind::Deref(e) => exp(e, v),
            LExpKind::Tuple(items) | LExpKind::VectorConcat(items) => {
                items.iter_mut().for_each(|x| lexp(x, v));
            }
            LExpKind::Vector { lexp: base, index } => {
                lexp(base, v);
                exp(index, v);
            }
            LExpKind::VectorRange { lexp: base, lo, hi } => {
                lexp(base, v);
                exp(lo, v);
                exp(hi, v);
            }
            LExpKind::Field { lexp: base, .. } => lexp(base, v),
        }
    }
    fn pexp(p: &mut PExp, v: &mut dyn Visit) {
        match &mut p.kind {
            PExpKind::PatExp { pat: pt, body } => {
                pat(pt, v);
                exp(body, v);
            }
            PExpKind::PatWhen { pat: pt, guard, body } => {
                pat(pt, v);
                exp(guard, v);
                exp(body, v);
            }
        }
    }
    fn exp(e: &mut Exp, v: &mut dyn Visit) {
        v.exp(e);
        match &mut e.kind {
            ExpKind::Block(items) | ExpKind::Tuple(items) | ExpKind::Vector(items) | ExpKind::List(items) => {
                items.iter_mut().for_each(|x| exp(x, v));
            }
            ExpKind::Id(_) | ExpKind::Lit(_) | ExpKind::Sizeof(_) | ExpKind::Config(_) | ExpKind::Ref(_)
            | ExpKind::Undef | ExpKind::InternalValue(_) | ExpKind::Constraint(_) => {}
            ExpKind::Typ { exp: inner, .. } => exp(inner, v),
            ExpKind::App { args, .. } => args.iter_mut().for_each(|x| exp(x, v)),
            ExpKind::If { cond, then_branch, else_branch } => {
                exp(cond, v);
                exp(then_branch, v);
                exp(else_branch, v);
            }
            ExpKind::Loop { measure, cond, body, .. } => {
                if let Some(m) = measure {
                    exp(m, v);
                }
                exp(cond, v);
                exp(body, v);
            }
            ExpKind::For { from, to, step, body, .. } => {
                exp(from, v);
                exp(to, v);
                exp(step, v);
                exp(body, v);
            }
            ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
                exp(a, v);
                exp(b, v);
            }
            ExpKind::Struct { fields, .. } => fields.iter_mut().for_each(|f| exp(&mut f.value, v)),
            ExpKind::StructUpdate { record, fields } => {
                exp(record, v);
                fields.iter_mut().for_each(|f| exp(&mut f.value, v));
            }
            ExpKind::Field { record, .. } => exp(record, v),
            ExpKind::Match { scrutinee, arms } => {
                exp(scrutinee, v);
                arms.iter_mut().for_each(|a| pexp(a, v));
            }
            ExpKind::Let { pat: pt, bind, body } | ExpKind::InternalPlet { pat: pt, bind, body } => {
                pat(pt, v);
                exp(bind, v);
                exp(body, v);
            }
            ExpKind::Assign { lexp: l, value } => {
                lexp(l, v);
                exp(value, v);
            }
            ExpKind::Return(x) | ExpKind::Exit(x) | ExpKind::Throw(x) | ExpKind::InternalReturn(x) => {
                exp(x, v);
            }
            ExpKind::Try { exp: tried, arms } => {
                exp(tried, v);
                arms.iter_mut().for_each(|a| pexp(a, v));
            }
            ExpKind::Assert { cond, msg } => {
                exp(cond, v);
                exp(msg, v);
            }
            ExpKind::Var { lexp: l, init, body } => {
                lexp(l, v);
                exp(init, v);
                exp(body, v);
            }
            ExpKind::InternalAssume { exp: inner, .. } => exp(inner, v),
        }
    }
    fn funcl(f: &mut Funcl, v: &mut dyn Visit) {
        pexp(&mut f.arm, v);
    }
    match &mut def.kind {
        DefKind::Fundef { rec, funcls, .. } => {
            if let RecOpt::Measure { pat: p, exp: e } = rec {
                pat(p, v);
                exp(e, v);
            }
            funcls.iter_mut().for_each(|f| funcl(f, v));
        }
        DefKind::Impl(f) => funcl(f, v),
        DefKind::Let { pat: p, exp: e } => {
            pat(p, v);
            exp(e, v);
        }
        DefKind::Register(spec) => {
            if let Some(e) = &mut spec.init {
                exp(e, v);
            }
        }
        DefKind::Measure { pat: p, exp: e, .. } => {
            pat(p, v);
            exp(e, v);
        }
        DefKind::Outcome { defs, .. } => {
            for d in defs {
                walk_def_mut(d, v);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets {
                set.funcls.iter_mut().for_each(|f| funcl(f, v));
            }
        }
        DefKind::Scattered(ScatteredDef::Funcl(f)) => funcl(f, v),
        _ => {}
    }
}
