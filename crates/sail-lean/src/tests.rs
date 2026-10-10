//! Printer tests (streaming writer assertions).

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn printer() -> LeanPrinter {
    LeanPrinter::with_default()
}

fn funsig(name: &str, ret: Typ, is_monadic: bool, early_return: bool, has_loop: bool) -> FunSig {
    FunSig {
        name: name.into(),
        args: vec![],
        ret,
        is_monadic,
        early_return,
        has_loop,
        span: Span::new(FileId(0), 1, 1),
    }
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
            "(Vector Bool k_n)",
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
    // Size variables are kids and print with the OCaml `k_` prefix
    // (`doc_kid`, `pretty_print_lean.ml#136-139`).
    let mut buf = String::new();
    let n = Nexp::Sum(
        Box::new(Nexp::Var("a".into())),
        Box::new(Nexp::Times(
            Box::new(Nexp::Var("b".into())),
            Box::new(Nexp::Var("c".into())),
        )),
    );
    LeanPrinter::print_nexp(&mut buf, &n).unwrap();
    assert_eq!(buf, "k_a + k_b * k_c");

    buf.clear();
    let m = Nexp::Pow2(Box::new(Nexp::Minus(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(1)),
    )));
    LeanPrinter::print_nexp(&mut buf, &m).unwrap();
    assert_eq!(buf, "2 ^ (k_n - 1)");

    buf.clear();
    LeanPrinter::print_nexp(&mut buf, &Nexp::Const(-3)).unwrap();
    assert_eq!(buf, "(-3)");

    // A quoted kid spells the same.
    buf.clear();
    LeanPrinter::print_nexp(&mut buf, &Nexp::Var("'n".into())).unwrap();
    assert_eq!(buf, "k_n");
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
    // Pure: no monad, no frame.
    let mut pure = funsig("add", Typ::Nat, false, false, false);
    pure.args = vec![
        ("x".into(), Typ::Nat),
        ("y".into(), Typ::Nat),
    ];
    let got = p.render_fun_stub(&pure, Some("x + y")).unwrap();
    assert_eq!(got, "def add (x : Nat) (y : Nat) : Nat := x + y\n");

    // Stateful, no early return: SailM do.
    let st = funsig("read_r", Typ::Nat, true, false, false);
    let got = p.render_fun_stub(&st, Some("readReg r")).unwrap();
    assert_eq!(got, "def read_r : SailM Nat := do\n  readReg r\n");

    // Stateful with early return: SailM return, SailME.run frame
    // (doc_funcl_init: the return prefix follows is_monadic alone).
    let mut er = funsig("f", Typ::Int, true, true, false);
    er.args = vec![("x".into(), Typ::Int)];
    let got = p.render_fun_stub(&er, Some("SailME.throw x")).unwrap();
    assert_eq!(
        got,
        "def f (x : Int) : SailM Int := SailME.run do\n  SailME.throw x\n"
    );

    // Pure with early return: plain return, ExceptM.run frame.
    let pe = funsig("g", Typ::Bool, false, true, false);
    let got = p.render_fun_stub(&pe, Some("throw x")).unwrap();
    assert_eq!(got, "def g : Bool := ExceptM.run do\n  throw x\n");

    // Pure with loop: plain return, Id.run frame.
    let pl = funsig("h", Typ::Nat, false, false, true);
    let got = p.render_fun_stub(&pl, Some("loop_vars")).unwrap();
    assert_eq!(got, "def h : Nat := Id.run do\n  loop_vars\n");
}

#[test]
fn effect_triple_matches_legacy_labels() {
    assert_eq!(Effect::Pure.triple(), (false, false));
    assert_eq!(Effect::Stateful.triple(), (true, false));
    assert_eq!(Effect::EarlyReturn.triple(), (true, true));
    let p = printer();
    assert_eq!(p.monad_of(Effect::Pure), None);
    assert_eq!(p.monad_of(Effect::Stateful), Some("SailM"));
    assert_eq!(p.monad_of(Effect::EarlyReturn), Some("SailM"));
    assert_eq!(LeanPrinter::ret_monad_of(true), Some("SailM"));
    assert_eq!(LeanPrinter::ret_monad_of(false), None);
}

