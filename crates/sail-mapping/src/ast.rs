//! Minimal Sail AST fragment for mappings.
//!
//! Only the shapes needed by `realize_mappings`, `mapping_patterns` and
//! `untranslatable_mapping` are modelled. Constructors mirror the OCaml AST
//! (`MPat_pat` / `MPat_when`, `MCL_bidir` / `MCL_forwards` /
//! `MCL_backwards`, `Pat_exp` / `Pat_when`, `E_let` / `E_match` / `E_app` /
//! `E_exit`).

#![forbid(unsafe_code)]

/// A mapping pattern (left- or right-hand side of a `<->` clause).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mpat {
    /// Enum constructor / literal, e.g. `BYTE`, `0b00`, `"ta"`.
    Lit(String),
    /// Variable binding, e.g. `s`, `sew`, `ta`.
    Var(String),
    /// Mapping application, e.g. `hex_bits(2, s)`, `ta_flag(sew)`.
    App(String, Vec<Mpat>),
    /// Tuple, e.g. `(ta, sew)`.
    Tuple(Vec<Mpat>),
    /// String concatenation, e.g. `ta_flag(sew) ^ ta_flag(ta)`.
    StrAppend(Vec<Mpat>),
    /// Bitvector concatenation, e.g. `ta @ sew`.
    VecAppend(Vec<Mpat>),
    /// Wildcard.
    Wild,
}

/// A match-arm pattern (after `pat_of_mpat`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pat {
    Lit(String),
    Var(String),
    App(String, Vec<Pat>),
    Tuple(Vec<Pat>),
    StrAppend(Vec<Pat>),
    VecAppend(Vec<Pat>),
    Wild,
}

/// A minimal expression language covering the bodies we elaborate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exp {
    /// Variable / function reference.
    Id(String),
    /// Literal.
    Lit(String),
    /// Function application `f(args)`.
    App(String, Vec<Exp>),
    /// Tuple.
    Tuple(Vec<Exp>),
    /// `let pat = value in body`. Only the body chain matters for
    /// [`crate::untranslatable_mapping`], but the binding is kept for fidelity.
    Let {
        bound: Pat,
        value: Box<Exp>,
        body: Box<Exp>,
    },
    /// `match scrutinee { arms }`.
    Match {
        scrutinee: Box<Exp>,
        arms: Vec<Arm>,
    },
    /// Sail `exit` (pattern-match failure / unreachable).
    Exit,
    /// Boolean literal.
    Bool(bool),
    /// Anything else (opaque payload for tests).
    Other(String),
}

/// One match arm: `pat [when guard] => body`.
///
/// `is_last` records the `mapping_last` attribute that
/// `rewrite_ast_realize_mappings` attaches to the final clause of each
/// direction (`annotate_pat ~last`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Exp>,
    pub body: Exp,
    pub is_last: bool,
}

impl Arm {
    pub fn plain(pat: Pat, body: Exp) -> Self {
        Arm {
            pat,
            guard: None,
            body,
            is_last: false,
        }
    }

    pub fn guarded(pat: Pat, guard: Exp, body: Exp) -> Self {
        Arm {
            pat,
            guard: Some(guard),
            body,
            is_last: false,
        }
    }
}

/// A mapping clause.
///
/// - `Bidir` = `mpexp1 <-> mpexp2` (`MCL_bidir`); each side is a pattern with
///   an optional `when` guard.
/// - `Forwards` = `forwards pat => exp` (`MCL_forwards`).
/// - `Backwards` = `backwards pat => exp` (`MCL_backwards`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingClause {
    Bidir {
        left: Mpat,
        left_guard: Option<Exp>,
        right: Mpat,
        right_guard: Option<Exp>,
    },
    Forwards {
        arm: Arm,
    },
    Backwards {
        arm: Arm,
    },
}

/// A mapping definition: name, bidirectional type, clauses.
///
/// Mirrors `MD_mapping (id, _, mapcls)` plus its `val` spec
/// `id : typ1 <-> typ2` (`Typ_bidir`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingDef {
    /// Mapping name, e.g. `size_bits`.
    pub name: String,
    /// Left type, e.g. `word_width`.
    pub typ1: String,
    /// Right type, e.g. `bits(2)`.
    pub typ2: String,
    pub clauses: Vec<MappingClause>,
}

impl MappingDef {
    pub fn forwards_id(&self) -> String {
        format!("{}_forwards", self.name)
    }

    pub fn backwards_id(&self) -> String {
        format!("{}_backwards", self.name)
    }

    pub fn forwards_matches_id(&self) -> String {
        format!("{}_forwards_matches", self.name)
    }

    pub fn backwards_matches_id(&self) -> String {
        format!("{}_backwards_matches", self.name)
    }
}

/// A realized (plain) function produced by `realize_mappings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunDef {
    pub name: String,
    pub arg_ty: String,
    pub ret_ty: String,
    pub arms: Vec<Arm>,
}

/// A realized bidirectional `val` spec split into one function spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValSpec {
    pub name: String,
    pub arg_ty: String,
    pub ret_ty: String,
}

/// Convert a mapping pattern into an arm pattern (`pat_of_mpat`).
pub fn pat_of_mpat(mpat: &Mpat) -> Pat {
    match mpat {
        Mpat::Lit(l) => Pat::Lit(l.clone()),
        Mpat::Var(v) => Pat::Var(v.clone()),
        Mpat::App(f, args) => Pat::App(f.clone(), args.iter().map(pat_of_mpat).collect()),
        Mpat::Tuple(ps) => Pat::Tuple(ps.iter().map(pat_of_mpat).collect()),
        Mpat::StrAppend(ps) => Pat::StrAppend(ps.iter().map(pat_of_mpat).collect()),
        Mpat::VecAppend(ps) => Pat::VecAppend(ps.iter().map(pat_of_mpat).collect()),
        Mpat::Wild => Pat::Wild,
    }
}

/// Convert a mapping pattern into an expression (`exp_of_mpat`).
pub fn exp_of_mpat(mpat: &Mpat) -> Exp {
    match mpat {
        Mpat::Lit(l) => Exp::Lit(l.clone()),
        Mpat::Var(v) => Exp::Id(v.clone()),
        Mpat::App(f, args) => Exp::App(f.clone(), args.iter().map(exp_of_mpat).collect()),
        Mpat::Tuple(ps) => Exp::Tuple(ps.iter().map(exp_of_mpat).collect()),
        // String / vector appends on the *expression* side become calls;
        // on the pattern side of an untranslatable direction Sail leaves an
        // `exit` behind (see `untranslatable_mapping`).
        Mpat::StrAppend(ps) => Exp::App(
            "string_append".to_string(),
            ps.iter().map(exp_of_mpat).collect(),
        ),
        Mpat::VecAppend(ps) => Exp::App(
            "vector_append".to_string(),
            ps.iter().map(exp_of_mpat).collect(),
        ),
        Mpat::Wild => Exp::Other("_".to_string()),
    }
}
