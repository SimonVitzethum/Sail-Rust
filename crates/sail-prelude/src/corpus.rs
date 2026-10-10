#![forbid(unsafe_code)]
//! Korpus-Gate-Modell (Lane 81): alle `sail/lib`-Dateien durchs Frontend.
//!
//! E3-Hinweis: reines Daten-/Klassifikationsmodul, kein I/O, keine Läufe.
//! Der ausführbare Treiber liegt in `tests/corpus_frontend.rs` (Rig).
//! Dieses Modul liefert Inventar (59 `.sail`-Dateien: 26 vendored Top-Level
//! + `core/corelib.sail` + 21 `float/*` + 11 `concurrency_interface/*`)
//! plus die Owner-Tabelle für Lücken. Statuswerte sind Gate-Zustände, keine
//! Lauf-Ergebnisse: `Open` = Rig-Lauf offen.

/// Eine Korpus-Datei, relativ zu `sail/lib/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusFile {
    /// Pfad relativ zu `sail/lib/`, z. B. `"arith.sail"`, `"float/add.sail"`.
    pub rel: &'static str,
    /// `true`, wenn in `sail-prelude/sail-lib/` vendored (Top-Level-26).
    pub vendored: bool,
    /// Owner-Lane bei Lücke (Lane-Spec-Pfad).
    pub owner: &'static str,
    /// Erwartete Lücke / Notiz (kurz, Rig verifiziert).
    pub note: &'static str,
}

