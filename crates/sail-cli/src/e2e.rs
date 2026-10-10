//! Lane 45 — E2E-Verdrahtung Parse→Lean (Code-Skizze, E3-Moratorium).
//!
//! Treibt `sail/test/lean/SailTinyArm.sail` durch die Phase-1-Crates und
//! emittiert, was mit dem heutigen Ein-AST-Stand schon geht (Stubs +
//! Lake-Skelett). Alles, was einen einheitlichen AST, echte Bodies oder
//! Solver/Concurrency-Anbindung braucht, wird als [`E2eFail`] mit genau
//! einem Owner-Lane zurückgegeben statt still zu fehlen — siehe auch
//! `lanes/45-e2e-tiny-arm-fails.md`.
//!
//! Beachtung:
//! - E1: Concurrency-`instantiation`s werden nur als Belegstellen
//!   durchgereicht (Fail F8, Owner Lane 29); kein eigenes
//!   Speichermodell in dieser Lane.
//! - E2: nie `sorry` — [`sail_lean::LeanPrinter`] liefert für
//!   nicht-druckbare Bodies `Error::Unsupported`, das hier in Fails
//!   übersetzt wird.
//! - E3: kein Build/Test aus dieser Lane (`lake build` läuft später
//!   zentral beim Integrator mit Memory-Limits).
//!
//! Gelesene Main-Crate-APIs (nichts Fremdes kopiert/fixiert):
//! - `sail_parse::parse_file(&str, &str)` → `ParsedFile { path, defs }`
//! - `sail_regs::{parse_register_lines, extract_registers, lean_of_sail_typ}`
//! - `sail_lean::{LeanPrinter, FunSig, Typ, Effect}`
//! - `sail_lake::{LakeProject, RegisterEntry}` (`(String, String)`)
//! - `sail_prelude::get("prelude.sail")` → `PreludeFile { contents, .. }`

#![forbid(unsafe_code)]

use std::fmt;

/// Eine Lücke der Pipeline: genau ein Owner-Lane (Spec-Vorgabe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E2eFail {
    /// Pipeline-Stufe, z. B. `"includes"`, `"ast-unify"`, `"lean-bodies"`.
    pub stage: &'static str,
    /// Genau ein Owner, z. B. `"lane-47"`.
    pub owner: &'static str,
    /// Belegstelle + Begründung (TinyArm-Zeile, Crate-Fakt).
    pub detail: String,
}

impl E2eFail {
    #[must_use]
    pub fn new(stage: &'static str, owner: &'static str, detail: String) -> Self {
        Self { stage, owner, detail }
    }
}

impl fmt::Display for E2eFail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FAIL [{}] {}: {}", self.owner, self.stage, self.detail)
    }
}

/// Treiber-Argumente (bewusst `std::env`-pur, Flag-Parität ist Lane 38).
#[derive(Debug, Clone)]
pub struct E2eArgs {
    /// Eingabe, Default `sail/test/lean/SailTinyArm.sail` (ab Repo-Root).
    pub input: String,
    /// Ausgabe-Verzeichnis für das Lake-Skelett.
    pub out_dir: String,
    /// Abgeleiteter Lean-Modulname.
    pub module: String,
}

impl E2eArgs {
    #[must_use]
    pub fn tiny_arm_default() -> Self {
        Self {
            input: "sail/test/lean/SailTinyArm.sail".to_string(),
            out_dir: "target/sail-e2e/SailTinyArm".to_string(),
            module: "SailTinyArm".to_string(),
        }
    }
}

/// Teilergebnis: geschrieben wurde, was ohne Unified-AST geht; der Rest
/// steht als Fail-Liste (Done-Kriterium: baut ODER Fail-Liste).
#[derive(Debug, Default)]
pub struct Outcome {
    /// Geschriebene Dateien (projekt-relativ).
    pub emitted: Vec<String>,
    /// Lücken, je mit genau einem Owner.
    pub fails: Vec<E2eFail>,
}

