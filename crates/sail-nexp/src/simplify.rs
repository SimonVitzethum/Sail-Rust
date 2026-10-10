#![forbid(unsafe_code)]
//! Simplification and canonical normal forms for [`NExp`]
//! and [`NConstraint`].
//!
//! All functions are pure and total. [`simplify`] applies one rewrite
//! pass (constant folding plus arithmetic identities);
//! [`normalize`] iterates it to a fixpoint (bounded) and sorts the
//! operands of associative-commutative `Sum`/`Times` for a canonical
//! syntactic form.
//!
//! Semantics contract: simplification preserves [`NExp::eval`] exactly —
//! for every environment, `simplify(e).eval(env) == e.eval(env)`,
//! *including* `None` (unbound variables, division by zero, overflow).
//! Rules that OCaml's `nexp_simp`/`constraint_simp` (`ast_util.ml`) lack
//! (`0 * x -> 0`, `a - a -> 0`, `x >= x -> True`, `if c then t else t ->
//! t`, `x % 1 -> 0`, `abs(2^x) -> 2^x`, empty-`Set` folding,
//! mixed-sort equality folding) are deliberately absent: under a partial
//! valuation they turn `None` into `Some`, and in the SMT (total-`Int`)
//! reading they belong to the solver, not to a syntactic rewrite. The
//! same holds for [`normalize`]/[`normalize_constraint`], which only
//! iterate this pass plus order-canonicalization.

use std::borrow::Cow;

use crate::expr::{NConstraint, NExp, NTyp, TypArg};

/// One rewrite pass over an [`NExp`].
pub fn simplify(e: &NExp) -> NExp {
    simplify_cow(e).into_owned()
}

/// One rewrite pass over an [`NExp`], borrowing `e` wherever the
/// rewrite leaves the node untouched.
///
/// RAM diet (Lane 90): the old `simplify` rebuilt every node (fresh
/// `Box`es, cloned leaves and payload strings) even when no rule fired.
/// A `Borrowed` result allocates nothing — no `clone`, no fresh `Box`.
/// Owned outputs are value-identical to [`simplify`]; identity rules
/// (`x + 0 -> x`, `--y -> y`, …) return the already-simplified child
/// directly instead of cloning it.
pub fn simplify_cow(e: &NExp) -> Cow<'_, NExp> {
    match e {
        NExp::Id(_) | NExp::Var(_) | NExp::Constant(_) => Cow::Borrowed(e),
        NExp::App(f, args) => {
            let simp: Vec<Cow<'_, NExp>> = args.iter().map(simplify_cow).collect();
            if simp.iter().all(|c| matches!(c, Cow::Borrowed(_))) && !app_folds(f, args) {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(simplify_app(
                    f,
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        NExp::If(c, t, b) => {
            let (cc, tc, bc) =
                (simplify_constraint_cow(c), simplify_cow(t), simplify_cow(b));
            let all_borrowed = matches!(cc, Cow::Borrowed(_))
                && matches!(tc, Cow::Borrowed(_))
                && matches!(bc, Cow::Borrowed(_));
            if all_borrowed {
                // Children untouched: only the `True`/`False` rules can
                // still fire (checked on refs, no allocation). There is
                // deliberately no `t == b -> t` rule (OCaml has none;
                // with an undefined condition it would turn `None` into
                // `Some`).
                let is_true = matches!(cc.as_ref(), NConstraint::True);
                let is_false = matches!(cc.as_ref(), NConstraint::False);
                if is_true {
                    tc
                } else if is_false {
                    bc
                } else {
                    Cow::Borrowed(e)
                }
            } else {
                let (c, t, b) = (cc.into_owned(), tc.into_owned(), bc.into_owned());
                Cow::Owned(match c {
                    NConstraint::True => t,
                    NConstraint::False => b,
                    _ => NExp::If(Box::new(c), Box::new(t), Box::new(b)),
                })
            }
        }
        NExp::Times(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            times_join(e, ra, rb)
        }
        NExp::Sum(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            sum_join(e, ra, rb)
        }
        NExp::Minus(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            minus_join(e, ra, rb)
        }
        NExp::Exp(x) => exp_join(e, simplify_cow(x)),
        NExp::Neg(x) => neg_join(e, simplify_cow(x)),
    }
}

fn num(e: &NExp) -> Option<i128> {
    match e {
        NExp::Constant(v) => Some(*v),
        _ => None,
    }
}

fn is_zero(e: &NExp) -> bool {
    num(e) == Some(0)
}

fn is_one(e: &NExp) -> bool {
    num(e) == Some(1)
}

/// Owned `sum` kernel: the pre-Lane-90 `sum` body, verbatim in
/// behaviour. Recursive compositions call the other `*_owned` kernels
/// (all inputs here are already-owned, so moving is optimal).
fn sum_owned(a: NExp, b: NExp) -> NExp {
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_add(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Sum(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_zero(&x) => y,
        (x, y) if is_zero(&y) => x,
        // -(a) + -(b) -> -(a + b) (kept as one negation for canonicity:
        // both operand orders reach the same term).
        (NExp::Neg(x), NExp::Neg(y)) => neg_owned(sum_owned(*x, *y)),
        // x + (-y) -> x - y, in either operand order.
        (x, NExp::Neg(y)) => minus_owned(x, *y),
        (NExp::Neg(x), y) => minus_owned(y, *x),
        (x, y) => NExp::Sum(Box::new(x), Box::new(y)),
    }
}

/// Join already-simplified `sum` children with the [`sum_owned`] rules.
///
/// Single home of the fold conditions: identity arms return the child
/// [`Cow`] directly (no clone either way); a node whose children are
/// all borrowed and which no rule fires on is borrowed as a whole.
/// Anything else delegates to [`sum_owned`] on materialized children,
/// so values always equal the old `simplify`.
fn sum_join<'a>(orig: &'a NExp, a: Cow<'a, NExp>, b: Cow<'a, NExp>) -> Cow<'a, NExp> {
    // Identities first: order-independent and value-safe against every
    // `sum_owned` arm (`0 + 0` reaches `0` either way).
    let take_b = is_zero(a.as_ref());
    let take_a = is_zero(b.as_ref());
    // Coarse fold test: a superset of the `sum_owned` folding arms
    // (`(Const, Const)`, `(Neg, Neg)`, `(x, -y)`, `(-x, y)`). Saying
    // `true` too often only costs a rebuild, never a wrong value;
    // saying `false` here means `sum_owned` would rebuild identically.
    let folds = matches!(
        (a.as_ref(), b.as_ref()),
        (NExp::Constant(_), NExp::Constant(_))
            | (NExp::Neg(_), NExp::Neg(_))
            | (_, NExp::Neg(_))
            | (NExp::Neg(_), _)
    );
    let unchanged = matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_));
    if take_b {
        return b;
    }
    if take_a {
        return a;
    }
    if !folds {
        return if unchanged {
            Cow::Borrowed(orig)
        } else {
            Cow::Owned(NExp::Sum(Box::new(a.into_owned()), Box::new(b.into_owned())))
        };
    }
    Cow::Owned(sum_owned(a.into_owned(), b.into_owned()))
}

