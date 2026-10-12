#![forbid(unsafe_code)]
//! Minimal value patterns for the bidirectional checker (Lane 79).
//!
//! Covers the pattern rules `P-Wild`, `P-Var`, `P-Lit`, `P-Tuple`, `P-Annot`
//! and `P-ArityErr`. Richer Sail patterns (`as`, `or`, vector/struct/list
//! patterns) stay deferred to the Lane 02 full-`Exp` port (rule `X-FullPat`)
//! — inside this subset there are no check gaps.
//!
//! Canonical pattern coverage (Lane 09) lives below the [`MiniPat`] subset:
//! [`plan`] classifies every [`sail_ast::PatKind`] against the OCaml
//! `bind_pat`/`infer_pat` rules (`type_check.ml`), [`match_plan`]
//! decomposes `match` arms with sound redundancy hints from
//! [`PatOverlap`], struct helpers mirror `check_struct_name` plus the
//! missing-field rule, and [`def_gate`] gives every top-level definition
//! an explicit status — global `let`s via their pattern plan, mappings,
//! scattered/outcome and termination measures as loud deferrals, never
//! silent acceptance.

use crate::nexp::MiniTyp;
use sail_ast::{
    Def, DefKind, Exp, Interner as _, PExp, PExpKind, Pat, PatKind, RecOpt, ScatteredDef,
    SimpleInterner, Span, StructName, Typ, TypPat, TypPatKind,
};

pub use crate::canonical_eq::PatOverlap;
pub use crate::canonical_literal::LitPatRule;

/// Minimal pattern (subset of Sail `pat_aux`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniPat {
    /// `_`
    Wild,
    /// `x` — binds the scrutinee type.
    Var(String),
    /// Integer literal pattern.
    IntLit(i128),
    /// Boolean literal pattern.
    BoolLit(bool),
    /// `(p1, p2, ...)` — scrutinee must be a tuple of the same arity.
    Tuple(Vec<MiniPat>),
    /// `(p : t)` — check `p` against `t`, then require `t <: expected`.
    Annot(Box<MiniPat>, MiniTyp),
}

// ---------------------------------------------------------------------------
// Canonical pattern coverage (Lane 09)
// ---------------------------------------------------------------------------

/// Canonical rule code implemented (or explicitly deferred) by a [`PatPlan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatRule {
    Wild,
    Id,
    Lit,
    Tuple,
    Annot,
    As,
    Var,
    Or,
    Not,
    Cons,
    StringAppend,
    List,
    App,
    Vector,
    VectorConcat,
    VectorSubrange,
    Struct,
}

impl PatRule {
    /// Stable rule code: the `P-*` family of `rules::RULE_TABLE`
    /// (`ARCHITECTURE.md` mirror), extended where the Mini subset stops.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wild => "P-Wild",
            Self::Id => "P-Id",
            Self::Lit => "P-Lit",
            Self::Tuple => "P-Tuple",
            Self::Annot => "P-Annot",
            Self::As => "P-As",
            Self::Var => "P-Var",
            Self::Or => "P-Or",
            Self::Not => "P-Not",
            Self::Cons => "P-Cons",
            Self::StringAppend => "P-StringAppend",
            Self::List => "P-List",
            Self::App => "P-App",
            Self::Vector => "P-Vector",
            Self::VectorConcat => "P-VectorConcat",
            Self::VectorSubrange => "P-VectorSubrange",
            Self::Struct => "P-Struct",
        }
    }
}

/// Environment/solver query the canonical checker must still discharge to
/// finish a planned rule. Plans never guess these: constructor/mapping
/// resolution (`Env.is_union_constructor`/`is_mapping`), record field types
/// (`Env.get_record`/`get_accessor` plus quantifier unification), and
/// width/quantifier obligations (solver `prove`, sound on `Unknown`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingGate {
    pub rule: &'static str,
    pub query: String,
}

impl PendingGate {
    fn new(rule: &'static str, query: String) -> Self {
        Self { rule, query }
    }
}

/// Constructor/mapping argument form (OCaml `bind_pat` `P_app`): several
/// arguments normalize to one tuple argument — `Ctor(x, y)` checks as
/// `Ctor((x, y))` — so the checker only ever deconstructs a single type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppForm<'a> {
    Single(&'a Pat),
    Multi(Vec<&'a Pat>),
}

/// Struct-pattern name (OCaml `check_struct_name`): anonymous patterns
/// accept any record, named ones must spell the scrutinee record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructNamePlan {
    Anonymous,
    Named(String),
}

