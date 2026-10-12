//! Lane 70 — Tiny-Modell Stufe 5: Lake-Skelett (Harness).
//!
//! Rendert das echte `sail-lake`-Projektskelett (`LakeProject::layout`)
//! fuer das Tiny-Modell — einmal mit Default (git-`require`) und einmal
//! mit lokalem `lib_path` (offline-faehig, zeigt auf den
//! `lean-sail`-Checkout wenn `LEAN_SAIL_PATH` existiert). Das Skelett
//! traegt nur Register-Templates, keine Funktions-Defs: das ist die
//! dokumentierte `lean-emit`-Luecke (kein Treiber am Base-Stand;
//! `run_compiler` endet nach Typecheck mit "compilation incomplete").
//! Der baubare Lean-Anteil (`Tiny.lean`) kommt aus `lane70_emit.rs`.

use sail_lake::{LakeProject, RegisterEntry};

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
fn lane70_tiny_lake_skeleton() {
    let registers: Vec<RegisterEntry> = vec![("status".to_string(), "Bool".to_string())];

    // Variante A: Defaults (git-require wie der OCaml-Treiber).
    let mut project = LakeProject::new("lane70tiny");
    project.registers = registers.clone();
    let files = project.layout().expect("default-layout rendert");
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert!(paths.contains(&"lakefile.toml"), "{paths:?}");
    assert!(paths.contains(&"lean-toolchain"), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.ends_with("/Defs.lean")),
        "{paths:?}"
    );
    let lakefile = files
        .iter()
        .find(|f| f.path == "lakefile.toml")
        .expect("lakefile");
    assert!(
        lakefile.contents.contains("git = \"https://github.com/rems-project/lean-sail\""),
        "{}",
        lakefile.contents
    );
    for f in &files {
        assert!(!has_hole(&f.contents), "Hole in {}:\n{}", f.path, f.contents);
    }

    // Variante B: lokaler Pfad (offline). Reiner Text-Render, keine
    // Netzwerk-/FS-Abhaengigkeit im Test selbst.
    let mut local = LakeProject::new("lane70tiny");
    local.registers = registers;
    local.options.lib_path = Some(
        std::env::var("LEAN_SAIL_PATH")
            .unwrap_or_else(|_| "/home/simon/Dokumente/Sail/lean-sail".to_string()),
    );
    let local_files = local.layout().expect("lib-path-layout rendert");
    let local_lakefile = local_files
        .iter()
        .find(|f| f.path == "lakefile.toml")
        .expect("lakefile");
    assert!(
        local_lakefile.contents.contains("path = "),
        "{}",
        local_lakefile.contents
    );

    // Ausschreiben fuer den Bericht (statisch, wird nicht gebaut —
    // Funktions-Defs fehlen by design, s. Modul-Doku).
    let out = out_dir().join("lake-skeleton");
    std::fs::create_dir_all(&out).expect("skeleton-dir anlegen");
    for f in &files {
        let dest = out.join(&f.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).expect("parent anlegen");
        }
        std::fs::write(&dest, &f.contents).expect("skeleton schreiben");
    }
    let defs = files
        .iter()
        .find(|f| f.path.ends_with("/Defs.lean"))
        .expect("Defs.lean");
    assert!(
        !defs.contents.contains("def const_one"),
        "Skelett darf keine Funktions-Defs enthalten:\n{}",
        defs.contents
    );
}