/// Owned `minus` kernel: constant folding, `x - 0`, `0 - x` and the
/// `(x + c1) - c2` chain rule. There is deliberately no `a - a -> 0`
/// rule (OCaml's `nexp_simp` has none; with an exceptional `a` such as
/// `1 div 0` it would turn `None` into `Some(0)`).
fn minus_owned(a: NExp, b: NExp) -> NExp {
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_sub(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Minus(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_zero(&y) => x,
        (x, y) if is_zero(&x) => neg_owned(y),
        // (x + c1) - c2 -> x + (c1 - c2)
        (NExp::Sum(x, c1), NExp::Constant(c2)) if matches!(*c1, NExp::Constant(_)) => {
            let NExp::Constant(c1v) = *c1 else {
                unreachable!()
            };
            match c1v.checked_sub(c2) {
                Some(0) => *x,
                Some(v) => sum_owned(*x, NExp::Constant(v)),
                None => NExp::Minus(
                    Box::new(NExp::Sum(Box::new(*x), Box::new(NExp::Constant(c1v)))),
                    Box::new(NExp::Constant(c2)),
                ),
            }
        }
        (x, y) => NExp::Minus(Box::new(x), Box::new(y)),
    }
}

/// Join already-simplified `minus` children (see [`sum_join`]).
fn minus_join<'a>(orig: &'a NExp, a: Cow<'a, NExp>, b: Cow<'a, NExp>) -> Cow<'a, NExp> {
    // `x - 0 -> x` returns the child directly. There is no `a - a -> 0`
    // shortcut (see [`minus_owned`]).
    let take_a = is_zero(b.as_ref());
    let folds = match (a.as_ref(), b.as_ref()) {
        (NExp::Constant(_), NExp::Constant(_)) => true,
        (NExp::Sum(_, c1), NExp::Constant(_)) => matches!(&**c1, NExp::Constant(_)),
        _ if is_zero(a.as_ref()) => true,
        _ => false,
    };
    let unchanged = matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_));
    if take_a {
        return a;
    }
    if !folds {
        return if unchanged {
            Cow::Borrowed(orig)
        } else {
            Cow::Owned(NExp::Minus(Box::new(a.into_owned()), Box::new(b.into_owned())))
        };
    }
    Cow::Owned(minus_owned(a.into_owned(), b.into_owned()))
}

/// Owned `times` kernel: constant folding, `1`-identities, `(-1) * x`
/// and `(-a) * (-b)` (all `None`-preserving and exact). There is
/// deliberately no `0 * x -> 0` rule (OCaml's `nexp_simp` has none;
/// with an exceptional `x` such as `1 div 0` it would turn `None`
/// into `Some(0)`).
fn times_owned(a: NExp, b: NExp) -> NExp {
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_mul(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Times(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_one(&x) => y,
        (x, y) if is_one(&y) => x,
        // (-1) * x -> -x
        (NExp::Constant(-1), x) | (x, NExp::Constant(-1)) => neg_owned(x),
        // (-a) * (-b) -> a * b
        (NExp::Neg(a), NExp::Neg(b)) => times_owned(*a, *b),
        (x, y) => NExp::Times(Box::new(x), Box::new(y)),
    }
}

/// Join already-simplified `times` children (see [`sum_join`]).
fn times_join<'a>(orig: &'a NExp, a: Cow<'a, NExp>, b: Cow<'a, NExp>) -> Cow<'a, NExp> {
    // `1 * y -> y`, `x * 1 -> x`: order-independent, value-safe against
    // every `times_owned` arm (`1 * 1` reaches `1` either way).
    let take_b = is_one(a.as_ref());
    let take_a = is_one(b.as_ref());
    let folds = matches!(
        (a.as_ref(), b.as_ref()),
        (NExp::Constant(_), NExp::Constant(_))
            | (NExp::Constant(-1), _)
            | (_, NExp::Constant(-1))
            | (NExp::Neg(_), NExp::Neg(_))
    );
    let unchanged = matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_));
    if take_b {
        return b;
    }
    if take_a {
        return a;
    }
    if !folds {
        return if unchanged {
            Cow::Borrowed(orig)
        } else {
            Cow::Owned(NExp::Times(Box::new(a.into_owned()), Box::new(b.into_owned())))
        };
    }
    Cow::Owned(times_owned(a.into_owned(), b.into_owned()))
}

/// Owned `neg` kernel: the pre-Lane-90 `neg` body, verbatim in behaviour.
fn neg_owned(x: NExp) -> NExp {
    match x {
        NExp::Constant(v) => match v.checked_neg() {
            Some(w) => NExp::Constant(w),
            None => NExp::Neg(Box::new(NExp::Constant(v))),
        },
        NExp::Neg(y) => *y,
        // -(a - b) -> b - a
        NExp::Minus(a, b) => minus_owned(*b, *a),
        y => NExp::Neg(Box::new(y)),
    }
}

/// Join an already-simplified `neg` child (see [`sum_join`]).
///
/// `--y -> y` borrows the inner node when it is borrowed: no clone, no
/// box. Folds that need owned values materialize this node only (one
/// clone); the old code rebuilt the whole spine instead.
fn neg_join<'a>(orig: &'a NExp, x: Cow<'a, NExp>) -> Cow<'a, NExp> {
    match x {
        Cow::Borrowed(xr) => match xr {
            NExp::Neg(y) => Cow::Borrowed(y),
            NExp::Constant(v) => match v.checked_neg() {
                Some(w) => Cow::Owned(NExp::Constant(w)),
                // Rebuild would equal `orig`.
                None => Cow::Borrowed(orig),
            },
            // `-(a - b) -> b - a` needs owned values.
            NExp::Minus(_, _) => Cow::Owned(neg_owned(xr.clone())),
            _ => Cow::Borrowed(orig),
        },
        Cow::Owned(xo) => Cow::Owned(neg_owned(xo)),
    }
}

/// Join an already-simplified `exp` child (see [`sum_join`]).
fn exp_join<'a>(orig: &'a NExp, x: Cow<'a, NExp>) -> Cow<'a, NExp> {
    match x {
        Cow::Borrowed(xr) => match xr {
            NExp::Constant(0) => Cow::Owned(NExp::Constant(1)),
            // `2^e` for `e < 0` rebuilds to `orig`.
            NExp::Constant(e) if *e < 0 => Cow::Borrowed(orig),
            NExp::Constant(e) if *e < 127 => Cow::Owned(NExp::Constant(1i128 << (*e as u32))),
            _ => Cow::Borrowed(orig),
        },
        Cow::Owned(xo) => Cow::Owned(match xo {
            NExp::Constant(0) => NExp::Constant(1),
            NExp::Constant(e) if e < 0 => NExp::Exp(Box::new(NExp::Constant(e))),
            NExp::Constant(e) if e < 127 => NExp::Constant(1i128 << (e as u32)),
            x => NExp::Exp(Box::new(x)),
        }),
    }
}

