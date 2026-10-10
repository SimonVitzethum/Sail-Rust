use sail_regs::{RegisterInfo, extract_registers, parse_register_lines};

const FIXTURE: &str = include_str!("aarch64_small_regs.sail");

#[test]
fn fixture_extracts_expected_registers() {
    let regs = extract_registers(&parse_register_lines(FIXTURE));
    assert_eq!(regs.len(), 3);
    assert_eq!(regs[0].name, "PC");
    assert_eq!(regs[1].name, "SP");
    assert_eq!(regs[2].name, "PSTATE");
    assert!(regs[0].has_init);
    assert!(!regs[1].has_init);
}

#[test]
fn fixture_lean_snippet_stable() {
    let info = RegisterInfo::new(extract_registers(&parse_register_lines(FIXTURE)));
    let out = info.doc_reg_info();
    for line in [
        "inductive Register : Type where",
        "  | PC",
        "  | SP",
        "  | PSTATE",
        "abbrev RegisterType : Register → Type",
        "  | .PC => (BitVec 64)",
        "  | .SP => (BitVec 64)",
        "  | .PSTATE => (BitVec 32)",
    ] {
        assert!(out.contains(line), "missing {line:?} in:\n{out}");
    }
    assert!(
        !out.contains("Inhabited"),
        "inhabited instances belong to doc_instantiations:\n{out}"
    );
    let inst = info.doc_instantiations();
    for line in [
        "instance : Inhabited (RegisterRef (BitVec 64)) where",
        "default := .Reg PC",
        "instance : Inhabited (RegisterRef (BitVec 32)) where",
        "default := .Reg PSTATE",
    ] {
        assert!(inst.contains(line), "missing {line:?} in:\n{inst}");
    }
    // Dedup: PC and SP share `(BitVec 64)` — exactly one instance.
    assert_eq!(
        inst.matches("Inhabited (RegisterRef (BitVec 64))").count(),
        1,
        "duplicate inhabited instance:\n{inst}"
    );
    let flat = info.regstate(false);
    assert!(flat.contains("  PSTATE : (BitVec 32)"), "{flat}");
    let grouped = info.regstate(true);
    assert!(grouped.contains("bits_64_reg"), "{grouped}");
    assert_eq!(info.read_reg("PC").as_deref(), Some("readReg PC"));
    assert_eq!(
        info.write_reg("PSTATE", "v").as_deref(),
        Some("writeReg PSTATE v")
    );
    let vals = info.initial_values();
    assert_eq!(vals.len(), 3);
    assert!(vals[0].from_reset && !vals[1].from_reset && vals[2].from_reset);
}