/// Alle 59 `.sail`-Dateien unter `sail/lib/` (Top-Level 26 + Unterverzeichnisse).
pub static CORPUS_FILES: &[CorpusFile] = &[
    // --- Top-Level (26, vendored) ---
    CorpusFile { rel: "arith.sail", vendored: true, owner: "lanes/07-rewrites-core.md", note: "Top-Level-`let` (smt/arith-Niveau) → Bridge-deferred, Lane 07" },
    CorpusFile { rel: "asl_compat.sail", vendored: true, owner: "lanes/02-parser.md", note: "val/pragma only, voll gebridged erwartet" },
    CorpusFile { rel: "concurrency_interface.sail", vendored: true, owner: "lanes/29-concurrency-archsem.md", note: "Deklarationshülle, voll gebridged erwartet" },
    CorpusFile { rel: "dec_bits.sail", vendored: true, owner: "lanes/02-parser.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "elf.sail", vendored: true, owner: "lanes/34-elfmain-executable.md", note: "val/pragma only, voll gebridged erwartet" },
    CorpusFile { rel: "exception.sail", vendored: true, owner: "lanes/11-sharding-incremental.md", note: "`scattered union exception` → Bridge-deferred, Lane 11 (+28)" },
    CorpusFile { rel: "exception_basic.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`union exception` → Bridge-deferred (Type), Lane 30" },
    CorpusFile { rel: "exception_result.sail", vendored: true, owner: "lanes/28-early-return-except.md", note: "`union clause exception` → Bridge-deferred, Lane 28 (+11)" },
    CorpusFile { rel: "float.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`type rounding_mode` → Bridge-deferred, Lane 30 (+24)" },
    CorpusFile { rel: "flow.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`type bit` → Bridge-deferred, Lane 30" },
    CorpusFile { rel: "generic_equality.sail", vendored: true, owner: "lanes/02-parser.md", note: "val/function/overload, voll gebridged erwartet" },
    CorpusFile { rel: "hex_bits.sail", vendored: true, owner: "lanes/22-bitvector-semantics.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "hex_bits_signed.sail", vendored: true, owner: "lanes/22-bitvector-semantics.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "isla.sail", vendored: true, owner: "lanes/02-parser.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "mapping.sail", vendored: true, owner: "lanes/20-mapping-encdec.md", note: "Mapdef gebridged, Body-Rohtext im Sidecar" },
    CorpusFile { rel: "mono_rewrites.sail", vendored: true, owner: "lanes/07-rewrites-core.md", note: "function/val, voll gebridged erwartet" },
    CorpusFile { rel: "option.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`union option` → Bridge-deferred, Lane 30" },
    CorpusFile { rel: "prelude.sail", vendored: true, owner: "lanes/11-sharding-incremental.md", note: "nur `$include`-Pragmas → Pragma-Nodes, Lane 11" },
    CorpusFile { rel: "real.sail", vendored: true, owner: "lanes/24-real-support.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "result.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`union result` → Bridge-deferred, Lane 30" },
    CorpusFile { rel: "reverse_endianness.sail", vendored: true, owner: "lanes/22-bitvector-semantics.md", note: "val/function, voll gebridged erwartet" },
    CorpusFile { rel: "smt.sail", vendored: true, owner: "lanes/07-rewrites-core.md", note: "Top-Level-`let` → Bridge-deferred, Lane 07" },
    CorpusFile { rel: "string.sail", vendored: true, owner: "lanes/02-parser.md", note: "val/overload/pragma, voll gebridged erwartet" },
    CorpusFile { rel: "vector.sail", vendored: true, owner: "lanes/30-structs-enums-unions.md", note: "`type bits` → Bridge-deferred, Lane 30 (+22)" },
    CorpusFile { rel: "vector_dec.sail", vendored: true, owner: "lanes/11-sharding-incremental.md", note: "nur `$include <vector.sail>`, Lane 11" },
    CorpusFile { rel: "vector_inc.sail", vendored: true, owner: "lanes/11-sharding-incremental.md", note: "nur `$include <vector.sail>`, Lane 11" },
    // --- core (1, extern) ---
    CorpusFile { rel: "core/corelib.sail", vendored: false, owner: "lanes/02-parser.md", note: "val/function only, voll gebridged erwartet" },
    // --- float (21, extern) ---
    CorpusFile { rel: "float/add.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/arith_internal.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/classify.sail", vendored: false, owner: "lanes/30-structs-enums-unions.md", note: "`enum float_class` → Bridge-deferred, Lane 30 (+24)" },
    CorpusFile { rel: "float/common.sail", vendored: false, owner: "lanes/19-register-model.md", note: "type/let/register/struct → deferred (30/07/19), Owner Register-Modell" },
    CorpusFile { rel: "float/eq.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/ge.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/ge_quiet.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/gt.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/gt_quiet.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/inf.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/interface.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/le.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/le_quiet.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/lt.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/lt_quiet.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/nan.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/ne.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/normal.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/rounding.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/sign.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    CorpusFile { rel: "float/zero.sail", vendored: false, owner: "lanes/24-real-support.md", note: "float-Ops, gebridged erwartet" },
    // --- concurrency_interface (11, extern) ---
    CorpusFile { rel: "concurrency_interface/barrier.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "`outcome sail_barrier` → Bridge-deferred, Lane 08 (+29)" },
    CorpusFile { rel: "concurrency_interface/cache_op.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "`outcome sail_cache_op` → Bridge-deferred, Lane 08 (+29)" },
    CorpusFile { rel: "concurrency_interface/common.sail", vendored: false, owner: "lanes/29-concurrency-archsem.md", note: "concurrency-Deklarationen, Lücke je Konstrukt (30/19/07/08)" },
    CorpusFile { rel: "concurrency_interface/emulator_memory.sail", vendored: false, owner: "lanes/29-concurrency-archsem.md", note: "concurrency-Deklarationen, Lücke je Konstrukt" },
    CorpusFile { rel: "concurrency_interface/exception.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "mehrere `outcome` → Bridge-deferred, Lane 08 (+29)" },
    CorpusFile { rel: "concurrency_interface/read_write.sail", vendored: false, owner: "lanes/29-concurrency-archsem.md", note: "concurrency-Deklarationen, Lücke je Konstrukt" },
    CorpusFile { rel: "concurrency_interface/read_write_v1.sail", vendored: false, owner: "lanes/30-structs-enums-unions.md", note: "struct/enum/union/register/let/outcome → deferred (30/19/07/08), Owner Typedef" },
    CorpusFile { rel: "concurrency_interface/read_write_v2.sail", vendored: false, owner: "lanes/04-nexp-constraint.md", note: "struct/register/let/outcome/constraint → deferred, Owner NExp/Constraint" },
    CorpusFile { rel: "concurrency_interface/sysreg.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "`outcome sail_sys_reg_*` → Bridge-deferred, Lane 08 (+29)" },
    CorpusFile { rel: "concurrency_interface/tlbi.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "`outcome sail_tlbi` → Bridge-deferred, Lane 08 (+29)" },
    CorpusFile { rel: "concurrency_interface/translation.sail", vendored: false, owner: "lanes/08-callgraph-effects.md", note: "`outcome sail_translation_*` → Bridge-deferred, Lane 08 (+29)" },
];

/// Anzahl aller Korpus-Dateien (26 + 1 + 21 + 11 = 59).
pub const CORPUS_LEN: usize = 59;

/// Anzahl vendored Dateien (in `sail-prelude/sail-lib/`).
pub const VENDORED_LEN: usize = 26;

/// Gate-Stufe des Korpus-Gates (parse → bridge → resolve).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStage {
    /// `sail-parse::parse_file` (Lane 02).
    Parse,
    /// `sail-parse::bridge_parsed` (Lane 63).
    Bridge,
    /// `sail-resolve::resolve` (Lane 03).
    Resolve,
}

impl GateStage {
    /// Stabile Bezeichnung für Reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GateStage::Parse => "parse",
            GateStage::Bridge => "bridge",
            GateStage::Resolve => "resolve",
        }
    }

    /// Owner-Lane der Stufe.
    #[must_use]
    pub fn owner(self) -> &'static str {
        match self {
            GateStage::Parse => "lanes/02-parser.md",
            GateStage::Bridge => "lanes/63-parse-unblock.md",
            GateStage::Resolve => "lanes/03-name-resolve.md",
        }
    }
}

/// Ergebnis-Zustand einer Korpus-Datei im Gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Rig-Lauf offen (E3: kein lokaler Lauf in dieser Lane).
    Open,
    /// Parse + Bridge + Resolve fehlerfrei (Rig verifiziert).
    Pass,
    /// Parse-Fehler (Owner: Lane 02).
    ParseError,
    /// Resolve-`distincto`-Fehler (Owner: Lane 03).
    ResolveError,
    /// Parsiert + resolvt, aber mit deferred Bridge-Rest (Owner je Datei).
    DeferredGap,
}

impl FileStatus {
    /// Stabile Bezeichnung für Reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FileStatus::Open => "open",
            FileStatus::Pass => "pass",
            FileStatus::ParseError => "parse-error",
            FileStatus::ResolveError => "resolve-error",
            FileStatus::DeferredGap => "deferred-gap",
        }
    }
}

/// Ein Gate-Bericht pro Datei (vom Treiber in `tests/corpus_frontend.rs` gefüllt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    /// Pfad relativ zu `sail/lib/`.
    pub rel: String,
    /// Gate-Status.
    pub status: FileStatus,
    /// Stufe des ersten Fehlers (nur bei `ParseError`/`ResolveError`).
    pub failed_stage: Option<GateStage>,
    /// Erste Fehlermeldung (nur bei Fehlern).
    pub message: Option<String>,
    /// Anzahl gebridgter `sail-ast`-Defs.
    pub bridged_defs: usize,
    /// Anzahl deferred `mini_ast`-Rest-Defs (Lücken-Indikator).
    pub deferred: usize,
    /// Anzahl Resolve-Fehler (`distincto`).
    pub resolve_errors: usize,
    /// Anzahl Resolve-Warnungen (Ctor-Kollisionen, nicht fatal).
    pub resolve_warnings: usize,
}

impl FileReport {
    /// Bericht für eine offene (noch nicht gelaufene) Datei.
    #[must_use]
    pub fn open(rel: &str) -> Self {
        FileReport {
            rel: rel.to_owned(),
            status: FileStatus::Open,
            failed_stage: None,
            message: None,
            bridged_defs: 0,
            deferred: 0,
            resolve_errors: 0,
            resolve_warnings: 0,
        }
    }

    /// `true`, wenn der Gate-Lauf für diese Datei fehlerfrei war
    /// (deferred-Rest ist kein Fehler, nur Lücke mit Owner).
    #[must_use]
    pub fn is_gate_ok(&self) -> bool {
        matches!(self.status, FileStatus::Pass | FileStatus::DeferredGap)
    }
}

/// Owner-Lane einer Korpus-Datei (`rel`-Pfad wie in [`CORPUS_FILES`]).
/// Unbekannte Pfade fallen auf den Parser-Gate-Owner zurück.
#[must_use]
pub fn owner_for(rel: &str) -> &'static str {
    for f in CORPUS_FILES {
        if f.rel == rel {
            return f.owner;
        }
    }
    GateStage::Parse.owner()
}

