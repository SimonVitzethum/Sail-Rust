//! Lane 45 — E2E-Verdrahtung Parse→Lean (Code-Skizze, E3-Moratorium).
//!
//! Treibt `sail/test/lean/SailTinyArm.sail` durch die Phase-1-Crates und
//! emittiert, was mit dem heutigen Ein-AST-Stand schon geht (Stubs +
//! Lake-Skelett). Alles, was einen einheitlichen AST, echte Bodies oder
//! Solver/Concurrency-Anbindung braucht, wird als [`E2eFail`] mit genau
//! einem Owner-Lane zurückgegeben statt still zu fehlen — siehe auch
//! `WORK_STATE.md`.
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
//! - `sail_lean::{LeanPrinter, FunSig, Typ}`
//! - `sail_lake::{LakeProject, RegisterEntry}` (`(String, String)`)
//! - `sail_prelude::get("prelude.sail")` → `PreludeFile { contents, .. }`

#![forbid(unsafe_code)]

use std::fmt;

use crate::ooc::{NoopHooks, OocDriver, OocError, OocHooks, OocStats};

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

    /// CLI-Verdrahtung (Lane 116): genau eine Eingabe plus `--out-dir`- und
    /// `--e2e-module`-Overrides; der Rest bleibt Tiny-Arm-Default.
    #[must_use]
    pub fn with_input(
        input: String,
        out_dir: Option<String>,
        module: Option<String>,
    ) -> Self {
        let mut args = Self::tiny_arm_default();
        args.input = input;
        if let Some(dir) = out_dir {
            args.out_dir = dir;
        }
        if let Some(name) = module {
            args.module = name;
        }
        args
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
    /// Lane 62 (E4): OoC-Zähler des Rewrite-Shard-Streamings (sealed/evicted,
    /// Warm-Spills, Memo-Hits/Misses, Summaries).
    pub ooc: OocStats,
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
             (siehe WORK_STATE.md). Blockiert: Resolver, BidirectionalChecker, MemoSolver, \
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

/// Funktions-/Val-Namen aus `ParsedFile`: eine OoC-Shard-Einheit je Name.
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
        is_monadic: stateful,
        early_return: false,
        has_loop: false,
        span: sail_lean::Span::new(sail_lean::FileId(0), 0, 0),
    }
}

/// Funktions-/Val-Namen aus `ParsedFile`: eine OoC-Shard-Einheit je Name
/// (Stub-Signaturen + Rewrite-Streaming teilen sich den Zensus).
fn fun_names(parsed: &sail_parse::ParsedFile) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for d in &parsed.defs {
        match &d.def {
            sail_parse::Def::Fundef { name, .. }
            | sail_parse::Def::Val { name, .. }
            | sail_parse::Def::Mapdef { name, .. } => {
                if !names.iter().any(|n| n == name) {
                    names.push(name.clone());
                }
            }
            _ => {}
        }
    }
    names
}

/// Lane-62-Treiberkern (E4): jede Shard-Einheit läuft über die OoC-Barriere —
/// genau ein heißer Shard (Rewrite), danach Seal (serialisieren + verwerfen),
/// Warm-Spill des fertigen Shards und Export-Summary statt Body in RAM.
/// Solver-Memos hängen am selben Hook (`OocDriver::memo_prove`).
fn stream_rewrite<H: OocHooks>(
    fun_names: &[String],
    driver: &mut OocDriver<H>,
) -> Result<(), OocError> {
    for name in fun_names {
        // Stub-Bytes: echte Rewrite-Pässe (sail-rewrite-`Pipeline`) + kanonische
        // Serialisierung folgen mit dem Unified-AST (Fail ast-unify, lane-47).
        // Der `content_key` ist bis dahin ein Platzhalter — kanonisch ein
        // sail-ooc-`ContentId` (Herkunft s. `ooc.rs`); ohne stabilen
        // Content-Hash invalidiert jeder Lauf den Cache (E4).
        let bytes = format!("stub:{name}\n").into_bytes();
        let summary = name.as_bytes().to_vec();
        let key = format!("{}:{name}", crate::ooc::OOC_PASS_VERSION);
        driver.run_shard(name, &key, || bytes, summary)?;
    }
    Ok(())
}

