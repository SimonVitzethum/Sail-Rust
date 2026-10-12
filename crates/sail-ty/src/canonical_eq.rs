//! Syntactic proof of canonical type-argument equality, without recursive
//! equality/serialization or assumptions about definition-local symbol IDs.
//!
//! Plus syntactic match-arm overlap (`pat_overlap`, Lane 09 pattern
//! coverage): whether two patterns of one `match` provably share (or
//! provably exclude) values. Only positive syntactic facts are reported —
//! constructor/struct/vector/list shapes and bare identifiers need
//! environment knowledge (union injectivity, enum members, record fields),
//! so they stay [`PatOverlap::Unknown`]. `Unknown` is always sound: the
//! checker may skip a redundancy hint, never a required arm.
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
/// Syntactic overlap of two patterns of one `match` (same scrutinee type).
///
/// - [`PatOverlap::Equal`]: both match provably the same value set (a later
///   arm is redundant).
/// - [`PatOverlap::Disjoint`]: no value matches both (different literal
///   values, or different tuple/list/vector arities — structural facts that
///   need no environment).
/// - [`PatOverlap::Unknown`]: everything else. In particular bare `Id`
///   patterns (binder matching everything versus an enum member matching
///   one value), constructor/mapping applications (injectivity needs the
///   union/record environment), struct/vector-concat/subrange shapes, and
///   cross-shape pairs (no scrutinee type is available at this layer).
///   Unknown is sound: a missed redundancy hint, never a wrong verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatOverlap {
    Equal,
    Disjoint,
    Unknown,
}

/// Fuel for [`pat_overlap`] recursion (pattern nesting depth). Exhaustion
/// yields [`PatOverlap::Unknown`], like the checker fuel policy elsewhere.
const MAX_OVERLAP_DEPTH: usize = 256;

/// Overlap of two patterns whose symbols live in `left`/`right`.
/// Cross-interner by construction so record/variant environments (which own
/// their interner) can compare against local patterns later.
pub(crate) fn pat_overlap(
    a: &Pat,
    b: &Pat,
    left: &SimpleInterner,
    right: &SimpleInterner,
) -> PatOverlap {
    overlap_fuel(a, b, left, right, MAX_OVERLAP_DEPTH)
}

fn overlap_fuel(
    a: &Pat,
    b: &Pat,
    left: &SimpleInterner,
    right: &SimpleInterner,
    fuel: usize,
) -> PatOverlap {
    if fuel == 0 {
        return PatOverlap::Unknown;
    }
    let fuel = fuel - 1;
    match (&a.kind, &b.kind) {
        (PatKind::Wild, PatKind::Wild) => PatOverlap::Equal,
        // `Id` needs the value environment (binder vs. enum member).
        (PatKind::Id(_), _) | (_, PatKind::Id(_)) => PatOverlap::Unknown,
        (PatKind::Lit(x), PatKind::Lit(y)) => lit_overlap(x, y, left, right),
        (PatKind::Tuple(xs), PatKind::Tuple(ys)) => {
            if xs.len() != ys.len() {
                return PatOverlap::Disjoint;
            }
            all_equal(xs, ys, left, right, fuel)
        }
        (PatKind::List(xs), PatKind::List(ys)) => {
            if xs.len() != ys.len() {
                return PatOverlap::Disjoint;
            }
            all_equal(xs, ys, left, right, fuel)
        }
        (PatKind::Vector(xs), PatKind::Vector(ys)) => {
            if xs.len() != ys.len() {
                return PatOverlap::Disjoint;
            }
            if xs.is_empty() {
                // `infer_pat` rejects empty vector patterns outright.
                return PatOverlap::Unknown;
            }
            all_equal(xs, ys, left, right, fuel)
        }
        // Same-head applications with pairwise-equal arguments match the
        // same set regardless of injectivity; anything weaker needs the
        // constructor/mapping environment — disjoint arguments stay
        // `Unknown` here (a non-injective mapping can merge them), unlike
        // the structural tuple/list/vector shapes below.
        (PatKind::App { func: f, args: xs }, PatKind::App { func: g, args: ys })
            if xs.len() == ys.len() && same_name(f, g, left, right) =>
        {
            match all_equal(xs, ys, left, right, fuel) {
                PatOverlap::Equal => PatOverlap::Equal,
                _ => PatOverlap::Unknown,
            }
        }
        // Annotations and `as` change neither match set.
        (PatKind::Typ { pat: x, .. }, _) => match overlap_fuel(x, b, left, right, fuel) {
            PatOverlap::Disjoint => PatOverlap::Disjoint,
            _ => PatOverlap::Unknown,
        },
        (_, PatKind::Typ { pat: y, .. }) => match overlap_fuel(a, y, left, right, fuel) {
            PatOverlap::Disjoint => PatOverlap::Disjoint,
            _ => PatOverlap::Unknown,
        },
        (PatKind::As { pat: x, .. }, _) => overlap_fuel(x, b, left, right, fuel),
        (_, PatKind::As { pat: y, .. }) => overlap_fuel(a, y, left, right, fuel),
        // `p1 | p2` overlaps `q` exactly when both sides agree. Two
        // unions compare aligned and swapped (both sound: equal unions
        // either way, disjoint only when all four cross pairs are), so
        // `1 | 2` vs `2 | 1` is equal, never misread as disjoint.
        (PatKind::Or(x1, x2), PatKind::Or(y1, y2)) => {
            let aligned = (
                overlap_fuel(x1, y1, left, right, fuel),
                overlap_fuel(x2, y2, left, right, fuel),
            );
            let swapped = (
                overlap_fuel(x1, y2, left, right, fuel),
                overlap_fuel(x2, y1, left, right, fuel),
            );
            if aligned == (PatOverlap::Equal, PatOverlap::Equal)
                || swapped == (PatOverlap::Equal, PatOverlap::Equal)
            {
                PatOverlap::Equal
            } else if aligned == (PatOverlap::Disjoint, PatOverlap::Disjoint)
                && swapped == (PatOverlap::Disjoint, PatOverlap::Disjoint)
            {
                PatOverlap::Disjoint
            } else {
                PatOverlap::Unknown
            }
        }
        (PatKind::Or(x1, x2), _) => join_or(
            overlap_fuel(x1, b, left, right, fuel),
            overlap_fuel(x2, b, left, right, fuel),
        ),
        (_, PatKind::Or(y1, y2)) => join_or(
            overlap_fuel(a, y1, left, right, fuel),
            overlap_fuel(a, y2, left, right, fuel),
        ),
        _ => PatOverlap::Unknown,
    }
}