/// One-level canonical check plan for a [`Pat`]: the rule plus the owned
/// decomposition the checker executes iteratively. Sub-patterns stay
/// borrowed so arbitrarily deep patterns plan without recursion.
///
/// Every shape from `type_check.ml` `bind_pat`/`infer_pat` is covered:
/// previously the canonical checker (`canonical_body::pattern`) only
/// handled wild/id/as/typ/tuple/lit and failed the rest with one shared
/// "constructor, vector, list, disjunctive or dependent pattern binding"
/// diagnostic. Variants carrying a [`PendingGate`] name the exact
/// environment/solver query still owed — status, not silence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatPlan<'a> {
    Wild,
    /// `x`: binds the scrutinee type — unless the name is an enum member
    /// (constant) or a register (hard shadowing error); both need the
    /// value environment, named by the returned gate.
    Id {
        name: String,
        gate: PendingGate,
    },
    Lit {
        rule: LitPatRule,
    },
    Tuple {
        items: Vec<&'a Pat>,
    },
    Annot {
        typ: &'a Typ,
        pat: &'a Pat,
    },
    As {
        id: String,
        pat: &'a Pat,
    },
    /// `P_var (pat, typ_pat)`: `bind_typ_pat` binds the type variables,
    /// the inner pattern checks against the bound type. `Wild`/`Var`
    /// binders resolve here; `App` needs constructor resolution (gate).
    Var {
        typ_pat: &'a TypPat,
        pat: &'a Pat,
        binders: Vec<String>,
        gate: Option<PendingGate>,
    },
    /// `p1 | p2`: both sides check in an isolated scope
    /// (`Env.no_bindings`) — no bindings escape, only guards.
    Or {
        left: &'a Pat,
        right: &'a Pat,
    },
    /// `~p`: checks in an isolated scope, no bindings escape.
    Not {
        pat: &'a Pat,
    },
    /// `hd :: tl`: scrutinee must expand to `list(t)`; `hd <= t`,
    /// `tl <= list(t)` (synonym expansion is checker-side).
    Cons {
        head: &'a Pat,
        tail: &'a Pat,
    },
    /// String append: scrutinee must be `string`/`string_literal`.
    StringAppend {
        items: Vec<&'a Pat>,
    },
    /// `[p1, ..]`: scrutinee must expand to `list(t)`; each `pi <= t`.
    List {
        items: Vec<&'a Pat>,
    },
    /// Union-constructor or mapping application (gate), with multi-arg
    /// normalization. Constructor path: unify the return type with the
    /// scrutinee, discharge quantifiers, open existentials, check the
    /// argument. Mapping path: bidirectional `bidir` type, backwards first
    /// (annotated `backwards`), then forwards — else a hard "not a union
    /// constructor or mapping" error.
    App {
        func: String,
        form: AppForm<'a>,
        gate: PendingGate,
    },
    /// `[p1, .., pn]`: every element against `bit`; all element types
    /// provably equal (`typ_equality`); the pattern type is
    /// `bitvector(len)`. Non-bit element vectors stay partial (OCaml
    /// `BVS TODO`), named by the gate.
    Vector {
        items: Vec<&'a Pat>,
        gate: PendingGate,
    },
    /// Concatenation: the summed widths must provably equal the scrutinee
    /// width (solver obligation, sound on `Unknown`).
    VectorConcat {
        items: Vec<&'a Pat>,
        gate: PendingGate,
    },
    /// `id[n..m]`: binds `id` to the subrange width; width/order
    /// elaboration is owed (gate).
    VectorSubrange {
        id: String,
        lo: String,
        hi: String,
        gate: PendingGate,
    },
    /// `struct { f = p, .. }`: scrutinee must expand to a record; each
    /// field checks against its accessor type (gate: `get_accessor` plus
    /// record-argument unification). Missing fields without `..` are a
    /// hard error (see [`missing_fields`); `..` fills `field = _`.
    Struct {
        name: StructNamePlan,
        fields: Vec<(String, &'a Pat)>,
        wildcard: bool,
        gate: PendingGate,
    },
}

