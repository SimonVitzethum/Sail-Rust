#![forbid(unsafe_code)]
//! Shared rewriting helpers: bottom-up folds and node constructors.

use sail_ast::{
    Annotation, Exp, ExpKind, FExp, Interner, LExp, LExpKind, Lit, LitKind, Name, Node, NodeId, PExp, PExpKind, Pat,
    PatKind, SimpleInterner, Span, Typ, TypKind,
};

use crate::ctx::Oracle;

/// Rewriting context for one definition.
pub struct Rw<'a> {
    /// The definition's interner, extended as passes create names.
    pub it: SimpleInterner,
    pub oracle: &'a dyn Oracle,
    counter: u32,
}

impl<'a> Rw<'a> {
    #[must_use]
    pub fn new(it: SimpleInterner, oracle: &'a dyn Oracle) -> Self {
        Rw { it, oracle, counter: 0 }
    }

    pub fn text(&self, name: &Name) -> String {
        Interner::resolve(&self.it, name.symbol).unwrap_or("?").to_owned()
    }

    pub fn name(&mut self, text: &str) -> Name {
        Name::dummy(self.it.intern(text))
    }

    /// `fresh_id prefix`: a name no earlier fresh name equals. Callers
    /// choose prefixes that the source cannot contain (`w__`, `tup__`, ...).
    pub fn fresh(&mut self, prefix: &str) -> Name {
        loop {
            let text = format!("{prefix}{}", self.counter);
            self.counter += 1;
            if self.it.lookup(&text).is_none() {
                return self.name(&text);
            }
        }
    }

    /// `unit` type in this interner.
    pub fn unit_typ(&mut self) -> Typ {
        let name = self.name("unit");
        Node::dummy(TypKind::Id(name))
    }

    pub fn bool_typ(&mut self) -> Typ {
        let name = self.name("bool");
        Node::dummy(TypKind::Id(name))
    }
}

/// Type annotation of a node.
#[must_use]
pub fn typ_of<K>(node: &Node<K>) -> Option<&Typ> {
    node.annotation.as_ref().and_then(|a| a.typ.as_deref())
}

pub fn set_typ<K>(node: &mut Node<K>, typ: Option<Typ>) {
    match typ {
        Some(typ) => node.annotation_mut().typ = Some(Box::new(typ)),
        None => {
            if let Some(a) = &mut node.annotation {
                a.typ = None;
            }
        }
    }
}

/// A node with the given kind and the span/annotation (including type) of `like`.
#[must_use]
pub fn exp_like(like: &Exp, kind: ExpKind) -> Exp {
    Node {
        id: NodeId::DUMMY,
        span: like.span,
        kind,
        annotation: like.annotation.clone(),
    }
}

#[must_use]
pub fn mk_exp(kind: ExpKind) -> Exp {
    Node::dummy(kind)
}

#[must_use]
pub fn mk_pat(kind: PatKind) -> Pat {
    Node::dummy(kind)
}

#[must_use]
pub fn unit_exp() -> Exp {
    mk_exp(ExpKind::Lit(Node::dummy(LitKind::Unit)))
}

#[must_use]
pub fn is_unit_lit(e: &Exp) -> bool {
    matches!(&e.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit))
}

#[must_use]
pub fn lit_kind(l: &Lit) -> LitKind {
    l.kind
}

#[must_use]
pub fn span_of(e: &Exp) -> Span {
    e.span
}

#[must_use]
pub fn annotation_of(e: &Exp) -> Option<Box<Annotation>> {
    e.annotation.clone()
}

