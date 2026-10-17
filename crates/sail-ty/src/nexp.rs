#![forbid(unsafe_code)]
//! Local minimal AST/nexp/typ types.
//!
//! These mirror the *shape* of `sail/src/libsail/ast.ml`
//! (`typ`, `nexp`, `n_constraint`, `typquant`) without copying any logic.
//! Once `sail-ast` (Lane 01) and `sail-nexp` (Lane 04) exist, replace these
//! with the real types — see `ARCHITECTURE.md` for the field-by-field mapping.
//!
//! Lane 127 bestaetigt: kanonische `NExp`-Wahrheit ist `sail-nexp`
//! (`NExp`/`NConstraint`/`TypArg`/`NTyp`, internierte `Kid`s); die
//! Vollmigration (`Nexp`/`NConstraint`/`MiniTyp`/Checker) bleibt
//! Owner-Folgearbeit nach Lane 47 — hier keine Logik-Aenderung.
//!
//! Design notes (low-RAM + multithreaded):
//! - Everything is `Clone + Send + Sync`, no interior `Rc`/raw pointers.
//! - `Nexp` is a boxed tree; [`Nexp::depth`] + [`CheckLimits`] bound recursion
//!   so checking without a solver always terminates.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::pat::MiniPat;

/// Monotonic gensym counter for capture-avoiding freshening.
///
/// Mirrors OCaml `Type_env.fresh_kid` (`type_env.ml`: a global `counter`
/// producing `'fv<N>#…` kids): every call yields a name no user-written
/// binder can collide with, because `#` is not a legal Sail identifier
/// character (OCaml uses it for the same reason in `builtin_typs`
/// `k#<n>` kids). Predictable `{base}{i}` names are UNSOUND here — a
/// scheme may already bind `inst0`/`f0`, and two freshenings with the
/// same base in one scope would capture each other.
static FRESH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Yield a globally fresh level-variable name derived from `base`.
///
/// The result always contains `#fv<N>` (`N` strictly increasing), so it
/// is distinct from every source-parsable name and from every name
/// returned by previous calls, regardless of `base`.
#[must_use]
pub fn gensym(base: &str) -> String {
    let n = FRESH_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{base}#fv{n}")
}

/// Numeric type-level expression (subset of Sail `nexp`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nexp {    /// Integer constant.
    Const(i128),
    /// Quantified variable, e.g. `'n`.
    Var(String),
    /// `n + m`
    Add(Box<Nexp>, Box<Nexp>),
    /// `n - m`
    Sub(Box<Nexp>, Box<Nexp>),
    /// `n * m`
    Mul(Box<Nexp>, Box<Nexp>),
    /// `-n`
    Neg(Box<Nexp>),
    /// `2 ^ n` (Sail `Nexp_exp`)
    Exp(Box<Nexp>),
    /// `n / m` (integer division; divisor must be nonzero)
    Div(Box<Nexp>, Box<Nexp>),
    /// `n mod m`
    Mod(Box<Nexp>, Box<Nexp>),
    /// `|n|`
    Abs(Box<Nexp>),
    /// `if c then n else m` (condition is a bool-level nexp encoded via Nc)
    If(Box<NConstraint>, Box<Nexp>, Box<Nexp>),
}

/// Shared iterative post-order evaluator for [`Nexp::as_const`] and
/// [`NConstraint::as_bool`] (no recursion). Children are visited before
/// their `Build` marker (LIFO), so results are present; structurally
/// identical outcomes, including `2 ^ n` bounds and decided `if`s.
fn eval_trees(
    root_n: Option<&Nexp>,
    root_c: Option<&NConstraint>,
) -> (
    std::collections::HashMap<*const Nexp, Option<i128>>,
    std::collections::HashMap<*const NConstraint, Option<bool>>,
) {
    enum Task<'a> {
        VisitN(&'a Nexp),
        VisitC(&'a NConstraint),
        BuildN(&'a Nexp),
        BuildC(&'a NConstraint),
    }
    use std::collections::HashMap;
    let mut stack = Vec::new();
    if let Some(root) = root_n {
        stack.push(Task::VisitN(root));
    }
    if let Some(root) = root_c {
        stack.push(Task::VisitC(root));
    }
    let mut nums: HashMap<*const Nexp, Option<i128>> = HashMap::new();
    let mut bools: HashMap<*const NConstraint, Option<bool>> = HashMap::new();
    let num = |nums: &HashMap<*const Nexp, Option<i128>>, node: &Nexp| -> Option<i128> {
        nums.get(&(node as *const Nexp)).copied().flatten()
    };
    let boolean = |bools: &HashMap<*const NConstraint, Option<bool>>,
                   node: &NConstraint|
     -> Option<bool> { bools.get(&(node as *const NConstraint)).copied().flatten() };
    while let Some(task) = stack.pop() {
        match task {
            Task::VisitN(node) => {
                if nums.contains_key(&(node as *const Nexp)) {
                    continue;
                }
                stack.push(Task::BuildN(node));
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(inner) | Nexp::Abs(inner) | Nexp::Exp(inner) => {
                        stack.push(Task::VisitN(inner));
                    }
                    Nexp::Add(first, second)
                    | Nexp::Sub(first, second)
                    | Nexp::Mul(first, second)
                    | Nexp::Div(first, second)
                    | Nexp::Mod(first, second) => {
                        stack.push(Task::VisitN(first));
                        stack.push(Task::VisitN(second));
                    }
                    Nexp::If(cond, then_branch, else_branch) => {
                        stack.push(Task::VisitN(then_branch));
                        stack.push(Task::VisitN(else_branch));
                        stack.push(Task::VisitC(cond));
                    }
                }
            }
            Task::VisitC(node) => {
                if bools.contains_key(&(node as *const NConstraint)) {
                    continue;
                }
                stack.push(Task::BuildC(node));
                match node {
                    NConstraint::True
                    | NConstraint::False
                    | NConstraint::Var(_)
                    | NConstraint::Id(_) => {}
                    NConstraint::Eq(first, second)
                    | NConstraint::Neq(first, second)
                    | NConstraint::Ge(first, second)
                    | NConstraint::Gt(first, second)
                    | NConstraint::Le(first, second)
                    | NConstraint::Lt(first, second) => {
                        stack.push(Task::VisitN(first));
                        stack.push(Task::VisitN(second));
                    }
                    NConstraint::And(first, second) | NConstraint::Or(first, second) => {
                        stack.push(Task::VisitC(first));
                        stack.push(Task::VisitC(second));
                    }
                    NConstraint::Not(inner) => stack.push(Task::VisitC(inner)),
                    NConstraint::Set(nexp, _) => stack.push(Task::VisitN(nexp)),
                    NConstraint::App(_, args) => {
                        for arg in args {
                            stack.push(Task::VisitN(arg));
                        }
                    }
                }
            }
            Task::BuildN(node) => {
                let value = match node {
                    Nexp::Const(c) => Some(*c),
                    Nexp::Var(_) => None,
                    Nexp::Neg(inner) => num(&nums, inner).and_then(|v| v.checked_neg()),
                    Nexp::Abs(inner) => num(&nums, inner).and_then(|v| v.checked_abs()),
                    Nexp::Exp(inner) => match num(&nums, inner) {
                        Some(v) if (0..=126).contains(&v) => 2_i128.checked_pow(v as u32),
                        _ => None,
                    },
                    Nexp::Add(first, second) => match (num(&nums, first), num(&nums, second)) {
                        (Some(x), Some(y)) => x.checked_add(y),
                        _ => None,
                    },
                    Nexp::Sub(first, second) => match (num(&nums, first), num(&nums, second)) {
                        (Some(x), Some(y)) => x.checked_sub(y),
                        _ => None,
                    },
                    Nexp::Mul(first, second) => match (num(&nums, first), num(&nums, second)) {
                        (Some(x), Some(y)) => x.checked_mul(y),
                        _ => None,
                    },
                    Nexp::Div(first, second) => match (num(&nums, first), num(&nums, second)) {
                        (Some(x), Some(y)) if y != 0 => x.checked_div(y),
                        _ => None,
                    },
                    Nexp::Mod(first, second) => match (num(&nums, first), num(&nums, second)) {
                        (Some(x), Some(y)) if y != 0 => x.checked_rem(y),
                        _ => None,
                    },
                    Nexp::If(cond, then_branch, else_branch) => match boolean(&bools, cond) {
                        Some(true) => num(&nums, then_branch),
                        Some(false) => num(&nums, else_branch),
                        None => None,
                    },
                };
                nums.insert(node as *const Nexp, value);
            }
            Task::BuildC(node) => {
                let structural =
                    |first: &Nexp, second: &Nexp| -> bool { first == second };
                let value = match node {
                    NConstraint::True => Some(true),
                    NConstraint::False => Some(false),
                    NConstraint::Eq(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x == y),
                            _ if structural(first, second) => Some(true),
                            _ => None,
                        }
                    }
                    NConstraint::Neq(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x != y),
                            _ if structural(first, second) => Some(false),
                            _ => None,
                        }
                    }
                    NConstraint::Ge(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x >= y),
                            _ if structural(first, second) => Some(true),
                            _ => None,
                        }
                    }
                    NConstraint::Gt(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x > y),
                            _ if structural(first, second) => Some(false),
                            _ => None,
                        }
                    }
                    NConstraint::Le(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x <= y),
                            _ if structural(first, second) => Some(true),
                            _ => None,
                        }
                    }
                    NConstraint::Lt(first, second) => {
                        match (num(&nums, first), num(&nums, second)) {
                            (Some(x), Some(y)) => Some(x < y),
                            _ if structural(first, second) => Some(false),
                            _ => None,
                        }
                    }
                    NConstraint::And(first, second) => {
                        match (boolean(&bools, first), boolean(&bools, second)) {
                            (Some(false), _) | (_, Some(false)) => Some(false),
                            (Some(true), Some(true)) => Some(true),
                            _ => None,
                        }
                    }
                    NConstraint::Or(first, second) => {
                        match (boolean(&bools, first), boolean(&bools, second)) {
                            (Some(true), _) | (_, Some(true)) => Some(true),
                            (Some(false), Some(false)) => Some(false),
                            _ => None,
                        }
                    }
                    NConstraint::Not(inner) => boolean(&bools, inner).map(|v| !v),
                    NConstraint::Set(nexp, members) => match num(&nums, nexp) {
                        Some(x) => Some(members.contains(&x)),
                        None if members.is_empty() => Some(false),
                        None => None,
                    },
                    NConstraint::App(_, _) | NConstraint::Var(_) | NConstraint::Id(_) => None,
                };
                bools.insert(node as *const NConstraint, value);
            }
        }
    }
    (nums, bools)
}

