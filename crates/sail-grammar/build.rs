fn main() {
    // No-op: `src/sail.lalrpop` ist Doku-Artefakt (Lane 17/59). Echte Generierung
    // (process_root) scheitert an ungelösten LALR-Konflikten — siehe GRAMMAR.md,
    // Follow-up-Lane mit Rig-Verifikation. Kein panic hier (brach Workspace-Check).
    println!("cargo:rerun-if-changed=src/sail.lalrpop");
}
