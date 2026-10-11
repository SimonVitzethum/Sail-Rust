#![forbid(unsafe_code)]
//! Rule catalogue: Sail typing rule → implementation status → covering test.
//!
//! Lane 79 requires full rule coverage of the bidirectional checker with no
//! `infer`/`check` gaps *inside the `MiniExp`/`MiniPat` subset*. Rules
//! outside that subset are explicitly [`RuleStatus::Deferred`] with an owner
//! lane — they are not silent gaps. The human-readable mirror is
//! `ARCHITECTURE.md`; the machine-readable source of truth is
//! [`RULE_TABLE`], kept in sync by `tests/typechecker_full.rs`
//! (`rule_table_complete`).

/// A single Sail typing rule covered (or explicitly deferred) by this lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleId {
    // -- NExp kinds (`K_int` / `K_nat`) --
    /// Constants: non-negative ⇒ `K_nat`, negative ⇒ `K_int`.
    KConst,
    /// Level variables: kind from the environment.
    KVar,
    /// `+`, `-`, `*` (`nat+nat=nat`, else `int`).
    KArith,
    /// `/`, `mod`: always `K_int`.
    KDivMod,
    /// Unary `-`: `K_int`.
    KNeg,
    /// `2^n`: `K_nat` for provably non-negative exponents, else `K_int`.
    KExp,
    /// `|n|`: `K_int`.
    KAbs,
    /// `if c then n else m`: condition is `K_bool`, branches join.
    KIf,
    /// `K_nat <: K_int` subsumption at bind sites.
    KNatSub,
    // -- Constraint kinds (`K_bool`) --
    /// `true` / `false`.
    CKTrueFalse,
    /// `==`, `!=`, `>=`, `>`, `<=`, `<` over `K_int` sides.
    CKCmp,
    /// `NC_set` membership (`K_int` scrutinee), `NC_app` over `K_int` args,
    /// `NC_var` bound at `K_bool`, `NC_id` opaque `K_bool`.
    CKSetApp,
    /// `/\`, `\/` over `K_bool` sides.
    CKAndOr,
    /// `~a` over a `K_bool` side.
    CKNot,
    // -- Type kinds (`K_type`) --
    /// `int`, `bool`, `bit`.
    TIntBoolBit,
    /// `bits(n)`: `n : K_int`-like.
    TBits,
    /// Provably negative `bits(n)` lengths are rejected.
    TBitsNeg,
    /// `range(lo,hi)`: both bounds `K_int`-like.
    TRange,
    /// Named abbreviations: all level arguments `K_int`-like.
    TNamed,
    /// `t1 -> t2`: both sides `K_type`.
    TFun,
    /// Tuples: every element `K_type`.
    TTuple,
    // -- Quantifiers / schemes --
    /// `forall telescope. nc => t`: kinds well-formed.
    QForall,
    /// No escaping free level variables (bound ⊇ free).
    QEscape,
    /// Depth bound ⇒ termination even for adversarial schemes.
    QDepthFuel,
    /// Instantiation at use sites: gensym freshen + discharge precondition.
    QInst,
    /// Capture-avoiding freshening of telescopes (gensym, OCaml `fresh_kid`).
    QFreshen,
    // -- Definitions --
    /// Monomorphic `val` body checked against the scheme type.
    DValMono,
    /// Polymorphic `val`: skolemized body check — telescope freshened with
    /// gensym names, precondition discharged, body checked against the
    /// freshened body type (full instantiation stays Lane 09).
    DValPoly,
    /// Refuted scheme preconditions are hard errors.
    DValPreRefuted,
    /// Monomorphic registers.
    DRegMono,
    /// Polymorphic registers (e.g. `forall 'n. 'n >= 0 => bits('n)`).
    DRegPoly,
    /// Ill-kinded register types are rejected.
    DRegBad,
    /// `scattered` clauses: closed schemes, clause `i` discharged under
    /// the constraints of `clauses[..i]` (accumulated solver context),
    /// summaries namespaced per `ScatteredKind` (`function`/`mapping`/
    /// `union`/`enum`).
    DScattered,
    // -- Inference (`infer`) --
    /// Integer literals synthesize `int`.
    IInt,
    /// Boolean literals synthesize `bool`.
    IBool,
    /// Monomorphic variables / locals.
    IVarMono,
    /// Polymorphic `val`/register use: instantiation (see `Q-Inst`); only
    /// `Proved` preconditions yield a type — open obligations have no
    /// channel on `infer` and are loud errors (rule `Z-UnknownSound`).
    IVarPoly,
    /// `e : t` synthesizes `t` after checking `e <= t`.
    IAnnot,
    /// Tuples synthesize pointwise.
    ITuple,
    /// `if`: condition `<= bool`, branches join.
    IIf,
    /// `let` (plain and annotated): bound checked, body in extended env.
    ILet,
    /// Function application against a `Fun` head.
    IApp,
    /// Non-`Fun` heads are application errors.
    IAppHeadErr,
    // -- Checking (`check`) --
    /// Integer literals against constant `range` bounds (syntactic).
    CLitRangeConst,
    /// Integer literals against symbolic `range` bounds (via solver).
    CLitRangeSolver,
    /// `bits(n)` length obligations (kind check + unification).
    CBitsLength,
    /// Fallback: infer + subsumption for every other shape.
    CFallbackSub,
    // -- Subsumption (`S-*`) / coercions (`X-*`) --
    /// Reflexivity.
    SRefl,
    /// `range <: int` (bounds forgotten).
    SRangeInt,
    /// `int <: range` rejected outside literal sites.
    SIntRangeFail,
    /// `range` inclusion via solver (`lo2<=lo1 /\ hi1<=hi2`).
    SRangeRange,
    /// `bits(n) <: bits(m)` iff `n == m` proved.
    SBitsEq,
    /// Functions: contravariant domain, covariant codomain.
    SFunContraCo,
    /// Tuples: pointwise; arity mismatch is `ArityMismatch`.
    STupleArity,
    /// Same-head abbreviations with provably equal level arguments.
    SNamedArgs,
    /// Distinct base types never unify.
    SBaseMismatch,
    /// Literal coercion `5 <= range(..)` (const + solver paths).
    XLitToRange,
    /// `range(..) <= int` coercion.
    XRangeToInt,
    // -- Patterns (`P-*`) --
    /// `_` binds nothing.
    PWild,
    /// `x` binds the scrutinee type.
    PVar,
    /// Literal patterns check the literal against the scrutinee type.
    PLit,
    /// Tuple patterns destructure pointwise.
    PTuple,
    /// `(p : t)` then `t <: expected`.
    PAnnot,
    /// Arity mismatch is `ArityMismatch`.
    PArityErr,
    // -- Solver policy / termination (`Z-*`) --
    /// Checker is generic over [`crate::solver::Solver`]; no Z3 import in
    /// `sail-ty` (Z3 plugs in via the trait in Lane 06/80).
    ZTraitOnly,
    /// [`crate::solver::StubSolver`] decides the obvious and is total.
    ZStubTotal,
    /// `Unknown` is sound: error at proof sites, never a fabricated proof.
    ZUnknownSound,
    /// [`crate::solver::CachingSolver`] stays capacity-bounded.
    ZCacheBounded,
    /// Fuel/depth limits guarantee termination without a solver.
    ZFuelTerm,
    /// `goal` proved under a [`crate::prove::SolverContext`] (kind-checked
    /// goal, `val`/`register`/`scattered` assumptions as one implication)
    /// through the [`crate::solver::Solver`] trait only.
    ZProveCtx,
    // -- Environment (`E-*`) --
    /// Locals shadow globals.
    ELocalShadow,
    /// Monomorphic schemes project into value lookup.
    EMonoProject,
    /// Polymorphic schemes stay opaque to plain lookup (via `Q-Inst` only).
    EPolyOpaque,
    // -- Explicitly deferred (owner lanes, not gaps) --
    /// Existential types (`Typ_exist`): Lane 09.
    XExistential,
    /// Full Sail `exp` (loops, vectors, structs, match, …): Lane 02.
    XFullExp,
    /// Full Sail `pat` (`as`/`or`/vector/struct/…): Lane 02.
    XFullPat,
    /// `NC_set` / `NC_app` / `NC_var` constraints: foreign shapes with no
    /// local counterpart (`NExp_id`, non-`NExp` `typ_arg`s, unparseable
    /// `Set` members) still lift to `None` in [`crate::conv`].
    XNcSetApp,
}

