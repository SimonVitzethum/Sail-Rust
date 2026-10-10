//! End-to-End: Mono-Spezialisierung -> Streaming-ANF-Lowering.
//!
//! Beweist die Lane-09-Eigenschaften im Verbund:
//! * polymorphe Defs werden mit Split-Limits instanziiert,
//! * Instanzen fließen als Generator (kein Gesamt-Vec) ins Shard-Lowering,
//! * jede gelowerte Def ist blockfrei und hat nur atomare Call-Args.

use sail_lower::{AnfConfig, Expr, FunDef, Lit, MonoConfig, Pat, ShardConfig, Ty, lower_stream, monomorphize};

fn poly_const() -> FunDef {
    // fn const<'a>(x: 'a) -> 'a { { 1; x } }  (Block absichtlich drin)
    FunDef::new(
        "const",
        vec!["a".to_string()],
        vec![("x".to_string(), Ty::Var("a".to_string()))],
        Ty::Var("a".to_string()),
        Expr::Block(vec![Expr::Lit(Lit::Int(1))], Box::new(Expr::Var("x".to_string()))),
    )
}

fn has_block(e: &Expr) -> bool {
    match e {
        Expr::Block(_, _) => true,
        Expr::Let(_, v, b) => has_block(v) || has_block(b),
        Expr::BinOp(_, l, r) => has_block(l) || has_block(r),
        Expr::Call(_, a) => a.iter().any(has_block),
        Expr::If(c, t, e) => has_block(c) || has_block(t) || has_block(e),
        Expr::Match(s, arms) => has_block(s) || arms.iter().any(|(_, e)| has_block(e)),
        _ => false,
    }
}

fn calls_have_atom_args(e: &Expr) -> bool {
    match e {
        Expr::Call(_, args) => {
            args.iter().all(|a| a.is_atom()) && args.iter().all(calls_have_atom_args)
        }
        Expr::Let(_, v, b) => calls_have_atom_args(v) && calls_have_atom_args(b),
        Expr::BinOp(_, l, r) => calls_have_atom_args(l) && calls_have_atom_args(r),
        Expr::Block(stmts, t) => {
            stmts.iter().all(calls_have_atom_args) && calls_have_atom_args(t)
        }
        Expr::If(c, t, e) => {
            calls_have_atom_args(c) && calls_have_atom_args(t) && calls_have_atom_args(e)
        }
        Expr::Match(s, arms) => {
            calls_have_atom_args(s) && arms.iter().all(|(_, e)| calls_have_atom_args(e))
        }
        _ => true,
    }
}

#[test]
fn mono_then_streaming_anf() {
    let mono_cfg = MonoConfig { max_specializations: 4, max_body_size: 10_000 };
    let instances = monomorphize(
        &poly_const(),
        &[vec![Ty::Named("int".to_string())], vec![Ty::Bits(32)]],
        &mono_cfg,
    )
    .unwrap();
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().all(|f| f.type_params.is_empty()));

    // Generator statt Vec: kein Gesamt-Clone im Stream.
    let spec_iter = instances.into_iter();
    let shard_cfg = ShardConfig { shard_size: 1, anf: AnfConfig::default() };
    let lowered: Vec<FunDef> =
        lower_stream(spec_iter, shard_cfg).collect::<Result<_, _>>().unwrap();
    assert_eq!(lowered.len(), 2);
    for f in &lowered {
        assert!(!has_block(&f.body), "block left in {}", f.name);
        assert!(calls_have_atom_args(&f.body), "non-atom call args in {}", f.name);
    }
    assert!(lowered[0].name.starts_with("const#"));
}

#[test]
fn wildcard_match_survives_pipeline() {
    let f = FunDef::new(
        "m",
        vec![],
        vec![],
        Ty::Named("int".to_string()),
        Expr::Match(
            Box::new(Expr::Call("g".to_string(), vec![Expr::Lit(Lit::Int(0))])),
            vec![
                (Pat::Lit(Lit::Int(0)), Expr::Lit(Lit::Int(1))),
                (Pat::Wildcard, Expr::Lit(Lit::Int(2))),
            ],
        ),
    );
    let out: Vec<FunDef> =
        lower_stream(core::iter::once(f), ShardConfig::default())
            .collect::<Result<_, _>>()
            .unwrap();
    assert_eq!(out.len(), 1);
    // Match-Scrutinee muss atomar sein (ggf. via let geliftet).
    match &out[0].body {
        Expr::Match(s, _) => assert!(s.is_atom()),
        Expr::Let(_, _, inner) => {
            assert!(matches!(inner.as_ref(), Expr::Match(s, _) if s.is_atom()));
        }
        other => panic!("unexpected shape: {other:?}"),
    }
}
