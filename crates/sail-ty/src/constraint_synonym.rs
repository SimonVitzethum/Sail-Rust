#![forbid(unsafe_code)]
//! Type-level synonyms of kind `Int` and `Bool`
//! (`type MAX_PL : Int = 256`, `type is_VL('vl : Int) -> Bool = ..`).
//! The SMT emitter has no synonym table, so applications are expanded into
//! the using definition's own symbols before they are stored in a scheme or
//! turned into a proof obligation (OCaml `Env.expand_nexp_synonyms` /
//! `expand_constraint_synonyms`).

use sail_ast::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug)]
pub struct ConstraintSynonym {
    pub binders: Vec<KOpt>,
    pub body: Constraint,
    pub interner: Arc<SimpleInterner>,
}

#[derive(Debug)]
pub struct NexpSynonym {
    pub binders: Vec<KOpt>,
    pub body: NExp,
    pub interner: Arc<SimpleInterner>,
}

#[derive(Clone)]
pub enum Synonym {
    Constraint(Arc<ConstraintSynonym>),
    NExp(Arc<NexpSynonym>),
}

pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<Synonym>;

struct Copier<'a, 'b> {
    from: &'a SimpleInterner,
    to: &'b mut SimpleInterner,
    subst: HashMap<String, TypArg>,
    lookup: Lookup<'a>,
}