/// Combine pairwise results for one structural shape (equal-length
/// tuples/lists/vectors, whose components are injective): all equal shares
/// the set, any disjoint pair splits it, anything weaker is
/// [`PatOverlap::Unknown`]. (Same-head applications use this only for the
/// `Equal` direction — see above.)
fn all_equal(
    xs: &[Pat],
    ys: &[Pat],
    left: &SimpleInterner,
    right: &SimpleInterner,
    fuel: usize,
) -> PatOverlap {
    let mut result = PatOverlap::Equal;
    for (x, y) in xs.iter().zip(ys.iter()) {
        match overlap_fuel(x, y, left, right, fuel) {
            PatOverlap::Disjoint => return PatOverlap::Disjoint,
            PatOverlap::Unknown => result = PatOverlap::Unknown,
            PatOverlap::Equal => {}
        }
    }
    result
}

/// `(p1 | p2)` vs `q`: equal only if both sides are, disjoint only if both
/// sides are — a split verdict says nothing about the union.
fn join_or(x: PatOverlap, y: PatOverlap) -> PatOverlap {
    match (x, y) {
        (PatOverlap::Equal, PatOverlap::Equal) => PatOverlap::Equal,
        (PatOverlap::Disjoint, PatOverlap::Disjoint) => PatOverlap::Disjoint,
        _ => PatOverlap::Unknown,
    }
}

fn same_name(a: &Name, b: &Name, left: &SimpleInterner, right: &SimpleInterner) -> bool {
    match (left.resolve(a.symbol), right.resolve(b.symbol)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Overlap of two literal patterns. Same-class literals decide
/// syntactically (numeric parse, boolean polarity, normalized bit digits,
/// resolved strings); cross-class pairs stay [`PatOverlap::Unknown`] — the
/// scrutinee type deciding them is not available at this layer.
fn lit_overlap(a: &Lit, b: &Lit, left: &SimpleInterner, right: &SimpleInterner) -> PatOverlap {
    match (&a.kind, &b.kind) {
        (LitKind::Num(x), LitKind::Num(y)) => match (left.resolve(*x), right.resolve(*y)) {
            (Some(x), Some(y)) => match (x.parse::<i128>(), y.parse::<i128>()) {
                (Ok(x), Ok(y)) if x == y => PatOverlap::Equal,
                (Ok(_), Ok(_)) => PatOverlap::Disjoint,
                _ => PatOverlap::Unknown,
            },
            _ => PatOverlap::Unknown,
        },
        (LitKind::True, LitKind::True) | (LitKind::False, LitKind::False) => PatOverlap::Equal,
        (LitKind::True, LitKind::False) | (LitKind::False, LitKind::True) => PatOverlap::Disjoint,
        (LitKind::Hex(x) | LitKind::Bin(x), LitKind::Hex(y) | LitKind::Bin(y)) => {
            match (left.resolve(*x), right.resolve(*y)) {
                (Some(x), Some(y)) if normalize_bits(x) == normalize_bits(y) => PatOverlap::Equal,
                (Some(_), Some(_)) => PatOverlap::Disjoint,
                _ => PatOverlap::Unknown,
            }
        }
        (LitKind::String(x), LitKind::String(y)) => match (left.resolve(*x), right.resolve(*y)) {
            (Some(x), Some(y)) if x == y => PatOverlap::Equal,
            (Some(_), Some(_)) => PatOverlap::Disjoint,
            _ => PatOverlap::Unknown,
        },
        (LitKind::Unit, LitKind::Unit) => PatOverlap::Equal,
        (LitKind::Real(x), LitKind::Real(y)) => match (left.resolve(*x), right.resolve(*y)) {
            // Decimal spellings vary (`1.50` vs `1.5`) without proving the
            // same value, so only identical spellings are equal.
            (Some(x), Some(y)) if x == y => PatOverlap::Equal,
            _ => PatOverlap::Unknown,
        },
        _ => PatOverlap::Unknown,
    }
}

/// Normalize hex/binary digits for comparison: case-insensitive, `_`
/// separators ignored (both spell the same bit pattern).
fn normalize_bits(digits: &str) -> String {
    digits
        .bytes()
        .filter(|b| *b != b'_')
        .map(|b| char::from(b.to_ascii_lowercase()))
        .collect()
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
