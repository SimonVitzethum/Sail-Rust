//! Lane 70 — Tiny-Modell Stufe 4: Emit (Harness).
//!
//! Druckt die deklarierten Typen der Gruen-Teilmenge (`sail-lean`
//! `typ_from_ast` ueber von Hand gespiegelten kanonischen Knoten —
//! `sail-lean` haengt an `sail-ast`, nicht an `sail-ty`, daher ist das
//! Spiegeln hier explizit und reviewbar) als Lean-Signaturen und schreibt
//! ein eigenstaendiges `Tiny.lean` (Core-Lean only, kein `import Sail`,
//! damit der `lake build` klein und offline bleibt).
//!
//! Ehrlichkeitsregeln: Bodies sind HANDUEBERSETZT (jede mit
//! `MANUAL BODY` markiert) — am Base-Stand gibt es keinen Body-Drucker
//! (`exps.rs` ist Lane-14-Branch `9961e01`, ungemergt). `inductive Colour`
//! ist ebenfalls manuell — Typedef-Emission fehlt (Owner lane-30,
//! `lean-typedefs`). Generiert sind: Typ-Senkung, Signaturkoepfe,
//! Datei-Layout. `sorry`/`admit` sind verboten (Token-Scan hier plus
//! `CheckLeanNoSorry`-Gate im Treiber).

use sail_ast::{Interner, Name, Node, NExpKind, SimpleInterner, Span, TypArgKind, TypKind};
use sail_lean::{FunSig, LeanPrinter, Typ};

/// Manuelle Bodies (1:1-Uebersetzungen der geprueften Sail-Bodies).
const BODIES: &[(&str, &str)] = &[
    ("const_one", "1"),
    ("choose", "if b then 1 else 0"),
    ("swap_pair", "(y, x)"),
    ("two_nums", "⟨#[1, 2], rfl⟩"),
    ("byte_pair", "0b10"),
    ("is_red", "match c with | .Red => true | .Green => false"),
];

fn id(it: &mut SimpleInterner, name: &str) -> sail_ast::Typ {
    Node::dummy(TypKind::Id(Name::dummy(it.intern(name))))
}

fn nat_const(value: &str) -> sail_ast::NExp {
    Node::dummy(NExpKind::Constant(value.to_string()))
}

fn app(
    it: &mut SimpleInterner,
    name: &str,
    args: Vec<sail_ast::TypArg>,
) -> sail_ast::Typ {
    Node::dummy(TypKind::App {
        name: Name::dummy(it.intern(name)),
        args,
    })
}

fn typarg_ty(t: sail_ast::Typ) -> sail_ast::TypArg {
    Node::dummy(TypArgKind::Typ(t))
}

fn typarg_nexp(n: sail_ast::NExp) -> sail_ast::TypArg {
    Node::dummy(TypArgKind::NExp(n))
}

/// Gespiegelte Schemes des Tiny-Modells (vgl. `lane70_tiny.rs`).
fn mirrored() -> (SimpleInterner, Vec<(String, sail_ast::Typ, Vec<String>)>) {
    let mut it = SimpleInterner::new();
    let int = id(&mut it, "int");
    let bool = id(&mut it, "bool");
    let unit = id(&mut it, "unit");
    let colour = id(&mut it, "Colour");
    let two = nat_const("2");
    let elem = id(&mut it, "int");
    let vector2int = app(&mut it, "vector", vec![typarg_nexp(two), typarg_ty(elem)]);
    let width = nat_const("2");
    let bits2 = app(&mut it, "bitvector", vec![typarg_nexp(width)]);
    let pair_in = Node::dummy(TypKind::Tuple(vec![int.clone(), bool.clone()]));
    let schemes = vec![
        ("const_one".to_string(), unit, vec![]),
        ("choose".to_string(), bool.clone(), vec!["b".to_string()]),
        (
            "swap_pair".to_string(),
            pair_in,
            vec!["x".to_string(), "y".to_string()],
        ),
        ("two_nums".to_string(), vector2int, vec![]),
        ("byte_pair".to_string(), bits2, vec![]),
        ("is_red".to_string(), colour, vec!["c".to_string()]),
    ];
    (it, schemes)
}

fn out_dir() -> std::path::PathBuf {
    std::env::var_os("LANE70_OUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lane70-tiny")
        })
}

fn has_hole(text: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\''))
        .any(|token| token == "sorry" || token == "admit")
}