/// Iterative constant fold entry for [`Nexp::as_const`].
fn eval_const_tree(root: &Nexp) -> Option<i128> {
    let (nums, _) = eval_trees(Some(root), None);
    nums.get(&(root as *const Nexp)).copied().flatten()
}

/// Iterative constant fold entry for [`NConstraint::as_bool`].
fn eval_bool_tree(root: &NConstraint) -> Option<bool> {
    let (_, bools) = eval_trees(None, Some(root));
    bools
        .get(&(root as *const NConstraint))
        .copied()
        .flatten()
}

/// Iterative syntactic tree depth entry (shares walkers below).
fn max_tree_depth(root: &Nexp) -> usize {
    enum Item<'a> {
        Nexp(&'a Nexp, usize),
        Constraint(&'a NConstraint, usize),
    }
    fn push_children<'a>(node: &'a NConstraint, depth: usize, stack: &mut Vec<Item<'a>>) {
        match node {
            NConstraint::True
            | NConstraint::False
            | NConstraint::Var(_)
            | NConstraint::Id(_) => {}
            NConstraint::Eq(first, second)
            | NConstraint::Neq(first, second)
            | NConstraint::Ge(first, second)
            | NConstraint::Gt(first, second)
            | NConstraint::Le(first, second)
            | NConstraint::Lt(first, second) => {
                stack.push(Item::Nexp(first, depth + 1));
                stack.push(Item::Nexp(second, depth + 1));
            }
            NConstraint::And(first, second) | NConstraint::Or(first, second) => {
                stack.push(Item::Constraint(first, depth + 1));
                stack.push(Item::Constraint(second, depth + 1));
            }
            NConstraint::Not(inner) => {
                stack.push(Item::Constraint(inner, depth + 1));
            }
            NConstraint::Set(nexp, _) => {
                stack.push(Item::Nexp(nexp, depth + 1));
            }
            NConstraint::App(_, args) => {
                for arg in args {
                    stack.push(Item::Nexp(arg, depth + 1));
                }
            }
        }
    }
    let mut best = 1;
    let mut stack = vec![Item::Nexp(root, 1)];
    while let Some(item) = stack.pop() {
        match item {
            Item::Nexp(node, depth) => {
                best = best.max(depth);
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(inner) | Nexp::Exp(inner) | Nexp::Abs(inner) => {
                        stack.push(Item::Nexp(inner, depth + 1));
                    }
                    Nexp::Add(first, second)
                    | Nexp::Sub(first, second)
                    | Nexp::Mul(first, second)
                    | Nexp::Div(first, second)
                    | Nexp::Mod(first, second) => {
                        stack.push(Item::Nexp(first, depth + 1));
                        stack.push(Item::Nexp(second, depth + 1));
                    }
                    Nexp::If(cond, then_branch, else_branch) => {
                        push_children(cond, depth + 1, &mut stack);
                        stack.push(Item::Nexp(then_branch, depth + 1));
                        stack.push(Item::Nexp(else_branch, depth + 1));
                    }
                }
            }
            Item::Constraint(node, depth) => {
                best = best.max(depth);
                push_children(node, depth, &mut stack);
            }
        }
    }
    best
}

/// Iterative depth entry for constraints (same walker shape as
/// above, seeded with a constraint).
fn max_tree_depth_constraint(root: &NConstraint) -> usize {
    enum Item<'a> {
        Nexp(&'a Nexp, usize),
        Constraint(&'a NConstraint, usize),
    }
    let mut best = 1;
    let mut stack = vec![Item::Constraint(root, 1)];
    while let Some(item) = stack.pop() {
        match item {
            Item::Nexp(node, depth) => {
                best = best.max(depth);
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(inner) | Nexp::Exp(inner) | Nexp::Abs(inner) => {
                        stack.push(Item::Nexp(inner, depth + 1));
                    }
                    Nexp::Add(first, second)
                    | Nexp::Sub(first, second)
                    | Nexp::Mul(first, second)
                    | Nexp::Div(first, second)
                    | Nexp::Mod(first, second) => {
                        stack.push(Item::Nexp(first, depth + 1));
                        stack.push(Item::Nexp(second, depth + 1));
                    }
                    Nexp::If(cond, then_branch, else_branch) => {
                        stack.push(Item::Constraint(cond, depth + 1));
                        stack.push(Item::Nexp(then_branch, depth + 1));
                        stack.push(Item::Nexp(else_branch, depth + 1));
                    }
                }
            }
            Item::Constraint(node, depth) => {
                best = best.max(depth);
                match node {
                    NConstraint::True
                    | NConstraint::False
                    | NConstraint::Var(_)
                    | NConstraint::Id(_) => {}
                    NConstraint::Eq(first, second)
                    | NConstraint::Neq(first, second)
                    | NConstraint::Ge(first, second)
                    | NConstraint::Gt(first, second)
                    | NConstraint::Le(first, second)
                    | NConstraint::Lt(first, second) => {
                        stack.push(Item::Nexp(first, depth + 1));
                        stack.push(Item::Nexp(second, depth + 1));
                    }
                    NConstraint::And(first, second) | NConstraint::Or(first, second) => {
                        stack.push(Item::Constraint(first, depth + 1));
                        stack.push(Item::Constraint(second, depth + 1));
                    }
                    NConstraint::Not(inner) => {
                        stack.push(Item::Constraint(inner, depth + 1));
                    }
                    NConstraint::Set(nexp, _) => {
                        stack.push(Item::Nexp(nexp, depth + 1));
                    }
                    NConstraint::App(_, args) => {
                        for arg in args {
                            stack.push(Item::Nexp(arg, depth + 1));
                        }
                    }
                }
            }
        }
    }
    best
}

impl Nexp {
    /// Syntactic tree depth. Used for fuel accounting.
    /// Iterative walk (no recursion).
    #[must_use]
    pub fn depth(&self) -> usize {
        max_tree_depth(self)
    }

    /// Collect free variables into `out` (dedup left to caller).
    /// Iterative pre-order walk (no recursion).
    pub fn free_vars(&self, out: &mut Vec<String>) {
        let mut todo = vec![self];
        while let Some(current) = todo.pop() {
            match current {
                Self::Const(_) => {}
                Self::Var(v) => out.push(v.clone()),
                Self::Neg(n) | Self::Exp(n) | Self::Abs(n) => todo.push(n),
                Self::Add(a, b)
                | Self::Sub(a, b)
                | Self::Mul(a, b)
                | Self::Div(a, b)
                | Self::Mod(a, b) => {
                    todo.push(a);
                    todo.push(b);
                }
                Self::If(c, a, b) => {
                    c.free_vars(out);
                    todo.push(a);
                    todo.push(b);
                }
            }
        }
    }

