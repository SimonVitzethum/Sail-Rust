//! Brücken-Tests (Lane 63, Code ohne Run per E3): `mini_ast` → `sail-ast`.
//!
//! Fixture-Parität zu `tests/parse_fixtures.rs`: gleiche Dateien, jetzt auf
//! `sail_ast`-Knoten (`Val`/`Fundef`/`Mapdef`/`Scattered` + Meta-Defs),
//! Shard-Marker erhalten, Rümpfe als Skelett auf `sail-ast`, nur `Other`
//! noch in `deferred`, JSON-Roundtrip.

use sail_ast::{Ast, DefKind, Interner as _, Order, ScatteredDef};
use sail_parse::{bridge_file, bridge_parsed, parse_file};

fn fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}")).unwrap()
}

fn resolve(out: &sail_parse::BridgeOut, sym: sail_ast::Symbol) -> Option<String> {
    out.interner.resolve(sym).map(str::to_owned)
}

#[test]
fn val_fundef_mapdef_land_on_sail_ast() {
    let src = fixture("fundefs.sail");
    let out = bridge_file("fundefs.sail", &src).unwrap();
    assert_eq!(out.files, vec!["fundefs.sail".to_owned()]);

    let val = out
        .ast
        .defs
        .iter()
        .find(|d| matches!(d.kind, DefKind::Val(_)))
        .expect("Val-Def fehlt");
    match &val.kind {
        DefKind::Val(spec) => {
            assert_eq!(resolve(&out, spec.name.symbol).as_deref(), Some("bitvector_eq"));
            assert!(!spec.name.is_operator);
            assert!(spec.typschm.quant.is_empty());
            assert!(matches!(spec.typschm.typ.kind, sail_ast::TypKind::Unknown));
        }
        other => panic!("erwarte Val, got {other:?}"),
    }

    let fun = out
        .ast
        .defs
        .iter()
        .find(|d| matches!(d.kind, DefKind::Fundef { .. }))
        .expect("Fundef fehlt");
    match &fun.kind {
        DefKind::Fundef { rec, funcls, .. } => {
            assert!(matches!(rec, sail_ast::RecOpt::NonRec));
            assert_eq!(funcls.len(), 1);
            assert_eq!(
                resolve(&out, funcls[0].name.symbol).as_deref(),
                Some("bitvector_eq")
            );
        }
        other => panic!("erwarte Fundef, got {other:?}"),
    }
    // Rohtext bleibt im Sidecar für Lane 02.
    assert!(out.unparsed.iter().any(|u| u.src.contains("x == y")));

    let map_src = fixture("registers_types.sail");
    let map_out = bridge_file("registers_types.sail", &map_src).unwrap();
    let map = map_out
        .ast
        .defs
        .iter()
        .find(|d| matches!(d.kind, DefKind::Mapdef { .. }))
        .expect("Mapdef fehlt");
    match &map.kind {
        DefKind::Mapdef { name, clauses, .. } => {
            assert_eq!(resolve(&map_out, name.symbol).as_deref(), Some("size_bits"));
            assert!(clauses.is_empty(), "Klauseln erst mit Lane 02/17");
        }
        other => panic!("erwarte Mapdef, got {other:?}"),
    }
}