/// Eintritt für die file-backed OoC-Bindung (Lane 60, `sail-ooc`): läuft den
/// normalen Treiber (`run`) und legt danach das Rewrite-Streaming über das
/// mitgegebene Backend (`driver`) — ohne Treiberänderung, sobald `sail-ooc`
/// ein `impl OocHooks` liefert. Gibt das `Outcome` mit echten OoC-Zählern
/// zurück; das Backend bleibt beim Aufrufer (inspizierbar nach `finish`).
pub fn run_with_ooc<H: OocHooks>(args: &E2eArgs, driver: &mut OocDriver<H>) -> Outcome {
    let mut out = run(args);
    if out
        .fails
        .iter()
        .any(|f| f.stage == "read-input" || f.stage == "parse")
    {
        return out;
    }
    // Namen erneut einsammeln (Re-Read, kein AST in RAM — E4: Shard-Streaming,
    // kein Alles-auf-einmal). Fehler hier sind in `run` schon als Fails
    // abgedeckt; der Rescan meldet sich notfalls selbst mit Owner lane-62.
    let parsed = match std::fs::read_to_string(&args.input)
        .map_err(|e| format!("Eingabe {} nicht lesbar: {e}", args.input))
        .and_then(|src| {
            sail_parse::parse_file(&args.input, &src).map_err(|e| format!("parse_file: {e}"))
        }) {
        Ok(p) => p,
        Err(detail) => {
            out.fails.push(E2eFail::new("ooc-rescan", "lane-62", detail));
            return out;
        }
    };
    let names = fun_names(&parsed);
    if let Err(e) = stream_rewrite(&names, driver) {
        out.fails.push(E2eFail::new(
            "ooc-wire",
            "lane-62",
            format!("OoC-Streaming: {e}"),
        ));
    }
    driver.finish();
    out.ooc = driver.stats().clone();
    out
}