    /// Try to fold to a constant. Returns `None` if not constant-foldable.
    /// Pure and total; bounded by tree depth (no solver needed).
    #[must_use]
    pub fn as_const(&self) -> Option<i128> {
        eval_const_tree(self)
    }

    /// Sound bottom-up simplifier: constant folding ([`Nexp::as_const`],
    /// including `2 ^ n` for `0 <= n <= 126` and decided `if`) plus
    /// unconditional integer identities:
    /// `x + 0` / `x - 0` / `x * 1` / `x / 1` (identity element),
    /// `x mod 1 = 0` (nonzero divisor), `x - x = 0` (syntactic),
    /// double negation, decided `if`, and equal-branch `if`.
    ///
    /// Total and terminating (one structural pass; every rewrite strictly
    /// shrinks the tree, depth never grows). Every rewrite is a
    /// mathematical-integer identity, so `simplify` preserves exactly the
    /// totality assumption the syntactic reflexivity paths already make
    /// (a proved `Eq(a, a)` assumes level terms denote).
    ///
    /// Deliberately *absent*: the annihilator (`0 * x` — needs totality
    /// of `x`), power unfolding (`2 ^ n` — needs bounds on `n`),
    /// division cancellation (`(x * k) / k` — needs nonzeroness), and any
    /// associative reassociation. Those need side conditions the local
    /// layer cannot discharge — they stay solver business (`Unknown`
    /// until proved, rule `Z-UnknownSound`).
    #[must_use]
    pub fn simplify(&self) -> Nexp {
        // Constants (including decided `if` and small powers) collapse here.
        if let Some(c) = self.as_const() {
            return Nexp::Const(c);
        }
        // Iterative post-order rebuild (no recursion): children first via
        // an explicit stack, assembled bottom-up keyed by node identity
        // (Mini trees are `Box` trees, so every node is visited once).
        // The `if` condition folds through the iterative constraint
        // walker; a missing child result falls back to the original
        // subtree (fail-closed, never guessed).
        enum Task<'a> {
            Visit(&'a Nexp),
            Build(&'a Nexp),
        }
        use std::collections::HashMap;
        let mut stack = vec![Task::Visit(self)];
        let mut built: HashMap<*const Nexp, Nexp> = HashMap::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Visit(node) => {
                    if built.contains_key(&(node as *const Nexp)) {
                        continue;
                    }
                    stack.push(Task::Build(node));
                    match node {
                        Nexp::Const(_) | Nexp::Var(_) => {}
                        Nexp::Neg(inner) | Nexp::Abs(inner) | Nexp::Exp(inner) => {
                            stack.push(Task::Visit(inner));
                        }
                        Nexp::Add(first, second)
                        | Nexp::Sub(first, second)
                        | Nexp::Mul(first, second)
                        | Nexp::Div(first, second)
                        | Nexp::Mod(first, second) => {
                            stack.push(Task::Visit(first));
                            stack.push(Task::Visit(second));
                        }
                        Nexp::If(cond, then_branch, else_branch) => {
                            stack.push(Task::Visit(then_branch));
                            stack.push(Task::Visit(else_branch));
                            let _ = cond;
                        }
                    }
                }
                Task::Build(node) => {
                    let child = |built: &mut HashMap<*const Nexp, Nexp>, node: &Nexp| {
                        built.remove(&(node as *const Nexp))
                    };
                    let fallback = || node.clone();
                    let simplified = match node {
                        Nexp::Const(_) | Nexp::Var(_) => node.clone(),
                        Nexp::Neg(inner) => match child(&mut built, inner) {
                            Some(Nexp::Neg(x)) => *x,
                            Some(x) => Nexp::Neg(Box::new(x)),
                            None => fallback(),
                        },
                        Nexp::Abs(inner) => match child(&mut built, inner) {
                            Some(x) => Nexp::Abs(Box::new(x)),
                            None => fallback(),
                        },
                        Nexp::Exp(inner) => match child(&mut built, inner) {
                            Some(x) => Nexp::Exp(Box::new(x)),
                            None => fallback(),
                        },
                        Nexp::Add(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => {
                                    if is_zero(&x) {
                                        y
                                    } else if is_zero(&y) {
                                        x
                                    } else {
                                        Nexp::Add(Box::new(x), Box::new(y))
                                    }
                                }
                                _ => fallback(),
                            }
                        }
                        Nexp::Sub(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => {
                                    if is_zero(&y) {
                                        x
                                    } else if x == y {
                                        Nexp::Const(0)
                                    } else {
                                        Nexp::Sub(Box::new(x), Box::new(y))
                                    }
                                }
                                _ => fallback(),
                            }
                        }
                        Nexp::Mul(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => {
                                    if is_one(&x) {
                                        y
                                    } else if is_one(&y) {
                                        x
                                    } else {
                                        Nexp::Mul(Box::new(x), Box::new(y))
                                    }
                                }
                                _ => fallback(),
                            }
                        }
                        Nexp::Div(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => {
                                    if is_one(&y) {
                                        x
                                    } else {
                                        Nexp::Div(Box::new(x), Box::new(y))
                                    }
                                }
                                _ => fallback(),
                            }
                        }
                        Nexp::Mod(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => {
                                    if is_one(&y) {
                                        Nexp::Const(0)
                                    } else {
                                        Nexp::Mod(Box::new(x), Box::new(y))
                                    }
                                }
                                _ => fallback(),
                            }
                        }
                        Nexp::If(cond, then_branch, else_branch) => {
                            let cc = cond.simplify();
                            match (child(&mut built, then_branch), child(&mut built, else_branch)) {
                                (Some(x), Some(y)) => match cc.as_bool() {
                                    Some(true) => x,
                                    Some(false) => y,
                                    None if x == y => x,
                                    _ => Nexp::If(Box::new(cc), Box::new(x), Box::new(y)),
                                },
                                _ => fallback(),
                            }
                        }
                    };
                    built.insert(node as *const Nexp, simplified);
                }
            }
        }
        built.remove(&(self as *const Nexp)).unwrap_or_else(|| self.clone())
    }
}

/// Exact linear (affine) form over level variables: `constant + Σ coeff·var`
/// with nonzero coefficients in a canonical (sorted) map.
///
/// Built by [`linear_form`]; only linear skeletons decompose (`Add`/`Sub`,
/// `Neg`, `Mul` with a constant side, constant-foldable subterms).
/// `Div`/`Mod`/`Exp`/`Abs`/undecided `If`/nonlinear `Mul` yield `None` —
/// sound openness, never an approximation. All coefficient arithmetic is
/// checked: overflow yields `None` (→ `Unknown`), never a wrong proof.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinearForm {
    /// Constant term.
    pub constant: i128,
    /// Nonzero coefficients by variable, sorted for canonicity.
    pub coeffs: std::collections::BTreeMap<String, i128>,
}

impl LinearForm {
    /// The constant form `c` (no variables).
    #[must_use]
    pub fn constant(c: i128) -> Self {
        Self { constant: c, coeffs: std::collections::BTreeMap::new() }
    }

