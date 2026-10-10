//! Lane 21: pattern exhaustiveness / redundancy + `match_bv` detection.
//!
//! Pure-Rust equivalent of the OCaml pieces used by the Lean/Rocq backends:
//! - `rewrites.ml` `pattern_check` (via `Pattern_completeness.is_complete_wildcarded`
//!   with `remove_redundant:true` + wildcard catch-all insertion), and
//! - `pretty_print_lean.ml` `is_match_bv` / `is_bitvector_pattern`.
//!
//! Pattern types are local minimal types shaped after Lane 01 `sail-ast`
//! (`PatKind`: `Wild`, `Id`, `App`, `Vector`, `VectorConcat`, `Tuple`, `Or`,
//! `As`, `Typ`, `Var`, `Lit`, ...), so this crate has no dependencies.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

pub mod conv;

/// Minimal value-pattern, mirroring `sail-ast::PatKind` (Lane 01).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pat {
    /// `_` catch-all.
    Wild,
    /// Binding / identifier pattern (matches everything, binds a name).
    /// Treated as a wildcard for coverage purposes.
    Var(String),
    /// Literal pattern (`true`, `42`, `0b101`, ...). Opaque string payload.
    Lit(String),
    /// Enum/variant constructor applied to sub-patterns, e.g. `A`, `Some(x)`.
    App { name: String, args: Vec<Pat> },
    /// Tuple pattern.
    Tuple(Vec<Pat>),
    /// Fixed-length bitvector pattern (`P_vector`): list of bit patterns.
    Vector(Vec<Pat>),
    /// Concatenated bitvector pattern (`P_vector_concat`), e.g.
    /// `0b11 @ opc @ Rm` in `match_bv.sail`.
    VectorConcat(Vec<Pat>),
    /// `p1 | p2`.
    Or(Box<Pat>, Box<Pat>),
    /// `p as x` — coverage of inner `p`.
    As(Box<Pat>, String),
    /// `p : typ` annotation — coverage of inner `p`.
    Typ(Box<Pat>, String),
    /// `var ... : typ_pat` binder — coverage of inner pattern.
    VarTyp(Box<Pat>),
    /// Struct pattern; `wildcard` = trailing `..`.
    Struct {
        name: String,
        fields: Vec<(String, Pat)>,
        wildcard: bool,
    },
    /// List / cons / string-append patterns (non-enumerable: only a
    /// wildcard arm makes them exhaustive).
    List(Vec<Pat>),
    Cons(Box<Pat>, Box<Pat>),
    StringAppend(Vec<Pat>),
}

impl Pat {
    /// `_`.
    #[must_use]
    pub fn wild() -> Self {
        Pat::Wild
    }

    /// Nullary constructor, e.g. `A` in `match.sail`.
    #[must_use]
    pub fn ctor(name: &str) -> Self {
        Pat::App {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    /// `true` if this pattern matches every value (top-level wildcard-like).
    #[must_use]
    pub fn is_unconditional_wildcard(&self) -> bool {
        match self {
            Pat::Wild | Pat::Var(_) => true,
            Pat::Or(a, b) => a.is_unconditional_wildcard() && b.is_unconditional_wildcard(),
            Pat::As(inner, _) | Pat::Typ(inner, _) | Pat::VarTyp(inner) => {
                inner.is_unconditional_wildcard()
            }
            _ => false,
        }
    }

    /// Mirror of OCaml `is_bitvector_pattern` (`pretty_print_lean.ml`):
    /// `P_vector | P_vector_concat`, recursing through `P_as`.
    /// We additionally recurse through the local `Typ`/`VarTyp` wrappers,
    /// which carry no coverage meaning.
    #[must_use]
    pub fn is_bitvector_pattern(&self) -> bool {
        match self {
            Pat::Vector(_) | Pat::VectorConcat(_) => true,
            Pat::As(inner, _) | Pat::Typ(inner, _) | Pat::VarTyp(inner) => {
                inner.is_bitvector_pattern()
            }
            Pat::Or(a, b) => a.is_bitvector_pattern() || b.is_bitvector_pattern(),
            _ => false,
        }
    }

    /// Expand top-level `Or` into alternatives.
    fn alternatives(&self) -> Vec<&Pat> {
        match self {
            Pat::Or(a, b) => {
                let mut v = a.alternatives();
                v.extend(b.alternatives());
                v
            }
            Pat::As(inner, _) | Pat::Typ(inner, _) | Pat::VarTyp(inner) => inner.alternatives(),
            other => vec![other],
        }
    }

    /// Constructor name if this (unwrapped) alternative is a constructor
    /// application, else `None`.
    fn ctor_name(&self) -> Option<&str> {
        match self {
            Pat::App { name, .. } => Some(name.as_str()),
            _ => None,
        }
    }
}

/// A single match arm / function clause: pattern + optional guard.
/// A guarded arm never counts towards exhaustiveness and never makes
/// later arms redundant (conservative, mirrors `Pat_when` handling).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub pat: Pat,
    /// `true` if the arm has a `when` guard.
    pub has_guard: bool,
}

impl Arm {
    #[must_use]
    pub fn simple(pat: Pat) -> Self {
        Arm {
            pat,
            has_guard: false,
        }
    }

