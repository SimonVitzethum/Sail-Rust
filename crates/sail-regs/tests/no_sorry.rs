//! E2 regression gate (Lane 52, sorry-elim): no generator output in
//! `sail-regs` may contain a `sorry` placeholder.
//!
//! `initial_regstate_snippet` reports a hard [`sail_diag::SailError`]
//! (span + `E3512`) instead of emitting a stub; every other emitter is
//! asserted sorry-free here so a future `sorry` string fails loudly.
//! (E3: this test is written, not executed, in this lane.)

use sail_regs::{RegisterInfo, extract_registers, parse_register_lines};

const FIXTURE: &str = include_str!("aarch64_small_regs.sail");

fn info() -> RegisterInfo {
    RegisterInfo::new(extract_registers(&parse_register_lines(FIXTURE)))
}

fn assert_no_sorry(haystack: &str, what: &str) {
    assert!(
        !haystack.to_lowercase().contains("sorry"),
        "{what} must not emit `sorry`:\n{haystack}"
    );
}

#[test]
fn all_emitted_snippets_are_sorry_free() {
    let full = info();
    assert_no_sorry(&full.doc_reg_info(), "doc_reg_info");
    assert_no_sorry(&full.register_enum(), "register_enum");
    assert_no_sorry(
        &full.register_type_abbrev(),
        "register_type_abbrev",
    );
    assert_no_sorry(&full.inhabited_instances(), "inhabited_instances");
    assert_no_sorry(&full.regstate(false), "regstate(flat)");
    assert_no_sorry(&full.regstate(true), "regstate(grouped)");
    assert_no_sorry(
        &RegisterInfo::empty().doc_reg_info(),
        "doc_reg_info(empty)",
    );
    assert_no_sorry(
        &RegisterInfo::empty().regstate(false),
        "regstate(empty)",
    );
    assert_no_sorry(
        &RegisterInfo::empty().regstate(true),
        "regstate(grouped, empty)",
    );
    // The empty initial state is exactly translatable and must stay clean.
    for grouped in [false, true] {
        let out = RegisterInfo::empty()
            .initial_regstate_snippet(grouped, sail_diag::Span::dummy())
            .expect("empty initial regstate must succeed");
        assert_no_sorry(&out, "initial_regstate_snippet(empty)");
    }
}

#[test]
fn nonempty_initial_regstate_is_hard_error_with_span_and_code() {
    let full = info();
    for grouped in [false, true] {
        let span = sail_diag::Span::new(sail_diag::FileId(7), 11, 13);
        let err = full
            .initial_regstate_snippet(grouped, span)
            .expect_err("non-empty initial regstate must be a hard error");
        assert_eq!(err.code, sail_diag::DiagCode::ExpressionNotTranslatable);
        assert_eq!(err.code.code_str(), "E3512");
        assert_eq!(err.span, span, "error must carry the caller span");
        let text = err.to_string();
        assert!(text.contains("E3512"), "{text}");
        assert_no_sorry(&text, "initial_regstate error");
    }
}