    /// `true` when no variable occurs (the form is just [`Self::constant`]).
    #[must_use]
    pub fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    /// Negation (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn neg(&self) -> Option<Self> {
        let mut coeffs = std::collections::BTreeMap::new();
        for (v, c) in &self.coeffs {
            coeffs.insert(v.clone(), c.checked_neg()?);
        }
        Some(Self { constant: self.constant.checked_neg()?, coeffs })
    }

    /// Sum (exact; `None` only on `i128` overflow). Zero coefficients are
    /// dropped so `is_constant` and variable scans stay exact.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        let mut coeffs = self.coeffs.clone();
        for (v, c) in &other.coeffs {
            let next = coeffs.get(v).copied().unwrap_or(0).checked_add(*c)?;
            if next == 0 {
                coeffs.remove(v);
            } else {
                coeffs.insert(v.clone(), next);
            }
        }
        Some(Self { constant: self.constant.checked_add(other.constant)?, coeffs })
    }

    /// Difference (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn sub(&self, other: &Self) -> Option<Self> {
        self.add(&other.neg()?)
    }

    /// Scale by a constant (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn scale(&self, factor: i128) -> Option<Self> {
        if factor == 0 {
            return Some(Self::constant(0));
        }
        let mut coeffs = std::collections::BTreeMap::new();
        for (v, c) in &self.coeffs {
            coeffs.insert(v.clone(), c.checked_mul(factor)?);
        }
        Some(Self {
            constant: self.constant.checked_mul(factor)?,
            coeffs,
        })
    }

    /// Rebuild a level term denoting exactly this form:
    /// `((t₁ + t₂) + … + constant)`, where each `tᵢ` is `v`, `-v`, or
    /// `c * v`. A constant form rebuilds to [`Nexp::Const`]; a form with
    /// terms and a zero constant rebuilds without a trailing `+ 0`.
    #[must_use]
    pub fn into_nexp(self) -> Nexp {
        let mut acc: Option<Nexp> = None;
        let push = |term: Nexp, acc: &mut Option<Nexp>| {
            *acc = Some(match acc.take() {
                None => term,
                Some(prev) => Nexp::Add(Box::new(prev), Box::new(term)),
            });
        };
        for (v, c) in self.coeffs {
            let var = Nexp::Var(v);
            push(
                match c {
                    1 => var,
                    -1 => Nexp::Neg(Box::new(var)),
                    _ => Nexp::Mul(Box::new(Nexp::Const(c)), Box::new(var)),
                },
                &mut acc,
            );
        }
        if self.constant != 0 || acc.is_none() {
            push(Nexp::Const(self.constant), &mut acc);
        }
        acc.expect("LinearForm rebuilds to at least a constant")
    }
}

/// Decompose `n` into an exact [`LinearForm`]; `None` for nonlinear or
/// otherwise non-affine skeletons (sound openness — never approximate).
///
/// Constant-foldable subterms (including small powers and decided `if`)
/// fold via [`Nexp::as_const`] first, so `2 ^ 3 + 'n` decomposes while
/// `2 ^ 'n + 'm` stays `None`.
#[must_use]
pub fn linear_form(n: &Nexp) -> Option<LinearForm> {
    if let Some(c) = n.as_const() {
        return Some(LinearForm::constant(c));
    }
    // Iterative post-order combine (no recursion): children first via
    // an explicit stack, forms assembled bottom-up keyed by identity.
    enum Task<'a> {
        Visit(&'a Nexp),
        Build(&'a Nexp),
    }
    use std::collections::HashMap;
    let mut stack = vec![Task::Visit(n)];
    let mut built: HashMap<*const Nexp, Option<LinearForm>> = HashMap::new();
    while let Some(task) = stack.pop() {
        match task {
            Task::Visit(node) => {
                if built.contains_key(&(node as *const Nexp)) {
                    continue;
                }
                // Constants short-circuit with their form inserted
                // (matches the per-node `as_const` exit).
                if let Some(c) = node.as_const() {
                    built.insert(node as *const Nexp, Some(LinearForm::constant(c)));
                    continue;
                }
                stack.push(Task::Build(node));
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(inner) => stack.push(Task::Visit(inner)),
                    Nexp::Add(first, second) | Nexp::Sub(first, second) => {
                        stack.push(Task::Visit(first));
                        stack.push(Task::Visit(second));
                    }
                    Nexp::Mul(first, second) => {
                        stack.push(Task::Visit(first));
                        stack.push(Task::Visit(second));
                    }
                    Nexp::Div(..)
                    | Nexp::Mod(..)
                    | Nexp::Exp(..)
                    | Nexp::Abs(..)
                    | Nexp::If(..) => {}
                }
            }
            Task::Build(node) => {
                let child = |built: &HashMap<*const Nexp, Option<LinearForm>>, node: &Nexp| {
                    built
                        .get(&(node as *const Nexp))
                        .cloned()
                        .flatten()
                };
                // NOTE: `as_const` above already handled constants; the
                // fallback below keeps leaves total.
                let form = match node {
                    Nexp::Const(c) => Some(LinearForm::constant(*c)),
                    Nexp::Var(v) => {
                        let mut coeffs = std::collections::BTreeMap::new();
                        coeffs.insert(v.clone(), 1);
                        Some(LinearForm { constant: 0, coeffs })
                    }
                    Nexp::Neg(inner) => child(&mut built, inner)?.neg(),
                    Nexp::Add(first, second) => {
                        child(&mut built, first)?.add(&child(&mut built, second)?)
                    }
                    Nexp::Sub(first, second) => {
                        child(&mut built, first)?.sub(&child(&mut built, second)?)
                    }
                    Nexp::Mul(first, second) => {
                        match (first.as_const(), second.as_const()) {
                            (Some(c), _) => child(&mut built, second)?.scale(c),
                            (_, Some(c)) => child(&mut built, first)?.scale(c),
                            _ => None,
                        }
                    }
                    Nexp::Div(..)
                    | Nexp::Mod(..)
                    | Nexp::Exp(..)
                    | Nexp::Abs(..)
                    | Nexp::If(..) => None,
                };
                built.insert(node as *const Nexp, form);
            }
        }
    }
    built.remove(&(n as *const Nexp)).flatten()
}

fn is_zero(n: &Nexp) -> bool {
    matches!(n, Nexp::Const(0))
}

fn is_one(n: &Nexp) -> bool {
    matches!(n, Nexp::Const(1))
}

impl fmt::Display for Nexp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const(c) => write!(f, "{c}"),
            Self::Var(v) => write!(f, "'{v}"),
            Self::Neg(n) => write!(f, "-({n})"),
            Self::Abs(n) => write!(f, "abs({n})"),
            Self::Exp(n) => write!(f, "2^({n})"),
            Self::Add(a, b) => write!(f, "({a} + {b})"),
            Self::Sub(a, b) => write!(f, "({a} - {b})"),
            Self::Mul(a, b) => write!(f, "({a} * {b})"),
            Self::Div(a, b) => write!(f, "({a} / {b})"),
            Self::Mod(a, b) => write!(f, "({a} mod {b})"),
            Self::If(c, a, b) => write!(f, "(if {c} then {a} else {b})"),
        }
    }
}

/// Boolean / numeric constraint over [`Nexp`] (subset of Sail `n_constraint`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NConstraint {
    /// `true`
    True,
    /// `false`
    False,
    /// `n == m`
    Eq(Box<Nexp>, Box<Nexp>),
    /// `n != m`
    Neq(Box<Nexp>, Box<Nexp>),
    /// `n >= m`
    Ge(Box<Nexp>, Box<Nexp>),
    /// `n > m`
    Gt(Box<Nexp>, Box<Nexp>),
    /// `n <= m`
    Le(Box<Nexp>, Box<Nexp>),
    /// `n < m`
    Lt(Box<Nexp>, Box<Nexp>),
    /// `a /\ b`
    And(Box<NConstraint>, Box<NConstraint>),
    /// `a \/ b`
    Or(Box<NConstraint>, Box<NConstraint>),
    /// `~a`
    Not(Box<NConstraint>),
    /// `n in {v1, ..}` — Sail `NC_set` (Lane 107 prove-path: constant
    /// scrutinees fold by membership, `[]` folds to `False`; anything else
    /// goes to the [`Solver`](crate::solver::Solver) via the trait).
    Set(Box<Nexp>, Vec<i128>),
    /// Uninterpreted predicate `f(n1, ..)` — Sail `NC_app` over `A_nexp`
    /// arguments only (Lane 107: non-`NExp` `typ_arg`s have no local
    /// counterpart and stay abstracted by the caller, see [`crate::conv`]).
    /// Folding is always `None` (sound); backends decide via the trait.
    App(String, Vec<Nexp>),
    /// Boolean kind variable used as a constraint — Sail `NC_var`.
    Var(String),
    /// Opaque boolean identifier — Sail `NC_id`.
    Id(String),
}

impl NConstraint {
    #[must_use]
    pub fn depth(&self) -> usize {
        max_tree_depth_constraint(self)
    }

    pub fn free_vars(&self, out: &mut Vec<String>) {
        // Iterative pre-order walk (no recursion).
        let mut todo = vec![self];
        while let Some(current) = todo.pop() {
            match current {
                Self::True | Self::False | Self::Id(_) => {}
                // Boolean level variable: same flat namespace as `Nexp::Var`
                // in this skeleton (kind-checked against `K_bool`, see
                // [`crate::kind::check_constraint_kind`]).
                Self::Var(v) => out.push(v.clone()),
                Self::Eq(a, b)
                | Self::Neq(a, b)
                | Self::Ge(a, b)
                | Self::Gt(a, b)
                | Self::Le(a, b)
                | Self::Lt(a, b) => {
                    a.free_vars(out);
                    b.free_vars(out);
                }
                Self::And(a, b) | Self::Or(a, b) => {
                    todo.push(a);
                    todo.push(b);
                }
                Self::Not(a) => todo.push(a),
                Self::Set(n, _) => n.free_vars(out),
                Self::App(_, args) => {
                    for a in args {
                        a.free_vars(out);
                    }
                }
            }
        }
    }