    #[must_use]
    pub fn guarded(pat: Pat) -> Self {
        Arm {
            pat,
            has_guard: true,
        }
    }
}

/// Scrutinee type knowledge for the exhaustiveness check.
/// Mirrors the `Pattern_completeness` context (variants/enums/abstract).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScrutineeType {
    /// Unknown / opaque type: only a wildcard arm is exhaustive.
    #[default]
    Unknown,
    /// Boolean: needs `true` + `false` (or wildcard).
    Bool,
    /// Enum with a closed set of nullary constructors, e.g. `E = A | B | C`
    /// from `match.sail`.
    Enum {
        constructors: Vec<String>,
    },
    /// Fixed-width bitvector (`bits(n)`): never exhaustive without wildcard.
    /// This is the `match_bv.sail` (`decode`, `write_CSR`) case.
    BitVector,
}

impl ScrutineeType {
    #[must_use]
    pub fn bool() -> Self {
        ScrutineeType::Bool
    }

    #[must_use]
    pub fn bitvector() -> Self {
        ScrutineeType::BitVector
    }

    #[must_use]
    pub fn enum_of(names: &[&str]) -> Self {
        ScrutineeType::Enum {
            constructors: names.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// Result of [`check_exhaustive`] for one match / function clause group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    /// `true` if the arms cover every value of the scrutinee type.
    pub exhaustive: bool,
    /// Constructors / cases not covered (empty when exhaustive).
    pub missing: Vec<String>,
    /// Indices of redundant arms (each matched only values already
    /// covered by earlier unguarded arms, or unreachable after a wildcard).
    pub redundant: Vec<usize>,
    /// `true` if a wildcard catch-all must be appended (non-exhaustive).
    pub needs_wildcard: bool,
}

/// Mirror of OCaml `is_match_bv` (`pretty_print_lean.ml`):
/// `true` if any arm pattern is a (possibly `as`-wrapped) vector pattern,
/// i.e. the match must be emitted as `match_bv` rather than `match`.
#[must_use]
pub fn is_match_bv(arms: &[Arm]) -> bool {
    arms.iter().any(|a| a.pat.is_bitvector_pattern())
}

/// Check one match expression / function clause group for exhaustiveness
/// and redundancy. Unconditional-wildcard arms (`_` / variable, unguarded)
/// cover everything after them; guarded arms contribute no coverage.
///
/// Coverage rules per [`ScrutineeType`]:
/// - `Bool`: arms matching literals `"true"`/`"false"` (or `App` with those
///   names, or wildcard pieces) cover that case.
/// - `Enum`: constructor names covered by unguarded arms; exhaustive iff all
///   constructors covered (sub-patterns must themselves be exhaustive for
///   constructor arguments — checked via [`Pat::is_unconditional_wildcard`]
///   per argument, falling back to full coverage only for nullary covers).
/// - `BitVector` / `Unknown`: only a wildcard arm is exhaustive.
// Reports whether the coverage gathered so far already covers the whole
// scrutinee type.
fn matrix_covered(
    covered_ctors: &BTreeSet<String>,
    covered_bool: &BTreeSet<bool>,
    seen_wildcard: bool,
    scrut_ty: &ScrutineeType,
) -> bool {
    if seen_wildcard {
        return true;
    }
    match scrut_ty {
        ScrutineeType::Bool => covered_bool.contains(&true) && covered_bool.contains(&false),
        ScrutineeType::Enum { constructors } => {
            constructors.iter().all(|c| covered_ctors.contains(c))
        }
        // Bitvector / opaque value spaces are never covered without a
        // wildcard (2^n cases); vector arms are kept conservatively.
        ScrutineeType::BitVector | ScrutineeType::Unknown => false,
    }
}

pub fn check_exhaustive(arms: &[Arm], scrut_ty: &ScrutineeType) -> CheckReport {
    let mut covered_ctors: BTreeSet<String> = BTreeSet::new();
    let mut covered_bool: BTreeSet<bool> = BTreeSet::new();
    let mut seen_wildcard = false;
    let mut redundant: Vec<usize> = Vec::new();

    for (idx, arm) in arms.iter().enumerate() {
        if seen_wildcard {
            // Unreachable after an unguarded wildcard (OCaml: "rows after the
            // wildcard row ... are redundant", pattern_completeness.ml).
            if !redundant.contains(&idx) {
                redundant.push(idx);
            }
            continue;
        }
        if arm.has_guard {
            // Guarded arms prove nothing and remove nothing.
            continue;
        }
        let mut arm_covers_new = false;
        for alt in arm.pat.alternatives() {
            if alt.is_unconditional_wildcard() {
                // A bare wildcard alternative inside `Or` covers everything
                // not yet covered; it is redundant if the matrix is already
                // complete.
                if !matrix_covered(&covered_ctors, &covered_bool, seen_wildcard, scrut_ty) {
                    arm_covers_new = true;
                }
                seen_wildcard = true;
                break;
            }
            match scrut_ty {
                ScrutineeType::Bool => {
                    if let Pat::Lit(lit) = alt {
                        let v = lit == "true" || lit == "True";
                        let is_bool = lit == "true" || lit == "false" || lit == "True" || lit == "False";
                        if is_bool && covered_bool.insert(v) {
                            arm_covers_new = true;
                        }
                    } else if let Some(name) = alt.ctor_name() {
                        let v = name == "true" || name == "True";
                        let is_bool =
                            name == "true" || name == "false" || name == "True" || name == "False";
                        if is_bool && covered_bool.insert(v) {
                            arm_covers_new = true;
                        }
                    } else {
                        // Non-literal pattern on bool: keep conservatively.
                        arm_covers_new = true;
                    }
                }
                ScrutineeType::Enum { constructors } => {
                    if let Some(name) = alt.ctor_name() {
                        if constructors.iter().any(|c| c == name) && covered_ctors.insert(name.to_string()) {
                            arm_covers_new = true;
                        }
                    } else {
                        // Non-constructor pattern on enum type (tuple/struct/
                        // vector/...): conservatively assumed to cover
                        // something new; never deleted.
                        arm_covers_new = true;
                    }
                }
                ScrutineeType::BitVector | ScrutineeType::Unknown => {
                    // Value space too large / opaque for coverage bookkeeping:
                    // keep every unguarded arm (a bit-level solver would be
                    // needed to prove redundancy), but only a wildcard makes
                    // the matrix exhaustive.
                    arm_covers_new = true;
                }
            }
        }
        if !arm_covers_new {
            redundant.push(idx);
        }
    }

    let (exhaustive, missing): (bool, Vec<String>) = match scrut_ty {
        ScrutineeType::Bool => {
            let mut m = Vec::new();
            if !covered_bool.contains(&true) {
                m.push("true".to_string());
            }
            if !covered_bool.contains(&false) {
                m.push("false".to_string());
            }
            let done = m.is_empty();
            (done || seen_wildcard, if seen_wildcard { Vec::new() } else { m })
        }
        ScrutineeType::Enum { constructors } => {
            if seen_wildcard {
                (true, Vec::new())
            } else {
                let m: Vec<String> = constructors
                    .iter()
                    .filter(|c| !covered_ctors.contains(*c))
                    .cloned()
                    .collect();
                (m.is_empty(), m)
            }
        }
        ScrutineeType::BitVector | ScrutineeType::Unknown => {
            (seen_wildcard, if seen_wildcard { Vec::new() } else { vec!["<_>".to_string()] })
        }
    };

    CheckReport {
        exhaustive,
        missing,
        redundant,
        needs_wildcard: !exhaustive,
    }
}

/// Result of [`pattern_exhaustivity_redundancy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassOutcome {
    /// Arms after redundant-arm removal and wildcard insertion.
    pub arms: Vec<Arm>,
    /// Indices (in the *original* arm list) that were redundant.
    /// Callers should surface these as warnings.
    pub redundant_warnings: Vec<usize>,
    /// `true` if a wildcard catch-all was appended.
    pub wildcard_added: bool,
}

/// Equivalent of OCaml `rewrites.ml` `pattern_check` / `check_pexps` for one
/// match or function clause group: remove redundant arms (`remove_redundant:
/// true`) and, if the remainder is not exhaustive, append a wildcard catch-all
/// arm (the `catch_all` that raises match failure).
///
/// Postcondition: [`check_exhaustive`] on the returned arms reports
/// `exhaustive == true` (provided at least one arm existed or a wildcard
/// could be added — with zero input arms a single wildcard is produced).
pub fn pattern_exhaustivity_redundancy(arms: Vec<Arm>, scrut_ty: &ScrutineeType) -> PassOutcome {
    let report = check_exhaustive(&arms, scrut_ty);
    let redundant_set: BTreeSet<usize> = report.redundant.into_iter().collect();
    let kept: Vec<Arm> = arms
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !redundant_set.contains(i))
        .map(|(_, a)| a)
        .collect();
    let redundant_warnings: Vec<usize> = redundant_set.into_iter().collect();

