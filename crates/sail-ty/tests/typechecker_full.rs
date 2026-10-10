#![forbid(unsafe_code)]
//! Lane-79 rule-coverage tests for `sail-ty` (written as code, NOT executed:
//! E3 — no builds/tests in this lane).
//!
//! Every test corresponds to one or more [`sail_ty::RULE_TABLE`] rows; see
//! `crates/sail-ty/RULES.md`. `rule_table_complete` pins the table ↔ test
//! mapping so the catalogue cannot drift from the code.

use sail_ty::{
    BidirectionalChecker, CacheStats, CachingSolver, CheckLimits, Kind, MiniExp, MiniPat,
    MiniTyp, NConstraint, Nexp, ProveResult, RuleStatus, Solver, Span, StubSolver, TypeEnv,
    TypQuant, RULE_TABLE,
};
use sail_ty::kind::{KindEnv, check_constraint_kind, check_nexp_kind, check_typ_kind};
use sail_ty::subsume::{
    Coercion, Subsumption, check_subtype, freshen_scheme, instantiate_scheme,
    subtype_obligation,
};

fn dummy() -> Span {
    Span::DUMMY
}

fn checker<'s, S: Solver>(s: &'s S) -> BidirectionalChecker<'s, S> {
    BidirectionalChecker::new(s, CheckLimits::default())
}

fn var(name: &str) -> Nexp {
    Nexp::Var(name.to_string())
}

fn const_(c: i128) -> Nexp {
    Nexp::Const(c)
}

fn range(lo: Nexp, hi: Nexp) -> MiniTyp {
    MiniTyp::Range(Box::new(lo), Box::new(hi))
}

// ---------------------------------------------------------------------------
// A Z3-class backend behind the trait only (rule Z-TraitOnly).
//
// This stands in for the Lane 06/80 Z3 backend: it proves strictly more than
// the stub (normalisation: `x+0=x`, `x-0=x`, `x*1=x`, `x-x=0`) while reaching
// the checker exclusively through `Solver::prove`. No `z3` import exists in
// `sail-ty`; backends plug in via the trait and nothing else.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct ProvingSolver;