/// Whether [`simplify_app`] would return anything other than a
/// rebuild of `args` (a fold or an identity returning a child).
///
/// Checked on refs so [`simplify_cow`] can borrow `App` nodes no rule
/// fires on. The conditions mirror the [`simplify_app`] arms exactly
/// (including which arms rebuild identically); a wrong `true` only
/// costs a rebuild, never a wrong value.
fn app_folds(f: &str, args: &[NExp]) -> bool {
    match f {
        "div" => match args {
            [a, b] => match (num(a), num(b)) {
                // `(_, Some(0))` rebuilds identically; `(Some, Some)` folds
                // unless checked division fails (rebuilds identically).
                (Some(x), Some(y)) => x.checked_div(y).is_some(),
                _ => is_one(b) || (is_zero(a) && matches!(num(b), Some(d) if d != 0)),
            },
            // Wrong arity rebuilds identically.
            _ => false,
        },
        "mod" => match args {
            [a, b] => match (num(a), num(b)) {
                (Some(x), Some(y)) => x.checked_rem(y).is_some(),
                // No `x % 1 -> 0` rule (same partiality reason as
                // `0 * x`; OCaml folds constants only).
                _ => is_zero(a) && matches!(num(b), Some(d) if d != 0),
            },
            _ => false,
        },
        "abs" => match args {
            [a] => match num(a) {
                // `checked_abs` fails only for `i128::MIN` (rebuilds).
                Some(v) => v.checked_abs().is_some(),
                // Nested `abs` unwraps one level; anything else rebuilds
                // identically. There is deliberately no
                // `abs(2^x) -> 2^x` rule: it claims `2^x >= 0` without a
                // backed premise (OCaml folds `abs` constants only).
                None => {
                    matches!(a, NExp::App(g, _) if g.as_str() == "abs")
                }
            },
            _ => false,
        },
        // Uninterpreted functions only simplify their arguments.
        _ => false,
    }
}

/// Owned `App` kernel: the pre-Lane-90 `simplify_app` body, verbatim in
/// behaviour, but taking already-simplified owned arguments by value so
/// every rebuild *moves* (the old `&[NExp]` version cloned `a`/`b`/the
/// fallback `args` vec on every path).
fn simplify_app(f: &str, args: Vec<NExp>) -> NExp {
    match f {
        "div" => match <[NExp; 2]>::try_from(args) {
            Ok([a, b]) => match (num(&a), num(&b)) {
                (_, Some(0)) => NExp::div(a, b),
                (Some(x), Some(y)) => match x.checked_div(y) {
                    Some(v) => NExp::Constant(v),
                    None => NExp::div(NExp::Constant(x), NExp::Constant(y)),
                },
                _ if is_one(&b) => a,
                // `0 div d -> 0` only for known non-zero `d` (`0 div 0`
                // is undefined and must stay unfolded).
                _ if is_zero(&a) && matches!(num(&b), Some(d) if d != 0) => {
                    NExp::Constant(0)
                }
                _ => NExp::div(a, b),
            },
            Err(args) => NExp::App(f.to_string(), args),
        },
        "mod" => match <[NExp; 2]>::try_from(args) {
            Ok([a, b]) => match (num(&a), num(&b)) {
                (_, Some(0)) => NExp::modulo(a, b),
                (Some(x), Some(y)) => match x.checked_rem(y) {
                    Some(v) => NExp::Constant(v),
                    None => NExp::modulo(NExp::Constant(x), NExp::Constant(y)),
                },
                _ if is_zero(&a) && matches!(num(&b), Some(d) if d != 0) => {
                    NExp::Constant(0)
                }
                _ => NExp::modulo(a, b),
            },
            Err(args) => NExp::App(f.to_string(), args),
        },
        "abs" => match <[NExp; 1]>::try_from(args) {
            Ok([a]) => match num(&a) {
                Some(v) => match v.checked_abs() {
                    Some(w) => NExp::Constant(w),
                    None => NExp::abs(NExp::Constant(v)),
                },
                None => match a {
                    // abs is idempotent on applications of abs (exact:
                    // `abs(abs(x))` and `abs(x)` agree on `None` too).
                    NExp::App(g, inner) if g == "abs" => {
                        NExp::abs(inner.into_iter().next().unwrap_or(NExp::Constant(0)))
                    }
                    NExp::Constant(_) => unreachable!(),
                    _ => NExp::abs(a),
                },
            },
            Err(args) => NExp::App(f.to_string(), args),
        },
        _ => NExp::App(f.to_string(), args),
    }
}

/// One rewrite pass over an [`NConstraint`].
pub fn simplify_constraint(c: &NConstraint) -> NConstraint {
    simplify_constraint_cow(c).into_owned()
}

/// One rewrite pass over an [`NConstraint`], borrowing `c` wherever the
/// rewrite leaves the node untouched (see [`simplify_cow`]).
///
/// Owned outputs are value-identical to [`simplify_constraint`].
pub fn simplify_constraint_cow(c: &NConstraint) -> Cow<'_, NConstraint> {
    match c {
        NConstraint::True | NConstraint::False | NConstraint::Id(_) | NConstraint::Var(_) => {
            Cow::Borrowed(c)
        }
        NConstraint::Equal(a, b) => {
            let (ra, rb) = (simplify_typarg_cow(a), simplify_typarg_cow(b));
            eq_join(c, ra, rb, NConstraint::Equal, true)
        }
        NConstraint::NotEqual(a, b) => {
            let (ra, rb) = (simplify_typarg_cow(a), simplify_typarg_cow(b));
            eq_join(c, ra, rb, NConstraint::NotEqual, false)
        }
        NConstraint::Ge(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            cmp_join(c, ra, rb, |x, y| x >= y, NConstraint::Ge)
        }
        NConstraint::Gt(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            cmp_join(c, ra, rb, |x, y| x > y, NConstraint::Gt)
        }
        NConstraint::Le(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            cmp_join(c, ra, rb, |x, y| x <= y, NConstraint::Le)
        }
        NConstraint::Lt(a, b) => {
            let (ra, rb) = (simplify_cow(a), simplify_cow(b));
            cmp_join(c, ra, rb, |x, y| x < y, NConstraint::Lt)
        }
        NConstraint::Set(n, vs) => {
            let rn = simplify_cow(n);
            if let Some(v) = num(rn.as_ref()) {
                return Cow::Owned(if vs.contains(&v) {
                    NConstraint::True
                } else {
                    NConstraint::False
                });
            }
            // No `[] -> False` fold for a non-constant member: OCaml's
            // `constraint_simp` folds `NC_set` only against constants,
            // and with an undefined member it would turn `None` into
            // `Some(false)`.
            if matches!(rn, Cow::Borrowed(_)) && is_sorted_dedup(vs) {
                // Unchanged child and an already-canonical set: also spares
                // the `vs.clone()` the old pass did unconditionally.
                Cow::Borrowed(c)
            } else {
                let mut sorted = vs.clone();
                sorted.sort();
                sorted.dedup();
                Cow::Owned(NConstraint::Set(Box::new(rn.into_owned()), sorted))
            }
        }
        NConstraint::App(f, args) => {
            // Uninterpreted: only simplify the arguments, never fold.
            let simp: Vec<Cow<'_, TypArg>> = args.iter().map(simplify_typarg_cow).collect();
            if simp.iter().all(|x| matches!(x, Cow::Borrowed(_))) {
                Cow::Borrowed(c)
            } else {
                Cow::Owned(NConstraint::App(
                    f.clone(),
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        NConstraint::And(a, b) => {
            let (ra, rb) = (simplify_constraint_cow(a), simplify_constraint_cow(b));
            and_join(c, ra, rb)
        }
        NConstraint::Or(a, b) => {
            let (ra, rb) = (simplify_constraint_cow(a), simplify_constraint_cow(b));
            or_join(c, ra, rb)
        }
        NConstraint::Not(x) => {
            let rx = simplify_constraint_cow(x);
            match rx {
                Cow::Borrowed(xr) => match xr {
                    NConstraint::True => Cow::Owned(NConstraint::False),
                    NConstraint::False => Cow::Owned(NConstraint::True),
                    // `!!y -> y` borrows the inner node.
                    NConstraint::Not(y) => Cow::Borrowed(y),
                    _ => Cow::Borrowed(c),
                },
                Cow::Owned(xo) => Cow::Owned(match xo {
                    NConstraint::True => NConstraint::False,
                    NConstraint::False => NConstraint::True,
                    NConstraint::Not(y) => *y,
                    y => NConstraint::Not(Box::new(y)),
                }),
            }
        }
    }
}

/// Whether `vs` is already sorted and deduplicated (strictly
/// increasing), i.e. the `Set` canonical form needs no rebuild.
fn is_sorted_dedup(vs: &[i128]) -> bool {
    vs.windows(2).all(|w| w[0] < w[1])
}

/// Join already-simplified `Equal`/`NotEqual` children over `typ_arg`
/// sorts.
///
/// `is_equal` selects the polarity (`true` for `Equal`, `false` for
/// `NotEqual`); the decisions mirror the pre-Lane-108 arms verbatim
/// (`a == b` first, then the fold), extended per sort:
/// - `(NExp, NExp)`: constant fold (as before);
/// - `(Bool, Bool)`: `True`/`False`-literal fold;
/// - mixed sorts and `(Typ, Typ)`: never folded (stay open). Different
///   `typ_arg` constructors cannot denote the same value, but claiming
///   `False` statically would be a proof about a possibly ill-formed
///   goal; OCaml's `constraint_simp` leaves such goals unfolded, and
///   the solver decides them if they ever arrive well-formed.
fn eq_join<'a>(
    orig: &'a NConstraint,
    a: Cow<'a, TypArg>,
    b: Cow<'a, TypArg>,
    rebuild: fn(Box<TypArg>, Box<TypArg>) -> NConstraint,
    is_equal: bool,
) -> Cow<'a, NConstraint> {
    // `a == b` was checked before the fold in the old arms.
    let provably_same = a.as_ref() == b.as_ref();
    let decided: Option<bool> = match (a.as_ref(), b.as_ref()) {
        (TypArg::NExp(x), TypArg::NExp(y)) => match (num(x), num(y)) {
            (Some(x), Some(y)) => Some(x == y),
            _ => None,
        },
        (TypArg::Bool(x), TypArg::Bool(y)) => match (x.as_ref(), y.as_ref()) {
            (NConstraint::True, NConstraint::True)
            | (NConstraint::False, NConstraint::False) => Some(true),
            (NConstraint::True, NConstraint::False)
            | (NConstraint::False, NConstraint::True) => Some(false),
            _ => None,
        },
        (TypArg::Typ(_), TypArg::Typ(_)) => None,
        // Mixed sorts stay open (see above).
        _ => None,
    };
    let unchanged = matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_));
    if provably_same {
        return Cow::Owned(if is_equal {
            NConstraint::True
        } else {
            NConstraint::False
        });
    }
    if let Some(are_equal) = decided {
        return Cow::Owned(if are_equal == is_equal {
            NConstraint::True
        } else {
            NConstraint::False
        });
    }
    if unchanged {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(a.into_owned()), Box::new(b.into_owned())))
    }
}