/// `true`, wenn die Datei vendored ist (sonst extern unter `sail/lib/`).
#[must_use]
pub fn is_vendored(rel: &str) -> bool {
    CORPUS_FILES.iter().any(|f| f.rel == rel && f.vendored)
}

/// Zusammenfassung über viele [`FileReport`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CorpusSummary {
    /// Anzahl reports gesamt.
    pub total: usize,
    /// Davon gate-ok (pass oder deferred-gap).
    pub ok: usize,
    /// Davon mit Parse-Fehler.
    pub parse_errors: usize,
    /// Davon mit Resolve-Fehler.
    pub resolve_errors: usize,
    /// Davon noch offen.
    pub open: usize,
    /// Summe deferred Rest-Defs (Lücken-Indikator).
    pub deferred_total: usize,
}

impl CorpusSummary {
    /// Faltet Reports zu einer Zusammenfassung.
    #[must_use]
    pub fn summarize(reports: &[FileReport]) -> Self {
        let mut s = CorpusSummary::default();
        s.total = reports.len();
        for r in reports {
            match r.status {
                FileStatus::Pass | FileStatus::DeferredGap => s.ok += 1,
                FileStatus::ParseError => s.parse_errors += 1,
                FileStatus::ResolveError => s.resolve_errors += 1,
                FileStatus::Open => s.open += 1,
            }
            s.deferred_total += r.deferred;
        }
        s
    }