impl RuleId {
    /// Stable rule code used in `ARCHITECTURE.md` and diagnostics.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KConst => "K-Const",
            Self::KVar => "K-Var",
            Self::KArith => "K-Arith",
            Self::KDivMod => "K-DivMod",
            Self::KNeg => "K-Neg",
            Self::KExp => "K-Exp",
            Self::KAbs => "K-Abs",
            Self::KIf => "K-If",
            Self::KNatSub => "K-NatSub",
            Self::CKTrueFalse => "CK-TrueFalse",
            Self::CKCmp => "CK-Cmp",
            Self::CKSetApp => "CK-SetApp",
            Self::CKAndOr => "CK-AndOr",
            Self::CKNot => "CK-Not",
            Self::TIntBoolBit => "T-IntBoolBit",
            Self::TBits => "T-Bits",
            Self::TBitsNeg => "T-BitsNeg",
            Self::TRange => "T-Range",
            Self::TNamed => "T-Named",
            Self::TFun => "T-Fun",
            Self::TTuple => "T-Tuple",
            Self::QForall => "Q-Forall",
            Self::QEscape => "Q-Escape",
            Self::QDepthFuel => "Q-DepthFuel",
            Self::QInst => "Q-Inst",
            Self::QFreshen => "Q-Freshen",
            Self::DValMono => "D-ValMono",
            Self::DValPoly => "D-ValPoly",
            Self::DValPreRefuted => "D-ValPreRefuted",
            Self::DRegMono => "D-RegMono",
            Self::DRegPoly => "D-RegPoly",
            Self::DRegBad => "D-RegBad",
            Self::DScattered => "D-Scattered",
            Self::IInt => "I-Int",
            Self::IBool => "I-Bool",
            Self::IVarMono => "I-VarMono",
            Self::IVarPoly => "I-VarPoly",
            Self::IAnnot => "I-Annot",
            Self::ITuple => "I-Tuple",
            Self::IIf => "I-If",
            Self::ILet => "I-Let",
            Self::IApp => "I-App",
            Self::IAppHeadErr => "I-AppHeadErr",
            Self::CLitRangeConst => "C-LitRangeConst",
            Self::CLitRangeSolver => "C-LitRangeSolver",
            Self::CBitsLength => "C-BitsLength",
            Self::CFallbackSub => "C-FallbackSub",
            Self::SRefl => "S-Refl",
            Self::SRangeInt => "S-RangeInt",
            Self::SIntRangeFail => "S-IntRangeFail",
            Self::SRangeRange => "S-RangeRange",
            Self::SBitsEq => "S-BitsEq",
            Self::SFunContraCo => "S-FunContraCo",
            Self::STupleArity => "S-TupleArity",
            Self::SNamedArgs => "S-NamedArgs",
            Self::SBaseMismatch => "S-BaseMismatch",
            Self::XLitToRange => "X-LitToRange",
            Self::XRangeToInt => "X-RangeToInt",
            Self::PWild => "P-Wild",
            Self::PVar => "P-Var",
            Self::PLit => "P-Lit",
            Self::PTuple => "P-Tuple",
            Self::PAnnot => "P-Annot",
            Self::PArityErr => "P-ArityErr",
            Self::ZTraitOnly => "Z-TraitOnly",
            Self::ZStubTotal => "Z-StubTotal",
            Self::ZUnknownSound => "Z-UnknownSound",
            Self::ZCacheBounded => "Z-CacheBounded",
            Self::ZFuelTerm => "Z-FuelTerm",
            Self::ZProveCtx => "Z-ProveCtx",
            Self::ELocalShadow => "E-LocalShadow",
            Self::EMonoProject => "E-MonoProject",
            Self::EPolyOpaque => "E-PolyOpaque",
            Self::XExistential => "X-Existential",
            Self::XFullExp => "X-FullExp",
            Self::XFullPat => "X-FullPat",
            Self::XNcSetApp => "X-NcSetApp",
        }
    }

    /// Short Sail-side reference (conceptual; exact OCaml symbols move).
    #[must_use]
    pub fn sail_ref(self) -> &'static str {
        match self {
            Self::KConst
            | Self::KVar
            | Self::KArith
            | Self::KDivMod
            | Self::KNeg
            | Self::KExp
            | Self::KAbs
            | Self::KIf => "Sail nexp kind rule (K_int introduction)",
            Self::KNatSub => "Sail K_nat <: K_int refinement (Lean Nat mapping)",
            Self::CKTrueFalse | Self::CKCmp | Self::CKAndOr | Self::CKNot => {
                "Sail n_constraint kind rule (K_bool introduction)"
            }
            Self::CKSetApp => "Sail NC_set/NC_app/NC_var/NC_id kind rule (K_bool introduction)",
            Self::TIntBoolBit
            | Self::TBits
            | Self::TBitsNeg
            | Self::TRange
            | Self::TNamed
            | Self::TFun
            | Self::TTuple => "Sail typ kind rule (K_type introduction)",
            Self::QForall | Self::QEscape | Self::QDepthFuel => {
                "Sail typquant well-formedness (forall telescope)"
            }
            Self::QInst | Self::QFreshen => "Sail scheme instantiation (freshen + discharge)",
            Self::DValMono | Self::DValPoly | Self::DValPreRefuted => {
                "Sail val-spec / funcl body checking"
            }
            Self::DScattered => "Sail scattered function clause checking",
            Self::DRegMono | Self::DRegPoly | Self::DRegBad => {
                "Sail register type checking"
            }
            Self::IInt
            | Self::IBool
            | Self::IVarMono
            | Self::IVarPoly
            | Self::IAnnot
            | Self::ITuple
            | Self::IIf
            | Self::ILet
            | Self::IApp
            | Self::IAppHeadErr => "Sail expression synthesis (infer)",
            Self::CLitRangeConst
            | Self::CLitRangeSolver
            | Self::CBitsLength
            | Self::CFallbackSub => "Sail expression checking (check) + numeric obligations",
            Self::SRefl
            | Self::SRangeInt
            | Self::SIntRangeFail
            | Self::SRangeRange
            | Self::SBitsEq
            | Self::SFunContraCo
            | Self::STupleArity
            | Self::SNamedArgs
            | Self::SBaseMismatch => "Sail subtyp relation",
            Self::XLitToRange | Self::XRangeToInt => "Sail numeric coercion",
            Self::PWild | Self::PVar | Self::PLit | Self::PTuple | Self::PAnnot | Self::PArityErr => {
                "Sail pattern typing"
            }
            Self::ZTraitOnly
            | Self::ZStubTotal
            | Self::ZUnknownSound
            | Self::ZCacheBounded
            | Self::ZFuelTerm => "Sail constraint solver interface (prove-or-unknown)",
            Self::ZProveCtx => "Sail solver context (assumptions + kind env, trait-only)",
            Self::ELocalShadow | Self::EMonoProject | Self::EPolyOpaque => {
                "Sail typing environment (locals / val specs / registers)"
            }
            Self::XExistential => "Sail Typ_exist (deferred: Lane 09)",
            Self::XFullExp => "Sail full exp_aux (deferred: Lane 02)",
            Self::XFullPat => "Sail full pat_aux (deferred: Lane 02)",
            Self::XNcSetApp => "Sail NC_set/NC_app/NC_var foreign shapes (no local counterpart)",
        }
    }
}