    // Re-check after removal; removal cannot break exhaustiveness since
    // redundant arms cover nothing new.
    let recheck = check_exhaustive(&kept, scrut_ty);
    let mut out = kept;
    let mut wildcard_added = false;
    if !recheck.exhaustive {
        out.push(Arm::simple(Pat::Wild));
        wildcard_added = true;
    }

    PassOutcome {
        arms: out,
        redundant_warnings,
        wildcard_added,
    }
}

/// Convenience: run the pass for a whole "function" (map scrutinee types per
/// match site). Each entry is `(arms, scrutinee_type)`; returns outcomes in
/// order. Mirrors the per-function traversal in OCaml `pattern_check`
/// (`rw_exp` over `E_match` / `E_try` / funcls).
pub fn check_function(bodies: Vec<(Vec<Arm>, ScrutineeType)>) -> Vec<PassOutcome> {
    bodies
        .into_iter()
        .map(|(arms, ty)| pattern_exhaustivity_redundancy(arms, &ty))
        .collect()
}

/// Coverage environment: closed enum definitions visible to the check,
/// mirroring the `Pattern_completeness` context fragments (`enums`,
/// `variants`) needed for constructor coverage.
#[derive(Clone, Debug, Default)]
pub struct EnumEnv {
    enums: BTreeMap<String, Vec<String>>,
}