    /// Boolean constant folding. `None` = not decidable syntactically.
    /// Iterative post-order evaluator (no recursion; shared with
    /// [`Nexp::as_const`]).
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        eval_bool_tree(self)
    }

    /// Sound boolean/arithmetic simplifier: embedded [`Nexp::simplify`] on
    /// every level term, `True`/`False` elimination in `And`/`Or`, double
    /// negation, then the [`NConstraint::as_bool`] decision (constant
    /// comparisons, constant `Set` membership, empty `Set`, syntactic
    /// reflexivity all collapse to `True`/`False`).
    ///
    /// Total and terminating (one structural pass). Uninterpreted
    /// predicates (`App`), opaque booleans (`Var`/`Id`), and symbolic
    /// `Set` scrutinees pass through untouched — they are never guessed,
    /// only propagated (rule `Z-UnknownSound`).
    #[must_use]
    pub fn simplify(&self) -> NConstraint {
        // Iterative post-order rebuild (no recursion): NExp children
        // fold through the iterative `Nexp::simplify`; only And/Or/Not
        // need the explicit stack. Final `as_bool` fold as before.
        enum Task<'a> {
            Visit(&'a NConstraint),
            Build(&'a NConstraint),
        }
        use std::collections::HashMap;
        let mut stack = vec![Task::Visit(self)];
        let mut built: HashMap<*const NConstraint, NConstraint> = HashMap::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Visit(node) => {
                    if built.contains_key(&(node as *const NConstraint)) {
                        continue;
                    }
                    stack.push(Task::Build(node));
                    match node {
                        NConstraint::And(first, second) | NConstraint::Or(first, second) => {
                            stack.push(Task::Visit(first));
                            stack.push(Task::Visit(second));
                        }
                        NConstraint::Not(inner) => stack.push(Task::Visit(inner)),
                        _ => {}
                    }
                }
                Task::Build(node) => {
                    let child = |built: &mut HashMap<*const NConstraint, NConstraint>,
                                 node: &NConstraint| {
                        built.remove(&(node as *const NConstraint))
                    };
                    let rebuilt = match node {
                        NConstraint::True
                        | NConstraint::False
                        | NConstraint::Var(_)
                        | NConstraint::Id(_) => node.clone(),
                        NConstraint::Eq(a, b) => {
                            NConstraint::Eq(simplified(a), simplified(b))
                        }
                        NConstraint::Neq(a, b) => {
                            NConstraint::Neq(simplified(a), simplified(b))
                        }
                        NConstraint::Ge(a, b) => {
                            NConstraint::Ge(simplified(a), simplified(b))
                        }
                        NConstraint::Gt(a, b) => {
                            NConstraint::Gt(simplified(a), simplified(b))
                        }
                        NConstraint::Le(a, b) => {
                            NConstraint::Le(simplified(a), simplified(b))
                        }
                        NConstraint::Lt(a, b) => {
                            NConstraint::Lt(simplified(a), simplified(b))
                        }
                        NConstraint::And(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => match (&x, &y) {
                                    (NConstraint::False, _)
                                    | (_, NConstraint::False) => NConstraint::False,
                                    (NConstraint::True, _) => y,
                                    (_, NConstraint::True) => x,
                                    _ => NConstraint::And(Box::new(x), Box::new(y)),
                                },
                                _ => node.clone(),
                            }
                        }
                        NConstraint::Or(first, second) => {
                            match (child(&mut built, first), child(&mut built, second)) {
                                (Some(x), Some(y)) => match (&x, &y) {
                                    (NConstraint::True, _) | (_, NConstraint::True) => {
                                        NConstraint::True
                                    }
                                    (NConstraint::False, _) => y,
                                    (_, NConstraint::False) => x,
                                    _ => NConstraint::Or(Box::new(x), Box::new(y)),
                                },
                                _ => node.clone(),
                            }
                        }
                        NConstraint::Not(inner) => match child(&mut built, inner) {
                            Some(NConstraint::True) => NConstraint::False,
                            Some(NConstraint::False) => NConstraint::True,
                            Some(NConstraint::Not(x)) => *x,
                            Some(x) => NConstraint::Not(Box::new(x)),
                            None => node.clone(),
                        },
                        NConstraint::Set(n, vs) => {
                            NConstraint::Set(Box::new(n.simplify()), vs.clone())
                        }
                        NConstraint::App(func, args) => NConstraint::App(
                            func.clone(),
                            args.iter().map(Nexp::simplify).collect(),
                        ),
                    };
                    built.insert(node as *const NConstraint, rebuilt);
                }
            }
        }
        let rebuilt = built
            .remove(&(self as *const NConstraint))
            .unwrap_or_else(|| self.clone());
        match rebuilt.as_bool() {
            Some(true) => NConstraint::True,
            Some(false) => NConstraint::False,
            None => rebuilt,
        }
    }

}

fn simplified(n: &Nexp) -> Box<Nexp> {
    Box::new(n.simplify())
}

impl fmt::Display for NConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => write!(f, "true"),
            Self::False => write!(f, "false"),
            Self::Eq(a, b) => write!(f, "({a} == {b})"),
            Self::Neq(a, b) => write!(f, "({a} != {b})"),
            Self::Ge(a, b) => write!(f, "({a} >= {b})"),
            Self::Gt(a, b) => write!(f, "({a} > {b})"),
            Self::Le(a, b) => write!(f, "({a} <= {b})"),
            Self::Lt(a, b) => write!(f, "({a} < {b})"),
            Self::And(a, b) => write!(f, "({a} & {b})"),
            Self::Or(a, b) => write!(f, "({a} | {b})"),
            Self::Not(a) => write!(f, "~({a})"),
            Self::Set(n, vs) => {
                write!(f, "({n} in {{")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "}})")
            }
            Self::App(func, args) => {
                write!(f, "{func}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Self::Var(v) => write!(f, "'{v}"),
            Self::Id(name) => write!(f, "{name}"),
        }
    }
}

/// Sail type (subset of Sail `typ`): enough for kind checking,
/// register types, and the `test/lean`-subset smoke tests.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MiniTyp {
    /// `int`
    Int,
    /// `bool`
    Bool,
    /// `bit`
    Bit,
    /// `bits('n)` / `bits(n)` — length-level nexp
    Bits(Box<Nexp>),
    /// `int('n)` — range-bounded int, Sail `atom` shorthand
    Range(Box<Nexp>, Box<Nexp>),
    /// Named type / abbreviation applied to nexps, e.g. `atom('n)`.
    Named {
        name: String,
        args: Vec<Nexp>,
    },
    /// `t1 -> t2`
    Fun(Box<MiniTyp>, Box<MiniTyp>),
    /// `(t1, t2, ...)` tuple
    Tuple(Vec<MiniTyp>),
}

impl MiniTyp {
    /// Collect all level nexps occurring in the type.
    pub fn nexps(&self, out: &mut Vec<Nexp>) {
        // Iterative pre-order walk (no recursion).
        let mut todo = vec![self];
        while let Some(current) = todo.pop() {
            match current {
                Self::Int | Self::Bool | Self::Bit => {}
                Self::Bits(n) => out.push((**n).clone()),
                Self::Range(lo, hi) => {
                    out.push((**lo).clone());
                    out.push((**hi).clone());
                }
                Self::Named { args, .. } => out.extend(args.iter().cloned()),
                Self::Fun(a, b) => {
                    todo.push(a);
                    todo.push(b);
                }
                Self::Tuple(ts) => {
                    for t in ts {
                        todo.push(t);
                    }
                }
            }
        }
    }
}

impl fmt::Display for MiniTyp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Bool => write!(f, "bool"),
            Self::Bit => write!(f, "bit"),
            Self::Bits(n) => write!(f, "bits({n})"),
            Self::Range(lo, hi) => write!(f, "range({lo}, {hi})"),
            Self::Named { name, args } => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Self::Fun(a, b) => write!(f, "({a} -> {b})"),
            Self::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// Universally quantified type scheme: `forall 'n 'm. nc => typ`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypQuant {
    /// Bound kinded variables: `('n, K_int)`.
    pub quants: Vec<(String, crate::kind::Kind)>,
    /// Precondition constraint.
    pub constraint: NConstraint,
    /// Body type.
    pub typ: MiniTyp,
}

