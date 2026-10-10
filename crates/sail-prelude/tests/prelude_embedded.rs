#![forbid(unsafe_code)]
//! Integration tests: embedding present + empty model skeleton.

use sail_prelude::{EMPTY_MODEL_SKELETON, PRELUDE_FILES, SAIL_VERSION, empty_model, get, names};
use std::collections::HashSet;

const EXPECTED_FILES: &[&str] = &[
    "arith.sail",
    "asl_compat.sail",
    "concurrency_interface.sail",
    "dec_bits.sail",
    "elf.sail",
    "exception.sail",
    "exception_basic.sail",
    "exception_result.sail",
    "float.sail",
    "flow.sail",
    "generic_equality.sail",
    "hex_bits.sail",
    "hex_bits_signed.sail",
    "isla.sail",
    "mapping.sail",
    "mono_rewrites.sail",
    "option.sail",
    "prelude.sail",
    "real.sail",
    "result.sail",
    "reverse_endianness.sail",
    "smt.sail",
    "string.sail",
    "vector.sail",
    "vector_dec.sail",
    "vector_inc.sail",
];

#[test]
fn inventory_matches_sail_lib() {
    let got: HashSet<&str> = names().collect();
    let want: HashSet<&str> = EXPECTED_FILES.iter().copied().collect();
    assert_eq!(got, want, "vendored inventory must match sail/lib/*.sail");
    assert_eq!(PRELUDE_FILES.len(), 26);
}

#[test]
fn embedding_present_and_consistent() {
    for f in PRELUDE_FILES {
        assert!(!f.contents.is_empty(), "{} must embed non-empty text", f.name);
        assert!(
            !f.bytes.is_empty(),
            "{} must embed non-empty bytes",
            f.name
        );
        assert_eq!(
            f.contents.as_bytes(),
            f.bytes,
            "{}: include_str! and include_bytes! must agree",
            f.name
        );
    }
    // Spot checks on known content.
    let prelude = get("prelude.sail").expect("prelude.sail must be embedded");
    assert!(prelude.contents.contains("$include <flow.sail>"));
    let vec_dec = get("vector_dec.sail").expect("vector_dec.sail must be embedded");
    assert!(vec_dec.contents.contains("$include <vector.sail>"));
}

#[test]
fn version_is_pinned_to_dune_project() {
    assert_eq!(SAIL_VERSION, "0.20.3");
}

#[test]
fn empty_model_skeleton_check() {
    assert!(!EMPTY_MODEL_SKELETON.is_empty());
    assert!(!empty_model().is_empty());
    assert!(empty_model().contains("prelude.sail"));
    assert!(empty_model().contains("function main"));
}

#[test]
fn lookup_unknown_returns_none() {
    assert!(get("does_not_exist.sail").is_none());
}
