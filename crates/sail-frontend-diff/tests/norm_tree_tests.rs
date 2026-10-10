#![forbid(unsafe_code)]
//! Normalizer + oracle-dump sifter tests (Lane 86, E3: code only, no runs).

use sail_frontend_diff::norm_tree::{
    NormKind, collapse_ws, from_rust, render_seq, sift_oracle_dump, strip_locations,
};

fn parsed_of(src: &str) -> sail_parse::ParsedFile {
    sail_parse::parse_file("t.sail", src).expect("fixture must parse")
}

#[test]
fn collapse_ws_runs() {
    assert_eq!(collapse_ws("  val  x :  int "), "val x : int");
    assert_eq!(collapse_ws("a\t\n b"), "a b");
    assert_eq!(collapse_ws(""), "");
}

#[test]
fn from_rust_drops_shard_markers() {
    let p = parsed_of("function foo(y : unit) -> unit = {\n  return y\n}\n");
    let (defs, dropped) = from_rust(&p);
    // file_start/file_end markers are dropped, the function remains.
    assert_eq!(dropped, 2);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].kind, NormKind::Fundef);
    assert_eq!(defs[0].name.as_deref(), Some("foo"));
}

#[test]
fn from_rust_val_and_register() {
    let p = parsed_of("val f : int -> int\nregister R0 : bits(64)\n");
    let (defs, _) = from_rust(&p);
    let kinds: Vec<NormKind> = defs.iter().map(|d| d.kind).collect();
    assert!(kinds.contains(&NormKind::Val), "kinds: {kinds:?}");
    assert!(kinds.contains(&NormKind::Register), "kinds: {kinds:?}");
    let val = defs.iter().find(|d| d.kind == NormKind::Val).unwrap();
    assert_eq!(val.name.as_deref(), Some("f"));
    assert!(!val.head_norm.contains("  "), "head: {:?}", val.head_norm);
}

#[test]
fn sift_oracle_dump_kinds_and_names() {
    let dump = "Sail 0.20.3\nDEF_val \"f\" : int -> int\nDEF_fundef \"foo\" l12\nnoise line\n";
    let defs = sift_oracle_dump(dump);
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0].kind, NormKind::Val);
    assert_eq!(defs[0].name.as_deref(), Some("f"));
    assert_eq!(defs[1].kind, NormKind::Fundef);
    // Banner gone, noise line without DEF_ token skipped.
    assert!(!defs.iter().any(|d| d.head_norm.contains("Sail")));
}

#[test]
fn sift_oracle_dump_unknown_suffix_is_other_with_token() {
    let a = sift_oracle_dump("DEF_val \"f\"\nDEF_frobnicate \"g\"\n");
    let b = sift_oracle_dump("DEF_val \"f\"\nDEF_whizzle \"g\"\n");
    assert_eq!(a[1].kind, NormKind::Other);
    // Different unknown constructors must not compare equal.
    assert_ne!(a[1].head_norm, b[1].head_norm);
}

#[test]
fn sift_oracle_dump_empty_on_empty() {
    assert!(sift_oracle_dump("").is_empty());
    assert!(sift_oracle_dump("\n\n").is_empty());
}

#[test]
fn strip_locations_only_exact_tokens() {
    // Exact location fragments go; identifiers containing 'l' survive.
    let got = strip_locations("DEF_fundef foo l12 l3.c4 let l");
    assert!(!got.split_whitespace().any(|t| t == "l12" || t == "l3.c4"));
    assert!(got.contains("foo"), "got: {got}");
    assert!(got.contains("let"), "got: {got}");
    // A bare 'l' is not a location.
    assert!(got.contains(" l"), "got: {got}");
}

#[test]
fn render_seq_stable_shape() {
    let p = parsed_of("val f : int -> int\n");
    let (defs, _) = from_rust(&p);
    let seq = render_seq(&defs);
    assert!(seq.starts_with("val:f "), "seq: {seq}");
    assert!(seq.ends_with('\n'));
    assert_eq!(render_seq(&[]), "");
}

#[test]
fn oracle_suffix_table() {
    assert_eq!(NormKind::of_oracle_suffix("val"), Some(NormKind::Val));
    assert_eq!(NormKind::of_oracle_suffix("reg"), Some(NormKind::Register));
    assert_eq!(NormKind::of_oracle_suffix("register"), Some(NormKind::Register));
    assert_eq!(NormKind::of_oracle_suffix("bogus"), None);
}