/// One rewrite pass over a [`TypArg`].
pub fn simplify_typarg(a: &TypArg) -> TypArg {
    simplify_typarg_cow(a).into_owned()
}

/// One rewrite pass over a [`TypArg`], borrowing `a` wherever the
/// rewrite leaves the node untouched (see [`simplify_cow`]).
///
/// `NExp` arguments fold with [`simplify_cow`], `Bool` arguments with
/// [`simplify_constraint_cow`], `Typ` payloads structurally (see
/// [`simplify_typ_cow`]). Owned outputs are value-identical to
/// [`simplify_typarg`].
pub fn simplify_typarg_cow(a: &TypArg) -> Cow<'_, TypArg> {
    match a {
        TypArg::NExp(e) => {
            let r = simplify_cow(e);
            if matches!(r, Cow::Borrowed(_)) {
                Cow::Borrowed(a)
            } else {
                Cow::Owned(TypArg::NExp(r.into_owned()))
            }
        }
        TypArg::Bool(c) => {
            let r = simplify_constraint_cow(c);
            if matches!(r, Cow::Borrowed(_)) {
                Cow::Borrowed(a)
            } else {
                Cow::Owned(TypArg::Bool(Box::new(r.into_owned())))
            }
        }
        TypArg::Typ(t) => {
            let r = simplify_typ_cow(t);
            if matches!(r, Cow::Borrowed(_)) {
                Cow::Borrowed(a)
            } else {
                Cow::Owned(TypArg::Typ(r.into_owned()))
            }
        }
    }
}

