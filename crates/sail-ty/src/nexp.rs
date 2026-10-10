#![forbid(unsafe_code)]
//! Local minimal AST/nexp/typ types.
//!
//! These mirror the *shape* of `sail/src/libsail/ast.ml`
//! (`typ`, `nexp`, `n_constraint`, `typquant`) without copying any logic.
//! Once `sail-ast` (Lane 01) and `sail-nexp` (Lane 04) exist, replace these
//! with the real types — see `TY_NEEDED.md` for the field-by-field mapping.
//!
//! Design notes (low-RAM + multithreaded):
//! - Everything is `Clone + Send + Sync`, no interior `Rc`/raw pointers.
//! - `Nexp` is a boxed tree; [`Nexp::depth`] + [`CheckLimits`] bound recursion
//!   so checking without a solver always terminates.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::pat::MiniPat;

/// Numeric type-level expression (subset of Sail `nexp`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nexp {
    /// Integer constant.
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

impl Nexp {
    /// Syntactic tree depth. Used for fuel accounting.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Const(_) | Self::Var(_) => 1,
            Self::Neg(n) | Self::Exp(n) | Self::Abs(n) => 1 + n.depth(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Mod(a, b) => 1 + a.depth().max(b.depth()),
            Self::If(c, a, b) => 1 + c.depth().max(a.depth()).max(b.depth()),
        }
    }

    /// Collect free variables into `out` (dedup left to caller).
    pub fn free_vars(&self, out: &mut Vec<String>) {
        match self {
            Self::Const(_) => {}
            Self::Var(v) => out.push(v.clone()),
            Self::Neg(n) | Self::Exp(n) | Self::Abs(n) => n.free_vars(out),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Mod(a, b) => {
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::If(c, a, b) => {
                c.free_vars(out);
                a.free_vars(out);
                b.free_vars(out);
            }
        }
    }

    /// Try to fold to a constant. Returns `None` if not constant-foldable.
    /// Pure and total; bounded by tree depth (no solver needed).
    #[must_use]
    pub fn as_const(&self) -> Option<i128> {
        match self {
            Self::Const(c) => Some(*c),
            Self::Var(_) => None,
            Self::Neg(n) => n.as_const().and_then(|v| v.checked_neg()),
            Self::Abs(n) => n.as_const().and_then(|v| v.checked_abs()),
            Self::Exp(n) => {
                let v = n.as_const()?;
                if !(0..=126).contains(&v) {
                    return None;
                }
                2_i128.checked_pow(v as u32)
            }
            Self::Add(a, b) => Some(a.as_const()?.checked_add(b.as_const()?)?),
            Self::Sub(a, b) => Some(a.as_const()?.checked_sub(b.as_const()?)?),
            Self::Mul(a, b) => Some(a.as_const()?.checked_mul(b.as_const()?)?),
            Self::Div(a, b) => {
                let d = b.as_const()?;
                if d == 0 {
                    return None;
                }
                a.as_const()?.checked_div(d)
            }
            Self::Mod(a, b) => {
                let d = b.as_const()?;
                if d == 0 {
                    return None;
                }
                a.as_const()?.checked_rem(d)
            }
            Self::If(c, a, b) => match c.as_bool() {
                Some(true) => a.as_const(),
                Some(false) => b.as_const(),
                None => None,
            },
        }
    }
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
}

impl NConstraint {
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::True | Self::False => 1,
            Self::Eq(a, b)
            | Self::Neq(a, b)
            | Self::Ge(a, b)
            | Self::Gt(a, b)
            | Self::Le(a, b)
            | Self::Lt(a, b) => 1 + a.depth().max(b.depth()),
            Self::And(a, b) | Self::Or(a, b) => 1 + a.depth().max(b.depth()),
            Self::Not(a) => 1 + a.depth(),
        }
    }

    pub fn free_vars(&self, out: &mut Vec<String>) {
        match self {
            Self::True | Self::False => {}
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
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::Not(a) => a.free_vars(out),
        }
    }

    /// Boolean constant folding. `None` = not decidable syntactically.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Eq(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x == y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Neq(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x != y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::Ge(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x >= y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Gt(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x > y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::Le(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x <= y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Lt(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x < y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::And(a, b) => match (a.as_bool(), b.as_bool()) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Self::Or(a, b) => match (a.as_bool(), b.as_bool()) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Self::Not(a) => a.as_bool().map(|v| !v),
        }
    }
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
        match self {
            Self::Int | Self::Bool | Self::Bit => {}
            Self::Bits(n) => out.push((**n).clone()),
            Self::Range(lo, hi) => {
                out.push((**lo).clone());
                out.push((**hi).clone());
            }
            Self::Named { args, .. } => out.extend(args.iter().cloned()),
            Self::Fun(a, b) => {
                a.nexps(out);
                b.nexps(out);
            }
            Self::Tuple(ts) => {
                for t in ts {
                    t.nexps(out);
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
    /// `var l = init in body` (`E_var`)
    Var {
        lexp: Box<MiniLexp>,
        init: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `undefined` (`E_undef`)
    Undef,
    /// `constraint(c)` (`E_constraint`)
    Constraint(NConstraint),
}