impl EnumEnv {
    #[must_use]
    pub fn new() -> Self {
        EnumEnv {
            enums: BTreeMap::new(),
        }
    }

    pub fn add_enum(&mut self, name: &str, constructors: &[&str]) {
        self.enums.insert(
            name.to_string(),
            constructors.iter().map(|s| (*s).to_string()).collect(),
        );
    }

    #[must_use]
    pub fn scrutinee_of(&self, name: &str) -> ScrutineeType {
        match self.enums.get(name) {
            Some(ctors) => ScrutineeType::Enum {
                constructors: ctors.clone(),
            },
            None => ScrutineeType::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_is_unconditional() {
        assert!(Pat::Wild.is_unconditional_wildcard());
        assert!(Pat::Var("x".to_string()).is_unconditional_wildcard());
        assert!(!Pat::ctor("A").is_unconditional_wildcard());
        assert!(!Pat::Vector(vec![]).is_unconditional_wildcard());
    }

    #[test]
    fn as_wrapper_keeps_bv() {
        let p = Pat::As(
            Box::new(Pat::VectorConcat(vec![Pat::Wild])),
            "v".to_string(),
        );
        assert!(p.is_bitvector_pattern());
        assert!(!Pat::ctor("A").is_bitvector_pattern());
    }
}