#[test]
fn lane70_tiny_emit() {
    let (it, schemes) = mirrored();
    let printer = LeanPrinter::with_default();

    // Stufe 4a: Typ-Senkung fuer jede Scheme-Form des Modells.
    let mut lowered: Vec<(String, Typ, Vec<String>)> = Vec::new();
    for (name, typ, binders) in &schemes {
        let lean = sail_lean::conv::typ_from_ast(typ, &it)
            .unwrap_or_else(|| panic!("typ_from_ast scheitert fuer {name}"));
        lowered.push((name.clone(), lean, binders.clone()));
    }
    let texts: Vec<String> = lowered
        .iter()
        .map(|(_, t, _)| {
            let mut s = String::new();
            printer.print_typ(&mut s, t).unwrap();
            s
        })
        .collect();
    assert_eq!(texts[0], "Unit");
    assert_eq!(texts[2], "(Int × Bool)");
    assert_eq!(texts[3], "(Vector Int 2)");
    assert_eq!(texts[4], "(BitVec 2)");
    assert_eq!(texts[5], "Colour");

    // Stufe 4b: Signaturkoepfe + manuelle Bodies -> Tiny.lean.
    let mut lean = String::new();
    lean.push_str("-- Lane-70 Tiny-Modell: Signaturen generiert (sail-lean),\n");
    lean.push_str("-- Bodies handuebersetzt (kein exps.rs am Base-Stand; Owner lane-14).\n");
    lean.push_str("-- `inductive Colour` manuell (keine Typedef-Emission; Owner lane-30).\n\n");
    lean.push_str("inductive Colour where\n  | Red\n  | Green\n\n");
    for ((name, typ, binders), body) in lowered.iter().zip(BODIES.iter().map(|(_, b)| *b)) {
        let args: Vec<(String, Typ)> = binders
            .iter()
            .zip(match name.as_str() {
                "choose" => vec![Typ::Bool],
                "swap_pair" => vec![Typ::Int, Typ::Bool],
                "is_red" => vec![Typ::Named {
                    name: "Colour".to_string(),
                    args: vec![],
                    span: Span::DUMMY,
                }],
                _ => vec![],
            })
            .map(|(n, t)| (n.clone(), t))
            .collect();
        let ret = match name.as_str() {
            "const_one" | "choose" | "describe" => Typ::Int,
            "swap_pair" => Typ::Tuple(vec![Typ::Bool, Typ::Int]),
            "two_nums" => Typ::Vector(Box::new(Typ::Int), Box::new(sail_lean::Nexp::Const(2))),
            "byte_pair" => Typ::BitVec(Box::new(sail_lean::Nexp::Const(2))),
            "is_red" => Typ::Bool,
            _ => typ.clone(),
        };
        let sig = FunSig {
            name: name.clone(),
            args,
            ret,
            is_monadic: false,
            early_return: false,
            has_loop: false,
            span: Span::DUMMY,
        };
        let rendered = printer
            .render_fun_stub(&sig, Some(body))
            .unwrap_or_else(|e| panic!("render {name}: {e}"));
        lean.push_str(&format!("-- MANUAL BODY ({name}): {body}\n"));
        lean.push_str(&rendered);
    }
    assert!(!has_hole(&lean), "Proof-Hole im Emit:\n{lean}");

    let out = out_dir();
    std::fs::create_dir_all(&out).expect("LANE70_OUT_DIR anlegen");
    // Lake-Layout wie `sail-lake` (`<Camel>/Tiny.lean` + Root): Module
    // muessen unter dem Lib-Praefix liegen, sonst `unknown module prefix`.
    std::fs::create_dir_all(out.join("Lane70tiny")).expect("lib-dir anlegen");
    std::fs::write(out.join("Lane70tiny/Tiny.lean"), &lean).expect("Tiny.lean schreiben");
    std::fs::write(
        out.join("lakefile.toml"),
        "name = \"lane70tiny\"\ndefaultTargets = [\"Lane70tiny\"]\n\n[[lean_lib]]\nname = \"Lane70tiny\"\n",
    )
    .expect("lakefile schreiben");
    std::fs::write(out.join("lean-toolchain"), "leanprover/lean4:v4.29.0\n")
        .expect("toolchain schreiben");
    std::fs::write(out.join("Lane70tiny.lean"), "import Lane70tiny.Tiny\n")
        .expect("root schreiben");
}