/// One rewrite pass over an [`NTyp`], borrowing `t` wherever the
/// rewrite leaves the node untouched (see [`simplify_cow`]).
///
/// Only the argument positions simplify (`App` arguments via
/// [`simplify_typarg_cow`], `Exist` bodies via
/// [`simplify_constraint_cow`]/recursion); binders and leaves never
/// change.
fn simplify_typ_cow(t: &NTyp) -> Cow<'_, NTyp> {
    match t {
        NTyp::Unknown | NTyp::Id(_) | NTyp::Var(_) => Cow::Borrowed(t),
        NTyp::Fn(args, ret) => {
            let simp: Vec<Cow<'_, NTyp>> = args.iter().map(simplify_typ_cow).collect();
            let rret = simplify_typ_cow(ret);
            if simp.iter().all(|x| matches!(x, Cow::Borrowed(_)))
                && matches!(rret, Cow::Borrowed(_))
            {
                Cow::Borrowed(t)
            } else {
                Cow::Owned(NTyp::Fn(
                    simp.into_iter().map(Cow::into_owned).collect(),
                    Box::new(rret.into_owned()),
                ))
            }
        }
        NTyp::Bidir(a, b) => {
            let (ra, rb) = (simplify_typ_cow(a), simplify_typ_cow(b));
            if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
                Cow::Borrowed(t)
            } else {
                Cow::Owned(NTyp::Bidir(
                    Box::new(ra.into_owned()),
                    Box::new(rb.into_owned()),
                ))
            }
        }
        NTyp::Tuple(ts) => {
            let simp: Vec<Cow<'_, NTyp>> = ts.iter().map(simplify_typ_cow).collect();
            if simp.iter().all(|x| matches!(x, Cow::Borrowed(_))) {
                Cow::Borrowed(t)
            } else {
                Cow::Owned(NTyp::Tuple(simp.into_iter().map(Cow::into_owned).collect()))
            }
        }
        NTyp::App(f, args) => {
            let simp: Vec<Cow<'_, TypArg>> =
                args.iter().map(simplify_typarg_cow).collect();
            if simp.iter().all(|x| matches!(x, Cow::Borrowed(_))) {
                Cow::Borrowed(t)
            } else {
                Cow::Owned(NTyp::App(
                    f.clone(),
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        NTyp::Exist(binders, c, ty) => {
            let (rc, rt) = (simplify_constraint_cow(c), simplify_typ_cow(ty));
            if matches!(rc, Cow::Borrowed(_)) && matches!(rt, Cow::Borrowed(_)) {
                Cow::Borrowed(t)
            } else {
                Cow::Owned(NTyp::Exist(
                    binders.clone(),
                    Box::new(rc.into_owned()),
                    Box::new(rt.into_owned()),
                ))
            }
        }
    }
}

/// Iterate [`simplify_typarg`] to a fixpoint (at most 64 rounds).
///
/// Same loop-shape guarantee as [`normalize`]: only the per-round pass
/// borrows via [`simplify_typarg_cow`]; the canonical step is the
/// single-pass [`canonical_typarg`], so results are canonical up to
/// operand order of distinct atoms (like [`normalize_constraint`]).
pub fn normalize_typarg(a: &TypArg) -> TypArg {
    let mut cur = a.clone();
    for _ in 0..64 {
        let simp = simplify_typarg_cow(&cur);
        let next = canonical_typarg(simp.as_ref());
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// Single-pass canonical form of an already-simplified [`TypArg`]:
/// embedded [`NExp`]s are [`normalize`]d, embedded constraints go
/// through the single-pass `canonical_constraint` (the outer
/// [`normalize_typarg`]/[`normalize_constraint`] loop iterates to the
/// fixpoint), types recurse structurally via [`canonical_typ`].
fn canonical_typarg(a: &TypArg) -> TypArg {
    match a {
        TypArg::NExp(e) => TypArg::NExp(normalize(e)),
        TypArg::Bool(c) => TypArg::Bool(Box::new(canonical_constraint(c))),
        TypArg::Typ(t) => TypArg::Typ(canonical_typ(t)),
    }
}

/// Single-pass canonical form of an already-simplified [`NTyp`]
/// (structural recursion; binders are already canonical).
fn canonical_typ(t: &NTyp) -> NTyp {
    match t {
        NTyp::Unknown | NTyp::Id(_) | NTyp::Var(_) => t.clone(),
        NTyp::Fn(args, ret) => NTyp::Fn(
            args.iter().map(canonical_typ).collect(),
            Box::new(canonical_typ(ret)),
        ),
        NTyp::Bidir(a, b) => NTyp::Bidir(
            Box::new(canonical_typ(a)),
            Box::new(canonical_typ(b)),
        ),
        NTyp::Tuple(ts) => NTyp::Tuple(ts.iter().map(canonical_typ).collect()),
        NTyp::App(f, args) => NTyp::App(f.clone(), args.iter().map(canonical_typarg).collect()),
        NTyp::Exist(binders, c, ty) => NTyp::Exist(
            binders.clone(),
            Box::new(canonical_constraint(c)),
            Box::new(canonical_typ(ty)),
        ),
    }
}

/// Owned `And` kernel: the pre-Lane-90 `And` arm, verbatim in behaviour.
fn and_owned(a: NConstraint, b: NConstraint) -> NConstraint {
    match (a, b) {
        (NConstraint::False, _) | (_, NConstraint::False) => NConstraint::False,
        (NConstraint::True, x) | (x, NConstraint::True) => x,
        (x, y) if x == y => x,
        (x, y) => NConstraint::And(Box::new(x), Box::new(y)),
    }
}

/// Join already-simplified `And` children.
///
/// The `True`/`False`/dedup identities return the surviving child
/// [`Cow`] directly (no clone either way); an untouched node is
/// borrowed; anything else delegates to [`and_owned`].
fn and_join<'a>(
    orig: &'a NConstraint,
    a: Cow<'a, NConstraint>,
    b: Cow<'a, NConstraint>,
) -> Cow<'a, NConstraint> {
    // Decided on refs first so the moves below never overlap a borrow.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pick {
        A,
        B,
        Rebuild,
    }
    let pick = match (a.as_ref(), b.as_ref()) {
        (NConstraint::False, _) | (_, NConstraint::True) => Pick::A,
        (_, NConstraint::False) | (NConstraint::True, _) => Pick::B,
        (x, y) if x == y => Pick::A,
        _ => Pick::Rebuild,
    };
    match pick {
        Pick::A => a,
        Pick::B => b,
        Pick::Rebuild => {
            if matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_)) {
                Cow::Borrowed(orig)
            } else {
                Cow::Owned(and_owned(a.into_owned(), b.into_owned()))
            }
        }
    }
}

/// Owned `Or` kernel: the pre-Lane-90 `Or` arm, verbatim in behaviour.
fn or_owned(a: NConstraint, b: NConstraint) -> NConstraint {
    match (a, b) {
        (NConstraint::True, _) | (_, NConstraint::True) => NConstraint::True,
        (NConstraint::False, x) | (x, NConstraint::False) => x,
        (x, y) if x == y => x,
        (x, y) => NConstraint::Or(Box::new(x), Box::new(y)),
    }
}

/// Join already-simplified `Or` children (see [`and_join`]).
fn or_join<'a>(
    orig: &'a NConstraint,
    a: Cow<'a, NConstraint>,
    b: Cow<'a, NConstraint>,
) -> Cow<'a, NConstraint> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pick {
        A,
        B,
        Rebuild,
    }
    let pick = match (a.as_ref(), b.as_ref()) {
        (NConstraint::True, _) | (_, NConstraint::False) => Pick::A,
        (_, NConstraint::True) | (NConstraint::False, _) => Pick::B,
        (x, y) if x == y => Pick::A,
        _ => Pick::Rebuild,
    };
    match pick {
        Pick::A => a,
        Pick::B => b,
        Pick::Rebuild => {
            if matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_)) {
                Cow::Borrowed(orig)
            } else {
                Cow::Owned(or_owned(a.into_owned(), b.into_owned()))
            }
        }
    }
}

/// Join already-simplified comparison children.
///
/// Constant-vs-constant folds only (like OCaml's `constraint_simp`).
/// There is deliberately no `a == b` rule (`x >= x -> True` and
/// friends): with an exceptional `a` such as `1 div 0` it would turn
/// `None` into `Some`. An untouched node is borrowed instead of
/// rebuilt.
fn cmp_join<'a>(
    orig: &'a NConstraint,
    a: Cow<'a, NExp>,
    b: Cow<'a, NExp>,
    decide: impl Fn(i128, i128) -> bool,
    rebuild: fn(Box<NExp>, Box<NExp>) -> NConstraint,
) -> Cow<'a, NConstraint> {
    let decided = match (num(a.as_ref()), num(b.as_ref())) {
        (Some(x), Some(y)) => Some(decide(x, y)),
        _ => None,
    };
    let unchanged = matches!(a, Cow::Borrowed(_)) && matches!(b, Cow::Borrowed(_));
    if let Some(v) = decided {
        return Cow::Owned(if v {
            NConstraint::True
        } else {
            NConstraint::False
        });
    }
    if unchanged {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(a.into_owned()), Box::new(b.into_owned())))
    }
}

