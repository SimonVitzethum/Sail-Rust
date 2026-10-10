//! Bridge-Rümpfe (Lane 110, Code ohne Run per E3): `Type`/`Let`/
//! `Constraint`/`Register`/`Measure`/`Outcome`/`Instantiation` landen als
//! Skelett-Knoten auf `sail_ast::DefKind` (echte Namen,
//! `Unknown`/`Wild`/`Undef`-Rümpfe, Rohtext im `unparsed`-Sidecar);
//! `deferred` bleibt nur für wirklich Unbekanntes (`Other`).

#![forbid(unsafe_code)]

use sail_ast::{
    ConstraintKind, DefKind, ExpKind, Interner as _, PatKind, Subst, TypeDefKind,
};
use sail_parse::{BridgeOut, bridge_parsed, parse_file};

fn bridge_src(src: &str) -> BridgeOut {
    let parsed = parse_file("bodies.sail", src).unwrap();
    bridge_parsed(&parsed)
}

fn resolve(out: &BridgeOut, sym: sail_ast::Symbol) -> Option<String> {
    out.interner.resolve(sym).map(str::to_owned)
}

fn single_def(src: &str) -> BridgeOut {
    let out = bridge_src(src);
    assert!(
        out.deferred.is_empty(),
        "Skelett erwartet, deferred: {:?}",
        out.deferred
    );
    out
}

#[test]
fn type_keyword_picks_typedefkind() {
    for (src, want, variant) in [
        ("type foo = bar\n", "foo", "abbrev"),
        ("newtype N = int\n", "N", "abbrev"),
        ("struct S = { x : int }\n", "S", "record"),
        ("enum E = A | B\n", "E", "enum"),
        ("union U = { C : int }\n", "U", "variant"),
        ("bitfield B : bits(8) = { f : 4 }\n", "B", "bitfield"),
    ] {
        let out = single_def(src);
        let td = out
            .ast
            .defs
            .iter()
            .find_map(|d| match &d.kind {
                DefKind::Type(t) => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("Type-Def fehlt für {src:?}"));
        let got = match &td.kind {
            TypeDefKind::Abbrev { name, quant, .. } => {
                assert!(quant.is_empty());
                assert_eq!(resolve(&out, name.symbol).as_deref(), Some(want));
                "abbrev"
            }
            TypeDefKind::Record { name, fields, .. } => {
                assert!(fields.is_empty(), "Felder erst mit Lane 02");
                assert_eq!(resolve(&out, name.symbol).as_deref(), Some(want));
                "record"
            }
            TypeDefKind::Enum { name, members, .. } => {
                assert!(members.is_empty(), "Member erst mit Lane 02");
                assert_eq!(resolve(&out, name.symbol).as_deref(), Some(want));
                "enum"
            }
            TypeDefKind::Variant { name, ctors, .. } => {
                assert!(ctors.is_empty(), "Ctors erst mit Lane 02");
                assert_eq!(resolve(&out, name.symbol).as_deref(), Some(want));
                "variant"
            }
            TypeDefKind::Bitfield { name, fields, .. } => {
                assert!(fields.is_empty(), "Felder erst mit Lane 02");
                assert_eq!(resolve(&out, name.symbol).as_deref(), Some(want));
                "bitfield"
            }
        };
        assert_eq!(got, variant, "falsche Sorte für {src:?}");
        assert!(
            out.unparsed.iter().any(|u| out.ast.defs.iter().any(|d| d.id == u.id)),
            "Rohtext-Sidecar fehlt für {src:?}"
        );
    }
}

#[test]
fn type_without_body_keeps_header() {
    // Abstrakter Typ ohne `=`: Header bleibt vollständig erhalten.
    let out = single_def("type foo : Type\n");
    let td = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Type(t) => Some(t),
            _ => None,
        })
        .expect("Type-Def fehlt");
    assert!(matches!(td.kind, TypeDefKind::Abbrev { .. }));
    assert!(
        out.unparsed.iter().any(|u| u.src.contains("foo : Type")),
        "Header verloren: {:?}",
        out.unparsed
    );
}

#[test]
fn let_becomes_wild_undef_skeleton() {
    let out = single_def("let x = 1\n");
    let (pat, exp) = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Let { pat, exp } => Some((pat, exp)),
            _ => None,
        })
        .expect("Let-Def fehlt");
    assert!(matches!(pat.kind, PatKind::Wild));
    assert!(matches!(exp.kind, ExpKind::Undef));
    assert!(out.unparsed.iter().any(|u| u.src.contains("x = 1")));
}

#[test]
fn constraint_becomes_true_placeholder() {
    let out = single_def("constraint true\n");
    let c = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Constraint(c) => Some(c),
            _ => None,
        })
        .expect("Constraint-Def fehlt");
    assert!(matches!(c.kind, ConstraintKind::True));
    assert!(out.unparsed.iter().any(|u| u.src.contains("true")));
}