impl Copier<'_, '_> {
    fn name(&mut self, name: &Name) -> Name {
        let text = self.from.resolve(name.symbol).unwrap_or_default();
        Name {
            symbol: self.to.intern(text),
            ..name.clone()
        }
    }
    fn symbol(&mut self, symbol: Symbol) -> Symbol {
        let text = self.from.resolve(symbol).unwrap_or_default();
        self.to.intern(text)
    }
    fn synonym(&self, name: &Name) -> Option<Synonym> {
        (self.lookup)(self.from.resolve(name.symbol)?)
    }
    fn binding(&mut self, binders: &[KOpt], interner: &SimpleInterner, args: &[TypArg]) -> HashMap<String, TypArg> {
        let args: Vec<TypArg> = args.iter().map(|a| self.arg(a)).collect();
        binders
            .iter()
            .zip(args)
            .filter_map(|(binder, arg)| Some((interner.resolve(binder.kid)?.to_owned(), arg)))
            .collect()
    }
    fn nexp(&mut self, n: &NExp) -> NExp {
        let kind = match &n.kind {
            NExpKind::Id(name) => {
                if let Some(Synonym::NExp(syn)) = self.synonym(name) {
                    if syn.binders.is_empty() {
                        return Copier {
                            from: &syn.interner.clone(),
                            to: &mut *self.to,
                            subst: HashMap::new(),
                            lookup: self.lookup,
                        }
                        .nexp(&syn.body);
                    }
                }
                NExpKind::Id(self.name(name))
            }
            NExpKind::Var(symbol) => {
                let text = self.from.resolve(*symbol).unwrap_or_default();
                if let Some(TypArg {
                    kind: TypArgKind::NExp(arg),
                    ..
                }) = self.subst.get(text)
                {
                    return arg.clone();
                }
                NExpKind::Var(self.symbol(*symbol))
            }
            NExpKind::Constant(text) => NExpKind::Constant(text.clone()),
            NExpKind::App { func, args } => {
                let typ_args: Vec<TypArg> = args
                    .iter()
                    .map(|a| Node {
                        kind: TypArgKind::NExp(a.clone()),
                        id: a.id,
                        span: a.span,
                        annotation: None,
                    })
                    .collect();
                if let Some(Synonym::NExp(syn)) = self.synonym(func) {
                    if syn.binders.len() == args.len() {
                        let subst = self.binding(&syn.binders, &syn.interner, &typ_args);
                        return Copier {
                            from: &syn.interner.clone(),
                            to: &mut *self.to,
                            subst,
                            lookup: self.lookup,
                        }
                        .nexp(&syn.body);
                    }
                }
                NExpKind::App {
                    func: self.name(func),
                    args: args.iter().map(|a| self.nexp(a)).collect(),
                }
            }
            NExpKind::If {
                cond,
                then_branch,
                else_branch,
            } => NExpKind::If {
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
        Node { kind, ..n.clone() }
    }
    fn arg(&mut self, a: &TypArg) -> TypArg {
        let kind = match &a.kind {
            TypArgKind::NExp(n) => TypArgKind::NExp(self.nexp(n)),
            TypArgKind::Bool(c) => TypArgKind::Bool(Box::new(self.constraint(c))),
            TypArgKind::Typ(t) => TypArgKind::Typ(self.typ(t)),
        };
        Node { kind, ..a.clone() }
    }
    fn constraint(&mut self, c: &Constraint) -> Constraint {
        let kind = match &c.kind {
            ConstraintKind::Equal(a, b) => {
                ConstraintKind::Equal(Box::new(self.arg(a)), Box::new(self.arg(b)))
            }
            ConstraintKind::NotEqual(a, b) => {
                ConstraintKind::NotEqual(Box::new(self.arg(a)), Box::new(self.arg(b)))
            }
            ConstraintKind::Ge(a, b) => ConstraintKind::Ge(self.nexp(a), self.nexp(b)),
            ConstraintKind::Gt(a, b) => ConstraintKind::Gt(self.nexp(a), self.nexp(b)),
            ConstraintKind::Le(a, b) => ConstraintKind::Le(self.nexp(a), self.nexp(b)),
            ConstraintKind::Lt(a, b) => ConstraintKind::Lt(self.nexp(a), self.nexp(b)),
            ConstraintKind::Set { nexp, members } => ConstraintKind::Set {
                nexp: self.nexp(nexp),
                members: members.clone(),
            },
            ConstraintKind::And(a, b) => {
                ConstraintKind::And(Box::new(self.constraint(a)), Box::new(self.constraint(b)))
            }
            ConstraintKind::Or(a, b) => {
                ConstraintKind::Or(Box::new(self.constraint(a)), Box::new(self.constraint(b)))
            }
            ConstraintKind::Not(a) => ConstraintKind::Not(Box::new(self.constraint(a))),
            ConstraintKind::App { func, args } => {
                if let Some(Synonym::Constraint(syn)) = self.synonym(func) {
                    if syn.binders.len() == args.len() {
                        let subst = self.binding(&syn.binders, &syn.interner, args);
                        return Copier {
                            from: &syn.interner.clone(),
                            to: &mut *self.to,
                            subst,
                            lookup: self.lookup,
                        }
                        .constraint(&syn.body);
                    }
                }
                ConstraintKind::App {
                    func: self.name(func),
                    args: args.iter().map(|a| self.arg(a)).collect(),
                }
            }
            ConstraintKind::Id(name) => ConstraintKind::Id(self.name(name)),
            ConstraintKind::Var(symbol) => {
                let text = self.from.resolve(*symbol).unwrap_or_default();
                if let Some(TypArg {
                    kind: TypArgKind::Bool(arg),
                    ..
                }) = self.subst.get(text)
                {
                    return (**arg).clone();
                }
                ConstraintKind::Var(self.symbol(*symbol))
            }
            ConstraintKind::True => ConstraintKind::True,
            ConstraintKind::False => ConstraintKind::False,
        };
        Node { kind, ..c.clone() }
    }
    fn typ(&mut self, t: &Typ) -> Typ {
        let kind = match &t.kind {
            TypKind::Unknown => TypKind::Unknown,
            TypKind::Id(name) => TypKind::Id(self.name(name)),
            TypKind::Var(symbol) => TypKind::Var(self.symbol(*symbol)),
            TypKind::Fn { args, ret } => TypKind::Fn {
                args: args.iter().map(|a| self.typ(a)).collect(),
                ret: Box::new(self.typ(ret)),
            },
            TypKind::Bidir { lhs, rhs } => TypKind::Bidir {
                lhs: Box::new(self.typ(lhs)),
                rhs: Box::new(self.typ(rhs)),
            },
            TypKind::Tuple(items) => TypKind::Tuple(items.iter().map(|a| self.typ(a)).collect()),
            TypKind::App { name, args } => TypKind::App {
                name: self.name(name),
                args: args.iter().map(|a| self.arg(a)).collect(),
            },
            TypKind::Exist {
                binders,
                constraint,
                typ,
            } => TypKind::Exist {
                binders: binders
                    .iter()
                    .map(|binder| KOpt {
                        kid: self.symbol(binder.kid),
                        ..*binder
                    })
                    .collect(),
                constraint: Box::new(self.constraint(constraint)),
                typ: Box::new(self.typ(typ)),
            },
        };
        Node { kind, ..t.clone() }
    }
    fn quant(&mut self, q: &QuantItem) -> QuantItem {
        match q {
            QuantItem::Id(binder) => QuantItem::Id(KOpt {
                kid: self.symbol(binder.kid),
                ..*binder
            }),
            QuantItem::Constraint(c) => QuantItem::Constraint(self.constraint(c)),
        }
    }
}

/// Expand synonyms in a scheme; the result owns a copy of the interner.
pub fn expand_scheme(
    typschm: &TypSchm,
    interner: &Arc<SimpleInterner>,
    lookup: Lookup<'_>,
) -> (TypSchm, Arc<SimpleInterner>) {
    let mut to = (**interner).clone();
    let mut copier = Copier {
        from: interner,
        to: &mut to,
        subst: HashMap::new(),
        lookup,
    };
    let quant = typschm.quant.iter().map(|q| copier.quant(q)).collect();
    let typ = copier.typ(&typschm.typ);
    (TypSchm { quant, typ }, Arc::new(to))
}

/// Expand synonyms in a type; symbols stay valid in the cloned interner.
pub fn expand_typ(typ: &Typ, interner: &Arc<SimpleInterner>, lookup: Lookup<'_>) -> (Typ, Arc<SimpleInterner>) {
    let mut to = (**interner).clone();
    let typ = Copier {
        from: interner,
        to: &mut to,
        subst: HashMap::new(),
        lookup,
    }
    .typ(typ);
    (typ, Arc::new(to))
}

/// Expand synonyms in a constraint, producing it in `to`'s symbol space.
pub fn expand_constraint(c: &Constraint, to: &mut SimpleInterner, lookup: Lookup<'_>) -> Constraint {
    let from = to.clone();
    Copier {
        from: &from,
        to,
        subst: HashMap::new(),
        lookup,
    }
    .constraint(c)
}

/// Expand synonyms in a numeric expression.
pub fn expand_nexp(n: &NExp, to: &mut SimpleInterner, lookup: Lookup<'_>) -> NExp {
    let from = to.clone();
    Copier {
        from: &from,
        to,
        subst: HashMap::new(),
        lookup,
    }
    .nexp(n)
}

enum Visit<'a> {
    Typ(&'a Typ),
    Arg(&'a TypArg),
    NExp(&'a NExp),
    Constraint(&'a Constraint),
}

/// Whether a type, scheme part or constraint applies a known synonym.
/// Pre-check that avoids cloning interners for synonym-free types.
fn applies_synonym(roots: Vec<Visit<'_>>, interner: &SimpleInterner, lookup: Lookup<'_>) -> bool {
    let known = |name: &Name| interner.resolve(name.symbol).is_some_and(|n| lookup(n).is_some());
    let mut stack = roots;
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Typ(t) => match &t.kind {
                TypKind::Fn { args, ret } => {
                    stack.extend(args.iter().map(Visit::Typ));
                    stack.push(Visit::Typ(ret));
                }
                TypKind::Bidir { lhs, rhs } => {
                    stack.push(Visit::Typ(lhs));
                    stack.push(Visit::Typ(rhs));
                }
                TypKind::Tuple(items) => stack.extend(items.iter().map(Visit::Typ)),
                TypKind::App { args, .. } => stack.extend(args.iter().map(Visit::Arg)),
                TypKind::Exist { constraint, typ, .. } => {
                    stack.push(Visit::Constraint(constraint));
                    stack.push(Visit::Typ(typ));
                }
                _ => {}
            },
            Visit::Arg(a) => match &a.kind {
                TypArgKind::Typ(t) => stack.push(Visit::Typ(t)),
                TypArgKind::NExp(n) => stack.push(Visit::NExp(n)),
                TypArgKind::Bool(c) => stack.push(Visit::Constraint(c)),
            },
            Visit::NExp(n) => match &n.kind {
                NExpKind::Id(name) => {
                    if known(name) {
                        return true;
                    }
                }
                NExpKind::App { func, args } => {
                    if known(func) {
                        return true;
                    }
                    stack.extend(args.iter().map(Visit::NExp));
                }
                NExpKind::If { cond, then_branch, else_branch } => {
                    stack.push(Visit::Constraint(cond));
                    stack.push(Visit::NExp(then_branch));
                    stack.push(Visit::NExp(else_branch));
                }
                NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
                    stack.push(Visit::NExp(a));
                    stack.push(Visit::NExp(b));
                }
                NExpKind::Exp(a) | NExpKind::Neg(a) => stack.push(Visit::NExp(a)),
                _ => {}
            },
            Visit::Constraint(c) => match &c.kind {
                ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
                    stack.push(Visit::Arg(a));
                    stack.push(Visit::Arg(b));
                }
                ConstraintKind::Ge(a, b)
                | ConstraintKind::Gt(a, b)
                | ConstraintKind::Le(a, b)
                | ConstraintKind::Lt(a, b) => {
                    stack.push(Visit::NExp(a));
                    stack.push(Visit::NExp(b));
                }
                ConstraintKind::Set { nexp, .. } => stack.push(Visit::NExp(nexp)),
                ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
                    stack.push(Visit::Constraint(a));
                    stack.push(Visit::Constraint(b));
                }
                ConstraintKind::Not(a) => stack.push(Visit::Constraint(a)),
                ConstraintKind::App { func, args } => {
                    if known(func) {
                        return true;
                    }
                    stack.extend(args.iter().map(Visit::Arg));
                }
                ConstraintKind::Id(name) => {
                    if known(name) {
                        return true;
                    }
                }
                _ => {}
            },
        }
    }
    false
}