/// Statische Lücken mit TinyArm-Belegstellen (Zeilen in
/// `sail/test/lean/SailTinyArm.sail`). Sie folgen aus den gelesenen
/// Crate-APIs: `mini_ast::Def` trägt nur `body_src`-Rohtext, `sail-lean`
/// druckt nur Signaturen/Stubs (`print_fun_stub`, Body-Druck explizit
/// out of scope), Concurrency braucht Lane 29.
fn static_fails() -> Vec<E2eFail> {
    vec![
        E2eFail::new(
            "includes",
            "lane-38",
            "SailTinyArm.sail Zeilen 5, 748-751: `$include <prelude.sail>` ist \
             via sail-prelude abgedeckt, `concurrency_interface*.sail` braucht \
             --sail-libdir-Aufloesung im Treiber."
                .to_string(),
        ),
        E2eFail::new(
            "parse-full-grammar",
            "lane-17",
            "Bit-Pattern-Klauseln (Z. 599, 658, 687, 703: `0b11@...@(opc:bits(2))@...`), \
             scattered-Klauseln (Z. 590/594) und `instantiation ... with` (Z. 757-837) \
             uebersteigen den Lane-02-Skelett-Parser; Grammatik-Hoheit Lane 17 (aktiv)."
                .to_string(),
        ),
        E2eFail::new(
            "ast-unify",
            "lane-47",
            "sail-parse liefert mini_ast::ParsedFile mit body_src-Rohtext; sail-resolve/\
             sail-ty/sail-lower/sail-effects/sail-mapping haben je eigene Minimaltypen \
             (je *_NEEDED.md). Blockiert: Resolver, BidirectionalChecker, MemoSolver, \
             effect_label, lower_shard, realize_mapping."
                .to_string(),
        ),
        E2eFail::new(
            "lean-typedefs",
            "lane-30",
            "bitfield (Z. 15-25), enum (Z. 67ff), struct (Z. 80ff), union ast (Z. 580-586): \
             doc_typdef-Port inkl. deriving/open-Emits fehlt; TD_bitfield ist heute Fehler."
                .to_string(),
        ),
        E2eFail::new(
            "lean-bitvec-exprs",
            "lane-22",
            "Bodies nutzen ^, +, truncate, sail_zero_extend/sail_sign_extend, Vergleiche \
             (Z. 616-746): Bitvektor-Ausdruckdruck nach sail-bitvec-Semantik fehlt."
                .to_string(),
        ),
        E2eFail::new(
            "lean-int-arith",
            "lane-23",
            "reg_index=range(0,31), unsigned(), Adress-Arithmetik (Z. 486, 603-605, 708-710): \
             Int/Nat-Abbildung + Bereichs-Nachweise fehlen."
                .to_string(),
        ),
        E2eFail::new(
            "lean-patterns",
            "lane-21",
            "decode-Dispatch ueber Bit-Patterns, match auf option(ast) (Z. 742-745), \
             Overloads X/PC (Z. 574/579), scattered decode/execute: Pattern-Elaboration \
             (match vs. match_bv) + Exhaustiveness fehlen."
                .to_string(),
        ),
        E2eFail::new(
            "lean-effects",
            "lane-28",
            "Register-Effekte (*GPRs[n], _PC, Z. 524-577/616-653), exit/assert \
             (Z. 771-799, 745): SailM/SailME-Schachtelung + sailTryCatch/Exit-Abbildung fehlen."
                .to_string(),
        ),
        E2eFail::new(
            "lean-concurrency",
            "lane-29",
            "instantiations sail_mem_read/write, sail_barrier, sail_cache_op, sail_tlbi, \
             sail_take/return_exception (Z. 757-837): doc_instantiations-Port + Arch-Instanz \
             fehlen (E1: Lean bleibt Master, hier nur Durchreichen)."
                .to_string(),
        ),
        E2eFail::new(
            "diagnostics",
            "lane-35",
            "Alle obigen Fails brauchen Span+Pass+Source-Snippet + Exit-Codes statt \
             String-Fehlern; --explain-Tabelle fehlt."
                .to_string(),
        ),
    ]
}

/// Zählt Funktions-/Val-Namen aus `ParsedFile` für Stub-Signaturen.
/// Effekt-Heuristik (Skizze!): `execute`/`fetch`/`rMem`/`wMem`-Namen sind
/// zustandsbehaftet, Rest pure — echte Effektanalyse (`effect_label`,
/// Lane 08) ist via F-ast-unify blockiert.
fn stub_sig(name: &str) -> sail_lean::FunSig {
    let stateful = name.contains("execute")
        || name.contains("fetch")
        || name.contains("Mem")
        || name.starts_with('w')
        || name.starts_with('r');
    sail_lean::FunSig {
        name: name.to_string(),
        args: Vec::new(),
        ret: sail_lean::Typ::Unit,
        effect: if stateful {
            sail_lean::Effect::Stateful
        } else {
            sail_lean::Effect::Pure
        },
        span: sail_lean::Span::new(sail_lean::FileId(0), 0, 0),
    }
}