impl TypQuant {
    #[must_use]
    pub fn monomorphic(typ: MiniTyp) -> Self {
        Self {
            quants: Vec::new(),
            constraint: NConstraint::True,
            typ,
        }
    }
}

/// Loop kind for [`MiniExp::Loop`] (mirrors Sail `loop`: `while`/`until`).
///
/// Lane-127-Notiz (Checker-Minimal, keine Logik-Aenderung): Wahrheit
/// ist `sail_loops::LoopKind`; dieses Minimal bleibt bis Lane 47
/// (Typvereinheitlichung, siehe `ARCHITECTURE.md`) bestehen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniLoop {
    While,
    Until,
}

/// Iteration order for [`MiniExp::For`] (mirrors Sail `order`: `inc`/`dec`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniOrder {
    Inc,
    Dec,
}

/// One struct field initializer: `field = value` (mirrors Sail `fexp`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniFExp {
    pub field: String,
    pub value: MiniExp,
}

/// One match/try arm: `pat [if guard] => body` (mirrors Sail `pexp`:
/// `PatExp` for `guard: None`, `PatWhen` for `guard: Some`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniPExp {
    pub pat: MiniPat,
    pub guard: Option<MiniExp>,
    pub body: MiniExp,
}

/// Left-hand-side expression for assignment targets (subset of Sail
/// `lexp_aux`; mirrors `sail_ast::LExpKind` without spans).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniLexp {
    /// `x`
    Id(String),
    /// `*e`
    Deref(Box<MiniExp>),
    /// `(x : t)` typed binder
    Typed { typ: MiniTyp, id: String },
    /// `(l1, l2, ...)`
    Tuple(Vec<MiniLexp>),
    /// Concatenated vector targets
    VectorConcat(Vec<MiniLexp>),
    /// `l[i]`
    Vector { lexp: Box<MiniLexp>, index: Box<MiniExp> },
    /// `l[hi .. lo]`
    VectorRange {
        lexp: Box<MiniLexp>,
        lo: Box<MiniExp>,
        hi: Box<MiniExp>,
    },
    /// `l.field`
    Field { lexp: Box<MiniLexp>, field: String },
}

/// Minimal expression: covers every non-internal Sail `exp_aux` constructor
/// (`E_*` in `sail/src/lib/extraction/Ast.ml`, one `E_lit` shared by
/// [`MiniExp::IntLit`]/[`MiniExp::BoolLit`). The bidirectional checker
/// still synthesizes only the Lane-79 subset
/// (`IntLit`/`BoolLit`/`Var`/`Let`/`App`/`Tuple`/`Annot`/`If`); all other
/// forms are representable plus convertible (`conv.rs`) but report a
/// mismatch at `infer` (checking deferred, no checker rules added).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniExp {
    /// Integer literal (`E_lit`)
    IntLit(i128),
    /// Boolean literal (`E_lit`)
    BoolLit(bool),
    /// Variable (`E_id`, also `E_ref` targets via [`MiniExp::Ref`])
    Var(String),
    /// `let x: t = e1 in e2` (`E_let`, `x` must be a plain/annotated id)
    Let {
        var: String,
        annot: Option<MiniTyp>,
        bound: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `f x` (`E_app` with a variable head; curried heads stay `None` in conv)
    App(Box<MiniExp>, Box<MiniExp>),
    /// `(e1, e2, ...)` (`E_tuple`)
    Tuple(Vec<MiniExp>),
    /// `e : t` annotation (`E_typ`)
    Annot(Box<MiniExp>, MiniTyp),
    /// `if c then a else b` (`E_if`)
    If(Box<MiniExp>, Box<MiniExp>, Box<MiniExp>),
    /// `e1; e2; ...` sequence (`E_block`)
    Block(Vec<MiniExp>),
    /// `while`/`until cond do body` (`E_loop`; `measure` is the
    /// `in_place_loop_measure`, `None` = `Measure_none`)
    Loop {
        kind: MiniLoop,
        measure: Option<Box<MiniExp>>,
        cond: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `for x = <from> to <to> step <step> (<order>) do <body>` (`E_for`)
    For {
        var: String,
        from: Box<MiniExp>,
        to: Box<MiniExp>,
        step: Box<MiniExp>,
        order: MiniOrder,
        body: Box<MiniExp>,
    },
    /// `[e1, e2, ...]` (`E_vector`)
    Vector(Vec<MiniExp>),
    /// `v1 @ v2` (`E_vector_append`)
    VectorAppend(Box<MiniExp>, Box<MiniExp>),
    /// `[e1, e2, ...]` list literal (`E_list`)
    List(Vec<MiniExp>),
    /// `h :: t` (`E_cons`)
    Cons(Box<MiniExp>, Box<MiniExp>),
    /// `struct { f = e, ... }` (`E_struct`; `name: None` = anonymous)
    Struct {
        name: Option<String>,
        fields: Vec<MiniFExp>,
    },
    /// `r with { f = e, ... }` (`E_struct_update`)
    StructUpdate {
        record: Box<MiniExp>,
        fields: Vec<MiniFExp>,
    },
    /// `r.field` (`E_field`)
    Field { record: Box<MiniExp>, field: String },
    /// `match scrut { pat [if g] => body, ... }` (`E_match`)
    Match {
        scrutinee: Box<MiniExp>,
        arms: Vec<MiniPExp>,
    },
    /// `l = v` (`E_assign`)
    Assign {
        lexp: Box<MiniLexp>,
        value: Box<MiniExp>,
    },
    /// `sizeof(n)` (`E_sizeof`)
    Sizeof(Nexp),
    /// `return e` (`E_return`)
    Return(Box<MiniExp>),
    /// `exit e` (`E_exit`)
    Exit(Box<MiniExp>),
    /// `%config` keys (`E_config`)
    Config(Vec<String>),
    /// `ref x` (`E_ref`)
    Ref(String),
    /// `throw e` (`E_throw`)
    Throw(Box<MiniExp>),
    /// `try e catch { pat [if g] => body, ... }` (`E_try`)
    Try {
        exp: Box<MiniExp>,
        arms: Vec<MiniPExp>,
    },
    /// `assert(cond, msg)` (`E_assert`)
    Assert { cond: Box<MiniExp>, msg: Box<MiniExp> },
    /// `var l = init in body` (`E_var`; distinct from [`MiniExp::Var`],
    /// which is the `E_id` variable reference)
    VarDecl {
        lexp: Box<MiniLexp>,
        init: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `undefined` (`E_undef`)
    Undef,
    /// `constraint(c)` (`E_constraint`)
    Constraint(NConstraint),
}

// ---------------------------------------------------------------------------
// Lane 10: canonical-path normalization, SMT-power detection, and
// arithmetic backward inference.
// ---------------------------------------------------------------------------
//
// Reference: OCaml `type_check.ml` (`nexp_simp`/`constraint_simp`,
// `constraint_power_variables`, `unify_nexp`); `sail-solver/src/canonical.rs`
// is read-only here. The canonical SMT emitter rejects `NExp::Exp` with a
// symbolic exponent and general numeric/constraint applications
// ("numeric application/power needs elaboration before proof",
// "constraint synonym needs expansion before proof"): those stay
// [`ProveResult::Unknown`](crate::solver::ProveResult) on the stub path and
// are decided only by stronger backends behind the [`Solver`] trait.
// What this section completes on the canonical path:
// - ground powers (`2^c`, `c` in `0..=126`) fold to constants;
// - symbolic powers are detected ([`Nexp::has_power`]/[`NConstraint::has_power`])
//   so callers can document/skip them instead of mis-proving;
// - cheap identities (`+0`, `-0`, `*0`, `*1`, `/1`, `mod 1`, double
//   negation, `if true/false`, boolean absorptions, `Set` sort/dedup)
//   are normalized away before proving, so `Eq`-reflexivity fires;
// - one-step backward inference ([`solve_for_var`]) inverts `Add`/`Sub`
//   (and constant `Mul`/`Div`) to solve omitted implicits from equations.

/// Bottom-up normalization of a level expression: ground subterms fold
/// via [`Nexp::as_const`], then one layer of total identities applies.
/// Pure, total, and shape-preserving for opaque subterms.
#[must_use]
pub fn normalize_nexp(n: &Nexp) -> Nexp {
    // Iterative post-order rebuild (no recursion): children first via
    // an explicit stack, assembled bottom-up keyed by node identity.
    // `if` conditions fold through the iterative constraint walker.
    enum Task<'a> {
        Visit(&'a Nexp),
        Build(&'a Nexp),
    }
    use std::collections::HashMap;
    let (constants, _) = eval_trees(Some(n), None);
    let mut stack = vec![Task::Visit(n)];
    let mut built: HashMap<*const Nexp, Nexp> = HashMap::new();
    while let Some(task) = stack.pop() {
        match task {
            Task::Visit(node) => {
                if built.contains_key(&(node as *const Nexp)) {
                    continue;
                }
                // Fold once per original node; a ground subtree needs no
                // rebuilt children (and no recursive drop of an unused branch).
                if let Some(value) = constants.get(&(node as *const Nexp)).copied().flatten() {
                    built.insert(node as *const Nexp, Nexp::Const(value));
                    continue;
                }
                stack.push(Task::Build(node));
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(inner) | Nexp::Abs(inner) | Nexp::Exp(inner) => {
                        stack.push(Task::Visit(inner));
                    }
                    Nexp::Add(first, second)
                    | Nexp::Sub(first, second)
                    | Nexp::Mul(first, second)
                    | Nexp::Div(first, second)
                    | Nexp::Mod(first, second) => {
                        stack.push(Task::Visit(first));
                        stack.push(Task::Visit(second));
                    }
                    Nexp::If(_, then_branch, else_branch) => {
                        stack.push(Task::Visit(then_branch));
                        stack.push(Task::Visit(else_branch));
                    }
                }
            }
            Task::Build(node) => {
                let child = |built: &mut HashMap<*const Nexp, Nexp>, node: &Nexp| {
                    built.remove(&(node as *const Nexp))
                };
                // Constant fold first (iterative `as_const`); on success
                // the folded constant wins, else the local identities.
                let folded =
                    |make: fn(Box<Nexp>, Box<Nexp>) -> Nexp, x: &Nexp, y: &Nexp| match (x, y) {
                        (Nexp::Const(a), Nexp::Const(b)) => {
                            make(Box::new(Nexp::Const(*a)), Box::new(Nexp::Const(*b))).as_const()
                        }
                        _ => None,
                    };
                let normalized = match node {
                    Nexp::Const(_) | Nexp::Var(_) => node.clone(),
                    Nexp::Neg(inner) => match child(&mut built, inner) {
                        Some(Nexp::Neg(x)) => *x,
                        Some(x) => Nexp::Neg(Box::new(x)),
                        None => node.clone(),
                    },
                    Nexp::Abs(inner) => match child(&mut built, inner) {
                        Some(Nexp::Abs(x)) => Nexp::Abs(x),
                        Some(x) => Nexp::Abs(Box::new(x)),
                        None => node.clone(),
                    },
                    Nexp::Exp(inner) => match child(&mut built, inner) {
                        Some(x) => Nexp::Exp(Box::new(x)),
                        None => node.clone(),
                    },
                    Nexp::Add(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => {
                                if let Some(c) = folded(Nexp::Add, &x, &y) {
                                    Nexp::Const(c)
                                } else {
                                    match (&x, &y) {
                                        (Nexp::Const(0), _) => y,
                                        (_, Nexp::Const(0)) => x,
                                        _ => Nexp::Add(Box::new(x), Box::new(y)),
                                    }
                                }
                            }
                            _ => node.clone(),
                        }
                    }
                    Nexp::Sub(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => {
                                if let Some(c) = folded(Nexp::Sub, &x, &y) {
                                    Nexp::Const(c)
                                } else if x == y {
                                    Nexp::Const(0)
                                } else {
                                    match &y {
                                        Nexp::Const(0) => x,
                                        _ => Nexp::Sub(Box::new(x), Box::new(y)),
                                    }
                                }
                            }
                            _ => node.clone(),
                        }
                    }
                    Nexp::Mul(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => {
                                if let Some(c) = folded(Nexp::Mul, &x, &y) {
                                    Nexp::Const(c)
                                } else {
                                    match (&x, &y) {
                                        (Nexp::Const(0), _) | (_, Nexp::Const(0)) => Nexp::Const(0),
                                        (Nexp::Const(1), _) => y,
                                        (_, Nexp::Const(1)) => x,
                                        _ => Nexp::Mul(Box::new(x), Box::new(y)),
                                    }
                                }
                            }
                            _ => node.clone(),
                        }
                    }
                    Nexp::Div(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => {
                                if let Some(c) = folded(Nexp::Div, &x, &y) {
                                    Nexp::Const(c)
                                } else {
                                    match &y {
                                        Nexp::Const(1) => x,
                                        _ => Nexp::Div(Box::new(x), Box::new(y)),
                                    }
                                }
                            }
                            _ => node.clone(),
                        }
                    }
                    Nexp::Mod(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => {
                                if let Some(c) = folded(Nexp::Mod, &x, &y) {
                                    Nexp::Const(c)
                                } else {
                                    match (&x, &y) {
                                        (_, Nexp::Const(1)) => Nexp::Const(0),
                                        (Nexp::Const(0), Nexp::Const(d)) if *d != 0 => {
                                            Nexp::Const(0)
                                        }
                                        _ => Nexp::Mod(Box::new(x), Box::new(y)),
                                    }
                                }
                            }
                            _ => node.clone(),
                        }
                    }
                    Nexp::If(cond, then_branch, else_branch) => {
                        let cc = normalize_constraint(cond);
                        match (
                            child(&mut built, then_branch),
                            child(&mut built, else_branch),
                        ) {
                            (Some(x), Some(y)) => match cc {
                                NConstraint::True => x,
                                NConstraint::False => y,
                                _ if x == y => x,
                                _ => Nexp::If(Box::new(cc), Box::new(x), Box::new(y)),
                            },
                            _ => node.clone(),
                        }
                    }
                };
                built.insert(node as *const Nexp, normalized);
            }
        }
    }
    built
        .remove(&(n as *const Nexp))
        .unwrap_or_else(|| n.clone())
}