impl<'a> PatPlan<'a> {
    /// Canonical rule code for this plan.
    #[must_use]
    pub fn rule(&self) -> PatRule {
        match self {
            Self::Wild => PatRule::Wild,
            Self::Id { .. } => PatRule::Id,
            Self::Lit { .. } => PatRule::Lit,
            Self::Tuple { .. } => PatRule::Tuple,
            Self::Annot { .. } => PatRule::Annot,
            Self::As { .. } => PatRule::As,
            Self::Var { .. } => PatRule::Var,
            Self::Or { .. } => PatRule::Or,
            Self::Not { .. } => PatRule::Not,
            Self::Cons { .. } => PatRule::Cons,
            Self::StringAppend { .. } => PatRule::StringAppend,
            Self::List { .. } => PatRule::List,
            Self::App { .. } => PatRule::App,
            Self::Vector { .. } => PatRule::Vector,
            Self::VectorConcat { .. } => PatRule::VectorConcat,
            Self::VectorSubrange { .. } => PatRule::VectorSubrange,
            Self::Struct { .. } => PatRule::Struct,
        }
    }

    /// The still-owed environment/solver query, if any.
    #[must_use]
    pub fn gate(&self) -> Option<&PendingGate> {
        match self {
            Self::Id { gate, .. }
            | Self::App { gate, .. }
            | Self::Vector { gate, .. }
            | Self::VectorConcat { gate, .. }
            | Self::VectorSubrange { gate, .. }
            | Self::Struct { gate, .. } => Some(gate),
            Self::Var { gate, .. } => gate.as_ref(),
            _ => None,
        }
    }
}

/// Explicit pattern deferral: rule code, reason, and span for the
/// [`TypeErrorKind::Unsupported`](crate::error::TypeErrorKind) diagnostic.
/// No pattern shape is silently accepted or silently dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatDefer {
    pub rule: &'static str,
    pub reason: String,
    pub span: Span,
}

impl PatDefer {
    fn new(pat: &Pat, rule: &'static str, reason: String) -> Self {
        Self {
            rule,
            reason,
            span: pat.span,
        }
    }

    /// The loud checker error for this deferral.
    #[must_use]
    pub fn into_error(self) -> crate::error::TypeError {
        crate::error::TypeError {
            span: self.span,
            kind: crate::error::TypeErrorKind::Unsupported(format!(
                "{}: {}",
                self.rule, self.reason
            )),
        }
    }
}

fn name_text(symbol: sail_ast::Symbol, it: &SimpleInterner) -> Option<String> {
    it.resolve(symbol).map(str::to_owned)
}