#[test]
fn scattered_operators_and_end_land_on_sail_ast() {
    let src = fixture("scattered.sail");
    let out = bridge_file("scattered.sail", &src).unwrap();

    let sc: Vec<&ScatteredDef> = out
        .ast
        .defs
        .iter()
        .filter_map(|d| match &d.kind {
            DefKind::Scattered(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(sc.len(), 5, "function + 2x clause + mapping-clause + end");

    // `scattered function step` → Function-Header.
    assert!(sc.iter().any(|s| matches!(
        s,
        ScatteredDef::Function { name, .. }
        if resolve(&out, name.symbol).as_deref() == Some("step")
    )));
    // Zwei `function clause` → Funcl-Skelette mit Namen.
    let funcls = sc
        .iter()
        .filter_map(|s| match s {
            ScatteredDef::Funcl(f) => Some(f),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(funcls.len(), 2);
    assert!(funcls
        .iter()
        .all(|f| resolve(&out, f.name.symbol).as_deref() == Some("step")));
    // `mapping clause` per `<->`-Heuristik → Mapcl.
    assert!(sc.iter().any(|s| matches!(
        s,
        ScatteredDef::Mapcl { name, .. }
        if resolve(&out, name.symbol).as_deref() == Some("csr_map")
    )));
    // `end step` → End.
    assert!(sc.iter().any(|s| matches!(
        s,
        ScatteredDef::End(n) if resolve(&out, n.symbol).as_deref() == Some("step")
    )));

    // Operator-Val `^^` mit Flag.
    let op = out
        .ast
        .defs
        .iter()
        .find(|d| matches!(&d.kind, DefKind::Val(s) if resolve(&out, s.name.symbol).as_deref() == Some("^^")))
        .expect("Operator-Val fehlt");
    match &op.kind {
        DefKind::Val(spec) => assert!(spec.name.is_operator),
        other => panic!("erwarte Val, got {other:?}"),
    }
}

#[test]
fn shard_markers_and_small_defs_preserved() {
    let src = fixture("fundefs.sail");
    let out = bridge_file("fundefs.sail", &src).unwrap();
    let pragmas: Vec<(&str, &str)> = out
        .ast
        .defs
        .iter()
        .filter_map(|d| match &d.kind {
            DefKind::Pragma { key, payload } => Some((key.as_str(), payload.as_str())),
            _ => None,
        })
        .collect();
    for marker in ["file_start", "file_end"] {
        assert!(
            pragmas.iter().any(|(k, _)| *k == marker),
            "Shard-Marker {marker} fehlt: {pragmas:?}"
        );
    }

    let fix = out
        .ast
        .defs
        .iter()
        .find(|d| matches!(d.kind, DefKind::Fixity { .. }))
        .expect("Fixity fehlt");
    match &fix.kind {
        DefKind::Fixity { level, id, .. } => {
            assert_eq!(level, "6");
            assert_eq!(resolve(&out, id.symbol).as_deref(), Some("^"));
            assert!(id.is_operator);
        }
        other => panic!("erwarte Fixity, got {other:?}"),
    }

    let ov = out
        .ast
        .defs
        .iter()
        .find(|d| matches!(d.kind, DefKind::Overload { .. }))
        .expect("Overload fehlt");
    match &ov.kind {
        DefKind::Overload { name, overloads } => {
            assert_eq!(resolve(&out, name.symbol).as_deref(), Some("foo"));
            let ids: Vec<String> =
                overloads.iter().filter_map(|n| resolve(&out, n.symbol)).collect();
            assert_eq!(ids, vec!["bar".to_owned(), "baz".to_owned()]);
        }
        other => panic!("erwarte Overload, got {other:?}"),
    }

    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Default(Order::Dec)
    )));
}

#[test]
fn rest_is_lossless_deferred_and_json_roundtrips() {
    let src = fixture("registers_types.sail");
    let parsed = parse_file("registers_types.sail", &src).unwrap();
    let out = bridge_parsed(&parsed);
    // Lane 110: Typ-/Register-Rümpfe landen als Skelett auf `sail-ast`
    // (echte Namen, `Unknown`-Rümpfe, Rohtext im Sidecar); `deferred`
    // bleibt nur für wirklich Unbekanntes — das Fixture hat keines.
    assert!(out.deferred.is_empty(), "deferred: {:?}", out.deferred);
    let types: Vec<_> = out
        .ast
        .defs
        .iter()
        .filter_map(|d| match &d.kind {
            DefKind::Type(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(types.len(), 3, "bitfield + enum + struct");
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Register(_)
    )));
    // Nichts geht verloren: gebrückt + deferred = geparst.
    let bridged = out.ast.defs.len() + out.deferred.len();
    assert_eq!(bridged, parsed.defs.len());
    for u in &out.unparsed {
        assert!(out.ast.defs.iter().any(|d| d.id == u.id));
    }

    let json = out.ast.to_json().unwrap();
    let back = Ast::from_json(&json).unwrap();
    assert_eq!(out.ast, back);
}