#[test]
fn frame_suffixes_match_ocaml_dec_val_end() {
    // (is_monadic, early_return, has_loop) — pretty_print_lean.ml:1167-1174.
    assert_eq!(LeanPrinter::function_frame_suffix(true, true, false), "SailME.run do");
    assert_eq!(LeanPrinter::function_frame_suffix(true, true, true), "SailME.run do");
    assert_eq!(LeanPrinter::function_frame_suffix(true, false, false), "do");
    assert_eq!(LeanPrinter::function_frame_suffix(true, false, true), "do");
    assert_eq!(LeanPrinter::function_frame_suffix(false, false, true), "Id.run do");
    assert_eq!(LeanPrinter::function_frame_suffix(false, true, false), "ExceptM.run do");
    assert_eq!(LeanPrinter::function_frame_suffix(false, true, true), "ExceptM.run do");
    assert_eq!(LeanPrinter::function_frame_suffix(false, false, false), "");
}

#[test]
fn throw_name_follows_sail_monad() {
    assert_eq!(LeanPrinter::throw_name(true), "SailME.throw");
    assert_eq!(LeanPrinter::throw_name(false), "throw");
}

#[test]
fn ascribed_monads_match_prepend_monad() {
    let p = printer();
    let mut buf = String::new();
    // (true, Some E, E) → `SailME E E`.
    p.print_ascribed_monad(&mut buf, true, Some(&Typ::Bool), &Typ::Bool)
        .unwrap();
    assert_eq!(buf, "SailME Bool Bool");
    // (false, Some, …) → `ExceptM Bool Nat`.
    buf.clear();
    p.print_ascribed_monad(&mut buf, false, Some(&Typ::Bool), &Typ::Nat)
        .unwrap();
    assert_eq!(buf, "ExceptM Bool Nat");
    buf.clear();
    p.print_ascribed_monad(&mut buf, true, None, &Typ::Unit)
        .unwrap();
    assert_eq!(buf, "SailM Unit");
    buf.clear();
    p.print_ascribed_monad(&mut buf, false, None, &Typ::Nat)
        .unwrap();
    assert_eq!(buf, "Id Nat");
}

#[test]
fn prelude_uses_presail_monads() {
    let p = printer();
    let mut buf = String::new();
    p.print_prelude(&mut buf).unwrap();
    assert!(buf.contains("abbrev exception := Unit"), "{buf}");
    assert!(buf.contains("abbrev SailM := PreSailM exception"), "{buf}");
    assert!(buf.contains("abbrev SailME := PreSailME exception"), "{buf}");
    assert!(!buf.contains("IO"), "{buf}");
    buf.clear();
    p.print_prelude_v1(&mut buf).unwrap();
    assert!(
        buf.contains("abbrev SailM := PreSailM RegisterType trivialChoiceSource exception"),
        "{buf}"
    );
    assert!(
        buf.contains("abbrev SailME := PreSailME RegisterType trivialChoiceSource exception"),
        "{buf}"
    );
}

#[test]
fn untranslatable_is_hard_error_with_span() {
    let p = printer();
    let mut bad = funsig("bad", Typ::Nat, false, false, false);
    bad.span = Span::new(FileId(0), 7, 3);
    let err = p.render_fun_stub(&bad, None).unwrap_err();
    assert_eq!(
        err,
        Error::Unsupported {
            msg: "function `bad` body is not translatable (no silent sorry)".into(),
            span: Span::new(FileId(0), 7, 3),
        }
    );
    assert!(err.to_string().contains("file0:7-3"));
    assert!(!err.to_string().contains("sorry") || err.to_string().contains("no silent sorry"));

    // Anonymous named type is also a hard error.
    let mut buf = String::new();
    let ty = Typ::Named {
        name: String::new(),
        args: vec![],
        span: Span::new(FileId(0), 2, 9),
    };
    let err = p.print_typ(&mut buf, &ty).unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }));
}

