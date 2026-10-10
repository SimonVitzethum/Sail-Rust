//! Printer tests (streaming writer assertions).

use super::*;
use std::collections::BTreeSet;

fn printer() -> LeanPrinter {
    LeanPrinter::with_default()
}

#[test]
fn prints_core_types() {
    let p = printer();
    let cases: &[(Typ, &str)] = &[
        (Typ::Unit, "Unit"),
        (Typ::Int, "Int"),
        (Typ::Nat, "Nat"),
        (Typ::Bool, "Bool"),
        (Typ::String, "String"),
        (
            Typ::BitVec(Box::new(Nexp::Const(8))),
            "(BitVec 8)",
        ),
        (Typ::Bit("b".into()), "(BitVec 1)"),
        (
            Typ::Option(Box::new(Typ::Nat)),
            "(Option Nat)",
        ),
        (Typ::List(Box::new(Typ::Bool)), "(List Bool)"),
        (
            Typ::Result(Box::new(Typ::String), Box::new(Typ::Nat)),
            "(Result String Nat)",
        ),
        (
            Typ::Tuple(vec![Typ::Nat, Typ::Bool]),
            "(Nat × Bool)",
        ),
        (
            Typ::Vector(Box::new(Typ::Bool), Box::new(Nexp::Var("n".into()))),
            "(Vector Bool n)",
        ),
    ];
    for (ty, want) in cases {
        let mut buf = String::new();
        p.print_typ(&mut buf, ty).unwrap();
        assert_eq!(&buf, want, "type {ty:?}");
    }
}

#[test]
fn prints_nexp_with_precedence() {
    // `a + b * c` must not gain parens; `2 ^ (n-1)` shapes correctly.
    let mut buf = String::new();
    let n = Nexp::Sum(
        Box::new(Nexp::Var("a".into())),
        Box::new(Nexp::Times(
            Box::new(Nexp::Var("b".into())),
            Box::new(Nexp::Var("c".into())),
        )),
    );
    LeanPrinter::print_nexp(&mut buf, &n).unwrap();
    assert_eq!(buf, "a + b * c");

    buf.clear();
    let m = Nexp::Pow2(Box::new(Nexp::Minus(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(1)),
    )));
    LeanPrinter::print_nexp(&mut buf, &m).unwrap();
    assert_eq!(buf, "2 ^ (n - 1)");

    buf.clear();
    LeanPrinter::print_nexp(&mut buf, &Nexp::Const(-3)).unwrap();
    assert_eq!(buf, "(-3)");
}

#[test]
fn prints_literals_like_ocaml_backend() {
    let p = printer();
    let cases: &[(Lit, &str)] = &[
        (Lit::Unit, "()"),
        (Lit::True, "true"),
        (Lit::False, "false"),
        (Lit::Num(42), "42"),
        (Lit::Num(-7), "(-7)"),
        (
            Lit::Hex {
                digits: String::new(),
                width: None,
            },
            "BitVec.nil",
        ),
        (
            Lit::Hex {
                digits: "ff".into(),
                width: Some(8),
            },
            "0xFF#8",
        ),
        (
            Lit::Bin {
                bits: "1".into(),
                width: Some(1),
            },
            "1#1",
        ),
        (
            Lit::Bin {
                bits: "101".into(),
                width: Some(3),
            },
            "0b101#3",
        ),
        (Lit::Str("a\"b".into()), "\"a\\\"b\""),
    ];
    for (lit, want) in cases {
        let mut buf = String::new();
        p.print_lit(&mut buf, lit).unwrap();
        assert_eq!(&buf, want, "lit {lit:?}");
    }
}

#[test]
fn prints_fun_signatures_with_monad_mapping() {
    let p = printer();
    let span = Span::new(1, 1);
    // Pure: no monad.
    let pure = FunSig {
        name: "add".into(),
        args: vec![
            ("x".into(), Typ::Nat),
            ("y".into(), Typ::Nat),
        ],
        ret: Typ::Nat,
        effect: Effect::Pure,
        span,
    };
    let got = p.render_fun_stub(&pure, Some("x + y")).unwrap();
    assert_eq!(got, "def add (x : Nat) (y : Nat) : Nat := x + y\n");

    // Stateful: SailM do.
    let st = FunSig {
        name: "read_r".into(),
        args: vec![],
        ret: Typ::Nat,
        effect: Effect::Stateful,
        span,
    };
    let got = p.render_fun_stub(&st, Some("readReg r")).unwrap();
    assert_eq!(got, "def read_r : SailM Nat := do\n  readReg r\n");

    // Early return: SailME do.
    let er = FunSig {
        name: "f".into(),
        args: vec![("x".into(), Typ::Int)],
        ret: Typ::Int,
        effect: Effect::EarlyReturn,
        span,
    };
    let got = p.render_fun_stub(&er, Some("SailME.throw x")).unwrap();
    assert_eq!(got, "def f (x : Int) : SailME Int := do\n  SailME.throw x\n");
}

#[test]
fn untranslatable_is_hard_error_with_span() {
    let p = printer();
    let sig = FunSig {
        name: "bad".into(),
        args: vec![],
        ret: Typ::Nat,
        effect: Effect::Pure,
        span: Span::new(7, 3),
    };
    let err = p.render_fun_stub(&sig, None).unwrap_err();
    assert_eq!(
        err,
        Error::Unsupported {
            msg: "function `bad` body is not translatable (no silent sorry)".into(),
            span: Span::new(7, 3),
        }
    );
    assert!(err.to_string().contains("7:3"));
    assert!(!err.to_string().contains("sorry") || err.to_string().contains("no silent sorry"));

    // Anonymous named type is also a hard error.
    let mut buf = String::new();
    let ty = Typ::Named {
        name: String::new(),
        args: vec![],
        span: Span::new(2, 9),
    };
    let err = p.print_typ(&mut buf, &ty).unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }));
}

#[test]
fn parallel_defs_keep_order_and_stream() {
    let p = printer();
    let span = Span::new(1, 1);
    let sigs: Vec<FunSig> = (0..8)
        .map(|i| FunSig {
            name: format!("f{i}"),
            args: vec![("x".into(), Typ::Nat)],
            ret: Typ::Nat,
            effect: Effect::Pure,
            span,
        })
        .collect();
    let bodies: Vec<Option<String>> = (0..8).map(|i| Some(format!("x + {i}"))).collect();
    let mut out = String::new();
    p.print_defs_parallel(&mut out, &sigs, &bodies).unwrap();
    for i in 0..8 {
        assert!(
            out.contains(&format!("def f{i} (x : Nat) : Nat := x + {i}")),
            "missing f{i}"
        );
    }
    // Order preserved.
    let mut last = 0;
    for i in 0..8 {
        let pos = out.find(&format!("def f{i}")).unwrap();
        assert!(pos >= last);
        last = pos;
    }
}

#[test]
fn options_drive_prefixes() {
    let opts = Options {
        line_width: 120,
        noncomputable: BTreeSet::from(["nc".to_string()]),
        partial: BTreeSet::from(["nc".to_string()]),
    };
    let p = LeanPrinter::new(opts);
    let sig = FunSig {
        name: "nc".into(),
        args: vec![],
        ret: Typ::Nat,
        effect: Effect::Pure,
        span: Span::new(1, 1),
    };
    let got = p.render_fun_stub(&sig, Some("0")).unwrap();
    assert_eq!(got, "noncomputable partial def nc : Nat := 0\n");
}