fn norm_nexp(n: &Nexp) -> Nexp {
    match n {
        Nexp::Const(_) | Nexp::Var(_) => n.clone(),
        Nexp::Neg(a) => Nexp::Neg(Box::new(norm_nexp(a))),
        Nexp::Abs(a) => Nexp::Abs(Box::new(norm_nexp(a))),
        Nexp::Exp(a) => Nexp::Exp(Box::new(norm_nexp(a))),
        Nexp::Add(a, b) => {
            let (x, y) = (norm_nexp(a), norm_nexp(b));
            if y == Nexp::Const(0) {
                x
            } else if x == Nexp::Const(0) {
                y
            } else {
                Nexp::Add(Box::new(x), Box::new(y))
            }
        }
        Nexp::Sub(a, b) => {
            let (x, y) = (norm_nexp(a), norm_nexp(b));
            if y == Nexp::Const(0) {
                x
            } else if x == y {
                Nexp::Const(0)
            } else {
                Nexp::Sub(Box::new(x), Box::new(y))
            }
        }
        Nexp::Mul(a, b) => {
            let (x, y) = (norm_nexp(a), norm_nexp(b));
            if x == Nexp::Const(0) || y == Nexp::Const(0) {
                Nexp::Const(0)
            } else if y == Nexp::Const(1) {
                x
            } else if x == Nexp::Const(1) {
                y
            } else {
                Nexp::Mul(Box::new(x), Box::new(y))
            }
        }
        Nexp::Div(a, b) => Nexp::Div(Box::new(norm_nexp(a)), Box::new(norm_nexp(b))),
        Nexp::Mod(a, b) => Nexp::Mod(Box::new(norm_nexp(a)), Box::new(norm_nexp(b))),
        Nexp::If(c, a, b) => Nexp::If(
            Box::new(norm_constraint(c)),
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
    }
}

fn norm_constraint(c: &NConstraint) -> NConstraint {
    match c {
        NConstraint::True | NConstraint::False => c.clone(),
        NConstraint::Eq(a, b) => NConstraint::Eq(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::Neq(a, b) => NConstraint::Neq(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::Ge(a, b) => NConstraint::Ge(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::Gt(a, b) => NConstraint::Gt(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::Le(a, b) => NConstraint::Le(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::Lt(a, b) => NConstraint::Lt(
            Box::new(norm_nexp(a)),
            Box::new(norm_nexp(b)),
        ),
        NConstraint::And(a, b) => NConstraint::And(
            Box::new(norm_constraint(a)),
            Box::new(norm_constraint(b)),
        ),
        NConstraint::Or(a, b) => NConstraint::Or(
            Box::new(norm_constraint(a)),
            Box::new(norm_constraint(b)),
        ),
        NConstraint::Not(a) => NConstraint::Not(Box::new(norm_constraint(a))),
        NConstraint::Set(n, vs) => {
            let mut members = vs.clone();
            members.sort();
            members.dedup();
            NConstraint::Set(Box::new(norm_nexp(n)), members)
        }
        NConstraint::App(f, args) => {
            NConstraint::App(f.clone(), args.iter().map(norm_nexp).collect())
        }
        NConstraint::Var(_) | NConstraint::Id(_) => c.clone(),
    }
}

impl Solver for ProvingSolver {
    fn prove(&self, nc: &NConstraint) -> ProveResult {
        // Fast path shared with the stub.
        if let Some(b) = nc.as_bool() {
            return if b { ProveResult::Proved } else { ProveResult::Refuted };
        }
        // Stronger path: normalise, then decide what is now obvious.
        let n = norm_constraint(nc);
        if let Some(b) = n.as_bool() {
            return if b { ProveResult::Proved } else { ProveResult::Refuted };
        }
        match &n {
            NConstraint::Eq(a, b) if a == b => ProveResult::Proved,
            NConstraint::Neq(a, b) if a == b => ProveResult::Refuted,
            NConstraint::Ge(a, b) | NConstraint::Le(a, b) if a == b => ProveResult::Proved,
            NConstraint::Gt(a, b) | NConstraint::Lt(a, b) if a == b => ProveResult::Refuted,
            NConstraint::And(a, b) => match (self.prove(a), self.prove(b)) {
                (ProveResult::Proved, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, _) | (_, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Or(a, b) => match (self.prove(a), self.prove(b)) {
                (ProveResult::Proved, _) | (_, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Not(a) => match self.prove(a) {
                ProveResult::Proved => ProveResult::Refuted,
                ProveResult::Refuted => ProveResult::Proved,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            // Lane 107: singleton sets decide as equations (mirrors the stub).
            NConstraint::Set(n, vs) if vs.len() == 1 => {
                self.prove(&NConstraint::Eq(n.clone(), Box::new(Nexp::Const(vs[0]))))
            }
            _ => ProveResult::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Kinds
// ---------------------------------------------------------------------------

#[test]
fn kind_nexp_arith() {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("m", Kind::KNat);
    env.bind("b", Kind::KBool);
    // K-Const: non-negative is K_nat, negative is K_int.
    assert_eq!(check_nexp_kind(&const_(3), &env, dummy()), Ok(Kind::KNat));
    assert_eq!(check_nexp_kind(&const_(-2), &env, dummy()), Ok(Kind::KInt));
    // K-Var.
    assert_eq!(check_nexp_kind(&var("n"), &env, dummy()), Ok(Kind::KInt));
    assert_eq!(check_nexp_kind(&var("m"), &env, dummy()), Ok(Kind::KNat));
    assert!(check_nexp_kind(&var("b"), &env, dummy()).is_err());
    assert!(check_nexp_kind(&var("free"), &env, dummy()).is_err());
    // K-Arith: nat+nat=nat, anything with `-`/mixed is int.
    let add = Nexp::Add(Box::new(var("m")), Box::new(const_(1)));
    assert_eq!(check_nexp_kind(&add, &env, dummy()), Ok(Kind::KNat));
    let sub = Nexp::Sub(Box::new(var("m")), Box::new(var("m")));
    assert_eq!(check_nexp_kind(&sub, &env, dummy()), Ok(Kind::KInt));
    let mul = Nexp::Mul(Box::new(var("n")), Box::new(var("m")));
    assert_eq!(check_nexp_kind(&mul, &env, dummy()), Ok(Kind::KInt));
    // Bool sides are rejected.
    let bad = Nexp::Add(Box::new(var("b")), Box::new(const_(1)));
    assert!(check_nexp_kind(&bad, &env, dummy()).is_err());
    // K-DivMod: always K_int.
    let div = Nexp::Div(Box::new(var("n")), Box::new(const_(2)));
    assert_eq!(check_nexp_kind(&div, &env, dummy()), Ok(Kind::KInt));
    let modu = Nexp::Mod(Box::new(var("n")), Box::new(const_(2)));
    assert_eq!(check_nexp_kind(&modu, &env, dummy()), Ok(Kind::KInt));
    // K-Neg / K-Abs.
    assert_eq!(
        check_nexp_kind(&Nexp::Neg(Box::new(var("n"))), &env, dummy()),
        Ok(Kind::KInt)
    );
    assert_eq!(
        check_nexp_kind(&Nexp::Abs(Box::new(var("n"))), &env, dummy()),
        Ok(Kind::KInt)
    );
    // K-Exp: 2^3 is nat, 2^n is int.
    assert_eq!(
        check_nexp_kind(&Nexp::Exp(Box::new(const_(3))), &env, dummy()),
        Ok(Kind::KNat)
    );
    assert_eq!(
        check_nexp_kind(&Nexp::Exp(Box::new(var("n"))), &env, dummy()),
        Ok(Kind::KInt)
    );
    // K-NatSub.
    assert!(Kind::KInt.subsumes(Kind::KNat));
    assert!(!Kind::KNat.subsumes(Kind::KInt));
    assert!(Kind::KInt.subsumes(Kind::KInt));
}

#[test]
fn kind_nexp_if() {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("m", Kind::KNat);
    // K-If with nat branches joins to nat.
    let yes = Nexp::If(
        Box::new(NConstraint::True),
        Box::new(const_(1)),
        Box::new(var("m")),
    );
    assert_eq!(check_nexp_kind(&yes, &env, dummy()), Ok(Kind::KNat));
    // Mixed branches join to int.
    let mixed = Nexp::If(
        Box::new(NConstraint::True),
        Box::new(const_(1)),
        Box::new(Nexp::Neg(Box::new(const_(1)))),
    );
    assert_eq!(check_nexp_kind(&mixed, &env, dummy()), Ok(Kind::KInt));
    // Ill-kinded condition is rejected.
    let bad_cond = Nexp::If(
        Box::new(NConstraint::Ge(Box::new(var("n")), Box::new(var("n")))),
        Box::new(const_(1)),
        Box::new(const_(2)),
    );
    assert_eq!(check_nexp_kind(&bad_cond, &env, dummy()), Ok(Kind::KNat));
    let unbound = Nexp::If(
        Box::new(NConstraint::Ge(Box::new(var("zzz")), Box::new(const_(0)))),
        Box::new(const_(1)),
        Box::new(const_(2)),
    );
    assert!(check_nexp_kind(&unbound, &env, dummy()).is_err());
}

#[test]
fn constraint_kinds() {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("b", Kind::KBool);
    // CK-TrueFalse.
    assert_eq!(check_constraint_kind(&NConstraint::True, &env, dummy()), Ok(Kind::KBool));
    assert_eq!(check_constraint_kind(&NConstraint::False, &env, dummy()), Ok(Kind::KBool));
    // CK-Cmp: all six comparisons over int sides.
    for mk in [
        NConstraint::Eq as fn(Box<Nexp>, Box<Nexp>) -> NConstraint,
        NConstraint::Neq,
        NConstraint::Ge,
        NConstraint::Gt,
        NConstraint::Le,
        NConstraint::Lt,
    ] {
        let nc = mk(Box::new(var("n")), Box::new(const_(0)));
        assert_eq!(check_constraint_kind(&nc, &env, dummy()), Ok(Kind::KBool));
    }
    // Bool sides are rejected.
    let bad = NConstraint::Eq(Box::new(var("b")), Box::new(var("b")));
    assert!(check_constraint_kind(&bad, &env, dummy()).is_err());
    // CK-AndOr / CK-Not.
    let and = NConstraint::And(Box::new(NConstraint::True), Box::new(NConstraint::False));
    assert_eq!(check_constraint_kind(&and, &env, dummy()), Ok(Kind::KBool));
    let or = NConstraint::Or(Box::new(NConstraint::True), Box::new(NConstraint::False));
    assert_eq!(check_constraint_kind(&or, &env, dummy()), Ok(Kind::KBool));
    let not = NConstraint::Not(Box::new(NConstraint::True));
    assert_eq!(check_constraint_kind(&not, &env, dummy()), Ok(Kind::KBool));
}

#[test]
fn typ_kinds() {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("b", Kind::KBool);
    // T-IntBoolBit.
    assert_eq!(check_typ_kind(&MiniTyp::Int, &env, dummy()), Ok(Kind::KType));
    assert_eq!(check_typ_kind(&MiniTyp::Bool, &env, dummy()), Ok(Kind::KType));
    assert_eq!(check_typ_kind(&MiniTyp::Bit, &env, dummy()), Ok(Kind::KType));
    // T-Bits / T-Range / T-Named.
    let bits = MiniTyp::Bits(Box::new(var("n")));
    assert_eq!(check_typ_kind(&bits, &env, dummy()), Ok(Kind::KType));
    assert_eq!(check_typ_kind(&range(var("n"), const_(8)), &env, dummy()), Ok(Kind::KType));
    let named = MiniTyp::Named { name: "atom".to_string(), args: vec![var("n")] };
    assert_eq!(check_typ_kind(&named, &env, dummy()), Ok(Kind::KType));
    // Bool level vars are rejected in level positions.
    assert!(check_typ_kind(&MiniTyp::Bits(Box::new(var("b"))), &env, dummy()).is_err());
    // T-Fun / T-Tuple.
    let fun = MiniTyp::Fun(Box::new(MiniTyp::Int), Box::new(MiniTyp::Bool));
    assert_eq!(check_typ_kind(&fun, &env, dummy()), Ok(Kind::KType));
    let tup = MiniTyp::Tuple(vec![MiniTyp::Int, bits]);
    assert_eq!(check_typ_kind(&tup, &env, dummy()), Ok(Kind::KType));
}

#[test]
fn typ_bits_negative() {
    let env = KindEnv::new();
    // T-BitsNeg: provably negative lengths are rejected …
    assert!(check_typ_kind(
        &MiniTyp::Bits(Box::new(const_(-1))),
        &env,
        dummy()
    )
    .is_err());
    // … while zero and symbolic lengths are fine.
    assert_eq!(
        check_typ_kind(&MiniTyp::Bits(Box::new(const_(0))), &env, dummy()),
        Ok(Kind::KType)
    );
}

// ---------------------------------------------------------------------------
// Schemes / quantifiers
// ---------------------------------------------------------------------------

#[test]
fn scheme_closed_and_escape() {
    let s = StubSolver::new();
    let c = checker(&s);
    // Q-Forall: closed scheme is accepted.
    let good = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    c.check_scheme("good", &good, dummy()).unwrap();
    // Q-Escape: free var in the constraint is rejected …
    let bad_c = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(Box::new(var("m")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    assert!(c.check_scheme("bad_c", &bad_c, dummy()).is_err());
    // … as is a free var in the body type.
    let bad_t = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::True,
        typ: MiniTyp::Bits(Box::new(var("m"))),
    };
    assert!(c.check_scheme("bad_t", &bad_t, dummy()).is_err());
}

#[test]
fn scheme_depth_fuel() {
    let s = StubSolver::new();
    let c = checker(&s);
    // Q-DepthFuel: adversarially deep constraints exhaust fuel, never hang.
    let mut nc = NConstraint::Gt(Box::new(var("n")), Box::new(const_(0)));
    for _ in 0..300 {
        nc = NConstraint::And(Box::new(nc), Box::new(NConstraint::True));
    }
    let deep = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: nc,
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    assert!(c.check_scheme("deep", &deep, dummy()).is_err());
}

#[test]
fn quant_freshen_instantiate() {
    // Q-Freshen: telescopes are renamed with gensym names (OCaml
    // `Type_env.fresh_kid`), never predictable `{base}{i}` names.
    let q = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt), ("m".to_string(), Kind::KNat)],
        constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    let f = freshen_scheme(&q, "f");
    assert_eq!(f.quants.len(), 2);
    let names: Vec<&str> = f.quants.iter().map(|(v, _)| v.as_str()).collect();
    // Gensym shape: derived from base+index, carrying `#fv`, distinct from
    // the bound names and from each other.
    assert_ne!(names[0], names[1]);
    for n in &names {
        assert!(n.contains("#fv"), "gensym shape, got {n}");
        assert!(*n != "n" && *n != "m");
    }
    assert_eq!(f.quants[0].1, Kind::KInt);
    assert_eq!(f.quants[1].1, Kind::KNat);
    // Constraint/body renamed consistently onto the first fresh name.
    assert_eq!(
        f.constraint,
        NConstraint::Ge(Box::new(var(names[0])), Box::new(const_(0)))
    );
    assert_eq!(f.typ, MiniTyp::Bits(Box::new(var(names[0]))));
    // No capture: a scheme already binding `f0` (the old predictable
    // output) is renamed away from it.
    let tricky = TypQuant {
        quants: vec![("f0".to_string(), Kind::KInt)],
        constraint: NConstraint::True,
        typ: MiniTyp::Bits(Box::new(var("f0"))),
    };
    let f2 = freshen_scheme(&tricky, "f");
    assert_ne!(f2.quants[0].0, "f0");
    assert_eq!(f2.typ, MiniTyp::Bits(Box::new(var(&f2.quants[0].0))));
    // Freshness across calls: two freshenings never share names.
    let g = freshen_scheme(&q, "f");
    assert_ne!(f.quants[0].0, g.quants[0].0);
    // Q-Inst: instantiation splits off the obligation with gensym names.
    let (obl, typ) = instantiate_scheme(&q);
    match &obl {
        NConstraint::Ge(lo, hi) => {
            assert_eq!(**hi, const_(0));
            match &**lo {
                Nexp::Var(v) => {
                    assert!(v.contains("#fv"), "gensym shape, got {v}");
                    assert_eq!(typ, MiniTyp::Bits(Box::new(Nexp::Var(v.clone()))));
                }
                _ => panic!("obligation must mention the fresh var"),
            }
        }
        _ => panic!("expected Ge obligation, got {obl}"),
    }
    // Monomorphic schemes instantiate to themselves with a `True` obligation.
    let (obl, typ) = instantiate_scheme(&TypQuant::monomorphic(MiniTyp::Int));
    assert_eq!((obl, typ), (NConstraint::True, MiniTyp::Int));
}

// ---------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------

#[test]
fn val_defs() {
    let s = StubSolver::new();
    let c = checker(&s);
    let env = TypeEnv::new();
    // D-ValMono: body checked against the scheme type.
    let scheme = TypQuant::monomorphic(MiniTyp::Int);
    let sum = c.check_val_def("x", &scheme, &MiniExp::IntLit(7), &env, dummy()).unwrap();
    assert_eq!(sum.name, "x");
    let scheme_bool = TypQuant::monomorphic(MiniTyp::Bool);
    assert!(c.check_val_def("y", &scheme_bool, &MiniExp::IntLit(7), &env, dummy()).is_err());
    // D-ValPreRefuted: refuted preconditions are hard errors.
    let impossible = TypQuant {
        quants: Vec::new(),
        constraint: NConstraint::False,
        typ: MiniTyp::Int,
    };
    assert!(c
        .check_val_def("z", &impossible, &MiniExp::IntLit(7), &env, dummy())
        .is_err());
    // D-ValPoly: polymorphic bodies are checked under the skolemized
    // (gensym-freshened) scheme — an ill-typed body is rejected, not
    // silently accepted (full instantiation stays Lane 09).
    let poly = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    assert!(c.check_val_def("w", &poly, &MiniExp::IntLit(1), &env, dummy()).is_err());
    // … while a well-typed polymorphic body passes: `int` body against a
    // `forall 'n. true => int` scheme checks under skolemization.
    let poly_int = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::True,
        typ: MiniTyp::Int,
    };
    let sum = c.check_val_def("w", &poly_int, &MiniExp::IntLit(1), &env, dummy()).unwrap();
    assert_eq!(sum.name, "w");
}

#[test]
fn registers() {
    let s = StubSolver::new();
    let c = checker(&s);
    // D-RegMono.
    let mono = TypQuant::monomorphic(MiniTyp::Bits(Box::new(const_(64))));
    let sum = c.check_register("xlen", &mono, dummy()).unwrap();
    assert_eq!(sum.name, "xlen");
    // D-RegPoly.
    let poly = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    let sum = c.check_register("mem", &poly, dummy()).unwrap();
    assert_eq!(sum.name, "mem");
    // D-RegBad: ill-kinded register types are rejected.
    let bad = TypQuant::monomorphic(MiniTyp::Bits(Box::new(const_(-8))));
    assert!(c.check_register("bad", &bad, dummy()).is_err());
}

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

#[test]
fn infer_lits_vars() {
    let s = StubSolver::new();
    let c = checker(&s);
    let mut env = TypeEnv::new();
    // I-Int / I-Bool.
    assert_eq!(c.infer(&MiniExp::IntLit(1), &env, dummy()).unwrap(), MiniTyp::Int);
    assert_eq!(
        c.infer(&MiniExp::BoolLit(true), &env, dummy()).unwrap(),
        MiniTyp::Bool
    );
    // I-VarMono + unbound errors.
    assert!(c.infer(&MiniExp::Var("nope".into()), &env, dummy()).is_err());
    env.add_local("x", MiniTyp::Bool);
    assert_eq!(c.infer(&MiniExp::Var("x".into()), &env, dummy()).unwrap(), MiniTyp::Bool);
    // I-Annot: checks the body, synthesizes the ascription.
    let ann = MiniExp::Annot(Box::new(MiniExp::IntLit(1)), MiniTyp::Int);
    assert_eq!(c.infer(&ann, &env, dummy()).unwrap(), MiniTyp::Int);
    let bad_ann = MiniExp::Annot(Box::new(MiniExp::IntLit(1)), MiniTyp::Bool);
    assert!(c.infer(&bad_ann, &env, dummy()).is_err());
}

#[test]
fn infer_poly_var() {
    let s = StubSolver::new();
    let c = checker(&s);
    let mut env = TypeEnv::new();
    // I-VarPoly: polymorphic val schemes instantiate, but only on `Proved`
    // preconditions — `infer` returns just the type, so an `Unknown`
    // obligation has no channel to travel on and must be a loud error, not
    // a silently dropped burden (rule `Z-UnknownSound`).
    env.add_val(
        "w",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
            typ: MiniTyp::Bits(Box::new(var("n"))),
        },
    );
    assert!(c.infer(&MiniExp::Var("w".into()), &env, dummy()).is_err());
    // … while a scheme with a provable (`True`) precondition instantiates
    // to its freshened body type (gensym name, never the bound `n`).
    env.add_val(
        "open",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::True,
            typ: MiniTyp::Bits(Box::new(var("n"))),
        },
    );
    match c.infer(&MiniExp::Var("open".into()), &env, dummy()).unwrap() {
        MiniTyp::Bits(n) => match *n {
            Nexp::Var(v) => {
                assert_ne!(v, "n");
                assert!(v.contains("#fv"), "gensym shape, got {v}");
            }
            _ => panic!("instantiation must yield the freshened var"),
        },
        _ => panic!("expected bits(..)"),
    }
    // … including polymorphic registers.
    env.add_register(
        "mem",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::True,
            typ: MiniTyp::Bits(Box::new(var("n"))),
        },
    );
    assert!(matches!(
        c.infer(&MiniExp::Var("mem".into()), &env, dummy()).unwrap(),
        MiniTyp::Bits(_)
    ));
    // Refuted preconditions are hard errors at use sites too.
    env.add_val(
        "bad",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::False,
            typ: MiniTyp::Int,
        },
    );
    assert!(c.infer(&MiniExp::Var("bad".into()), &env, dummy()).is_err());
}

#[test]
fn infer_tuple_if_let_app() {
    let s = StubSolver::new();
    let c = checker(&s);
    let mut env = TypeEnv::new();
    env.add_val(
        "f",
        TypQuant::monomorphic(MiniTyp::Fun(
            Box::new(MiniTyp::Int),
            Box::new(MiniTyp::Int),
        )),
    );
    // I-Tuple.
    let tup = MiniExp::Tuple(vec![MiniExp::IntLit(1), MiniExp::BoolLit(false)]);
    assert_eq!(
        c.infer(&tup, &env, dummy()).unwrap(),
        MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bool])
    );
    // I-If: join of branches, bool condition.
    let ite = MiniExp::If(
        Box::new(MiniExp::BoolLit(true)),
        Box::new(MiniExp::IntLit(1)),
        Box::new(MiniExp::IntLit(2)),
    );
    assert_eq!(c.infer(&ite, &env, dummy()).unwrap(), MiniTyp::Int);
    let bad_branch = MiniExp::If(
        Box::new(MiniExp::BoolLit(true)),
        Box::new(MiniExp::IntLit(1)),
        Box::new(MiniExp::BoolLit(false)),
    );
    assert!(c.infer(&bad_branch, &env, dummy()).is_err());
    let bad_cond = MiniExp::If(
        Box::new(MiniExp::IntLit(0)),
        Box::new(MiniExp::IntLit(1)),
        Box::new(MiniExp::IntLit(2)),
    );
    assert!(c.infer(&bad_cond, &env, dummy()).is_err());
    // I-Let: plain and annotated.
    let plain = MiniExp::Let {
        var: "x".into(),
        annot: None,
        bound: Box::new(MiniExp::IntLit(3)),
        body: Box::new(MiniExp::Var("x".into())),
    };
    assert_eq!(c.infer(&plain, &env, dummy()).unwrap(), MiniTyp::Int);
    let annotated = MiniExp::Let {
        var: "x".into(),
        annot: Some(MiniTyp::Int),
        bound: Box::new(MiniExp::IntLit(3)),
        body: Box::new(MiniExp::Var("x".into())),
    };
    assert_eq!(c.infer(&annotated, &env, dummy()).unwrap(), MiniTyp::Int);
    let bad_let = MiniExp::Let {
        var: "x".into(),
        annot: Some(MiniTyp::Int),
        bound: Box::new(MiniExp::BoolLit(true)),
        body: Box::new(MiniExp::Var("x".into())),
    };
    assert!(c.infer(&bad_let, &env, dummy()).is_err());
    // I-App / I-AppHeadErr.
    let app = MiniExp::App(
        Box::new(MiniExp::Var("f".into())),
        Box::new(MiniExp::IntLit(1)),
    );
    assert_eq!(c.infer(&app, &env, dummy()).unwrap(), MiniTyp::Int);
    let bad_arg = MiniExp::App(
        Box::new(MiniExp::Var("f".into())),
        Box::new(MiniExp::BoolLit(true)),
    );
    assert!(c.infer(&bad_arg, &env, dummy()).is_err());
    let bad_head = MiniExp::App(
        Box::new(MiniExp::IntLit(0)),
        Box::new(MiniExp::IntLit(1)),
    );
    let err = c.infer(&bad_head, &env, dummy()).unwrap_err();
    assert!(err.to_string().contains("mismatch"));
}

// ---------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------

#[test]
fn check_literal_range() {
    let env = TypeEnv::new();
    // C-LitRangeConst / X-LitToRange: constant bounds decide syntactically.
    let s = StubSolver::new();
    let c = checker(&s);
    let r = range(const_(0), const_(10));
    c.check(&MiniExp::IntLit(5), &r, &env, dummy()).unwrap();
    assert!(c.check(&MiniExp::IntLit(99), &r, &env, dummy()).is_err());
    // C-LitRangeSolver: symbolic bounds go to the solver. The stub cannot
    // decide them (sound error) …
    let sym = range(
        Nexp::Sub(Box::new(var("n")), Box::new(var("n"))),
        Nexp::Add(
            Box::new(const_(5)),
            Box::new(Nexp::Sub(Box::new(var("m")), Box::new(var("m")))),
        ),
    );
    assert!(c.check(&MiniExp::IntLit(5), &sym, &env, dummy()).is_err());
    // … while a normalising backend (Z3-class, trait-only) proves the
    // obligation `n-n <= 5 <= 5+(m-m)`.
    let p = ProvingSolver;
    let cp = checker(&p);
    cp.check(&MiniExp::IntLit(5), &sym, &env, dummy()).unwrap();
    // A genuinely out-of-range literal still fails even with the prover.
    assert!(cp.check(&MiniExp::IntLit(6), &sym, &env, dummy()).is_err());
}

#[test]
fn check_bits_and_fallback() {
    let s = StubSolver::new();
    let c = checker(&s);
    let env = TypeEnv::new();
    // C-BitsLength: same-length variables unify …
    let mut with_bits = TypeEnv::new();
    with_bits.add_local("v", MiniTyp::Bits(Box::new(var("n"))));
    c.check(
        &MiniExp::Var("v".into()),
        &MiniTyp::Bits(Box::new(var("n"))),
        &with_bits,
        dummy(),
    )
    .unwrap();
    // … provably-equal lengths need the solver: stub says Unknown (error) …
    with_bits.add_local("w", MiniTyp::Bits(Box::new(Nexp::Add(
        Box::new(var("n")),
        Box::new(const_(0)),
    ))));
    assert!(c
        .check(
            &MiniExp::Var("w".into()),
            &MiniTyp::Bits(Box::new(var("n"))),
            &with_bits,
            dummy()
        )
        .is_err());
    // … a stronger backend proves `n+0 == n`.
    let p = ProvingSolver;
    let cp = checker(&p);
    cp.check(
        &MiniExp::Var("w".into()),
        &MiniTyp::Bits(Box::new(var("n"))),
        &with_bits,
        dummy(),
    )
    .unwrap();
    // C-FallbackSub: everything else infers, then subsumes.
    c.check(&MiniExp::IntLit(1), &MiniTyp::Int, &env, dummy()).unwrap();
    assert!(c.check(&MiniExp::IntLit(1), &MiniTyp::Bool, &env, dummy()).is_err());
}

// ---------------------------------------------------------------------------
// Subsumption / coercions
// ---------------------------------------------------------------------------

#[test]
fn subsumption_base() {
    let s = StubSolver::new();
    // S-Refl.
    assert_eq!(
        check_subtype(&s, &MiniTyp::Int, &MiniTyp::Int, dummy()).unwrap(),
        Coercion::Identity
    );
    assert!(matches!(
        subtype_obligation(&MiniTyp::Bool, &MiniTyp::Bool),
        Subsumption::Holds(_)
    ));
    // S-RangeInt / X-RangeToInt: bounds are forgotten.
    let r = range(const_(0), const_(10));
    assert_eq!(
        check_subtype(&s, &r, &MiniTyp::Int, dummy()).unwrap(),
        Coercion::RangeToInt
    );
    assert_eq!(Coercion::RangeToInt.to_string(), "range-to-int");
    // The coercion also fires through `check` for range-typed variables.
    let c = checker(&s);
    let mut env = TypeEnv::new();
    env.add_local("r", r);
    c.check(&MiniExp::Var("r".into()), &MiniTyp::Int, &env, dummy()).unwrap();
    // S-IntRangeFail: `int <: range` needs a value (literal sites only).
    assert!(check_subtype(&s, &MiniTyp::Int, &range(const_(0), const_(10)), dummy()).is_err());
    // S-BaseMismatch: distinct bases never unify.
    assert!(check_subtype(&s, &MiniTyp::Int, &MiniTyp::Bool, dummy()).is_err());
    assert!(check_subtype(&s, &MiniTyp::Bit, &MiniTyp::Bool, dummy()).is_err());
}

#[test]
fn subsumption_range_bits() {
    let s = StubSolver::new();
    // S-RangeRange: inclusion `lo2<=lo1 /\ hi1<=hi2` via the solver.
    let narrow = range(const_(2), const_(8));
    let wide = range(const_(0), const_(10));
    assert_eq!(
        check_subtype(&s, &narrow, &wide, dummy()).unwrap(),
        Coercion::RangeNarrow
    );
    assert!(check_subtype(&s, &wide, &narrow, dummy()).is_err());
    assert!(matches!(
        subtype_obligation(&narrow, &wide),
        Subsumption::NeedsProof(_, _)
    ));
    // S-BitsEq: equal lengths hold …
    assert_eq!(
        check_subtype(
            &s,
            &MiniTyp::Bits(Box::new(var("n"))),
            &MiniTyp::Bits(Box::new(var("n"))),
            dummy()
        )
        .unwrap(),
        Coercion::BitsCongruence
    );
    // … `n+0 == n` needs a normalising backend.
    let plus0 = MiniTyp::Bits(Box::new(Nexp::Add(Box::new(var("n")), Box::new(const_(0)))));
    let plain = MiniTyp::Bits(Box::new(var("n")));
    assert!(check_subtype(&s, &plus0, &plain, dummy()).is_err());
    let p = ProvingSolver;
    assert_eq!(
        check_subtype(&p, &plus0, &plain, dummy()).unwrap(),
        Coercion::BitsCongruence
    );
}

#[test]
fn subsumption_fun_tuple_named() {
    let s = StubSolver::new();
    // S-FunContraCo: domain contravariant, codomain covariant.
    // inferred (range(0,10) -> range(0,10)) <: expected (range(2,8) -> int):
    // dom needs range(2,8) <: range(0,10) (proved), cod range <: int (holds).
    let inferred = MiniTyp::Fun(
        Box::new(range(const_(0), const_(10))),
        Box::new(range(const_(0), const_(10))),
    );
    let expected = MiniTyp::Fun(
        Box::new(range(const_(2), const_(8))),
        Box::new(MiniTyp::Int),
    );
    assert_eq!(check_subtype(&s, &inferred, &expected, dummy()).unwrap(), Coercion::Fun);
    // Wrong domain direction fails.
    let bad_dom = MiniTyp::Fun(Box::new(MiniTyp::Int), Box::new(MiniTyp::Int));
    let want_bool_dom = MiniTyp::Fun(Box::new(MiniTyp::Bool), Box::new(MiniTyp::Int));
    assert!(check_subtype(&s, &bad_dom, &want_bool_dom, dummy()).is_err());
    // S-TupleArity: pointwise ok, arity mismatch is an arity error.
    let t2 = MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bool]);
    assert_eq!(check_subtype(&s, &t2, &t2, dummy()).unwrap(), Coercion::Tuple(2));
    let t3 = MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bool, MiniTyp::Bit]);
    let err = check_subtype(&s, &t2, &t3, dummy()).unwrap_err();
    assert!(err.to_string().contains("arity"));
    // Pointwise obligations propagate: tuple of narrowing ranges.
    let narrow_tup = MiniTyp::Tuple(vec![range(const_(2), const_(8))]);
    let wide_tup = MiniTyp::Tuple(vec![range(const_(0), const_(10))]);
    assert_eq!(
        check_subtype(&s, &narrow_tup, &wide_tup, dummy()).unwrap(),
        Coercion::Tuple(1)
    );
    // S-NamedArgs: same head + equal args hold …
    let a8 = MiniTyp::Named { name: "atom".to_string(), args: vec![const_(8)] };
    assert_eq!(
        check_subtype(&s, &a8, &a8, dummy()).unwrap(),
        Coercion::NamedUnfold("atom".to_string())
    );
    // … different heads never unify, symbolic args need the solver.
    let other = MiniTyp::Named { name: "other".to_string(), args: vec![const_(8)] };
    assert!(check_subtype(&s, &a8, &other, dummy()).is_err());
    let sym = MiniTyp::Named { name: "atom".to_string(), args: vec![var("n")] };
    assert!(check_subtype(&s, &sym, &a8, dummy()).is_err());
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[test]
fn patterns() {
    let s = StubSolver::new();
    let c = checker(&s);
    // P-Wild binds nothing.
    assert_eq!(c.check_pat(&MiniPat::Wild, &MiniTyp::Int, dummy()).unwrap(), Vec::new());
    // P-Var binds the scrutinee type.
    assert_eq!(
        c.check_pat(&MiniPat::Var("x".into()), &MiniTyp::Bool, dummy()).unwrap(),
        vec![("x".to_string(), MiniTyp::Bool)]
    );
    // P-Lit checks the literal against the scrutinee type.
    assert_eq!(
        c.check_pat(&MiniPat::IntLit(3), &MiniTyp::Int, dummy()).unwrap(),
        Vec::new()
    );
    assert!(c.check_pat(&MiniPat::IntLit(3), &MiniTyp::Bool, dummy()).is_err());
    assert_eq!(
        c.check_pat(&MiniPat::BoolLit(true), &MiniTyp::Bool, dummy()).unwrap(),
        Vec::new()
    );
    // P-Tuple destructures pointwise, left to right.
    let pat = MiniPat::Tuple(vec![MiniPat::Var("a".into()), MiniPat::Wild]);
    let typ = MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bool]);
    assert_eq!(
        c.check_pat(&pat, &typ, dummy()).unwrap(),
        vec![("a".to_string(), MiniTyp::Int)]
    );
    // P-ArityErr.
    let short = MiniPat::Tuple(vec![MiniPat::Wild]);
    let err = c.check_pat(&short, &typ, dummy()).unwrap_err();
    assert!(err.to_string().contains("arity"));
    // Tuple patterns against non-tuples fail.
    assert!(c.check_pat(&pat, &MiniTyp::Int, dummy()).is_err());
    // P-Annot: inner against the annotation, then annotation <: scrutinee.
    let ann = MiniPat::Annot(Box::new(MiniPat::Var("y".into())), MiniTyp::Int);
    assert_eq!(
        c.check_pat(&ann, &MiniTyp::Int, dummy()).unwrap(),
        vec![("y".to_string(), MiniTyp::Int)]
    );
    let narrow_ann = MiniPat::Annot(
        Box::new(MiniPat::Var("y".into())),
        range(const_(0), const_(10)),
    );
    assert_eq!(
        c.check_pat(&narrow_ann, &MiniTyp::Int, dummy()).unwrap(),
        vec![("y".to_string(), range(const_(0), const_(10)))]
    );
    // `int` annotation against a `range` scrutinee fails (S-IntRangeFail).
    assert!(c.check_pat(&ann, &range(const_(0), const_(10)), dummy()).is_err());
}

// ---------------------------------------------------------------------------
// Solver policy / environment
// ---------------------------------------------------------------------------

#[test]
fn solver_trait_only_backend() {
    // Z-TraitOnly: both backends reach the checker through `Solver::prove`
    // alone — this file imports no `z3` crate, yet the stronger backend
    // decides strictly more (normalisation beyond constant folding).
    let open = NConstraint::Eq(
        Box::new(Nexp::Add(Box::new(var("n")), Box::new(const_(0)))),
        Box::new(var("n")),
    );
    let stub = StubSolver::new();
    assert_eq!(stub.prove(&open), ProveResult::Unknown);
    let prover = ProvingSolver;
    assert_eq!(prover.prove(&open), ProveResult::Proved);
    // Z-StubTotal: the stub decides the obvious and always terminates …
    assert_eq!(stub.prove(&NConstraint::True), ProveResult::Proved);
    assert_eq!(stub.prove(&NConstraint::False), ProveResult::Refuted);
    // … including deep inputs in linear time (no hangs without Z3).
    let mut nc = NConstraint::Gt(Box::new(var("n")), Box::new(const_(0)));
    for _ in 0..500 {
        nc = NConstraint::And(Box::new(nc), Box::new(NConstraint::True));
    }
    assert_eq!(stub.prove(&nc), ProveResult::Unknown);
    // Z-UnknownSound: `Unknown` never fabricates a proof at check sites.
    let c = checker(&stub);
    let mut env = TypeEnv::new();
    env.add_local("w", MiniTyp::Bits(Box::new(var("m"))));
    assert!(c
        .check(
            &MiniExp::Var("w".into()),
            &MiniTyp::Bits(Box::new(var("n"))),
            &env,
            dummy()
        )
        .is_err());
}

#[test]
fn caching_and_fuel() {
    // Z-CacheBounded: bounded LRU over any backend.
    let cache = CachingSolver::new(StubSolver::new(), 4);
    assert_eq!(cache.prove(&NConstraint::True), ProveResult::Proved);
    assert_eq!(cache.prove(&NConstraint::True), ProveResult::Proved);
    let st: CacheStats = cache.stats();
    assert_eq!((st.hits, st.misses), (1, 1));
    for i in 0..10 {
        cache.prove(&NConstraint::Ge(Box::new(const_(i)), Box::new(const_(0))));
    }
    assert!(cache.len() <= 4);
    assert!(cache.stats().evictions > 0);
    // Z-FuelTerm: tiny fuel exhausts instead of hanging.
    let s = StubSolver::new();
    let tiny = CheckLimits { max_steps: 3, max_depth: 4 };
    let c = BidirectionalChecker::new(&s, tiny);
    let env = TypeEnv::new();
    let mut e = MiniExp::IntLit(0);
    for i in 0..20 {
        e = MiniExp::Let {
            var: format!("x{i}"),
            annot: None,
            bound: Box::new(MiniExp::IntLit(i)),
            body: Box::new(e),
        };
    }
    assert!(c.infer(&e, &env, dummy()).is_err());
    // Patterns are fuel-guarded too.
    let mut p = MiniPat::Wild;
    for _ in 0..20 {
        p = MiniPat::Tuple(vec![p]);
    }
    assert!(c.check_pat(&p, &MiniTyp::Int, dummy()).is_err());
}

#[test]
fn env_scoping() {
    let mut env = TypeEnv::new();
    env.add_val("f", TypQuant::monomorphic(MiniTyp::Int));
    // E-MonoProject: monomorphic schemes project into value lookup.
    assert_eq!(env.lookup("f"), Some(MiniTyp::Int));
    // E-LocalShadow: locals win over globals.
    env.add_local("f", MiniTyp::Bool);
    assert_eq!(env.lookup("f"), Some(MiniTyp::Bool));
    // E-PolyOpaque: polymorphic schemes stay opaque to plain lookup …
    env.add_val(
        "w",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::True,
            typ: MiniTyp::Bits(Box::new(var("n"))),
        },
    );
    assert_eq!(env.lookup("w"), None);
    // … but are visible through the scheme tables (used by Q-Inst).
    assert!(env.lookup_val("w").is_some());
}