/// Rebuild `e` with `f` applied to every direct sub-expression (including the
/// bodies and guards of match arms and index expressions of l-values).
pub fn map_children(e: Exp, f: &mut dyn FnMut(Exp) -> Exp) -> Exp {
    let Node { id, span, kind, annotation } = e;
    let b = |x: Box<Exp>, f: &mut dyn FnMut(Exp) -> Exp| Box::new(f(*x));
    let v = |xs: Vec<Exp>, f: &mut dyn FnMut(Exp) -> Exp| xs.into_iter().map(|x| f(x)).collect::<Vec<_>>();
    let kind = match kind {
        ExpKind::Block(items) => ExpKind::Block(v(items, f)),
        ExpKind::Tuple(items) => ExpKind::Tuple(v(items, f)),
        ExpKind::Vector(items) => ExpKind::Vector(v(items, f)),
        ExpKind::List(items) => ExpKind::List(v(items, f)),
        ExpKind::Typ { typ, exp } => ExpKind::Typ { typ, exp: b(exp, f) },
        ExpKind::App { func, args } => ExpKind::App { func, args: v(args, f) },
        ExpKind::If { cond, then_branch, else_branch } => ExpKind::If {
            cond: b(cond, f),
            then_branch: b(then_branch, f),
            else_branch: b(else_branch, f),
        },
        ExpKind::Loop { loop_kind, measure, cond, body } => ExpKind::Loop {
            loop_kind,
            measure: measure.map(|m| b(m, f)),
            cond: b(cond, f),
            body: b(body, f),
        },
        ExpKind::For { var, from, to, step, order, body } => ExpKind::For {
            var,
            from: b(from, f),
            to: b(to, f),
            step: b(step, f),
            order,
            body: b(body, f),
        },
        ExpKind::VectorAppend(x, y) => ExpKind::VectorAppend(b(x, f), b(y, f)),
        ExpKind::Cons(x, y) => ExpKind::Cons(b(x, f), b(y, f)),
        ExpKind::Struct { name, fields } => ExpKind::Struct {
            name,
            fields: fields.into_iter().map(|fe| map_fexp(fe, f)).collect(),
        },
        ExpKind::StructUpdate { record, fields } => ExpKind::StructUpdate {
            record: b(record, f),
            fields: fields.into_iter().map(|fe| map_fexp(fe, f)).collect(),
        },
        ExpKind::Field { record, field } => ExpKind::Field { record: b(record, f), field },
        ExpKind::Match { scrutinee, arms } => ExpKind::Match {
            scrutinee: b(scrutinee, f),
            arms: arms.into_iter().map(|a| map_pexp(a, f)).collect(),
        },
        ExpKind::Let { pat, bind, body } => ExpKind::Let { pat, bind: b(bind, f), body: b(body, f) },
        ExpKind::InternalPlet { pat, bind, body } => ExpKind::InternalPlet { pat, bind: b(bind, f), body: b(body, f) },
        ExpKind::Assign { lexp, value } => ExpKind::Assign {
            lexp: Box::new(map_lexp(*lexp, f)),
            value: b(value, f),
        },
        ExpKind::Return(x) => ExpKind::Return(b(x, f)),
        ExpKind::Exit(x) => ExpKind::Exit(b(x, f)),
        ExpKind::Throw(x) => ExpKind::Throw(b(x, f)),
        ExpKind::InternalReturn(x) => ExpKind::InternalReturn(b(x, f)),
        ExpKind::Try { exp, arms } => ExpKind::Try {
            exp: b(exp, f),
            arms: arms.into_iter().map(|a| map_pexp(a, f)).collect(),
        },
        ExpKind::Assert { cond, msg } => ExpKind::Assert { cond: b(cond, f), msg: b(msg, f) },
        ExpKind::Var { lexp, init, body } => ExpKind::Var {
            lexp: Box::new(map_lexp(*lexp, f)),
            init: b(init, f),
            body: b(body, f),
        },
        ExpKind::InternalAssume { constraint, exp } => ExpKind::InternalAssume { constraint, exp: b(exp, f) },
        other @ (ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_)) => other,
    };
    Node { id, span, kind, annotation }
}

