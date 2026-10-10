//! Echte Einstiege (Lane 77, Code ohne Run per E3).
//!
//! Strategie: Fehlerpfad + Datei-Ebene sind heute aktiv testbar (ehrliche
//! Span-Fehler, verlustfreies `deferred`-Parken). Erfolgsfälle brauchen die
//! generierten LALRPOP-Tabellen und sind `ignore`d (siehe `ADAPTER.md`);
//! sie kompilieren trotzdem gegen die Schnittstelle.

#![forbid(unsafe_code)]

use sail_ast::{DefKind, ExpKind, PatKind, TypKind};
use sail_parse::{
    FileId, RealParser, parse_def, parse_exp, parse_file, parse_file_real, parse_pat, parse_typ,
};

fn fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}")).unwrap()
}

// ---------------------------------------------------------------------------
// Aktiv: Fehler tragen Spans (gilt mit und ohne generierte Tabellen).
// ---------------------------------------------------------------------------

#[test]
fn fragment_errors_carry_spans() {
    // Lexikalisch ungültig: Lexer-Fehler mit Fragment-Span, kein Panic.
    let err = parse_exp("`nope`").unwrap_err();
    assert_eq!(err.span.file, FileId(0));
    assert_eq!((err.span.start, err.span.end), (0, 6));
    assert!(err.message.contains("unexpected character"), "got: {}", err.message);

    // Leeres Fragment: Grammatik-Fehler (heute Skelett, später Syntax).
    let err = parse_typ("").unwrap_err();
    assert_eq!(err.span.file, FileId(0));
    assert_eq!((err.span.start, err.span.end), (0, 0));
    assert!(!err.message.is_empty());

    // Fehler-Display enthält Span + Meldung.
    let text = err.to_string();
    assert!(text.contains("parse error"), "got: {text}");

    // Alle Einstiege melden Fehler mit gültigem Span.
    let e_pat = parse_pat("`x`").unwrap_err();
    assert!(e_pat.span.start <= e_pat.span.end);
    let e_def = parse_def("`x`").unwrap_err();
    assert!(e_def.span.start <= e_def.span.end);
}

#[test]
fn real_parser_starts_empty_and_reports_spans() {
    let mut p = RealParser::new();
    assert!(p.interner().is_empty());
    let span = sail_parse::Span::new(FileId(0), 0, 4);
    let err = p.parse_exp("`ab`", span).unwrap_err();
    assert_eq!(err.span, span);
}

// ---------------------------------------------------------------------------
// Aktiv: Datei-Ebene ist verlustfrei (delegiert + parkt).
// ---------------------------------------------------------------------------

#[test]
fn file_real_wraps_empty_file_in_shard_markers() {
    let out = parse_file_real("empty.sail", "\n  // only a comment\n").unwrap();
    assert_eq!(out.files, vec!["empty.sail".to_owned()]);
    assert_eq!(out.ast.defs.len(), 2);
    assert!(out.ast.defs.iter().all(|d| matches!(
        d.kind,
        DefKind::Pragma { .. }
    )));
    assert!(out.deferred.is_empty());
    assert!(out.unparsed.is_empty());
}

#[test]
fn file_real_is_lossless_over_fixtures() {
    for name in ["fundefs.sail", "registers_types.sail", "pragma_shards.sail", "scattered.sail"] {
        let src = fixture(name);
        let parsed = parse_file(name, &src).unwrap();
        let out = parse_file_real(name, &src).unwrap();
        assert_eq!(out.files, vec![name.to_owned()]);
        // Nichts geht verloren: gebrückt + deferred = geparst.
        assert_eq!(
            out.ast.defs.len() + out.deferred.len(),
            parsed.defs.len(),
            "lossless violated for {name}"
        );
        // IDs sind je Datei eindeutig (Brücke + Echt-Pfad teilen einen Raum).
        let mut ids: Vec<_> = out.ast.defs.iter().map(|d| d.id).collect();
        ids.sort_by_key(|id| (id.index, id.generation));
        ids.dedup();
        assert_eq!(ids.len(), out.ast.defs.len(), "duplicate NodeIds in {name}");
    }
}

// ---------------------------------------------------------------------------
// Ignoriert: Erfolge brauchen die generierten Tabellen (ADAPTER.md).
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs generated LALRPOP tables (see ADAPTER.md)"]
fn real_typ_bool_is_id() {
    let out = parse_typ("bool").unwrap();
    match &out.value.kind {
        TypKind::Id(n) => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(n.symbol), Some("bool"));
            assert!(!out.value.span.is_dummy());
        }
        other => panic!("expected Id, got {other:?}"),
    }
}

#[test]
#[ignore = "needs generated LALRPOP tables (see ADAPTER.md)"]
fn real_exp_app_and_let() {
    let app = parse_exp("f(x, 1)").unwrap();
    assert!(matches!(app.value.kind, ExpKind::App { .. }), "{:?}", app.value.kind);
    let letexp = parse_exp("let x = 1 in x").unwrap();
    match &letexp.value.kind {
        ExpKind::Let { pat, .. } => assert!(matches!(pat.kind, PatKind::Id(_))),
        other => panic!("expected Let, got {other:?}"),
    }
}

#[test]
#[ignore = "needs generated LALRPOP tables (see ADAPTER.md)"]
fn real_pat_some_is_app() {
    let out = parse_pat("Some(x)").unwrap();
    match &out.value.kind {
        PatKind::App { func, args } => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(func.symbol), Some("Some"));
            assert_eq!(args.len(), 1);
        }
        other => panic!("expected App, got {other:?}"),
    }
}

#[test]
#[ignore = "needs generated LALRPOP tables (see ADAPTER.md)"]
fn real_def_val_has_typschm() {
    let out = parse_def("val f : int -> bool").unwrap();
    match &out.value.kind {
        DefKind::Val(spec) => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(spec.name.symbol), Some("f"));
            assert!(matches!(spec.typschm.typ.kind, TypKind::Fn { .. }));
        }
        other => panic!("expected Val, got {other:?}"),
    }
}

#[test]
#[ignore = "needs generated LALRPOP tables (see ADAPTER.md)"]
fn file_real_lowers_let_register_constraint() {
    let src = "let x = 1\nregister R : bits(64)\nconstraint true\n";
    let out = parse_file_real("mini.sail", src).unwrap();
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Let { .. }
    )));
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Register(_)
    )));
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Constraint(_)
    )));
    assert!(out.deferred.is_empty(), "deferred: {:?}", out.deferred);
}