#[test]
fn parallel_defs_keep_order_and_stream() {
    let p = printer();
    let sigs: Vec<FunSig> = (0..8)
        .map(|i| {
            let mut s = funsig(&format!("f{i}"), Typ::Nat, false, false, false);
            s.args = vec![("x".into(), Typ::Nat)];
            s
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
        ctor_renames: BTreeMap::new(),
        enum_members: BTreeSet::new(),
        defined_functions: BTreeSet::new(),
        out_name_camel: String::new(),
    };
    let p = LeanPrinter::new(opts);
    let s = funsig("nc", Typ::Nat, false, false, false);
    let got = p.render_fun_stub(&s, Some("0")).unwrap();
    assert_eq!(got, "noncomputable partial def nc : Nat := 0\n");
}

#[test]
fn fixup_match_id_matches_ocaml() {
    // `pretty_print_lean.ml#506-510`.
    assert_eq!(fixup_match_id("Some"), "some");
    assert_eq!(fixup_match_id("None"), "none");
    assert_eq!(fixup_match_id("early_return"), "throw");
    assert_eq!(fixup_match_id("plain"), "plain");
    assert_eq!(fixup_match_id("main"), "sail_main");
}

#[test]
fn sanitize_matches_ocaml_fix_id() {
    // `pretty_print_lean.ml#98-107`: `?`, `#`, OCaml keywords.
    let p = printer();
    let mut buf = String::new();
    p.print_id_ctor(&mut buf, "a#b", false).unwrap();
    assert_eq!(buf, "a_b");
    buf.clear();
    p.print_id_ctor(&mut buf, "?", false).unwrap();
    assert_eq!(buf, "questionMark");
    buf.clear();
    p.print_id_ctor(&mut buf, "rec", false).unwrap();
    assert_eq!(buf, "rec'");
    buf.clear();
    p.print_id_ctor(&mut buf, "_lean_wildcard", false).unwrap();
    assert_eq!(buf, "_");
    buf.clear();
    p.print_id_ctor(&mut buf, "main", false).unwrap();
    assert_eq!(buf, "sail_main");
}

#[test]
fn print_lit_width_flag_matches_ocaml() {
    // Expressions: `width:true` (#861); patterns: `width:false` (#525).
    let p = printer();
    let hex = Lit::Hex { digits: "ff".into(), width: Some(8) };
    let mut buf = String::new();
    p.print_lit(&mut buf, &hex).unwrap();
    assert_eq!(buf, "0xFF#8");
    buf.clear();
    p.print_lit_with_width(&mut buf, &hex, false).unwrap();
    assert_eq!(buf, "0xFF");
    let bit = Lit::Bin { bits: "1".into(), width: Some(1) };
    buf.clear();
    p.print_lit_with_width(&mut buf, &bit, false).unwrap();
    assert_eq!(buf, "1");
}

#[test]
fn string_escapes_match_ocaml_lean_escape_string() {
    // `lean_escape_string` (#395) ersetzt nur `"`.
    let p = printer();
    let mut buf = String::new();
    p.print_lit(&mut buf, &Lit::Str("a\"b\\c".into())).unwrap();
    assert_eq!(buf, "\"a\\\"b\\c\"");
}

#[test]
fn kid_names_gain_k_prefix() {
    // `doc_kid` (#136-139).
    assert_eq!(print_kid_name("'n"), "k_n");
    assert_eq!(print_kid_name("n"), "k_n");
    let p = printer();
    let mut buf = String::new();
    p.print_typ(&mut buf, &Typ::Var("'n".into())).unwrap();
    assert_eq!(buf, "k_n");
}

#[test]
fn ctor_renames_and_shadowing_match_ocaml() {
    // `rename_ctor` (#109-115) + `compute_ctor_renames` (#1673-1686).
    let mut renames = BTreeMap::new();
    renames.insert("C".to_string(), "T".to_string());
    let opts = Options {
        line_width: 100,
        noncomputable: BTreeSet::new(),
        partial: BTreeSet::new(),
        ctor_renames: renames,
        enum_members: BTreeSet::new(),
        defined_functions: BTreeSet::from(["shadowed".to_string()]),
        out_name_camel: "Out".to_string(),
    };
    let p = LeanPrinter::new(opts);
    let mut buf = String::new();
    p.print_id_ctor(&mut buf, "C", true).unwrap();
    assert_eq!(buf, "T.C");
    buf.clear();
    p.print_id_ctor(&mut buf, "C", false).unwrap();
    assert_eq!(buf, "C");
    buf.clear();
    p.print_id_ctor(&mut buf, "plain", true).unwrap();
    assert_eq!(buf, "plain");
    // `doc_id_typ_ctor` (#128-134).
    buf.clear();
    p.print_id_typ_ctor(&mut buf, "shadowed").unwrap();
    assert_eq!(buf, "Out.shadowed");
    buf.clear();
    p.print_id_typ_ctor(&mut buf, "free").unwrap();
    assert_eq!(buf, "free");
}

#[test]
fn conv_vector_order_bitvector_and_bool_drop() {
    use sail_ast::{
        Interner, Name, NExpKind, SimpleInterner, TypArgKind, TypKind,
    };

    let mut it = SimpleInterner::new();
    let name = |it: &mut SimpleInterner, text: &str| Name::dummy(it.intern(text));

    // `vector(8, bool)` — Länge zuerst — wird `(Vector Bool 8)` (#302-304).
    let eight = sail_ast::Node::dummy(NExpKind::Constant("8".to_string()));
    let bool_ty = sail_ast::Node::dummy(TypKind::Id(name(&mut it, "bool")));
    let vector = sail_ast::Node::dummy(TypKind::App {
        name: name(&mut it, "vector"),
        args: vec![
            sail_ast::Node::dummy(TypArgKind::NExp(eight)),
            sail_ast::Node::dummy(TypArgKind::Typ(bool_ty)),
        ],
    });
    let lowered = crate::conv::typ_from_ast(&vector, &it).unwrap();
    assert_eq!(
        lowered,
        Typ::Vector(Box::new(Typ::Bool), Box::new(Nexp::Const(8)))
    );
    let mut buf = String::new();
    printer().print_typ(&mut buf, &lowered).unwrap();
    assert_eq!(buf, "(Vector Bool 8)");

    // `bits(n)` → `(BitVec n)` (#311-313); `bit` → `(BitVec 1)`.
    let n = sail_ast::Node::dummy(NExpKind::Var(it.intern("n")));
    let bits = sail_ast::Node::dummy(TypKind::App {
        name: name(&mut it, "bits"),
        args: vec![sail_ast::Node::dummy(TypArgKind::NExp(n))],
    });
    assert_eq!(
        crate::conv::typ_from_ast(&bits, &it).unwrap(),
        Typ::BitVec(Box::new(Nexp::Var("n".into())))
    );
    let bit = sail_ast::Node::dummy(TypKind::Id(name(&mut it, "bit")));
    assert_eq!(
        crate::conv::typ_from_ast(&bit, &it).unwrap(),
        Typ::Bit("bit".into())
    );

    // `A_bool`-Argumente entfallen (`Only_relevant`, #290-296).
    let t = sail_ast::Node::dummy(TypKind::Id(name(&mut it, "bool")));
    let nc = sail_ast::Node::dummy(sail_ast::ConstraintKind::True);
    let app = sail_ast::Node::dummy(TypKind::App {
        name: name(&mut it, "mylist"),
        args: vec![
            sail_ast::Node::dummy(TypArgKind::Typ(t)),
            sail_ast::Node::dummy(TypArgKind::Bool(Box::new(nc))),
        ],
    });
    assert_eq!(
        crate::conv::typ_from_ast(&app, &it).unwrap(),
        Typ::Named {
            name: "mylist".to_string(),
            args: vec![crate::TypArg::Typ(Typ::Bool)],
            span: sail_ast::Span::DUMMY,
        }
    );
}