    /// Einzeiler für Rig-Logs.
    #[must_use]
    pub fn one_line(&self) -> String {
        format!(
            "corpus: {}/{} ok (parse-errors={} resolve-errors={} open={} deferred={})",
            self.ok, self.total, self.parse_errors, self.resolve_errors, self.open, self.deferred_total
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn inventory_counts_match() {
        assert_eq!(CORPUS_FILES.len(), CORPUS_LEN);
        assert_eq!(CORPUS_FILES.iter().filter(|f| f.vendored).count(), VENDORED_LEN);
        assert_eq!(CORPUS_FILES.len() - VENDORED_LEN, 33);
    }

    #[test]
    fn inventory_paths_unique_and_suffixed() {
        let mut seen = HashSet::new();
        for f in CORPUS_FILES {
            assert!(f.rel.ends_with(".sail"), "unexpected path {}", f.rel);
            assert!(seen.insert(f.rel), "duplicate path {}", f.rel);
        }
    }

    #[test]
    fn vendored_matches_prelude_inventory() {
        let names: HashSet<&str> = crate::names().collect();
        for f in CORPUS_FILES.iter().filter(|f| f.vendored) {
            assert!(names.contains(f.rel), "vendored {} not in PRELUDE_FILES", f.rel);
        }
        assert_eq!(names.len(), VENDORED_LEN);
    }

    #[test]
    fn owner_lookup_falls_back_to_parser() {
        assert_eq!(owner_for("arith.sail"), "lanes/07-rewrites-core.md");
        assert_eq!(owner_for("float/common.sail"), "lanes/19-register-model.md");
        assert_eq!(owner_for("nope.sail"), GateStage::Parse.owner());
    }

    #[test]
    fn summary_folds_reports() {
        let mut pass = FileReport::open("a.sail");
        pass.status = FileStatus::Pass;
        let mut gap = FileReport::open("b.sail");
        gap.status = FileStatus::DeferredGap;
        gap.deferred = 3;
        let mut perr = FileReport::open("c.sail");
        perr.status = FileStatus::ParseError;
        let s = CorpusSummary::summarize(&[pass, gap, perr, FileReport::open("d.sail")]);
        assert_eq!(s.total, 4);
        assert_eq!(s.ok, 2);
        assert_eq!(s.parse_errors, 1);
        assert_eq!(s.open, 1);
        assert_eq!(s.deferred_total, 3);
        assert!(s.one_line().contains("2/4 ok"));
    }
}