/// Implementation status of a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleStatus {
    /// Implemented in `sail-ty`, covered by the linked test.
    Implemented,
    /// Explicitly out of scope with an owner lane and reason.
    Deferred(&'static str),
}

impl RuleStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Deferred(_) => "deferred",
        }
    }
}

/// One catalogue row: rule → status → covering test.
#[derive(Clone, Copy, Debug)]
pub struct RuleEntry {
    pub id: RuleId,
    pub status: RuleStatus,
    pub test: &'static str,
}

const I: RuleStatus = RuleStatus::Implemented;

/// The rule table (Regel → Status → Test). Source of truth; `ARCHITECTURE.md`
/// mirrors it for readers.
pub static RULE_TABLE: &[RuleEntry] = &[
    RuleEntry { id: RuleId::KConst, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KVar, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KArith, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KDivMod, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KNeg, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KExp, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KAbs, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::KIf, status: I, test: "kind_nexp_if" },
    RuleEntry { id: RuleId::KNatSub, status: I, test: "kind_nexp_arith" },
    RuleEntry { id: RuleId::CKTrueFalse, status: I, test: "constraint_kinds" },
    RuleEntry { id: RuleId::CKCmp, status: I, test: "constraint_kinds" },
    RuleEntry { id: RuleId::CKSetApp, status: I, test: "prove_set_app" },
    RuleEntry { id: RuleId::CKAndOr, status: I, test: "constraint_kinds" },
    RuleEntry { id: RuleId::CKNot, status: I, test: "constraint_kinds" },
    RuleEntry { id: RuleId::TIntBoolBit, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::TBits, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::TBitsNeg, status: I, test: "typ_bits_negative" },
    RuleEntry { id: RuleId::TRange, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::TNamed, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::TFun, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::TTuple, status: I, test: "typ_kinds" },
    RuleEntry { id: RuleId::QForall, status: I, test: "scheme_closed_and_escape" },
    RuleEntry { id: RuleId::QEscape, status: I, test: "scheme_closed_and_escape" },
    RuleEntry { id: RuleId::QDepthFuel, status: I, test: "scheme_depth_fuel" },
    RuleEntry { id: RuleId::QInst, status: I, test: "quant_freshen_instantiate" },
    RuleEntry { id: RuleId::QFreshen, status: I, test: "quant_freshen_instantiate" },
    RuleEntry { id: RuleId::DValMono, status: I, test: "val_defs" },
    RuleEntry { id: RuleId::DValPoly, status: I, test: "val_defs" },
    RuleEntry { id: RuleId::DValPreRefuted, status: I, test: "val_defs" },
    RuleEntry { id: RuleId::DRegMono, status: I, test: "registers" },
    RuleEntry { id: RuleId::DRegPoly, status: I, test: "registers" },
    RuleEntry { id: RuleId::DRegBad, status: I, test: "registers" },
    RuleEntry { id: RuleId::DScattered, status: I, test: "scattered_clauses" },
    RuleEntry { id: RuleId::IInt, status: I, test: "infer_lits_vars" },
    RuleEntry { id: RuleId::IBool, status: I, test: "infer_lits_vars" },
    RuleEntry { id: RuleId::IVarMono, status: I, test: "infer_lits_vars" },
    RuleEntry { id: RuleId::IVarPoly, status: I, test: "infer_poly_var" },
    RuleEntry { id: RuleId::IAnnot, status: I, test: "infer_lits_vars" },
    RuleEntry { id: RuleId::ITuple, status: I, test: "infer_tuple_if_let_app" },
    RuleEntry { id: RuleId::IIf, status: I, test: "infer_tuple_if_let_app" },
    RuleEntry { id: RuleId::ILet, status: I, test: "infer_tuple_if_let_app" },
    RuleEntry { id: RuleId::IApp, status: I, test: "infer_tuple_if_let_app" },
    RuleEntry { id: RuleId::IAppHeadErr, status: I, test: "infer_tuple_if_let_app" },
    RuleEntry { id: RuleId::CLitRangeConst, status: I, test: "check_literal_range" },
    RuleEntry { id: RuleId::CLitRangeSolver, status: I, test: "check_literal_range" },
    RuleEntry { id: RuleId::CBitsLength, status: I, test: "check_bits_and_fallback" },
    RuleEntry { id: RuleId::CFallbackSub, status: I, test: "check_bits_and_fallback" },
    RuleEntry { id: RuleId::SRefl, status: I, test: "subsumption_base" },
    RuleEntry { id: RuleId::SRangeInt, status: I, test: "subsumption_base" },
    RuleEntry { id: RuleId::SIntRangeFail, status: I, test: "subsumption_base" },
    RuleEntry { id: RuleId::SRangeRange, status: I, test: "subsumption_range_bits" },
    RuleEntry { id: RuleId::SBitsEq, status: I, test: "subsumption_range_bits" },
    RuleEntry { id: RuleId::SFunContraCo, status: I, test: "subsumption_fun_tuple_named" },
    RuleEntry { id: RuleId::STupleArity, status: I, test: "subsumption_fun_tuple_named" },
    RuleEntry { id: RuleId::SNamedArgs, status: I, test: "subsumption_fun_tuple_named" },
    RuleEntry { id: RuleId::SBaseMismatch, status: I, test: "subsumption_base" },
    RuleEntry { id: RuleId::XLitToRange, status: I, test: "check_literal_range" },
    RuleEntry { id: RuleId::XRangeToInt, status: I, test: "subsumption_base" },
    RuleEntry { id: RuleId::PWild, status: I, test: "patterns" },
    RuleEntry { id: RuleId::PVar, status: I, test: "patterns" },
    RuleEntry { id: RuleId::PLit, status: I, test: "patterns" },
    RuleEntry { id: RuleId::PTuple, status: I, test: "patterns" },
    RuleEntry { id: RuleId::PAnnot, status: I, test: "patterns" },
    RuleEntry { id: RuleId::PArityErr, status: I, test: "patterns" },
    RuleEntry { id: RuleId::ZTraitOnly, status: I, test: "solver_trait_only_backend" },
    RuleEntry { id: RuleId::ZStubTotal, status: I, test: "solver_trait_only_backend" },
    RuleEntry { id: RuleId::ZUnknownSound, status: I, test: "solver_trait_only_backend" },
    RuleEntry { id: RuleId::ZCacheBounded, status: I, test: "caching_and_fuel" },
    RuleEntry { id: RuleId::ZFuelTerm, status: I, test: "caching_and_fuel" },
    RuleEntry { id: RuleId::ZProveCtx, status: I, test: "prove_solver_context" },
    RuleEntry { id: RuleId::ELocalShadow, status: I, test: "env_scoping" },
    RuleEntry { id: RuleId::EMonoProject, status: I, test: "env_scoping" },
    RuleEntry { id: RuleId::EPolyOpaque, status: I, test: "env_scoping" },
    RuleEntry {
        id: RuleId::XExistential,
        status: RuleStatus::Deferred("Typ_exist elaboration lives in Lane 09"),
        test: "deferred_documented",
    },
    RuleEntry {
        id: RuleId::XFullExp,
        status: RuleStatus::Deferred("full exp_aux port lives in Lane 02"),
        test: "deferred_documented",
    },
    RuleEntry {
        id: RuleId::XFullPat,
        status: RuleStatus::Deferred("full pat_aux port lives in Lane 02"),
        test: "deferred_documented",
    },
    RuleEntry {
        id: RuleId::XNcSetApp,
        status: RuleStatus::Deferred("foreign NC_set/NC_app/NC_var shapes (NExp_id, non-NExp typ_args, unparseable Set members) lift to None in conv"),
        test: "deferred_documented",
    },
];

/// Look up a rule entry by id.
#[must_use]
pub fn find(id: RuleId) -> Option<&'static RuleEntry> {
    RULE_TABLE.iter().find(|e| e.id == id)
}

/// Number of implemented (non-deferred) rules.
#[must_use]
pub fn implemented_count() -> usize {
    RULE_TABLE
        .iter()
        .filter(|e| e.status == RuleStatus::Implemented)
        .count()
}