/// Bottom-up normalization of a constraint: numeric sides via
/// [`normalize_nexp`], boolean absorptions, double negation, and `Set`
/// sort/dedup. Ground goals fold via [`NConstraint::as_bool`].
/// Uninterpreted predicates (`App`) and opaque booleans (`Var`/`Id`)
/// are preserved (soundly opaque, like the canonical emitter).
#[must_use]
pub fn normalize_constraint(c: &NConstraint) -> NConstraint {
    // Iterative post-order rebuild (no recursion): NExp children fold
    // through the iterative `normalize_nexp`; only And/Or/Not use the
    // explicit stack. Final `as_bool` fold as before.
    enum Task<'a> {
        Visit(&'a NConstraint),
        Build(&'a NConstraint),
    }
    use std::collections::HashMap;
    let mut stack = vec![Task::Visit(c)];
    let mut built: HashMap<*const NConstraint, NConstraint> = HashMap::new();
    while let Some(task) = stack.pop() {
        match task {
            Task::Visit(node) => {
                if built.contains_key(&(node as *const NConstraint)) {
                    continue;
                }
                stack.push(Task::Build(node));
                match node {
                    NConstraint::And(first, second) | NConstraint::Or(first, second) => {
                        stack.push(Task::Visit(first));
                        stack.push(Task::Visit(second));
                    }
                    NConstraint::Not(inner) => stack.push(Task::Visit(inner)),
                    _ => {}
                }
            }
            Task::Build(node) => {
                let child = |built: &mut HashMap<*const NConstraint, NConstraint>,
                             node: &NConstraint| {
                    built.remove(&(node as *const NConstraint))
                };
                let fold_bool = |made: NConstraint| match made.as_bool() {
                    Some(true) => NConstraint::True,
                    Some(false) => NConstraint::False,
                    None => made,
                };
                let rebuilt = match node {
                    NConstraint::True
                    | NConstraint::False
                    | NConstraint::Var(_)
                    | NConstraint::Id(_) => node.clone(),
                    NConstraint::Eq(a, b) => {
                        let (x, y) = (normalize_nexp(a), normalize_nexp(b));
                        fold_bool(NConstraint::Eq(Box::new(x), Box::new(y)))
                    }
                    NConstraint::Neq(a, b) => {
                        let (x, y) = (normalize_nexp(a), normalize_nexp(b));
                        fold_bool(NConstraint::Neq(Box::new(x), Box::new(y)))
                    }
                    NConstraint::Ge(a, b) => {
                        cmp_fold(NConstraint::Ge, a, b)
                    }
                    NConstraint::Gt(a, b) => {
                        cmp_fold(NConstraint::Gt, a, b)
                    }
                    NConstraint::Le(a, b) => {
                        cmp_fold(NConstraint::Le, a, b)
                    }
                    NConstraint::Lt(a, b) => {
                        cmp_fold(NConstraint::Lt, a, b)
                    }
                    NConstraint::And(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => match (&x, &y) {
                                (NConstraint::False, _) | (_, NConstraint::False) => {
                                    NConstraint::False
                                }
                                (NConstraint::True, _) => y,
                                (_, NConstraint::True) => x,
                                _ if x == y => x,
                                _ => NConstraint::And(Box::new(x), Box::new(y)),
                            },
                            _ => node.clone(),
                        }
                    }
                    NConstraint::Or(first, second) => {
                        match (child(&mut built, first), child(&mut built, second)) {
                            (Some(x), Some(y)) => match (&x, &y) {
                                (NConstraint::True, _) | (_, NConstraint::True) => {
                                    NConstraint::True
                                }
                                (NConstraint::False, _) => y,
                                (_, NConstraint::False) => x,
                                _ if x == y => x,
                                _ => NConstraint::Or(Box::new(x), Box::new(y)),
                            },
                            _ => node.clone(),
                        }
                    }
                    NConstraint::Not(inner) => match child(&mut built, inner) {
                        Some(NConstraint::True) => NConstraint::False,
                        Some(NConstraint::False) => NConstraint::True,
                        Some(NConstraint::Not(x)) => *x,
                        Some(x) => NConstraint::Not(Box::new(x)),
                        None => node.clone(),
                    },
                    NConstraint::Set(n, vs) => {
                        let x = normalize_nexp(n);
                        let mut members = vs.clone();
                        members.sort();
                        members.dedup();
                        fold_bool(NConstraint::Set(Box::new(x), members))
                    }
                    NConstraint::App(func, args) => NConstraint::App(
                        func.clone(),
                        args.iter().map(normalize_nexp).collect(),
                    ),
                };
                built.insert(node as *const NConstraint, rebuilt);
            }
        }
    }
    built
        .remove(&(c as *const NConstraint))
        .unwrap_or_else(|| c.clone())
}