/// Iterate [`simplify`] to a fixpoint (at most 64 rounds) and sort the
/// operands of `Sum`/`Times` into a canonical order.
///
/// The loop shape (seed, 64-round cap, fixpoint comparison) is exactly
/// the pre-Lane-90 one; only each round's two passes run through the
/// borrowing [`simplify_cow`]/[`canonical_cow`] variants, so unchanged
/// regions cross a round with one shared borrow instead of two full
/// rebuilds.
pub fn normalize(e: &NExp) -> NExp {
    let mut cur = e.clone();
    for _ in 0..64 {
        let simp = simplify_cow(&cur);
        let next = canonical_cow(simp.as_ref()).into_owned();
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// Iterate [`simplify_constraint`] to a fixpoint (at most 64 rounds).
/// `And`/`Or` are additionally flattened and sorted (like `Sum`/`Times`
/// in [`normalize`); embedded [`NExp`]s are [`normalize`]d, so results
/// are canonical up to operand order of distinct atoms.
///
/// Same loop-shape guarantee as [`normalize`]: only the per-round
/// constraint pass borrows via [`simplify_constraint_cow`].
pub fn normalize_constraint(c: &NConstraint) -> NConstraint {
    let mut cur = c.clone();
    for _ in 0..64 {
        let simp = simplify_constraint_cow(&cur);
        let next = canonical_constraint(simp.as_ref());
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// Flatten nested `And`/`Or`, sort operands canonically, normalize
/// embedded [`NExp`]s.
fn canonical_constraint(c: &NConstraint) -> NConstraint {
    match c {
        NConstraint::And(_, _) => {
            let mut parts = Vec::new();
            flatten_and(c, &mut parts);
            let mut parts: Vec<NConstraint> =
                parts.into_iter().map(|p| canonical_constraint(&p)).collect();
            parts.sort();
            parts.dedup();
            rebuild_and(parts)
        }
        NConstraint::Or(_, _) => {
            let mut parts = Vec::new();
            flatten_or(c, &mut parts);
            let mut parts: Vec<NConstraint> =
                parts.into_iter().map(|p| canonical_constraint(&p)).collect();
            parts.sort();
            parts.dedup();
            rebuild_or(parts)
        }
        NConstraint::Not(x) => NConstraint::Not(Box::new(canonical_constraint(x))),
        NConstraint::Equal(a, b) => {
            NConstraint::Equal(Box::new(canonical_typarg(a)), Box::new(canonical_typarg(b)))
        }
        NConstraint::NotEqual(a, b) => {
            NConstraint::NotEqual(Box::new(canonical_typarg(a)), Box::new(canonical_typarg(b)))
        }
        NConstraint::Ge(a, b) => NConstraint::Ge(norm_box(a), norm_box(b)),
        NConstraint::Gt(a, b) => NConstraint::Gt(norm_box(a), norm_box(b)),
        NConstraint::Le(a, b) => NConstraint::Le(norm_box(a), norm_box(b)),
        NConstraint::Lt(a, b) => NConstraint::Lt(norm_box(a), norm_box(b)),
        NConstraint::Set(n, vs) => {
            let mut sorted = vs.clone();
            sorted.sort();
            sorted.dedup();
            NConstraint::Set(Box::new(normalize(n)), sorted)
        }
        NConstraint::App(f, args) => {
            NConstraint::App(f.clone(), args.iter().map(canonical_typarg).collect())
        }
        _ => c.clone(),
    }
}

fn norm_box(e: &NExp) -> Box<NExp> {
    Box::new(normalize(e))
}

fn flatten_and(c: &NConstraint, out: &mut Vec<NConstraint>) {
    match c {
        NConstraint::And(a, b) => {
            flatten_and(a, out);
            flatten_and(b, out);
        }
        _ => out.push(c.clone()),
    }
}

fn flatten_or(c: &NConstraint, out: &mut Vec<NConstraint>) {
    match c {
        NConstraint::Or(a, b) => {
            flatten_or(a, out);
            flatten_or(b, out);
        }
        _ => out.push(c.clone()),
    }
}

fn rebuild_and(mut parts: Vec<NConstraint>) -> NConstraint {
    debug_assert!(!parts.is_empty());
    let mut acc = parts.remove(0);
    for p in parts {
        acc = NConstraint::And(Box::new(acc), Box::new(p));
    }
    acc
}

fn rebuild_or(mut parts: Vec<NConstraint>) -> NConstraint {
    debug_assert!(!parts.is_empty());
    let mut acc = parts.remove(0);
    for p in parts {
        acc = NConstraint::Or(Box::new(acc), Box::new(p));
    }
    acc
}

/// Flatten nested `Sum`/`Times` and sort operands canonically.
///
/// `Sum`/`Times` nodes always rebuild: flattening re-nests the spine
/// (left-nested), so borrowing the input could return a different shape
/// than the old pass even for sorted leaves. All other nodes borrow
/// when their children are untouched; owned outputs are value-identical
/// to the pre-Lane-90 `canonical`.
fn canonical_cow(e: &NExp) -> Cow<'_, NExp> {
    match e {
        NExp::Sum(_, _) => Cow::Owned(canonical_sum(e)),
        NExp::Times(_, _) => Cow::Owned(canonical_times(e)),
        NExp::Minus(a, b) => {
            let (ra, rb) = (canonical_cow(a), canonical_cow(b));
            if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(NExp::Minus(Box::new(ra.into_owned()), Box::new(rb.into_owned())))
            }
        }
        NExp::Neg(x) => {
            let rx = canonical_cow(x);
            if matches!(rx, Cow::Borrowed(_)) {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(NExp::Neg(Box::new(rx.into_owned())))
            }
        }
        NExp::Exp(x) => {
            let rx = canonical_cow(x);
            if matches!(rx, Cow::Borrowed(_)) {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(NExp::Exp(Box::new(rx.into_owned())))
            }
        }
        NExp::If(c, t, b) => {
            let (rc, rt, rb) =
                (simplify_constraint_cow(c), canonical_cow(t), canonical_cow(b));
            if matches!(rc, Cow::Borrowed(_))
                && matches!(rt, Cow::Borrowed(_))
                && matches!(rb, Cow::Borrowed(_))
            {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(NExp::If(
                    Box::new(rc.into_owned()),
                    Box::new(rt.into_owned()),
                    Box::new(rb.into_owned()),
                ))
            }
        }
        NExp::App(f, args) => {
            let simp: Vec<Cow<'_, NExp>> = args.iter().map(|t| canonical_cow(t)).collect();
            if simp.iter().all(|x| matches!(x, Cow::Borrowed(_))) {
                Cow::Borrowed(e)
            } else {
                Cow::Owned(NExp::App(
                    f.clone(),
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        NExp::Id(_) | NExp::Var(_) | NExp::Constant(_) => Cow::Borrowed(e),
    }
}

/// Flatten nested `Sum` and sort operands canonically (always rebuilds).
fn canonical_sum(e: &NExp) -> NExp {
    let mut terms = Vec::new();
    flatten_sum(e, &mut terms);
    let mut terms: Vec<NExp> = terms.into_iter().map(|t| canonical(&t)).collect();
    terms.sort();
    rebuild_sum(terms)
}

/// Flatten nested `Times` and sort operands canonically (always rebuilds).
fn canonical_times(e: &NExp) -> NExp {
    let mut factors = Vec::new();
    flatten_times(e, &mut factors);
    let mut factors: Vec<NExp> = factors.into_iter().map(|f| canonical(&f)).collect();
    factors.sort();
    rebuild_times(factors)
}

/// Owned canonical form (thin wrapper over [`canonical_cow`]).
fn canonical(e: &NExp) -> NExp {
    canonical_cow(e).into_owned()
}

fn flatten_sum(e: &NExp, out: &mut Vec<NExp>) {
    match e {
        NExp::Sum(a, b) => {
            flatten_sum(a, out);
            flatten_sum(b, out);
        }
        _ => out.push(e.clone()),
    }
}

fn flatten_times(e: &NExp, out: &mut Vec<NExp>) {
    match e {
        NExp::Times(a, b) => {
            flatten_times(a, out);
            flatten_times(b, out);
        }
        _ => out.push(e.clone()),
    }
}

fn rebuild_sum(mut terms: Vec<NExp>) -> NExp {
    debug_assert!(!terms.is_empty());
    let mut acc = terms.remove(0);
    for t in terms {
        acc = NExp::Sum(Box::new(acc), Box::new(t));
    }
    acc
}

fn rebuild_times(mut factors: Vec<NExp>) -> NExp {
    debug_assert!(!factors.is_empty());
    let mut acc = factors.remove(0);
    for f in factors {
        acc = NExp::Times(Box::new(acc), Box::new(f));
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{Kid, NExp::*};
    use sail_ast::{Interner as _, SimpleInterner};
    use std::collections::BTreeMap;

    fn v(k: Kid) -> NExp {
        NExp::Var(k)
    }
    fn c(i: i128) -> NExp {
        NExp::Constant(i)
    }
    /// Fresh table with `n`/`m`/`a`/`b` pre-interned (tuple order).
    /// The table itself is dropped: [`Kid`]s are plain `Copy` handles.
    fn kids() -> (Kid, Kid, Kid, Kid) {
        let mut ix = SimpleInterner::new();
        (
            ix.intern("n"),
            ix.intern("m"),
            ix.intern("a"),
            ix.intern("b"),
        )
    }

    #[test]
    fn folds_constants() {
        let e = Sum(Box::new(c(2)), Box::new(c(3)));
        assert_eq!(simplify(&e), c(5));
        assert_eq!(simplify(&NExp::div(c(7), c(2))), c(3));
        assert_eq!(simplify(&NExp::abs(c(-4))), c(4));
        assert_eq!(simplify(&NExp::Exp(Box::new(c(4)))), c(16));
    }

    #[test]
    fn identities() {
        let (n, _m, _a, _b) = kids();
        assert_eq!(simplify(&Sum(Box::new(v(n)), Box::new(c(0)))), v(n));
        assert_eq!(simplify(&Times(Box::new(v(n)), Box::new(c(1)))), v(n));
        assert_eq!(simplify(&Neg(Box::new(Neg(Box::new(v(n)))))), v(n));
    }

    #[test]
    fn unsound_folds_are_gone() {
        // Lane 124: rewrites without an OCaml (`nexp_simp`/
        // `constraint_simp`) counterpart stay unfolded, so `eval` is
        // preserved exactly (including `None` under partial envs).
        let (n, _m, _a, _b) = kids();
        let keep: Vec<NExp> = vec![
            // `0 * x -> 0` would hide exceptional `x` (`1 div 0`).
            Times(Box::new(c(0)), Box::new(v(n))),
            Times(Box::new(v(n)), Box::new(c(0))),
            // `a - a -> 0` likewise.
            Minus(Box::new(v(n)), Box::new(v(n))),
            // `x % 1 -> 0` likewise.
            NExp::modulo(v(n), c(1)),
            // `abs(2^x) -> 2^x` claims `2^x >= 0` unbacked.
            NExp::abs(NExp::Exp(Box::new(v(n)))),
        ];
        for e in &keep {
            assert_eq!(simplify(e), *e, "must stay unfolded: {e:?}");
        }
        let empty: BTreeMap<Kid, i128> = BTreeMap::new();
        for e in &keep {
            assert_eq!(simplify(e).eval(&empty), None, "eval stays None: {e:?}");
        }
        // Same for constraints: self-comparisons, non-constant empty
        // `Set`, mixed-sort equality (plus `if`-dedup below, which is an
        // `NExp`).
        let wrap = |e: NExp| Box::new(TypArg::NExp(e));
        let keep_c: Vec<NConstraint> = vec![
            NConstraint::Ge(Box::new(v(n)), Box::new(v(n))),
            NConstraint::Gt(Box::new(v(n)), Box::new(v(n))),
            NConstraint::Le(Box::new(v(n)), Box::new(v(n))),
            NConstraint::Lt(Box::new(v(n)), Box::new(v(n))),
            NConstraint::Set(Box::new(v(n)), vec![]),
            NConstraint::Equal(wrap(v(n)), Box::new(TypArg::Bool(Box::new(NConstraint::True)))),
            NConstraint::NotEqual(wrap(v(n)), Box::new(TypArg::Bool(Box::new(NConstraint::True)))),
        ];
        for cc in &keep_c {
            assert_eq!(simplify_constraint(cc), *cc, "must stay unfolded: {cc:?}");
        }
        for cc in &keep_c {
            assert_eq!(
                simplify_constraint(cc).eval(&empty),
                cc.eval(&empty),
                "constraint eval preserved: {cc:?}"
            );
        }
        // `if c then t else t -> t` is gone as well.
        let ite = NExp::If(
            Box::new(NConstraint::Var(n)),
            Box::new(v(n)),
            Box::new(v(n)),
        );
        assert_eq!(simplify(&ite), ite);
        assert_eq!(simplify(&ite).eval(&empty), ite.eval(&empty));
    }

    #[test]
    fn if_true_false_branches() {
        let (_n, _m, _a, b) = kids();
        let t = NExp::If(Box::new(NConstraint::True), Box::new(c(1)), Box::new(c(2)));
        assert_eq!(simplify(&t), c(1));
        // No `t == b -> t` dedup (Lane 124): identical branches stay.
        let same = NExp::If(
            Box::new(NConstraint::Var(b)),
            Box::new(c(1)),
            Box::new(c(1)),
        );
        assert_eq!(simplify(&same), same);
    }

    #[test]
    fn negated_sum_is_order_independent() {
        let (_n, _m, a, b) = kids();
        let neg_a = Neg(Box::new(v(a)));
        let neg_b = Neg(Box::new(NExp::abs(v(b))));
        let ab = Sum(Box::new(neg_a.clone()), Box::new(neg_b.clone()));
        let ba = Sum(Box::new(neg_b), Box::new(neg_a));
        // `simplify` alone does not sort; `normalize` must agree.
        assert_eq!(normalize(&ab), normalize(&ba));
        // Mixed negated / plain operands agree as well.
        let an = Sum(Box::new(Neg(Box::new(v(a)))), Box::new(v(b)));
        let na = Sum(Box::new(v(b)), Box::new(Neg(Box::new(v(a)))));
        assert_eq!(simplify(&an), simplify(&na));
        assert_eq!(normalize(&an), normalize(&na));
    }

    #[test]
    fn normalize_is_canonical_for_ac() {
        let (_n, _m, a, b) = kids();
        let x = Sum(
            Box::new(v(b)),
            Box::new(Sum(Box::new(v(a)), Box::new(c(1)))),
        );
        let y = Sum(
            Box::new(c(1)),
            Box::new(Sum(Box::new(v(a)), Box::new(v(b)))),
        );
        assert_eq!(normalize(&x), normalize(&y));
    }

    #[test]
    fn normalize_preserves_semantics() {
        let (n, _m, _a, _b) = kids();
        // Lane 124: `0 * n` no longer folds (stays a `Times`); only the
        // constant half still folds to `0`, absorbed by `x + 0 -> x`.
        let e = Sum(
            Box::new(Times(Box::new(v(n)), Box::new(c(0)))),
            Box::new(Minus(Box::new(c(5)), Box::new(c(5)))),
        );
        assert_eq!(normalize(&e), Times(Box::new(v(n)), Box::new(c(0))));
        let env: BTreeMap<Kid, i128> = [(n, 7)].into_iter().collect();
        let orig = Sum(Box::new(v(n)), Box::new(c(0)));
        assert_eq!(orig.eval(&env), normalize(&orig).eval(&env));
    }

    #[test]
    fn constraint_simplification() {
        let (_n, _m, _a, b) = kids();
        assert_eq!(
            simplify_constraint(&NConstraint::And(
                Box::new(NConstraint::True),
                Box::new(NConstraint::Var(b))
            )),
            NConstraint::Var(b)
        );
        assert_eq!(
            simplify_constraint(&NConstraint::Equal(
                Box::new(TypArg::NExp(c(2))),
                Box::new(TypArg::NExp(c(3)))
            )),
            NConstraint::False
        );
    }

    #[test]
    fn simplify_cow_borrows_untouched_trees() {
        use std::borrow::Cow;
        let (n, _m, _a, b) = kids();
        // No rule fires anywhere: the whole tree is borrowed.
        let e = Sum(
            Box::new(v(n)),
            Box::new(Times(Box::new(v(b)), Box::new(NExp::Id("f".to_string())))),
        );
        assert!(matches!(simplify_cow(&e), Cow::Borrowed(_)));
        let cc = NConstraint::Ge(Box::new(v(n)), Box::new(c(0)));
        assert!(matches!(simplify_constraint_cow(&cc), Cow::Borrowed(_)));
        // Unknown app with unchanged args borrows (old pass cloned `f`
        // and rebuilt the vec every time).
        let app = NExp::App("f".to_string(), vec![v(n), c(1)]);
        assert!(matches!(simplify_cow(&app), Cow::Borrowed(_)));
        // Already-sorted `Set` with unchanged child borrows (old pass
        // cloned `vs` unconditionally).
        let set = NConstraint::Set(Box::new(v(n)), vec![1, 2, 3]);
        assert!(matches!(simplify_constraint_cow(&set), Cow::Borrowed(_)));
    }

    #[test]
    fn simplify_cow_fold_values_match_owned_contract() {
        use std::borrow::Cow;
        let (n, _m, _a, b) = kids();
        // Hand-written expectations (independent of the cow plumbing):
        // constant folds, identities returning children, app rules.
        let cases: Vec<(NExp, NExp)> = vec![
            (NExp::div(c(7), c(2)), c(3)),
            (NExp::div(c(1), c(0)), NExp::div(c(1), c(0))),
            (NExp::div(v(n), c(1)), v(n)),
            (NExp::div(c(0), c(5)), c(0)),
            (NExp::modulo(c(7), c(2)), c(1)),
            // Lane 124: `x % 1` stays unfolded (no solver backing).
            (
                NExp::modulo(v(n), c(1)),
                NExp::modulo(v(n), c(1)),
            ),
            (
                NExp::abs(NExp::Exp(Box::new(v(n)))),
                NExp::abs(NExp::Exp(Box::new(v(n)))),
            ),
            (
                NExp::abs(NExp::App("abs".to_string(), vec![v(n)])),
                NExp::abs(v(n)),
            ),
            (
                Sum(Box::new(Neg(Box::new(v(n)))), Box::new(Neg(Box::new(c(4))))),
                Neg(Box::new(Sum(Box::new(v(n)), Box::new(c(4))))),
            ),
            (
                Sum(Box::new(v(n)), Box::new(Neg(Box::new(c(1))))),
                Minus(Box::new(v(n)), Box::new(c(1))),
            ),
            (
                Minus(Box::new(Sum(Box::new(v(n)), Box::new(c(5)))), Box::new(c(3))),
                Sum(Box::new(v(n)), Box::new(c(2))),
            ),
            (
                Times(Box::new(c(-1)), Box::new(v(n))),
                Neg(Box::new(v(n))),
            ),
            (
                Times(Box::new(Neg(Box::new(v(n)))), Box::new(Neg(Box::new(c(2))))),
                Times(Box::new(v(n)), Box::new(c(2))),
            ),
            (
                Neg(Box::new(Minus(Box::new(v(n)), Box::new(c(1))))),
                Minus(Box::new(c(1)), Box::new(v(n))),
            ),
            (NExp::Exp(Box::new(c(-2))), NExp::Exp(Box::new(c(-2)))),
        ];
        for (input, expected) in &cases {
            assert_eq!(simplify(input), *expected, "owned: {input:?}");
            assert_eq!(simplify_cow(input).into_owned(), *expected, "cow: {input:?}");
        }
        assert!(matches!(
            simplify_cow(&Sum(Box::new(v(n)), Box::new(c(1)))),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            simplify_cow(&Sum(Box::new(v(n)), Box::new(c(0)))),
            Cow::Owned(_)
        ));
        // Constraint folds with hand-written expectations.
        let ccases: Vec<(NConstraint, NConstraint)> = vec![
            (
                NConstraint::Equal(
                    Box::new(TypArg::NExp(c(2))),
                    Box::new(TypArg::NExp(c(2))),
                ),
                NConstraint::True,
            ),
            (
                NConstraint::NotEqual(
                    Box::new(TypArg::NExp(v(n))),
                    Box::new(TypArg::NExp(v(n))),
                ),
                NConstraint::False,
            ),
            (
                NConstraint::Ge(Box::new(v(n)), Box::new(v(n))),
                NConstraint::Ge(Box::new(v(n)), Box::new(v(n))),
            ),
            (
                NConstraint::Lt(Box::new(v(n)), Box::new(v(n))),
                NConstraint::Lt(Box::new(v(n)), Box::new(v(n))),
            ),
            (
                NConstraint::Set(Box::new(c(2)), vec![3, 1, 2]),
                NConstraint::True,
            ),
            (
                NConstraint::Set(Box::new(v(n)), vec![3, 1, 1, 2]),
                NConstraint::Set(Box::new(v(n)), vec![1, 2, 3]),
            ),
            (
                NConstraint::Or(Box::new(NConstraint::False), Box::new(NConstraint::Var(b))),
                NConstraint::Var(b),
            ),
            (
                NConstraint::Not(Box::new(NConstraint::Not(Box::new(NConstraint::Var(b))))),
                NConstraint::Var(b),
            ),
        ];
        for (input, expected) in &ccases {
            assert_eq!(simplify_constraint(input), *expected, "owned: {input:?}");
            assert_eq!(
                simplify_constraint_cow(input).into_owned(),
                *expected,
                "cow: {input:?}"
            );
        }
    }

    #[test]
    fn normalize_is_idempotent_and_canonical() {
        let (n, _m, a, b) = kids();
        let e = Sum(
            Box::new(v(b)),
            Box::new(Sum(
                Box::new(Neg(Box::new(Neg(Box::new(v(a)))))),
                Box::new(Times(Box::new(v(n)), Box::new(c(1)))),
            )),
        );
        let once = normalize(&e);
        // Fixpoint: normalizing again changes nothing.
        assert_eq!(normalize(&once), once);
        // Fixpoint on a second shape as well.
        let f = Times(Box::new(c(2)), Box::new(v(n)));
        let f1 = normalize(&f);
        assert_eq!(normalize(&f1), f1);
        // Constraint fixpoint likewise.
        let cc = NConstraint::And(
            Box::new(NConstraint::Var(b)),
            Box::new(NConstraint::And(
                Box::new(NConstraint::True),
                Box::new(NConstraint::Var(b)),
            )),
        );
        let conc = normalize_constraint(&cc);
        assert_eq!(conc, NConstraint::Var(b));
        assert_eq!(normalize_constraint(&conc), conc);
    }
}