/// Classify one pattern level into its canonical check plan.
///
/// Pure and total over [`PatKind`]: every shape yields either a [`PatPlan`]
/// (possibly with a [`PendingGate`] naming the owed environment/solver
/// query) or a [`PatDefer`] with rule code and diagnosis. Symbol
/// resolution failures (unknown interner entries) are the only plan-time
/// errors; everything else — constructor/mapping lookup, record accessors,
/// width proofs — stays an explicit gate for the checker, never a guess.
pub fn plan<'a>(pat: &'a Pat, it: &SimpleInterner) -> Result<PatPlan<'a>, PatDefer> {
    match &pat.kind {
        PatKind::Wild => Ok(PatPlan::Wild),
        PatKind::Id(name) => {
            let name = name_text(name.symbol, it)
                .ok_or_else(|| PatDefer::new(pat, "P-Id", "unresolvable pattern name".into()))?;
            Ok(PatPlan::Id {
                gate: PendingGate::new(
                    "P-Id",
                    format!(
                        "disambiguate '{name}': local binding vs enum member vs register shadow error"
                    ),
                ),
                name,
            })
        }
        PatKind::Lit(lit) => {
            let rule = crate::canonical_literal::pattern_rule(lit, it).map_err(|e| {
                PatDefer::new(pat, "P-Lit", format!("literal classification failed: {e}"))
            })?;
            Ok(PatPlan::Lit { rule })
        }
        PatKind::Tuple(items) => Ok(PatPlan::Tuple {
            items: items.iter().collect(),
        }),
        PatKind::Typ { typ, pat: inner } => Ok(PatPlan::Annot { typ, pat: inner }),
        PatKind::As { pat: inner, id } => {
            let id = name_text(id.symbol, it)
                .ok_or_else(|| PatDefer::new(pat, "P-As", "unresolvable as-binding name".into()))?;
            Ok(PatPlan::As { id, pat: inner })
        }
        PatKind::Var {
            pat: inner,
            typ_pat,
        } => {
            let (binders, gate) = match &typ_pat.kind {
                TypPatKind::Wild => (Vec::new(), None),
                TypPatKind::Var(symbol) => match name_text(*symbol, it) {
                    Some(binder) => (vec![binder], None),
                    None => {
                        return Err(PatDefer::new(
                            pat,
                            "P-Var",
                            "unresolvable type-pattern binder".into(),
                        ))
                    }
                },
                TypPatKind::App { .. } => (
                    Vec::new(),
                    Some(PendingGate::new(
                        "P-Var",
                        "type-pattern application needs constructor resolution (bind_typ_pat)"
                            .into(),
                    )),
                ),
            };
            Ok(PatPlan::Var {
                typ_pat,
                pat: inner,
                binders,
                gate,
            })
        }
        PatKind::Or(left, right) => Ok(PatPlan::Or { left, right }),
        PatKind::Not(inner) => Ok(PatPlan::Not { pat: inner }),
        PatKind::Cons(head, tail) => Ok(PatPlan::Cons { head, tail }),
        PatKind::StringAppend(items) => Ok(PatPlan::StringAppend {
            items: items.iter().collect(),
        }),
        PatKind::List(items) => Ok(PatPlan::List {
            items: items.iter().collect(),
        }),
        PatKind::App { func, args } => {
            let func = name_text(func.symbol, it).ok_or_else(|| {
                PatDefer::new(pat, "P-App", "unresolvable constructor name".into())
            })?;
            let form = if args.len() == 1 {
                AppForm::Single(&args[0])
            } else {
                AppForm::Multi(args.iter().collect())
            };
            Ok(PatPlan::App {
                gate: PendingGate::new(
                    "P-App",
                    format!(
                        "resolve '{func}' as union constructor or mapping (is_union_constructor/is_mapping); \
                         multi-arg normalizes to one tuple argument"
                    ),
                ),
                func,
                form,
            })
        }
        PatKind::Vector(items) => {
            if items.is_empty() {
                return Err(PatDefer::new(
                    pat,
                    "P-Vector",
                    "cannot infer the element type of an empty vector pattern".into(),
                ));
            }
            Ok(PatPlan::Vector {
                items: items.iter().collect(),
                gate: PendingGate::new(
                    "P-Vector",
                    "element types provably equal (typ_equality); non-bit elements are partial (BVS TODO)".into(),
                ),
            })
        }
        PatKind::VectorConcat(items) => {
            if items.is_empty() {
                return Err(PatDefer::new(
                    pat,
                    "P-VectorConcat",
                    "empty vector-concatenation pattern".into(),
                ));
            }
            Ok(PatPlan::VectorConcat {
                items: items.iter().collect(),
                gate: PendingGate::new(
                    "P-VectorConcat",
                    "summed subpattern widths provably equal the scrutinee width (solver)".into(),
                ),
            })
        }
        PatKind::VectorSubrange { id, lo, hi } => {
            let id = name_text(id.symbol, it).ok_or_else(|| {
                PatDefer::new(
                    pat,
                    "P-VectorSubrange",
                    "unresolvable subrange binding".into(),
                )
            })?;
            Ok(PatPlan::VectorSubrange {
                id,
                lo: lo.clone(),
                hi: hi.clone(),
                gate: PendingGate::new(
                    "P-VectorSubrange",
                    "subrange width/order elaboration (bitvector_typ_from_vector_subrange)".into(),
                ),
            })
        }
        PatKind::Struct {
            name,
            fields,
            wildcard,
        } => {
            let name = match name {
                StructName::Anon => StructNamePlan::Anonymous,
                StructName::Id(id) => {
                    StructNamePlan::Named(name_text(id.symbol, it).ok_or_else(|| {
                        PatDefer::new(pat, "P-Struct", "unresolvable struct name".into())
                    })?)
                }
            };
            let mut planned = Vec::with_capacity(fields.len());
            for (field, sub) in fields {
                let field = name_text(field.symbol, it).ok_or_else(|| {
                    PatDefer::new(pat, "P-Struct", "unresolvable struct field name".into())
                })?;
                planned.push((field, sub));
            }
            Ok(PatPlan::Struct {
                name,
                fields: planned,
                wildcard: *wildcard,
                gate: PendingGate::new(
                    "P-Struct",
                    "record field types via get_record/get_accessor plus record-argument unification".into(),
                ),
            })
        }
    }
}