fn cmp_fold(
    mk: fn(Box<Nexp>, Box<Nexp>) -> NConstraint,
    a: &Nexp,
    b: &Nexp,
) -> NConstraint {
    let (x, y) = (normalize_nexp(a), normalize_nexp(b));
    let folded = mk(Box::new(x.clone()), Box::new(y.clone()));
    if let Some(v) = folded.as_bool() {
        return if v {
            NConstraint::True
        } else {
            NConstraint::False
        };
    }
    folded
}

fn free_in(var: &str, n: &Nexp) -> bool {
    let mut out = Vec::new();
    n.free_vars(&mut out);
    out.iter().any(|v| v == var)
}

/// Solve the equation `lhs == rhs` for `var` with one-step arithmetic
/// backward inference (OCaml `unify_nexp`, linear skeleton only).
///
/// Handles `Var`, `Add`/`Sub` inversion, `Neg`, constant `Mul`/`Div`
/// with exact division, and mirrored orientations. Returns the solved
/// expression (normalized) or `None` when the shape is non-linear,
/// under-constrained, or fails the occurs check. Total.
#[must_use]
pub fn solve_for_var(var: &str, lhs: &Nexp, rhs: &Nexp) -> Option<Nexp> {
    solve_side(var, lhs, rhs)
        .or_else(|| solve_side(var, rhs, lhs))
        .map(|n| normalize_nexp(&n))
}

fn solve_side(var: &str, lhs: &Nexp, rhs: &Nexp) -> Option<Nexp> {
    // Iterative backward inference (no recursion): each step descends
    // `lhs` structurally while rebuilding `rhs`, so the loop is bounded
    // by the input depth. Owned state, no borrowing across iterations.
    let mut lhs = lhs.clone();
    let mut rhs = rhs.clone();
    loop {
        match lhs {
            Nexp::Var(ref v) if v == var => {
                if free_in(var, &rhs) {
                    return None;
                } else {
                    return Some(rhs);
                }
            }
            Nexp::Neg(a) => {
                rhs = Nexp::Neg(Box::new(rhs));
                lhs = *a;
            }
            Nexp::Add(a, b) => {
                let in_a = free_in(var, &a);
                let in_b = free_in(var, &b);
                match (in_a, in_b) {
                    (true, false) => {
                        rhs = Nexp::Sub(Box::new(rhs), Box::new((*b).clone()));
                        lhs = *a;
                    }
                    (false, true) => {
                        rhs = Nexp::Sub(Box::new(rhs), Box::new((*a).clone()));
                        lhs = *b;
                    }
                    _ => return None,
                }
            }
            Nexp::Sub(a, b) => {
                let in_a = free_in(var, &a);
                let in_b = free_in(var, &b);
                match (in_a, in_b) {
                    (true, false) => {
                        rhs = Nexp::Add(Box::new(rhs), Box::new((*b).clone()));
                        lhs = *a;
                    }
                    (false, true) => {
                        // `a - var == rhs`  =>  `var == a - rhs`.
                        let solved =
                            Nexp::Sub(Box::new((*a).clone()), Box::new(rhs.clone()));
                        if free_in(var, &solved) {
                            return None;
                        } else {
                            return Some(solved);
                        }
                    }
                    _ => return None,
                }
            }
            Nexp::Mul(a, b) => {
                let in_a = free_in(var, &a);
                let in_b = free_in(var, &b);
                match (in_a, in_b, (&*a, &*b)) {
                    (true, false, (_, Nexp::Const(c))) if *c != 0 => {
                        match rhs.as_const() {
                            Some(r) if r % *c == 0 => {
                                rhs = Nexp::Const(r / *c);
                                lhs = *a;
                            }
                            _ => return None,
                        }
                    }
                    (false, true, (Nexp::Const(c), _)) if *c != 0 => {
                        match rhs.as_const() {
                            Some(r) if r % *c == 0 => {
                                rhs = Nexp::Const(r / *c);
                                lhs = *b;
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
            Nexp::Div(a, b) => match (&*b, free_in(var, &a), free_in(var, &b)) {
                (Nexp::Const(c), true, false) if *c != 0 => {
                    rhs = Nexp::Mul(Box::new(rhs), Box::new(Nexp::Const(*c)));
                    lhs = *a;
                }
                _ => return None,
            },
            _ => return None,
        }
    }
}


impl Nexp {
    /// Whether this term contains a *symbolic* power (`2^n` with
    /// non-constant `n`). Ground powers fold via [`Nexp::as_const`]; only
    /// symbolic ones need SMT elaboration on the canonical path
    /// (mirrors OCaml `constraint_power_variables`).
    #[must_use]
    pub fn has_power(&self) -> bool {
        // Iterative walk with early exit (no recursion).
        let mut todo = vec![self];
        while let Some(current) = todo.pop() {
            match current {
                Self::Const(_) | Self::Var(_) => {}
                Self::Exp(a) => {
                    if a.as_const().is_none() {
                        return true;
                    }
                    todo.push(a);
                }
                Self::Neg(a) | Self::Abs(a) => todo.push(a),
                Self::Add(a, b)
                | Self::Sub(a, b)
                | Self::Mul(a, b)
                | Self::Div(a, b)
                | Self::Mod(a, b) => {
                    todo.push(a);
                    todo.push(b);
                }
                Self::If(c, a, b) => {
                    if c.has_power() {
                        return true;
                    }
                    todo.push(a);
                    todo.push(b);
                }
            }
        }
        false
    }
}

impl NConstraint {
    /// Whether this goal mentions a symbolic power. Such goals are
    /// soundly `Unknown` on the stub path (never mis-proved); the SMT
    /// backend decides them.
    #[must_use]
    pub fn has_power(&self) -> bool {
        // Iterative walk with early exit (no recursion).
        let mut todo = vec![self];
        while let Some(current) = todo.pop() {
            match current {
                Self::True | Self::False | Self::Var(_) | Self::Id(_) => {}
                Self::Eq(a, b)
                | Self::Neq(a, b)
                | Self::Ge(a, b)
                | Self::Gt(a, b)
                | Self::Le(a, b)
                | Self::Lt(a, b) => {
                    if a.has_power() || b.has_power() {
                        return true;
                    }
                }
                Self::And(a, b) | Self::Or(a, b) => {
                    todo.push(a);
                    todo.push(b);
                }
                Self::Not(a) => todo.push(a),
                Self::Set(n, _) => {
                    if n.has_power() {
                        return true;
                    }
                }
                Self::App(_, args) => {
                    if args.iter().any(Nexp::has_power) {
                        return true;
                    }
                }
            }
        }
        false
    }
}
