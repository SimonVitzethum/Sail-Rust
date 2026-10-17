#![forbid(unsafe_code)]
//! Copy AST fragments between definition interners (symbols are indices into
//! per-definition tables).

use sail_ast::{
    Annotation, Constraint, ConstraintKind, Exp, ExpKind, FExp, Interner, KOpt, LExp, LExpKind, Lit, LitKind, Name,
    NExp, NExpKind, Node, PExp, PExpKind, Pat, PatKind, SimpleInterner, StructName, Symbol, Typ, TypArg, TypArgKind,
    TypKind, TypPat, TypPatKind,
};

pub struct Remap<'a> {
    pub from: &'a SimpleInterner,
    pub to: &'a mut SimpleInterner,
}

impl Remap<'_> {
    pub fn sym(&mut self, s: Symbol) -> Symbol {
        let text = Interner::resolve(self.from, s).unwrap_or("?").to_owned();
        self.to.intern(&text)
    }

    pub fn name(&mut self, n: &Name) -> Name {
        Name { symbol: self.sym(n.symbol), ..n.clone() }
    }

    fn annotation(&mut self, a: &Option<Box<Annotation>>) -> Option<Box<Annotation>> {
        a.as_ref().map(|a| {
            let mut a = a.clone();
            if let Some(t) = &a.typ {
                a.typ = Some(Box::new(self.typ(t)));
            }
            a
        })
    }

    fn node<K, L>(&mut self, n: &Node<K>, kind: L) -> Node<L> {
        Node { id: n.id, span: n.span, kind, annotation: self.annotation(&n.annotation) }
    }

    pub fn lit(&mut self, l: &Lit) -> Lit {
        let kind = match l.kind {
            LitKind::Num(s) => LitKind::Num(self.sym(s)),
            LitKind::Hex(s) => LitKind::Hex(self.sym(s)),
            LitKind::Bin(s) => LitKind::Bin(self.sym(s)),
            LitKind::String(s) => LitKind::String(self.sym(s)),
            LitKind::Real(s) => LitKind::Real(self.sym(s)),
            k => k,
        };
        self.node(l, kind)
    }

    pub fn typ(&mut self, t: &Typ) -> Typ {
        let kind = match &t.kind {
            TypKind::Unknown => TypKind::Unknown,
            TypKind::Id(n) => TypKind::Id(self.name(n)),
            TypKind::Var(s) => TypKind::Var(self.sym(*s)),
            TypKind::Fn { args, ret } => TypKind::Fn {
                args: args.iter().map(|a| self.typ(a)).collect(),
                ret: Box::new(self.typ(ret)),
            },
            TypKind::Bidir { lhs, rhs } => {
                TypKind::Bidir { lhs: Box::new(self.typ(lhs)), rhs: Box::new(self.typ(rhs)) }
            }
            TypKind::Tuple(items) => TypKind::Tuple(items.iter().map(|a| self.typ(a)).collect()),
            TypKind::App { name, args } => TypKind::App {
                name: self.name(name),
                args: args.iter().map(|a| self.arg(a)).collect(),
            },
            TypKind::Exist { binders, constraint, typ } => TypKind::Exist {
                binders: binders.iter().map(|b| KOpt { kid: self.sym(b.kid), ..*b }).collect(),
                constraint: Box::new(self.constraint(constraint)),
                typ: Box::new(self.typ(typ)),
            },
        };
        self.node(t, kind)
    }

    pub fn arg(&mut self, a: &TypArg) -> TypArg {
        let kind = match &a.kind {
            TypArgKind::NExp(n) => TypArgKind::NExp(self.nexp(n)),
            TypArgKind::Typ(t) => TypArgKind::Typ(self.typ(t)),
            TypArgKind::Bool(c) => TypArgKind::Bool(Box::new(self.constraint(c))),
        };
        self.node(a, kind)
    }

    pub fn nexp(&mut self, n: &NExp) -> NExp {
        let kind = match &n.kind {
            NExpKind::Id(x) => NExpKind::Id(self.name(x)),
            NExpKind::Var(s) => NExpKind::Var(self.sym(*s)),
            NExpKind::Constant(c) => NExpKind::Constant(c.clone()),
            NExpKind::App { func, args } => NExpKind::App {
                func: self.name(func),
                args: args.iter().map(|a| self.nexp(a)).collect(),
            },
            NExpKind::If { cond, then_branch, else_branch } => NExpKind::If {
                cond: Box::new(self.constraint(cond)),
                then_branch: Box::new(self.nexp(then_branch)),
                else_branch: Box::new(self.nexp(else_branch)),
            },
            NExpKind::Times(a, b) => NExpKind::Times(Box::new(self.nexp(a)), Box::new(self.nexp(b))),
            NExpKind::Sum(a, b) => NExpKind::Sum(Box::new(self.nexp(a)), Box::new(self.nexp(b))),
            NExpKind::Minus(a, b) => NExpKind::Minus(Box::new(self.nexp(a)), Box::new(self.nexp(b))),
            NExpKind::Exp(a) => NExpKind::Exp(Box::new(self.nexp(a))),
            NExpKind::Neg(a) => NExpKind::Neg(Box::new(self.nexp(a))),
        };
        self.node(n, kind)
    }

    pub fn constraint(&mut self, c: &Constraint) -> Constraint {
        let kind = match &c.kind {
            ConstraintKind::Equal(a, b) => ConstraintKind::Equal(Box::new(self.arg(a)), Box::new(self.arg(b))),
            ConstraintKind::NotEqual(a, b) => ConstraintKind::NotEqual(Box::new(self.arg(a)), Box::new(self.arg(b))),
            ConstraintKind::Ge(a, b) => ConstraintKind::Ge(self.nexp(a), self.nexp(b)),
            ConstraintKind::Gt(a, b) => ConstraintKind::Gt(self.nexp(a), self.nexp(b)),
            ConstraintKind::Le(a, b) => ConstraintKind::Le(self.nexp(a), self.nexp(b)),
            ConstraintKind::Lt(a, b) => ConstraintKind::Lt(self.nexp(a), self.nexp(b)),
            ConstraintKind::Set { nexp, members } => {
                ConstraintKind::Set { nexp: self.nexp(nexp), members: members.clone() }
            }
            ConstraintKind::And(a, b) => ConstraintKind::And(Box::new(self.constraint(a)), Box::new(self.constraint(b))),
            ConstraintKind::Or(a, b) => ConstraintKind::Or(Box::new(self.constraint(a)), Box::new(self.constraint(b))),
            ConstraintKind::Not(a) => ConstraintKind::Not(Box::new(self.constraint(a))),
            ConstraintKind::App { func, args } => ConstraintKind::App {
                func: self.name(func),
                args: args.iter().map(|a| self.arg(a)).collect(),
            },
            ConstraintKind::Id(n) => ConstraintKind::Id(self.name(n)),
            ConstraintKind::Var(s) => ConstraintKind::Var(self.sym(*s)),
            ConstraintKind::True => ConstraintKind::True,
            ConstraintKind::False => ConstraintKind::False,
        };
        self.node(c, kind)
    }

    fn typ_pat(&mut self, p: &TypPat) -> TypPat {
        let kind = match &p.kind {
            TypPatKind::Wild => TypPatKind::Wild,
            TypPatKind::Var(s) => TypPatKind::Var(self.sym(*s)),
            TypPatKind::App { name, args } => TypPatKind::App {
                name: self.name(name),
                args: args.iter().map(|a| self.typ_pat(a)).collect(),
            },
        };
        self.node(p, kind)
    }

    fn struct_name(&mut self, n: &StructName) -> StructName {
        match n {
            StructName::Id(x) => StructName::Id(self.name(x)),
            StructName::Anon => StructName::Anon,
        }
    }

    pub fn pat(&mut self, p: &Pat) -> Pat {
        let ps = |this: &mut Self, xs: &[Pat]| -> Vec<Pat> { xs.iter().map(|x| this.pat(x)).collect() };
        let kind = match &p.kind {
            PatKind::Lit(l) => PatKind::Lit(self.lit(l)),
            PatKind::Wild => PatKind::Wild,
            PatKind::Or(a, b) => PatKind::Or(Box::new(self.pat(a)), Box::new(self.pat(b))),
            PatKind::Not(a) => PatKind::Not(Box::new(self.pat(a))),
            PatKind::As { pat, id } => PatKind::As { pat: Box::new(self.pat(pat)), id: self.name(id) },
            PatKind::Typ { typ, pat } => PatKind::Typ { typ: self.typ(typ), pat: Box::new(self.pat(pat)) },
            PatKind::Id(n) => PatKind::Id(self.name(n)),
            PatKind::Var { pat, typ_pat } => {
                PatKind::Var { pat: Box::new(self.pat(pat)), typ_pat: self.typ_pat(typ_pat) }
            }
            PatKind::App { func, args } => PatKind::App { func: self.name(func), args: ps(self, args) },
            PatKind::Vector(xs) => PatKind::Vector(ps(self, xs)),
            PatKind::VectorConcat(xs) => PatKind::VectorConcat(ps(self, xs)),
            PatKind::VectorSubrange { id, lo, hi } => {
                PatKind::VectorSubrange { id: self.name(id), lo: lo.clone(), hi: hi.clone() }
            }
            PatKind::Tuple(xs) => PatKind::Tuple(ps(self, xs)),
            PatKind::List(xs) => PatKind::List(ps(self, xs)),
            PatKind::Cons(a, b) => PatKind::Cons(Box::new(self.pat(a)), Box::new(self.pat(b))),
            PatKind::StringAppend(xs) => PatKind::StringAppend(ps(self, xs)),
            PatKind::Struct { name, fields, wildcard } => PatKind::Struct {
                name: self.struct_name(name),
                fields: fields.iter().map(|(n, p)| (self.name(n), self.pat(p))).collect(),
                wildcard: *wildcard,
            },
        };
        self.node(p, kind)
    }

    pub fn lexp(&mut self, l: &LExp) -> LExp {
        let kind = match &l.kind {
            LExpKind::Id(n) => LExpKind::Id(self.name(n)),
            LExpKind::Deref(e) => LExpKind::Deref(Box::new(self.exp(e))),
            LExpKind::Typ { typ, id } => LExpKind::Typ { typ: self.typ(typ), id: self.name(id) },
            LExpKind::Tuple(xs) => LExpKind::Tuple(xs.iter().map(|x| self.lexp(x)).collect()),
            LExpKind::VectorConcat(xs) => LExpKind::VectorConcat(xs.iter().map(|x| self.lexp(x)).collect()),
            LExpKind::Vector { lexp, index } => {
                LExpKind::Vector { lexp: Box::new(self.lexp(lexp)), index: Box::new(self.exp(index)) }
            }
            LExpKind::VectorRange { lexp, lo, hi } => LExpKind::VectorRange {
                lexp: Box::new(self.lexp(lexp)),
                lo: Box::new(self.exp(lo)),
                hi: Box::new(self.exp(hi)),
            },
            LExpKind::Field { lexp, field } => {
                LExpKind::Field { lexp: Box::new(self.lexp(lexp)), field: self.name(field) }
            }
        };
        self.node(l, kind)
    }

    fn fexp(&mut self, f: &FExp) -> FExp {
        FExp { id: f.id, span: f.span, field: self.name(&f.field), value: self.exp(&f.value) }
    }

    pub fn pexp(&mut self, p: &PExp) -> PExp {
        let kind = match &p.kind {
            PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat: self.pat(pat), body: self.exp(body) },
            PExpKind::PatWhen { pat, guard, body } => {
                PExpKind::PatWhen { pat: self.pat(pat), guard: self.exp(guard), body: self.exp(body) }
            }
        };
        PExp { id: p.id, span: p.span, kind }
    }

    pub fn exp(&mut self, e: &Exp) -> Exp {
        let es = |this: &mut Self, xs: &[Exp]| -> Vec<Exp> { xs.iter().map(|x| this.exp(x)).collect() };
        let b = |this: &mut Self, x: &Exp| Box::new(this.exp(x));
        let kind = match &e.kind {
            ExpKind::Block(xs) => ExpKind::Block(es(self, xs)),
            ExpKind::Id(n) => ExpKind::Id(self.name(n)),
            ExpKind::Lit(l) => ExpKind::Lit(self.lit(l)),
            ExpKind::Typ { typ, exp } => ExpKind::Typ { typ: self.typ(typ), exp: b(self, exp) },
            ExpKind::App { func, args } => ExpKind::App { func: self.name(func), args: es(self, args) },
            ExpKind::Tuple(xs) => ExpKind::Tuple(es(self, xs)),
            ExpKind::If { cond, then_branch, else_branch } => ExpKind::If {
                cond: b(self, cond),
                then_branch: b(self, then_branch),
                else_branch: b(self, else_branch),
            },
            ExpKind::Loop { loop_kind, measure, cond, body } => ExpKind::Loop {
                loop_kind: *loop_kind,
                measure: measure.as_ref().map(|m| b(self, m)),
                cond: b(self, cond),
                body: b(self, body),
            },
            ExpKind::For { var, from, to, step, order, body } => ExpKind::For {
                var: self.name(var),
                from: b(self, from),
                to: b(self, to),
                step: b(self, step),
                order: *order,
                body: b(self, body),
            },
            ExpKind::Vector(xs) => ExpKind::Vector(es(self, xs)),
            ExpKind::VectorAppend(x, y) => ExpKind::VectorAppend(b(self, x), b(self, y)),
            ExpKind::List(xs) => ExpKind::List(es(self, xs)),
            ExpKind::Cons(x, y) => ExpKind::Cons(b(self, x), b(self, y)),
            ExpKind::Struct { name, fields } => ExpKind::Struct {
                name: self.struct_name(name),
                fields: fields.iter().map(|f| self.fexp(f)).collect(),
            },
            ExpKind::StructUpdate { record, fields } => ExpKind::StructUpdate {
                record: b(self, record),
                fields: fields.iter().map(|f| self.fexp(f)).collect(),
            },
            ExpKind::Field { record, field } => {
                ExpKind::Field { record: b(self, record), field: self.name(field) }
            }
            ExpKind::Match { scrutinee, arms } => ExpKind::Match {
                scrutinee: b(self, scrutinee),
                arms: arms.iter().map(|a| self.pexp(a)).collect(),
            },
            ExpKind::Let { pat, bind, body } => {
                ExpKind::Let { pat: self.pat(pat), bind: b(self, bind), body: b(self, body) }
            }
            ExpKind::InternalPlet { pat, bind, body } => {
                ExpKind::InternalPlet { pat: self.pat(pat), bind: b(self, bind), body: b(self, body) }
            }
            ExpKind::Assign { lexp, value } => {
                ExpKind::Assign { lexp: Box::new(self.lexp(lexp)), value: b(self, value) }
            }
            ExpKind::Sizeof(n) => ExpKind::Sizeof(self.nexp(n)),
            ExpKind::Return(x) => ExpKind::Return(b(self, x)),
            ExpKind::Exit(x) => ExpKind::Exit(b(self, x)),
            ExpKind::Config(k) => ExpKind::Config(k.clone()),
            ExpKind::Ref(n) => ExpKind::Ref(self.name(n)),
            ExpKind::Throw(x) => ExpKind::Throw(b(self, x)),
            ExpKind::Try { exp, arms } => ExpKind::Try {
                exp: b(self, exp),
                arms: arms.iter().map(|a| self.pexp(a)).collect(),
            },
            ExpKind::Assert { cond, msg } => ExpKind::Assert { cond: b(self, cond), msg: b(self, msg) },
            ExpKind::Var { lexp, init, body } => ExpKind::Var {
                lexp: Box::new(self.lexp(lexp)),
                init: b(self, init),
                body: b(self, body),
            },
            ExpKind::Undef => ExpKind::Undef,
            ExpKind::InternalReturn(x) => ExpKind::InternalReturn(b(self, x)),
            ExpKind::InternalValue(v) => ExpKind::InternalValue(v.clone()),
            ExpKind::InternalAssume { constraint, exp } => {
                ExpKind::InternalAssume { constraint: self.constraint(constraint), exp: b(self, exp) }
            }
            ExpKind::Constraint(c) => ExpKind::Constraint(self.constraint(c)),
        };
        self.node(e, kind)
    }
}