fn map_fexp(fe: FExp, f: &mut dyn FnMut(Exp) -> Exp) -> FExp {
    FExp { value: f(fe.value), ..fe }
}

pub fn map_pexp(p: PExp, f: &mut dyn FnMut(Exp) -> Exp) -> PExp {
    let kind = match p.kind {
        PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat, body: f(body) },
        PExpKind::PatWhen { pat, guard, body } => PExpKind::PatWhen { pat, guard: f(guard), body: f(body) },
    };
    PExp { kind, ..p }
}

pub fn map_lexp(l: LExp, f: &mut dyn FnMut(Exp) -> Exp) -> LExp {
    let Node { id, span, kind, annotation } = l;
    let kind = match kind {
        LExpKind::Deref(e) => LExpKind::Deref(Box::new(f(*e))),
        LExpKind::Tuple(items) => LExpKind::Tuple(items.into_iter().map(|x| map_lexp(x, f)).collect()),
        LExpKind::VectorConcat(items) => {
            LExpKind::VectorConcat(items.into_iter().map(|x| map_lexp(x, f)).collect())
        }
        LExpKind::Vector { lexp, index } => LExpKind::Vector {
            lexp: Box::new(map_lexp(*lexp, f)),
            index: Box::new(f(*index)),
        },
        LExpKind::VectorRange { lexp, lo, hi } => LExpKind::VectorRange {
            lexp: Box::new(map_lexp(*lexp, f)),
            lo: Box::new(f(*lo)),
            hi: Box::new(f(*hi)),
        },
        LExpKind::Field { lexp, field } => LExpKind::Field { lexp: Box::new(map_lexp(*lexp, f)), field },
        other @ (LExpKind::Id(_) | LExpKind::Typ { .. }) => other,
    };
    Node { id, span, kind, annotation }
}

/// Bottom-up fold: children first, then `f` on the rebuilt node
/// (`fold_exp alg` with `e_aux = f`).
pub fn fold_exp(e: Exp, f: &mut dyn FnMut(Exp) -> Exp) -> Exp {
    let rebuilt = map_children(e, &mut |child| fold_exp(child, f));
    f(rebuilt)
}

/// Apply `f` to every function clause body of a definition (and the
/// measure expression), mirroring `rewrite_ast_base` for function bodies.
pub fn map_def_bodies(def: &mut sail_ast::Def, f: &mut dyn FnMut(Exp) -> Exp) {
    use sail_ast::DefKind;
    match &mut def.kind {
        DefKind::Fundef { funcls, .. } => {
            for funcl in funcls {
                let arm = std::mem::replace(
                    &mut funcl.arm,
                    PExp {
                        id: NodeId::DUMMY,
                        span: Span::DUMMY,
                        kind: PExpKind::PatExp { pat: mk_pat(PatKind::Wild), body: unit_exp() },
                    },
                );
                funcl.arm = map_arm_top(arm, f);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &mut set.funcls {
                    let arm = std::mem::replace(
                        &mut funcl.arm,
                        PExp {
                            id: NodeId::DUMMY,
                            span: Span::DUMMY,
                            kind: PExpKind::PatExp { pat: mk_pat(PatKind::Wild), body: unit_exp() },
                        },
                    );
                    funcl.arm = map_arm_top(arm, f);
                }
            }
        }
        DefKind::Let { exp, .. } => {
            let e = std::mem::replace(exp, unit_exp());
            *exp = f(e);
        }
        DefKind::Register(spec) => {
            if let Some(e) = spec.init.take() {
                spec.init = Some(f(e));
            }
        }
        _ => {}
    }
}

fn map_arm_top(arm: PExp, f: &mut dyn FnMut(Exp) -> Exp) -> PExp {
    let kind = match arm.kind {
        PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat, body: f(body) },
        PExpKind::PatWhen { pat, guard, body } => PExpKind::PatWhen { pat, guard: f(guard), body: f(body) },
    };
    PExp { kind, ..arm }
}