#[test]
fn register_keeps_name_and_init_presence() {
    let out = single_def("register R : bits(64)\n");
    let spec = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Register(s) => Some(s),
            _ => None,
        })
        .expect("Register-Def fehlt");
    assert_eq!(resolve(&out, spec.name.symbol).as_deref(), Some("R"));
    assert!(spec.init.is_none());

    let out = single_def("register S : int = 1\n");
    let spec = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Register(s) => Some(s),
            _ => None,
        })
        .expect("Register-Def mit Init fehlt");
    assert_eq!(resolve(&out, spec.name.symbol).as_deref(), Some("S"));
    let init = spec.init.as_ref().expect("Init-Präsenz geht verloren");
    assert!(matches!(init.kind, ExpKind::Undef));
}

#[test]
fn measure_fun_vs_loop_measures() {
    let out = single_def("termination_measure f(x) = x\n");
    let name = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Measure { name, pat, exp } => {
                assert!(matches!(pat.kind, PatKind::Wild));
                assert!(matches!(exp.kind, ExpKind::Undef));
                Some(name.clone())
            }
            _ => None,
        })
        .expect("Measure-Def fehlt");
    assert_eq!(resolve(&out, name.symbol).as_deref(), Some("f"));

    for src in [
        "termination_measure F while x\n",
        "termination_measure AArch64_S2Walk repeat 100, repeat 100\n",
    ] {
        let out = single_def(src);
        let (name, no_measures) = out
            .ast
            .defs
            .iter()
            .find_map(|d| match &d.kind {
                DefKind::LoopMeasures { name, measures } => {
                    Some((name.clone(), measures.is_empty()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("LoopMeasures-Def fehlt für {src:?}"));
        assert!(no_measures, "Maße erst mit Lane 02");
        assert!(resolve(&out, name.symbol).is_some());
    }
}

#[test]
fn outcome_becomes_skeleton() {
    let out = single_def("outcome o : int -> int with 'a\n");
    let name = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Outcome { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("Outcome-Def fehlt");
    assert_eq!(resolve(&out, name.symbol).as_deref(), Some("o"));
    assert!(out.unparsed.iter().any(|u| u.src.contains("with")));
}

#[test]
fn instantiation_extracts_id_substs() {
    let out = single_def("instantiation i with a = b\n");
    let (name, substs) = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Instantiation { name, substs } => Some((name.clone(), substs.clone())),
            _ => None,
        })
        .expect("Instantiation-Def fehlt");
    assert_eq!(resolve(&out, name.symbol).as_deref(), Some("i"));
    assert_eq!(substs.len(), 1);
    match &substs[0] {
        Subst::Id { from, to } => {
            assert_eq!(resolve(&out, from.symbol).as_deref(), Some("a"));
            assert_eq!(resolve(&out, to.symbol).as_deref(), Some("b"));
        }
        other => panic!("erwarte Id-Subst, got {other:?}"),
    }

    // Ohne `with`-Liste: leere Substitutionen, kein Fake.
    let out = single_def("instantiation i\n");
    let substs = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Instantiation { substs, .. } => Some(substs.clone()),
            _ => None,
        })
        .expect("Instantiation ohne with fehlt");
    assert!(substs.is_empty());

    // Mehrere Paare + trailing Komma (Grammatik-Form).
    let out = single_def("instantiation j with a = b, c = d\n");
    let substs = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Instantiation { substs, .. } => Some(substs.clone()),
            _ => None,
        })
        .expect("Mehrfach-Instantiation fehlt");
    assert_eq!(substs.len(), 2);

    // Typsubstis bleiben Lane 02 (leere Liste, Volltext im Sidecar).
    let out = single_def("instantiation k with 'n = 8\n");
    let (substs, has_sidecar) = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Instantiation { substs, .. } => Some((substs.clone(), true)),
            _ => None,
        })
        .expect("Typsubst-Instantiation fehlt");
    assert!(substs.is_empty());
    assert!(has_sidecar && out.unparsed.iter().any(|u| u.src.contains("'n")));
}

#[test]
fn other_stays_deferred_and_lossless() {
    let src = "let x = 1\nmutual foo = bar\n";
    let parsed = parse_file("mixed.sail", src).unwrap();
    let out = bridge_parsed(&parsed);
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Let { .. }
    )));
    assert_eq!(out.deferred.len(), 1, "nur Other bleibt deferred");
    assert_eq!(out.deferred[0].def.kind(), "other");
    assert_eq!(
        out.ast.defs.len() + out.deferred.len(),
        parsed.defs.len()
    );
    for u in &out.unparsed {
        assert!(out.ast.defs.iter().any(|d| d.id == u.id));
    }
}
