#![forbid(unsafe_code)]
//! Canonical env views onto `sail-ast` (Lane 72, Lane-61-Reste #23/#24).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Prueft den Lane-72-Entscheid
//! "Envs bleiben lokal, Sichten aufs Kanonische":
//! - [`KindEnv`] als sortiertes `KOpt`-Teleskop (`kenv_to_kopts`) und zurueck
//!   (`kenv_from_kopts`); `KNat` erlischt zu `Int` und wird NIE zurueckgeraten.
//! - [`TypeEnv`]-Schemes (`val`/`register`) als `sail_ast::TypSchm`
//!   (`val_scheme_ast` / `register_scheme_ast`).

use sail_ast::{Interner as _, SimpleInterner};
use sail_ty::kind::KindEnv;
use sail_ty::{Kind, MiniTyp, NConstraint, Nexp, TypeEnv, TypQuant, kenv_from_kopts, kenv_to_kopts};

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

#[test]
fn kenv_telescope_covers_all_bindings_sorted() {
    let mut it = SimpleInterner::new();
    let kopts = kenv_to_kopts(&sample_kind_env(), &mut it);
    let names: Vec<String> = kopts
        .iter()
        .map(|k| it.resolve(k.kid).expect("binder resolves").to_owned())
        .collect();
    assert_eq!(names, vec!["b", "m", "n", "t"]);
}

#[test]
fn kenv_roundtrip_keeps_all_but_knat_refinement() {
    let mut it = SimpleInterner::new();
    let env = sample_kind_env();
    let back = kenv_from_kopts(&kenv_to_kopts(&env, &mut it), &it).expect("roundtrips");
    assert_eq!(back.lookup("n"), Some(Kind::KInt));
    assert_eq!(back.lookup("b"), Some(Kind::KBool));
    assert_eq!(back.lookup("t"), Some(Kind::KType));
    // Dokumentierte Erasure: `KNat -> Int`, kein stilles Zurueckraten.
    assert_eq!(back.lookup("m"), Some(Kind::KInt));
}

#[test]
fn type_env_schemes_lower_onto_typschm() {
    let mut it = SimpleInterner::new();
    let mut env = TypeEnv::new();
    env.add_val("mem_read", quantified_bits());
    env.add_register("xlen", TypQuant::monomorphic(MiniTyp::Int));

    let val = env.val_scheme_ast("mem_read", &mut it).expect("known val");
    // Ein Binder plus eine Constraint-Position (vgl. `typquant_to_ast`).
    assert_eq!(val.quant.len(), 2);

    let reg = env
        .register_scheme_ast("xlen", &mut it)
        .expect("known register");
    assert_eq!(reg.quant.len(), 1, "monomorphic: nur die Constraint-Position");

    assert_eq!(env.val_scheme_ast("xlen", &mut it), None, "val/register getrennt");
    assert_eq!(env.register_scheme_ast("mem_read", &mut it), None);
    assert_eq!(env.val_scheme_ast("missing", &mut it), None);
}