/// OCaml `check_struct_name`: anonymous struct patterns accept any record;
/// named ones must spell the scrutinee record id, else
/// "Struct type X found, Y expected".
#[must_use]
pub fn struct_name_matches(plan: &StructNamePlan, record_id: &str) -> bool {
    match plan {
        StructNamePlan::Anonymous => true,
        StructNamePlan::Named(name) => name == record_id,
    }
}

/// OCaml missing-field rule: every declared record field absent from the
/// pattern is reported (in declaration order). With `..` the checker
/// instead fills `field = _` for exactly these; without it, a non-empty
/// result is a hard "struct pattern missing fields: ..." error.
#[must_use]
pub fn missing_fields<'a>(declared: &[&'a str], given: &[String]) -> Vec<&'a str> {
    declared
        .iter()
        .filter(|field| !given.iter().any(|name| name == **field))
        .copied()
        .collect()
}

// ---------------------------------------------------------------------------
// Match arms
// ---------------------------------------------------------------------------

/// One `match`/`try` arm decomposed (OCaml `pexp`: `PatExp` or `PatWhen`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm<'a> {
    pub pat: &'a Pat,
    pub guard: Option<&'a Exp>,
    pub body: &'a Exp,
}

/// Split `pexp` arms into pattern/guard/body triples.
pub fn match_arms(pexps: &[PExp]) -> Vec<MatchArm<'_>> {
    pexps
        .iter()
        .map(|arm| match &arm.kind {
            PExpKind::PatExp { pat, body } => MatchArm {
                pat,
                guard: None,
                body,
            },
            PExpKind::PatWhen { pat, guard, body } => MatchArm {
                pat,
                guard: Some(guard),
                body,
            },
        })
        .collect()
}

/// One planned arm: its pattern plan plus whether a guard refines it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArmPlan<'a> {
    pub pat: PatPlan<'a>,
    pub has_guard: bool,
}

/// Whole-`match` plan: every arm planned, plus provably redundant arm
/// pairs `(earlier, later)` — later arms whose match set provably equals an
/// earlier one ([`PatOverlap::Equal`]). Informational only: [`PatOverlap`]
/// reports `Equal` solely on positive syntactic facts, so a flagged arm is
/// genuinely dead, while unflagged arms may still overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPlan<'a> {
    pub arms: Vec<MatchArmPlan<'a>>,
    pub redundant: Vec<(usize, usize)>,
}

/// Plan every arm of a `match`/`try` against one scrutinee.
///
/// An empty arm list is a hard error (no value can be matched); arm
/// exhaustiveness beyond that needs the completion environment
/// (`Pattern_completeness`: abstract types, variants, records, enums plus
/// scattered openness and constraints) and stays an explicit gate on the
/// returned plan via [`MatchPlan::redundant`] being purely syntactic.
pub fn match_plan<'a>(pexps: &'a [PExp], it: &SimpleInterner) -> Result<MatchPlan<'a>, PatDefer> {
    if pexps.is_empty() {
        return Err(PatDefer {
            rule: "E-Match",
            reason: "match with no arms matches no value".into(),
            span: Span::DUMMY,
        });
    }
    let arms = match_arms(pexps);
    let mut planned = Vec::with_capacity(arms.len());
    for arm in &arms {
        planned.push(MatchArmPlan {
            pat: plan(arm.pat, it)?,
            has_guard: arm.guard.is_some(),
        });
    }
    // Pairwise redundancy over the raw patterns (same interner both sides).
    let mut redundant = Vec::new();
    for (i, first) in arms.iter().enumerate() {
        for (j, second) in arms.iter().enumerate().skip(i + 1) {
            if crate::canonical_eq::pat_overlap(first.pat, second.pat, it, it) == PatOverlap::Equal
            {
                redundant.push((i, j));
            }
        }
    }
    Ok(MatchPlan {
        arms: planned,
        redundant,
    })
}

/// Sound overlap query for two patterns sharing one interner
/// (the common `match`-arm case of [`crate::canonical_eq::pat_overlap`]).
#[must_use]
pub fn arm_overlap(a: &Pat, b: &Pat, it: &SimpleInterner) -> PatOverlap {
    crate::canonical_eq::pat_overlap(a, b, it, it)
}

// ---------------------------------------------------------------------------
// Top-level definition gates
// ---------------------------------------------------------------------------}