pub fn scheme_applies_synonym(typschm: &TypSchm, interner: &SimpleInterner, lookup: Lookup<'_>) -> bool {
    let mut roots = vec![Visit::Typ(&typschm.typ)];
    for q in &typschm.quant {
        if let QuantItem::Constraint(c) = q {
            roots.push(Visit::Constraint(c));
        }
    }
    applies_synonym(roots, interner, lookup)
}

pub fn typ_applies_synonym(typ: &Typ, interner: &SimpleInterner, lookup: Lookup<'_>) -> bool {
    applies_synonym(vec![Visit::Typ(typ)], interner, lookup)
}

pub fn constraint_applies_synonym(c: &Constraint, interner: &SimpleInterner, lookup: Lookup<'_>) -> bool {
    applies_synonym(vec![Visit::Constraint(c)], interner, lookup)
}

/// Expand synonyms in the payload types of a record, union or bitfield
/// definition. `None` when no payload applies a synonym.
pub fn expand_type_def(
    def: &TypeDef,
    interner: &Arc<SimpleInterner>,
    lookup: Lookup<'_>,
) -> Option<(TypeDef, Arc<SimpleInterner>)> {
    let applies = match &def.kind {
        TypeDefKind::Record { fields, .. } => fields
            .iter()
            .any(|(_, typ)| typ_applies_synonym(typ, interner, lookup)),
        TypeDefKind::Variant { ctors, .. } => ctors
            .iter()
            .any(|ctor| typ_applies_synonym(&ctor.typ, interner, lookup)),
        TypeDefKind::Bitfield { typ, .. } => typ_applies_synonym(typ, interner, lookup),
        _ => false,
    };
    if !applies {
        return None;
    }
    let mut to = (**interner).clone();
    let mut expanded = def.clone();
    {
        let mut copier = Copier {
            from: interner,
            to: &mut to,
            subst: HashMap::new(),
            lookup,
        };
        match &mut expanded.kind {
            TypeDefKind::Record { fields, .. } => {
                for (_, typ) in fields.iter_mut() {
                    *typ = copier.typ(typ);
                }
            }
            TypeDefKind::Variant { ctors, .. } => {
                for ctor in ctors.iter_mut() {
                    ctor.typ = copier.typ(&ctor.typ);
                }
            }
            TypeDefKind::Bitfield { typ, .. } => *typ = copier.typ(typ),
            _ => {}
        }
    }
    Some((expanded, Arc::new(to)))
}
