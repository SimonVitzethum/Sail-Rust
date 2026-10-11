//! Syntactic proof of canonical type-argument equality, without recursive
//! equality/serialization or assumptions about definition-local symbol IDs.
use sail_ast::*;

enum Pair<'a> {
    Numeric(&'a NExp, &'a NExp),
    Boolean(&'a Constraint, &'a Constraint),
    Argument(&'a TypArg, &'a TypArg),
    Type(&'a Typ, &'a Typ),
}

pub(crate) fn argument(
    a: &TypArg,
    b: &TypArg,
    left: &SimpleInterner,
    right: &SimpleInterner,
    same_scope: bool,
) -> bool {
    equal(vec![Pair::Argument(a, b)], left, right, same_scope)
}
pub(crate) fn typ(
    a: &Typ,
    b: &Typ,
    left: &SimpleInterner,
    right: &SimpleInterner,
    same_scope: bool,
) -> bool {
    equal(vec![Pair::Type(a, b)], left, right, same_scope)
}
pub(crate) fn constraint(
    a: &Constraint,
    b: &Constraint,
    left: &SimpleInterner,
    right: &SimpleInterner,
    same_scope: bool,
) -> bool {
    equal(vec![Pair::Boolean(a, b)], left, right, same_scope)
}
fn equal(
    mut work: Vec<Pair<'_>>,
    left: &SimpleInterner,
    right: &SimpleInterner,
    same_scope: bool,
) -> bool {
    let names = |a, b| match (left.resolve(a), right.resolve(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    while let Some(pair) = work.pop() {
        match pair {
            Pair::Numeric(a, b) => match (&a.kind, &b.kind) {
                (NExpKind::Constant(a), NExpKind::Constant(b)) if a == b => {}
                (NExpKind::Var(a), NExpKind::Var(b)) if same_scope && names(*a, *b) => {}
                (NExpKind::Id(a), NExpKind::Id(b)) if names(a.symbol, b.symbol) => {}
                (NExpKind::Times(a, b), NExpKind::Times(c, d))
                | (NExpKind::Sum(a, b), NExpKind::Sum(c, d))
                | (NExpKind::Minus(a, b), NExpKind::Minus(c, d)) => {
                    work.push(Pair::Numeric(a, c));
                    work.push(Pair::Numeric(b, d));
                }
                (NExpKind::Neg(a), NExpKind::Neg(b)) | (NExpKind::Exp(a), NExpKind::Exp(b)) => {
                    work.push(Pair::Numeric(a, b))
                }
                (
                    NExpKind::If {
                        cond: a,
                        then_branch: b,
                        else_branch: c,
                    },
                    NExpKind::If {
                        cond: d,
                        then_branch: e,
                        else_branch: f,
                    },
                ) => {
                    work.push(Pair::Boolean(a, d));
                    work.push(Pair::Numeric(b, e));
                    work.push(Pair::Numeric(c, f));
                }
                (NExpKind::App { func: a, args: aa }, NExpKind::App { func: b, args: ba })
                    if names(a.symbol, b.symbol) && aa.len() == ba.len() =>
                {
                    work.extend(aa.iter().zip(ba).map(|(a, b)| Pair::Numeric(a, b)))
                }
                _ => return false,
            },
            Pair::Boolean(a, b) => match (&a.kind, &b.kind) {
                (ConstraintKind::True, ConstraintKind::True)
                | (ConstraintKind::False, ConstraintKind::False) => {}
                (ConstraintKind::Var(a), ConstraintKind::Var(b)) if same_scope && names(*a, *b) => {
                }
                (ConstraintKind::Id(a), ConstraintKind::Id(b)) if names(a.symbol, b.symbol) => {}
                (ConstraintKind::Equal(a, b), ConstraintKind::Equal(c, d))
                | (ConstraintKind::NotEqual(a, b), ConstraintKind::NotEqual(c, d)) => {
                    work.push(Pair::Argument(a, c));
                    work.push(Pair::Argument(b, d));
                }
                (ConstraintKind::Ge(a, b), ConstraintKind::Ge(c, d))
                | (ConstraintKind::Gt(a, b), ConstraintKind::Gt(c, d))
                | (ConstraintKind::Le(a, b), ConstraintKind::Le(c, d))
                | (ConstraintKind::Lt(a, b), ConstraintKind::Lt(c, d)) => {
                    work.push(Pair::Numeric(a, c));
                    work.push(Pair::Numeric(b, d));
                }
                (ConstraintKind::And(a, b), ConstraintKind::And(c, d))
                | (ConstraintKind::Or(a, b), ConstraintKind::Or(c, d)) => {
                    work.push(Pair::Boolean(a, c));
                    work.push(Pair::Boolean(b, d));
                }
                (ConstraintKind::Not(a), ConstraintKind::Not(b)) => work.push(Pair::Boolean(a, b)),
                (
                    ConstraintKind::Set {
                        nexp: a,
                        members: aa,
                    },
                    ConstraintKind::Set {
                        nexp: b,
                        members: ba,
                    },
                ) if aa == ba => work.push(Pair::Numeric(a, b)),
                (
                    ConstraintKind::App { func: a, args: aa },
                    ConstraintKind::App { func: b, args: ba },
                ) if names(a.symbol, b.symbol) && aa.len() == ba.len() => {
                    work.extend(aa.iter().zip(ba).map(|(a, b)| Pair::Argument(a, b)))
                }
                _ => return false,
            },
            Pair::Argument(a, b) => work.push(match (&a.kind, &b.kind) {
                (TypArgKind::Typ(a), TypArgKind::Typ(b)) => Pair::Type(a, b),
                (TypArgKind::NExp(a), TypArgKind::NExp(b)) => Pair::Numeric(a, b),
                (TypArgKind::Bool(a), TypArgKind::Bool(b)) => Pair::Boolean(a, b),
                _ => return false,
            }),
            Pair::Type(a, b) => match (&a.kind, &b.kind) {
                (TypKind::Id(a), TypKind::Id(b)) if names(a.symbol, b.symbol) => {}
                (TypKind::Var(a), TypKind::Var(b)) if same_scope && names(*a, *b) => {}
                (TypKind::Tuple(aa), TypKind::Tuple(ba)) if aa.len() == ba.len() => {
                    work.extend(aa.iter().zip(ba).map(|(a, b)| Pair::Type(a, b)))
                }
                (TypKind::App { name: a, args: aa }, TypKind::App { name: b, args: ba })
                    if names(a.symbol, b.symbol) && aa.len() == ba.len() =>
                {
                    work.extend(aa.iter().zip(ba).map(|(a, b)| Pair::Argument(a, b)))
                }
                (TypKind::Fn { args: aa, ret: a }, TypKind::Fn { args: ba, ret: b })
                    if aa.len() == ba.len() =>
                {
                    work.push(Pair::Type(a, b));
                    work.extend(aa.iter().zip(ba).map(|(a, b)| Pair::Type(a, b)));
                }
                (TypKind::Bidir { lhs: a, rhs: b }, TypKind::Bidir { lhs: c, rhs: d }) => {
                    work.push(Pair::Type(a, c));
                    work.push(Pair::Type(b, d));
                }
                // Existential alpha-equivalence needs fresh binders/proofs.
                _ => return false,
            },
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_numeric_equivalence_uses_names_scopes_and_an_explicit_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut left = SimpleInterner::new();
                let mut right = SimpleInterner::new();
                right.intern("different slot zero");
                let mut a = Node::dummy(NExpKind::Var(left.intern("'n")));
                let mut b = Node::dummy(NExpKind::Var(right.intern("'n")));
                for _ in 0..10_000 {
                    a = Node::dummy(NExpKind::Sum(
                        Box::new(a),
                        Box::new(Node::dummy(NExpKind::Constant("1".into()))),
                    ));
                    b = Node::dummy(NExpKind::Sum(
                        Box::new(b),
                        Box::new(Node::dummy(NExpKind::Constant("1".into()))),
                    ));
                }
                let a = Node::dummy(TypArgKind::NExp(a));
                let b = Node::dummy(TypArgKind::NExp(b));
                assert!(argument(&a, &b, &left, &right, true));
                assert!(!argument(&a, &b, &left, &right, false));
                for arg in [a, b] {
                    sail_ast::storage::drop_definition(Node::dummy(DefKind::Type(Node::dummy(
                        TypeDefKind::Abbrev {
                            name: Name::dummy(Symbol::DUMMY),
                            quant: vec![],
                            arg,
                        },
                    ))));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