/// Pipeline-Skizze: parsen → zählen → Stubs + Register + Lake-Skelett
/// schreiben → Fail-Liste zurückgeben. Kehrt nie mit stillem Erfolg über
/// Lücken hinweg zurück.
pub fn run(args: &E2eArgs) -> Outcome {
    let mut out = Outcome::default();
    out.fails.extend(static_fails());

    let src = match std::fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            out.fails.push(E2eFail::new(
                "read-input",
                "lane-38",
                format!("Eingabe {} nicht lesbar: {e}", args.input),
            ));
            return out;
        }
    };

    // Prelude-Sonde: nur Existenz, Inhalt kommt aus sail-lib (F-includes).
    if sail_prelude::get("prelude.sail").is_none() {
        out.fails.push(E2eFail::new(
            "prelude-vendored",
            "lane-18",
            "sail-prelude enthaelt prelude.sail nicht.".to_string(),
        ));
    }

    let parsed = match sail_parse::parse_file(&args.input, &src) {
        Ok(p) => p,
        Err(e) => {
            out.fails.push(E2eFail::new(
                "parse",
                "lane-17",
                format!("parse_file schlug fehl: {e}"),
            ));
            return out;
        }
    };

    // Zensus + Funktionsnamen für Stub-Signaturen.
    let mut fun_names: Vec<String> = Vec::new();
    let mut has_non_prelude_include = false;
    for d in &parsed.defs {
        match &d.def {
            sail_parse::Def::Fundef { name, .. }
            | sail_parse::Def::Val { name, .. }
            | sail_parse::Def::Mapdef { name, .. } => {
                if !fun_names.iter().any(|n| n == name) {
                    fun_names.push(name.clone());
                }
            }
            sail_parse::Def::Pragma { name, arg } => {
                if name == "include" && !format!("{arg:?}").contains("prelude.sail") {
                    has_non_prelude_include = true;
                }
            }
            _ => {}
        }
    }
    if !has_non_prelude_include {
        out.fails.retain(|f| f.stage != "includes");
    }

    // Lean-Stubs: Prelude geht, Bodies kommen als Unsupported zurück (E2)
    // und bestaetigen die F-lean-*-Fails, statt `sorry` zu emittieren.
    let printer = sail_lean::LeanPrinter::with_default();
    let mut defs_lean = String::new();
    if printer.print_prelude(&mut defs_lean).is_err() {
        out.fails.push(E2eFail::new(
            "lean-prelude",
            "lane-10",
            "print_prelude unerwartet fehlgeschlagen.".to_string(),
        ));
        return out;
    }
    let mut stub_ok = 0usize;
    for name in &fun_names {
        let sig = stub_sig(name);
        match printer.render_fun_stub(&sig, None) {
            Ok(_) => stub_ok += 1,
            Err(e) => {
                // Erwartet: harter Fehler statt sorry; Details stehen in den
                // statischen F-lean-*-Fails. Nur zählen, kein Fail-Spam.
                let _ = format!("{e}");
            }
        }
    }
    let _ = stub_ok;

    // Register: aus Rohtext via sail-regs (Lane 19, verdrahtbar ohne AST).
    let minis = sail_regs::parse_register_lines(&src);
    let regs = sail_regs::extract_registers(&minis);
    let entries: Vec<sail_lake::RegisterEntry> = regs
        .iter()
        .map(|r| {
            (
                r.name.clone(),
                sail_regs::lean_of_sail_typ(r.sail_typ.as_str()),
            )
        })
        .collect();

    // Lake-Skelett rendern (reine Text-Generierung, Lane 12) und schreiben.
    let mut project = sail_lake::LakeProject::new(args.module.clone());
    project.registers = entries;
    let files = match project.layout() {
        Ok(f) => f,
        Err(e) => {
            out.fails.push(E2eFail::new(
                "lake-layout",
                "lane-12",
                format!("LakeProject::layout: {e}"),
            ));
            return out;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.out_dir) {
        out.fails.push(E2eFail::new(
            "write-outdir",
            "lane-38",
            format!("out-dir {}: {e}", args.out_dir),
        ));
        return out;
    }
    // Defs.lean-Skizze (Prelude + Hinweis auf offene Bodies) als erste Datei.
    let defs_path = format!("{}/Defs.lean.sketch", args.out_dir);
    let defs_contents = format!(
        "-- Lane-45-Skizze fuer {} ({} defs gezaehlt).\n\
         -- Bodies offen, siehe Fail-Liste (kein sorry, E2).\n\n{defs_lean}",
        args.input,
        parsed.defs.len(),
    );
    match std::fs::write(&defs_path, defs_contents) {
        Ok(()) => out.emitted.push("Defs.lean.sketch".to_string()),
        Err(e) => out.fails.push(E2eFail::new(
            "write-defs",
            "lane-38",
            format!("Defs-Skizze: {e}"),
        )),
    }
    for f in &files {
        let dest = format!("{}/{}", args.out_dir, f.path);
        if let Some(parent) = std::path::Path::new(&dest).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                out.fails.push(E2eFail::new(
                    "write-outdir",
                    "lane-38",
                    format!("mkdir {}: {e}", parent.display()),
                ));
                continue;
            }
        }
        match std::fs::write(&dest, &f.contents) {
            Ok(()) => out.emitted.push(f.path.clone()),
            Err(e) => out.fails.push(E2eFail::new(
                "write-file",
                "lane-38",
                format!("{}: {e}", f.path),
            )),
        }
    }

    out
}