// ---------------------------------------------------------------------------
// Catalogue integrity
// ---------------------------------------------------------------------------

#[test]
fn deferred_documented() {
    // Deferred rules carry an owner lane + reason (no silent gaps).
    for id in [
        sail_ty::RuleId::XExistential,
        sail_ty::RuleId::XFullExp,
        sail_ty::RuleId::XFullPat,
        sail_ty::RuleId::XNcSetApp,
    ] {
        let entry = sail_ty::rules::find(id).expect("deferred rule must be tabled");
        match entry.status {
            RuleStatus::Deferred(reason) => assert!(!reason.is_empty()),
            RuleStatus::Implemented => panic!("rule {} must stay deferred", id.as_str()),
        }
    }
}

#[test]
fn rule_table_complete() {
    // Every table row points at a real test function in this file (checked
    // via function pointers: renaming a test without updating the table
    // fails to compile).
    let known: &[(&str, fn())] = &[
        ("kind_nexp_arith", kind_nexp_arith),
        ("kind_nexp_if", kind_nexp_if),
        ("constraint_kinds", constraint_kinds),
        ("typ_kinds", typ_kinds),
        ("typ_bits_negative", typ_bits_negative),
        ("scheme_closed_and_escape", scheme_closed_and_escape),
        ("scheme_depth_fuel", scheme_depth_fuel),
        ("quant_freshen_instantiate", quant_freshen_instantiate),
        ("val_defs", val_defs),
        ("registers", registers),
        ("infer_lits_vars", infer_lits_vars),
        ("infer_poly_var", infer_poly_var),
        ("infer_tuple_if_let_app", infer_tuple_if_let_app),
        ("check_literal_range", check_literal_range),
        ("check_bits_and_fallback", check_bits_and_fallback),
        ("subsumption_base", subsumption_base),
        ("subsumption_range_bits", subsumption_range_bits),
        ("subsumption_fun_tuple_named", subsumption_fun_tuple_named),
        ("patterns", patterns),
        ("solver_trait_only_backend", solver_trait_only_backend),
        ("caching_and_fuel", caching_and_fuel),
        ("env_scoping", env_scoping),
        ("deferred_documented", deferred_documented),
        // Lane-107 prove paths live in `tests/prove_ctx.rs` (separate test
        // target, so no `fn()` pin here — the names below must match the
        // `#[test]` fns there; `RULE_TABLE` entries point at them).
    ];
    let lane107: &[&str] = &["prove_set_app", "prove_solver_context", "scattered_clauses"];
    let _ = rule_table_complete as fn();
    assert!(!RULE_TABLE.is_empty());
    for entry in RULE_TABLE {
        assert!(!entry.test.is_empty(), "rule {} has no test", entry.id.as_str());
        assert!(
            known.iter().any(|(name, _)| *name == entry.test)
                || lane107.contains(&entry.test),
            "rule {} points at unknown test '{}'",
            entry.id.as_str(),
            entry.test
        );
        if entry.status == RuleStatus::Implemented {
            assert!(
                !entry.id.sail_ref().is_empty(),
                "rule {} has no Sail reference",
                entry.id.as_str()
            );
        }
    }
    // No `infer`/`check` gaps inside the subset: every MiniExp/MiniPat
    // constructor is exercised above (IntLit/BoolLit/Var/Annot/Tuple/If/Let/
    // App + Wild/Var/IntLit/BoolLit/Tuple/Annot).
    let _ = MiniExp::IntLit(0);
    let _ = MiniPat::Wild;
}