/// Explicit status of one top-level definition for the canonical checker:
/// either covered by an implemented rule, or deferred with rule code and
/// diagnosis. There is no third state — nothing is silently accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefGate {
    Covered { rule: &'static str },
    Deferred { rule: &'static str, reason: String },
}

impl DefGate {
    #[must_use]
    pub fn is_covered(&self) -> bool {
        matches!(self, Self::Covered { .. })
    }

    #[must_use]
    pub fn rule(&self) -> &'static str {
        match self {
            Self::Covered { rule } | Self::Deferred { rule, .. } => rule,
        }
    }

    /// One-line diagnosis for reports and `--explain`-style output.
    #[must_use]
    pub fn diagnosis(&self) -> String {
        match self {
            Self::Covered { rule } => format!("{rule}: covered"),
            Self::Deferred { rule, reason } => format!("{rule}: deferred ({reason})"),
        }
    }
}

fn scattered_name(kind: &ScatteredDef) -> &'static str {
    match kind {
        ScatteredDef::Function { .. } | ScatteredDef::Funcl(_) => "function",
        ScatteredDef::Variant { .. } | ScatteredDef::Unioncl { .. } => "union",
        ScatteredDef::Mapping { .. } | ScatteredDef::Mapcl { .. } => "mapping",
        ScatteredDef::Enum(_) | ScatteredDef::Enumcl { .. } => "enum",
        ScatteredDef::End(_) => "end",
    }
}

/// Gate one top-level definition, mirroring the `canonical_body::check`
/// dispatch so the two can never drift silently: signature-only and
/// register definitions are covered; `function` clauses are covered for
/// non-measure recursion; global `let`s are covered exactly when their
/// pattern plans ([`plan`]); mappings, scattered blocks, outcomes,
/// instantiations, mutual-recursion bundles and termination measures
/// (function-level `RecOpt::Measure`, `Measure` and `LoopMeasures` defs)
/// are explicit deferrals with rule codes.
pub fn def_gate(def: &Def, it: &SimpleInterner) -> DefGate {
    match &def.kind {
        DefKind::Type(_)
        | DefKind::Val(_)
        | DefKind::Constraint(_)
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => DefGate::Covered { rule: "D-Sig" },
        DefKind::Register(_) => DefGate::Covered { rule: "D-Reg" },
        DefKind::Fundef { rec, .. } => match rec {
            RecOpt::NonRec | RecOpt::Rec => DefGate::Covered { rule: "D-Fun" },
            RecOpt::Measure { .. } => DefGate::Deferred {
                rule: "D-Measure",
                reason: "termination-measure typing (RecOpt::Measure)".into(),
            },
        },
        DefKind::Let { pat, .. } => match plan(pat, it) {
            Ok(planned) => DefGate::Covered {
                rule: planned.rule().as_str(),
            },
            Err(deferred) => DefGate::Deferred {
                rule: deferred.rule,
                reason: format!("global let pattern: {}", deferred.reason),
            },
        },
        DefKind::Mapdef { .. } => DefGate::Deferred {
            rule: "D-Mapping",
            reason: "mapping definition (bidirectional clause elaboration)".into(),
        },
        DefKind::Impl(_) => DefGate::Deferred {
            rule: "D-Impl",
            reason: "impl definition (overload instance body)".into(),
        },
        DefKind::Scattered(kind) => DefGate::Deferred {
            rule: "D-Scattered",
            reason: format!(
                "scattered {} block (open clauses, end-linkage pending)",
                scattered_name(kind)
            ),
        },
        DefKind::Outcome { .. } => DefGate::Deferred {
            rule: "D-Outcome",
            reason: "outcome definition (instantiation-guarded body)".into(),
        },
        DefKind::Instantiation { .. } => DefGate::Deferred {
            rule: "D-Instantiation",
            reason: "instantiation definition (substitution application)".into(),
        },
        DefKind::Measure { .. } => DefGate::Deferred {
            rule: "D-Measure",
            reason: "termination-measure definition".into(),
        },
        DefKind::LoopMeasures { .. } => DefGate::Deferred {
            rule: "D-LoopMeasure",
            reason: "loop termination-measure definition".into(),
        },
        DefKind::InternalMutrec(_) => DefGate::Deferred {
            rule: "D-Mutrec",
            reason: "mutual-recursion bundle (simultaneous signatures)".into(),
        },
    }
}
