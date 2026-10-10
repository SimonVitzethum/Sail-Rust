//! Integration tests over mini `.sail` fixtures.
//!
//! Each fixture is parsed via [`sail_parse::parse_file`] and via
//! [`sail_parse::parse_files_parallel`]; assertions pin the definition
//! kinds, extracted names, shard markers and error spans.

use sail_parse::{Def, PragmaArg, parse_file, parse_files_parallel};

fn fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}")).unwrap()
}

fn kinds_of(path: &str, src: &str) -> Vec<&'static str> {
    parse_file(path, src).unwrap().defs.iter().map(|d| d.def.kind()).collect()
}

#[test]
fn fundefs_val_register_type_overload_fixity() {
    let src = fixture("fundefs.sail");
    let pf = parse_file("fundefs.sail", &src).unwrap();

    // file_start ... file_end wrapping
    assert!(matches!(&pf.defs[0].def, Def::Pragma { name, .. } if name == "file_start"));
    assert!(matches!(&pf.defs.last().unwrap().def, Def::Pragma { name, .. } if name == "file_end"));

    let kinds = kinds_of("fundefs.sail", &src);
    assert_eq!(
        kinds,
        vec![
            "pragma", // file_start
            "default",
            "pragma", // $include <prelude.sail>
            "val",
            "function",
            "pragma", // $include <arith.sail>
            "register",
            "type",
            "overload",
            "fixity",
            "pragma", // file_end
        ]
    );

    // Structural spot checks.
    let val = pf.defs.iter().find(|d| d.def.kind() == "val").unwrap();
    match &val.def {
        Def::Val { name, sig_src } => {
            assert_eq!(name, "bitvector_eq");
            assert!(sig_src.contains("(bits(16), bits(16)) -> bool"), "sig: {sig_src}");
        }
        other => panic!("expected Val, got {other:?}"),
    }
    let fun = pf.defs.iter().find(|d| d.def.kind() == "function").unwrap();
    match &fun.def {
        Def::Fundef { name, body_src, .. } => {
            assert_eq!(name, "bitvector_eq");
            assert!(body_src.contains("x == y"), "body: {body_src}");
        }
        other => panic!("expected Fundef, got {other:?}"),
    }
    let reg = pf.defs.iter().find(|d| d.def.kind() == "register").unwrap();
    match &reg.def {
        Def::Register { name, type_src, init_src } => {
            assert_eq!(name, "R");
            assert_eq!(type_src.trim(), "R : bits(64)");
            assert_eq!(init_src, &None);
        }
        other => panic!("expected Register, got {other:?}"),
    }
    let ov = pf.defs.iter().find(|d| d.def.kind() == "overload").unwrap();
    match &ov.def {
        Def::Overload { name, ids } => {
            assert_eq!(name, "foo");
            assert_eq!(ids, &vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()][1..]);
        }
        other => panic!("expected Overload, got {other:?}"),
    }
    let fx = pf.defs.iter().find(|d| d.def.kind() == "fixity").unwrap();
    match &fx.def {
        Def::Fixity { prec, level, name } => {
            assert_eq!(prec, "infixr");
            assert_eq!(level, "6");
            assert_eq!(name, "^");
        }
        other => panic!("expected Fixity, got {other:?}"),
    }

    // Spans: function body on lines 6-8 (1-based), file recorded.
    assert_eq!(fun.span.file, "fundefs.sail");
    assert_eq!(fun.span.start_line, 6);
    assert_eq!(fun.span.end_line, 8);
}