/// Exit-Code-Abbildung für den CLI-Rand (Lane-116-Verdrahtung,
/// Lane-38-Konvention `EXIT_OK/EXIT_ERROR/EXIT_FATAL = 0/1/2`): 0 ohne Fails,
/// 2 bei Ein-/Ausgabe-Fails, sonst 1 (offene Pipeline-Fails, kein stiller
/// Erfolg über Lücken hinweg).
#[must_use]
pub fn exit_code_for(outcome: &Outcome) -> i32 {
    if outcome.fails.is_empty() {
        return 0;
    }
    let io = outcome.fails.iter().any(|f| {
        matches!(
            f.stage,
            "read-input" | "write-outdir" | "write-defs" | "write-file"
        )
    });
    if io { 2 } else { 1 }
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

    // Zensus: Funktionsnamen für Stub-Signaturen + OoC-Shard-Einheiten.
    let fun_names = fun_names(&parsed);
    let mut has_non_prelude_include = false;
    for d in &parsed.defs {
        if let sail_parse::Def::Pragma { name, arg } = &d.def {
            if name == "include" && !format!("{arg:?}").contains("prelude.sail") {
                has_non_prelude_include = true;
            }
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

    // Lane 62 (E4): Rewrite-Pässe laufen shardweise hinter der OoC-Barriere —
    // genau ein heißer Shard, danach Seal (serialisieren + verwerfen),
    // Warm-Spill fertiger Shards + Export-Summary statt Body in RAM.
    // Mit NoopHooks zählt nur die Barriere (kein Dateisystem-Effekt); die
    // file-backed Bindung an sail-ooc (Lane 60) erfolgt über run_with_ooc.
    let mut driver = OocDriver::new(NoopHooks::default());
    if let Err(e) = stream_rewrite(&fun_names, &mut driver) {
        out.fails.push(E2eFail::new(
            "ooc-wire",
            "lane-62",
            format!("OoC-Streaming: {e}"),
        ));
    }
    driver.finish();
    out.ooc = driver.stats().clone();

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

#[cfg(test)]
mod tests {
    //! Lane-62-Verdrahtung als Code (E3-Moratorium: läuft später im Test-Rig).
    use super::*;
    use crate::ooc::RecordingHooks;

    fn pos_with(events: &[String], prefix: &str) -> usize {
        events
            .iter()
            .position(|e| e.starts_with(prefix))
            .unwrap_or_else(|| panic!("Event fehlt: {prefix}"))
    }

    #[test]
    fn streaming_versiegelt_jeden_shard_in_reihenfolge() {
        let mut driver = OocDriver::new(RecordingHooks::default());
        let names = vec!["execute".to_string(), "decode".to_string()];
        stream_rewrite(&names, &mut driver).expect("streaming");
        driver.finish();
        assert_eq!(driver.stats().sealed, 2);
        assert_eq!(driver.stats().warm_spills, 2);
        assert_eq!(driver.stats().summaries, 2);
        assert_eq!(driver.hooks().summary_count(), 2);
        // Je Shard: begin → seal → spill → summary (Barriere um Rewrite).
        let ev = &driver.hooks().events;
        for name in &names {
            let key = format!("{}:{name}", crate::ooc::OOC_PASS_VERSION);
            assert!(pos_with(ev, &format!("begin:{name}")) < pos_with(ev, &format!("seal:{name}=")));
            assert!(pos_with(ev, &format!("seal:{name}=")) < pos_with(ev, &format!("spill:{key}")));
            assert!(pos_with(ev, &format!("spill:{key}")) < pos_with(ev, &format!("summary:{name}=")));
        }
        // Bodies stehen nie im Treiber-RAM — nur Keys + Summaries.
        assert!(ev.iter().all(|e| !e.contains("stub:execute\n")));
    }

    #[test]
    fn solver_memo_spill_wird_run_uebergreifend_genutzt() {
        let mut driver = OocDriver::new(RecordingHooks::default());
        let mut calls = 0u32;
        let query = format!("{}/(atom x>0)", crate::ooc::MEMO_DOMAIN_SOLVER_Z3);
        let first = driver.memo_prove(&query, || {
            calls += 1;
            b"sat".to_vec()
        });
        let second = driver.memo_prove(&query, || {
            calls += 1;
            b"neu-berechnet".to_vec()
        });
        assert_eq!(calls, 1);
        assert_eq!(first, b"sat");
        assert_eq!(second, b"sat");
        assert_eq!((driver.stats().memo_hits, driver.stats().memo_misses), (1, 1));
    }

    #[test]
    fn warm_budget_fehler_wird_fail_mit_genau_einem_owner() {
        // Backend ohne Budget: schon der erste Spill scheitert.
        let mut driver = OocDriver::new(RecordingHooks::default().with_max_live_bytes(0));
        let err = stream_rewrite(&["execute".to_string()], &mut driver).expect_err("Budget 0");
        assert!(matches!(err, OocError::WarmBudget { .. }));
        // Barriere ist frei (Seal war vor dem Spill), kein Shard versiegelt.
        assert_eq!(driver.hot(), None);
        assert_eq!(driver.stats().sealed, 0);
        // Abbildung auf Fail: genau ein Owner (Spec-Vorgabe).
        let fail = E2eFail::new("ooc-wire", "lane-62", format!("OoC-Streaming: {err}"));
        assert_eq!(fail.owner, "lane-62");
        assert!(fail.to_string().contains("Warm-Budget"));
    }

    #[test]
    fn with_input_ueberschreibt_nur_out_dir_und_modul() {
        let args = E2eArgs::with_input("a.sail".to_string(), Some("out".to_string()), None);
        assert_eq!(args.input, "a.sail");
        assert_eq!(args.out_dir, "out");
        assert_eq!(args.module, E2eArgs::tiny_arm_default().module);
    }

    #[test]
    fn exit_code_0_ohne_fails_2_bei_io_1_bei_pipeline_fails() {
        assert_eq!(exit_code_for(&Outcome::default()), 0);
        let mut io = Outcome::default();
        io.fails.push(E2eFail::new(
            "read-input",
            "lane-38",
            "weg".to_string(),
        ));
        assert_eq!(exit_code_for(&io), 2);
        let mut pipe = Outcome::default();
        pipe.fails.extend(static_fails());
        assert_eq!(exit_code_for(&pipe), 1);
    }
}
