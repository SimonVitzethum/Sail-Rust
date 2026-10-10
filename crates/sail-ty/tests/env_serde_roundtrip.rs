#![forbid(unsafe_code)]
//! JSON roundtrips for env types (Lane 65, Lane-61-Follow-up).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Prueft, dass `KindEnv` und
//! `TypeEnv` (inkl. aller transitiven Typen) `serde_json`-stabil sind:
//! `from_str(to_string(env)) == env`.

use sail_ty::kind::KindEnv;
use sail_ty::{Kind, MiniTyp, NConstraint, Nexp, TypeEnv, TypQuant};

fn sample_kind_env() -> KindEnv {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("m", Kind::KNat);
    env.bind("b", Kind::KBool);
    env.bind("t", Kind::KType);
    env
}

fn quantified_bits() -> TypQuant {
    TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Const(0)),
        ),
        typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    }
}

fn sample_type_env() -> TypeEnv {
    let mut env = TypeEnv::new();
    // Val spec mit Quantor + Constraint.
    env.add_val("mem_read", quantified_bits());
    // Monomorphe Val spec (Fun-Typ).
    env.add_val(
        "f",
        TypQuant::monomorphic(MiniTyp::Fun(
            Box::new(MiniTyp::Int),
            Box::new(MiniTyp::Bool),
        )),
    );
    // Locals decken Tuple / Range / Named ab.
    env.add_local("p", MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Bit]));
    env.add_local(
        "r",
        MiniTyp::Range(Box::new(Nexp::Const(0)), Box::new(Nexp::Const(10))),
    );
    env.add_local(
        "a",
        MiniTyp::Named {
            name: "atom".to_string(),
            args: vec![Nexp::Add(
                Box::new(Nexp::Var("n".into())),
                Box::new(Nexp::Const(1)),
            )],
        },
    );
    // Register mit Constraint-Kombinatoren (And / Neq / Not).
    env.add_register(
        "xlen",
        TypQuant {
            quants: vec![],
            constraint: NConstraint::And(
                Box::new(NConstraint::Neq(
                    Box::new(Nexp::Const(0)),
                    Box::new(Nexp::Const(64)),
                )),
                Box::new(NConstraint::Not(Box::new(NConstraint::False))),
            ),
            typ: MiniTyp::Bits(Box::new(Nexp::Const(64))),
        },
    );
    env
}

#[test]
fn kind_env_json_roundtrip() {
    let env = sample_kind_env();
    let json = serde_json::to_string(&env).expect("KindEnv serializes to JSON");
    let back: KindEnv =
        serde_json::from_str(&json).expect("KindEnv deserializes from JSON");
    assert_eq!(env, back);
}

#[test]
fn type_env_json_roundtrip() {
    let env = sample_type_env();
    let json =
        serde_json::to_string_pretty(&env).expect("TypeEnv serializes to JSON");
    let back: TypeEnv =
        serde_json::from_str(&json).expect("TypeEnv deserializes from JSON");
    assert_eq!(env, back);
}

#[test]
fn empty_envs_json_roundtrip() {
    let kenv = KindEnv::new();
    let kback: KindEnv =
        serde_json::from_str(&serde_json::to_string(&kenv).expect("KindEnv serializes"))
            .expect("KindEnv deserializes");
    assert_eq!(kenv, kback);

    let tenv = TypeEnv::new();
    let tback: TypeEnv =
        serde_json::from_str(&serde_json::to_string(&tenv).expect("TypeEnv serializes"))
            .expect("TypeEnv deserializes");
    assert_eq!(tenv, tback);
}