#[test]
fn registers_types_mapping() {
    let src = fixture("registers_types.sail");
    let kinds = kinds_of("registers_types.sail", &src);
    assert_eq!(
        kinds,
        vec![
            "pragma", "default", "pragma", "type", "register", "type", "type", "mapping",
            "pragma",
        ]
    );
    let pf = parse_file("registers_types.sail", &src).unwrap();
    let names: Vec<String> = pf
        .defs
        .iter()
        .filter_map(|d| match &d.def {
            Def::Type { name, .. } => Some(format!("type:{name}")),
            Def::Register { name, .. } => Some(format!("register:{name}")),
            Def::Mapdef { name, .. } => Some(format!("mapping:{name}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "type:cr_type",
            "register:R",
            "type:E",
            "type:pkt",
            "mapping:size_bits",
        ]
    );
    // Braceless enum body captured until next def.
    let e = pf
        .defs
        .iter()
        .find(|d| matches!(&d.def, Def::Type { name, .. } if name == "E"))
        .unwrap();
    match &e.def {
        Def::Type { body_src, .. } => assert!(body_src.contains("A | B | C"), "body: {body_src}"),
        other => panic!("expected Type, got {other:?}"),
    }
}

#[test]
fn pragma_shard_markers_preserved() {
    let src = fixture("pragma_shards.sail");
    let pf = parse_file("pragma_shards.sail", &src).unwrap();
    let kinds = kinds_of("pragma_shards.sail", &src);
    assert_eq!(
        kinds,
        vec![
            "pragma", // file_start
            "pragma", // $include_start
            "function",
            "pragma", // $[no_enum_number_conversions]
            "type",
            "pragma", // $opt { ... }
            "scattered",
            "scattered",
            "pragma", // $include_end
            "pragma", // file_end
        ]
    );

    // `and`-clause stays inside the function body (not a new def).
    let fun = pf.defs.iter().find(|d| d.def.kind() == "function").unwrap();
    match &fun.def {
        Def::Fundef { body_src, .. } => {
            assert!(body_src.contains("and f(0)"), "and-clause must stay in body: {body_src}");
            // Comment/string traps must not break brace matching.
            assert!(body_src.contains("unbalanced brace"));
        }
        other => panic!("expected Fundef, got {other:?}"),
    }

    // Shard markers in order.
    let markers: Vec<String> = pf
        .defs
        .iter()
        .filter(|d| d.def.is_shard_marker())
        .map(|d| match &d.def {
            Def::Pragma { name, .. } => name.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(markers, vec!["file_start", "include_start", "include_end", "file_end"]);

    // Structured pragma arg preserved raw.
    let opt = pf
        .defs
        .iter()
        .find(|d| matches!(&d.def, Def::Pragma { name, .. } if name == "opt"))
        .unwrap();
    match &opt.def {
        Def::Pragma { arg: PragmaArg::Structured(s), .. } => {
            assert!(s.contains("level = 2"), "arg: {s}")
        }
        other => panic!("expected structured pragma, got {other:?}"),
    }

    // `$[...]` attribute preserved as pragma node.
    assert!(pf.defs.iter().any(|d| matches!(&d.def,
        Def::Pragma { name, .. } if name == "no_enum_number_conversions")));
}

#[test]
fn errors_carry_spans() {
    // Unterminated brace: error, not panic; span points into the file.
    let err = parse_file("bad.sail", "function f(x) = {\n  x + 1\n").unwrap_err();
    assert_eq!(err.span.file, "bad.sail");
    assert!(err.span.start_line >= 1, "span: {}", err.span);
    assert!(err.message.contains("unterminated"), "msg: {}", err.message);

    // Stray text at top level.
    let err2 = parse_file("bad2.sail", "hello world\n").unwrap_err();
    assert_eq!(err2.span.file, "bad2.sail");
    assert_eq!(err2.span.start_line, 1);
    assert!(err2.message.contains("hello") || err2.message.contains("expected"), "{}", err2.message);

    // Empty file still yields exactly the shard markers.
    let pf = parse_file("empty.sail", "\n  // only a comment\n").unwrap();
    assert_eq!(pf.defs.len(), 2);
    assert!(pf.defs.iter().all(|d| d.def.is_shard_marker()));
}

#[test]
fn parallel_matches_sequential() {
    let a = fixture("fundefs.sail");
    let b = fixture("registers_types.sail");
    let c = fixture("pragma_shards.sail");
    let inputs = vec![
        ("fundefs.sail", a.as_str()),
        ("registers_types.sail", b.as_str()),
        ("pragma_shards.sail", c.as_str()),
    ];
    let par = parse_files_parallel(&inputs);
    assert_eq!(par.len(), 3);
    for ((path, src), res) in inputs.iter().zip(par.iter()) {
        let seq = parse_file(path, src).unwrap();
        assert_eq!(&seq, res.as_ref().unwrap(), "parallel != sequential for {path}");
    }
}

#[test]
fn scattered_clauses_end_operator_names_nested_comments() {
    // Nested block comment with an unbalanced `{` inside must not break
    // brace tracking; the file must parse as a whole.
    let src = fixture("scattered.sail");
    let pf = parse_file("scattered.sail", &src).unwrap();
    let kinds: Vec<&'static str> = pf.defs.iter().map(|d| d.def.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            "pragma", // file_start
            "val", "fixity", "overload",
            "scattered", // scattered function step
            "scattered", // function clause (braced body)
            "scattered", // function clause (braceless body)
            "scattered", // mapping clause
            "scattered", // end step
            "pragma", // file_end
        ]
    );

    // Operator names survive the `operator` keyword prefix.
    let val = pf.defs.iter().find(|d| d.def.kind() == "val").unwrap();
    assert!(matches!(&val.def, Def::Val { name, .. } if name == "^^"), "{:?}", val.def);
    let fx = pf.defs.iter().find(|d| d.def.kind() == "fixity").unwrap();
    assert!(
        matches!(&fx.def, Def::Fixity { level, name, .. } if level == "4" && name == ">=_s"),
        "{:?}", fx.def
    );
    let ov = pf.defs.iter().find(|d| d.def.kind() == "overload").unwrap();
    assert!(matches!(&ov.def, Def::Overload { name, .. } if name == "<<"), "{:?}", ov.def);

    // Clause names skip the `clause` keyword; `end` keeps its target.
    let names: Vec<String> = pf
        .defs
        .iter()
        .filter_map(|d| match &d.def {
            Def::Scattered { head_src, .. } => Some(head_src.clone()),
            _ => None,
        })
        .collect();
    assert!(names.iter().any(|h| h.starts_with("function step")), "{names:?}");
    assert!(names.iter().any(|h| h.starts_with("clause step")), "{names:?}");
    assert!(names.iter().any(|h| h.starts_with("clause csr_map")), "{names:?}");
    assert!(names.iter().any(|h| h == "end step"), "{names:?}");
}
