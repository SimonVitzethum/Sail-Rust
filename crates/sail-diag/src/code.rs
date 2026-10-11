#![forbid(unsafe_code)]
//! `--explain` catalog for every current `failwith` / `unreachable` site in
//! `sail/src/sail_lean_backend/pretty_print_lean.ml`.
//!
//! Each site becomes one [`DiagCode`] variant with a stable `E35xx` code, a
//! title, the OCaml location (`pretty_print_lean.ml:<line> (<fn>)`), the
//! message pattern that site prints, a workaround hint, and the current work
//! document. Every error points at maintained documentation plus a
//! workaround — that is exactly what [`DiagEntry`] carries.
//!
//! Categories: NExp (E3501) / Typ (E3502) / Pattern (E3503–E3505, E3517–E3518)
//! / Loop+Match (E3506–E3510) / Assign (E3511) / Expression (E3512) / Fundef
//! (E3513–E3515) / Typedef (E3516) / Def dispatch (E3519–E3520) / Import
//! files (E3521–E3522) / Instantiation deps (E3523).
//!
//! Note: the two `collect_import_files_aux` stack-underflow sites share one
//! function but are distinct match arms, so they get distinct codes.

use crate::Pass;
use std::fmt;
use std::str::FromStr;

/// Error returned when parsing an `E35xx` code string fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnknownDiagCode;

impl fmt::Display for UnknownDiagCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown diagnostic code")
    }
}

impl std::error::Error for UnknownDiagCode {}

/// Severity class of a catalog entry; drives the process exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Known Lean-backend limitation (`failwith "... not translatable
    /// yet"`, unsupported syntax). The user can work around it; see
    /// [`DiagEntry::workaround`]. Maps to exit code 4.
    BackendLimit,
    /// Violated compiler invariant (`unreachable` / `err_unreachable` /
    /// impossible-input `failwith`). Always a bug in the compiler, never
    /// in the Sail source; file it with a reproducer. Maps to exit 101.
    Internal,
}

/// One row of the `--explain` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagEntry {
    /// Stable code, e.g. `"E3501"`.
    pub code: &'static str,
    /// Short title, e.g. `"NExp not translatable"`.
    pub title: &'static str,
    /// Pass that raises this error (today: always [`Pass::LeanPrinter`]).
    pub pass: Pass,
    /// OCaml site, e.g. `"pretty_print_lean.ml:267 (doc_nexp/atomic)"`.
    pub ocaml_site: &'static str,
    /// Message pattern printed at the site (with `<...>` placeholders).
    pub trigger: &'static str,
    /// What the user can do right now.
    pub workaround: &'static str,
    /// Current documentation path; the legacy field name is kept for API compatibility.
    pub lane: &'static str,
    /// Severity class (exit-code mapping).
    pub kind: ErrorKind,
}

/// Diagnostic code for a known Lean-backend failure mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagCode {
    /// `pretty_print_lean.ml:267` — `doc_nexp` atomic fallback.
    NexpNotTranslatable,
    /// `pretty_print_lean.ml:340` — `doc_typ` fallback.
    TypNotTranslatable,
    /// `pretty_print_lean.ml:565` — `doc_pat` fallback.
    DocPatternNotTranslatable,
    /// `pretty_print_lean.ml:580` — bitvector subpattern without width.
    BitvectorPatternWidthUnknown,
    /// `pretty_print_lean.ml:584` — unexpected vector_concat subpattern.
    BitvectorPatternUnexpected,
    /// `pretty_print_lean.ml:706` — loop variable is not id/unit.
    LoopVarBadExpression,
    /// `pretty_print_lean.ml:753` — `when` guard outside `match_bv`.
    WhenClauseUnsupported,
    /// `pretty_print_lean.ml:779` — loop combinator arity (`doc_loop`).
    LoopCombinatorArity,
    /// `pretty_print_lean.ml:897` — loop variable not found in body.
    LoopVarNotFound,
    /// `pretty_print_lean.ml:917` — foreach combinator arity.
    ForeachCombinatorArity,
    /// `pretty_print_lean.ml:1048` — `E_assign` on complex l-exp.
    AssignNotImplemented,
    /// `pretty_print_lean.ml:1077` — `doc_exp` fallback.
    ExpressionNotTranslatable,
    /// `pretty_print_lean.ml:1126` — fundef without function type.
    FunctionWithoutFunctionType,
    /// `pretty_print_lean.ml:1250` — `FD_function` with empty list.
    FundefEmpty,
    /// `pretty_print_lean.ml:1253` — `FD_function` with several clauses.
    FundefMultiClause,
    /// `pretty_print_lean.ml:1342` — `doc_typdef` fallback.
    TypedefNotTranslatable,
    /// `pretty_print_lean.ml:1366` — `doc_val` let-pattern fallback.
    ValPatternNotTranslatable,
    /// `pretty_print_lean.ml:200` — `untuple_args_pat` shape mismatch.
    PatternTypeCombinationUnexpected,
    /// `pretty_print_lean.ml:1411` — unhandled def in `doc_defs_rec`.
    DefsRecUnreachable,
    /// `pretty_print_lean.ml:1444` — unhandled def in imports walk.
    CollectImportsUnreachable,
    /// `pretty_print_lean.ml:1658` — file-stack underflow on end-pragma.
    ImportFilesStackUnderflowEnd,
    /// `pretty_print_lean.ml:1666` — file-stack underflow on other def.
    ImportFilesStackUnderflowDef,
    /// `pretty_print_lean.ml:1709` — instantiation defs not one block.
    InstantiationDepsNotSingle,
}

impl DiagCode {
    /// Stable `E35xx` code string.
    #[must_use]
    pub fn code_str(self) -> &'static str {
        self.entry().code
    }

    /// The full catalog row for this code.
    #[must_use]
    pub fn entry(self) -> &'static DiagEntry {
        // Linear scan over 23 entries is plenty fast for diagnostics.
        for e in ALL {
            if e.code == self.code_str_inner() {
                return e;
            }
        }
        unreachable!("ALL covers every DiagCode variant (see catalog completeness test)");
    }

    fn code_str_inner(self) -> &'static str {
        match self {
            DiagCode::NexpNotTranslatable => "E3501",
            DiagCode::TypNotTranslatable => "E3502",
            DiagCode::DocPatternNotTranslatable => "E3503",
            DiagCode::BitvectorPatternWidthUnknown => "E3504",
            DiagCode::BitvectorPatternUnexpected => "E3505",
            DiagCode::LoopVarBadExpression => "E3506",
            DiagCode::WhenClauseUnsupported => "E3507",
            DiagCode::LoopCombinatorArity => "E3508",
            DiagCode::LoopVarNotFound => "E3509",
            DiagCode::ForeachCombinatorArity => "E3510",
            DiagCode::AssignNotImplemented => "E3511",
            DiagCode::ExpressionNotTranslatable => "E3512",
            DiagCode::FunctionWithoutFunctionType => "E3513",
            DiagCode::FundefEmpty => "E3514",
            DiagCode::FundefMultiClause => "E3515",
            DiagCode::TypedefNotTranslatable => "E3516",
            DiagCode::ValPatternNotTranslatable => "E3517",
            DiagCode::PatternTypeCombinationUnexpected => "E3518",
            DiagCode::DefsRecUnreachable => "E3519",
            DiagCode::CollectImportsUnreachable => "E3520",
            DiagCode::ImportFilesStackUnderflowEnd => "E3521",
            DiagCode::ImportFilesStackUnderflowDef => "E3522",
            DiagCode::InstantiationDepsNotSingle => "E3523",
        }
    }

    /// Parse `"E3501"` via [`std::str::FromStr`] (ASCII case-insensitive).
    pub fn parse_code(s: &str) -> Result<DiagCode, UnknownDiagCode> {
        s.parse()
    }
}

impl FromStr for DiagCode {
    type Err = UnknownDiagCode;

    fn from_str(s: &str) -> Result<DiagCode, UnknownDiagCode> {
        explain_str(s).map(|e| e.diag_code()).ok_or(UnknownDiagCode)
    }
}

impl DiagEntry {
    /// The [`DiagCode`] variant for this row.
    #[must_use]
    pub fn diag_code(self) -> DiagCode {
        match self.code {
            "E3501" => DiagCode::NexpNotTranslatable,
            "E3502" => DiagCode::TypNotTranslatable,
            "E3503" => DiagCode::DocPatternNotTranslatable,
            "E3504" => DiagCode::BitvectorPatternWidthUnknown,
            "E3505" => DiagCode::BitvectorPatternUnexpected,
            "E3506" => DiagCode::LoopVarBadExpression,
            "E3507" => DiagCode::WhenClauseUnsupported,
            "E3508" => DiagCode::LoopCombinatorArity,
            "E3509" => DiagCode::LoopVarNotFound,
            "E3510" => DiagCode::ForeachCombinatorArity,
            "E3511" => DiagCode::AssignNotImplemented,
            "E3512" => DiagCode::ExpressionNotTranslatable,
            "E3513" => DiagCode::FunctionWithoutFunctionType,
            "E3514" => DiagCode::FundefEmpty,
            "E3515" => DiagCode::FundefMultiClause,
            "E3516" => DiagCode::TypedefNotTranslatable,
            "E3517" => DiagCode::ValPatternNotTranslatable,
            "E3518" => DiagCode::PatternTypeCombinationUnexpected,
            "E3519" => DiagCode::DefsRecUnreachable,
            "E3520" => DiagCode::CollectImportsUnreachable,
            "E3521" => DiagCode::ImportFilesStackUnderflowEnd,
            "E3522" => DiagCode::ImportFilesStackUnderflowDef,
            "E3523" => DiagCode::InstantiationDepsNotSingle,
            _ => unreachable!("ALL contains only known codes"),
        }
    }
}

/// The full `--explain` table: one row per `failwith`/`unreachable` site.
pub static ALL: &[DiagEntry] = &[
    DiagEntry {
        code: "E3501",
        title: "NExp not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:267 (doc_nexp/atomic)",
        trigger: "NExp <con> <nexp> not translatable yet.",
        workaround: "Rewrite the type-level numeric expression with supported operators (literals, variables, +, -, *, unary -, 2^, if-then-else, div/mod/abs) or specialize it to a concrete nat literal.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3502",
        title: "Type not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:340 (doc_typ)",
        trigger: "Type <con> <typ> not translatable yet.",
        workaround: "Restate the type with supported constructors (bitvector/bits/vector, int/nat/bool/string/unit, option/list/tuple, register, range over provably-nneg bounds, existentials). Bidirectional and internal-unknown types need a typechecker (Lane 05) or structs/enums (Lane 30) fix.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3503",
        title: "Doc pattern not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:565 (doc_pat)",
        trigger: "Doc Pattern <con> <pat> not translatable yet.",
        workaround: "Use plain pattern forms (wildcards, literals, tuples, vectors, struct/cons/app patterns, as-patterns). Exotic pattern constructors need Lane 21 (exhaustiveness) support.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3504",
        title: "Bitvector subpattern has unclear width",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:580 (doc_vector_concat/doc_part P_id)",
        trigger: "Found subpattern with unclear width in bitvector pattern",
        workaround: "Give the bitvector variable an explicit width via a type ascription (P_typ) so the environment records a constant bitvector length.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3505",
        title: "Unexpected pattern in vector_concat",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:584 (doc_vector_concat/doc_part)",
        trigger: "Unexpected pattern in match_bv vector_concat pattern <pat>",
        workaround: "Restrict match_bv vector-concat parts to bit literals (binary/hex), width-annotated identifiers, nested vectors, or type-ascribed patterns.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3506",
        title: "Bad expression for loop variable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:706 (doc_loop_var)",
        trigger: "Bad expression for variable in loop: <exp>",
        workaround: "Bind loop variables to plain identifiers or unit before the loop; move computed loop state into a tuple of identifiers (Lane 27).",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3507",
        title: "'when' clause outside match_bv unsupported",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:753 (doc_match_clause Pat_when)",
        trigger: "The Lean backend does not support 'when' clauses in patterns:",
        workaround: "Move the 'when' guard into the branch body as an if-expression, or switch the match to match_bv form where guards are supported.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3508",
        title: "Loop combinator arity mismatch",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:779 (doc_loop)",
        trigger: "Unexpected number of arguments for loop combinator",
        workaround: "Internal desugaring error: the loop did not lower to 3 (cond/vars/body) or 4 (+measure) arguments. Minimize the Sail source and file it against Lane 27.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3509",
        title: "Loop variable not found in body",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:897 (foreach loop-var search)",
        trigger: "Unable to find loop variable in <body>",
        workaround: "Name the foreach loop variable with a plain identifier pattern (optionally type-ascribed) directly bound by the loop body; avoid shadowing or nesting it inside other binders.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3510",
        title: "Foreach combinator arity mismatch",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:917 (foreach combinator)",
        trigger: "Unexpected number of arguments for loop combinator",
        workaround: "Internal desugaring error in the foreach path. Minimize the Sail source and file it against Lane 27.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3511",
        title: "Assignment target not implemented",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1048 (E_assign)",
        trigger: "assign <lexp> not implemented yet",
        workaround: "Assign only to plain registers (LE_id/LE_typ) or register references (LE_deref). Split field/vector/range updates into explicit temporaries or struct-update expressions (Lane 19).",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3512",
        title: "Expression not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1077 (doc_exp)",
        trigger: "Expression <con> <exp> not translatable yet.",
        workaround: "Desugar the expression into supported forms (let/if/match/loops, struct ops, monadic combinators) or check the owning feature lane (extern/FFI Lane 25, early-return Lane 28, concurrency Lane 29).",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3513",
        title: "Function without function type",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1126 (doc_funcl_init)",
        trigger: "Function <id> does not have function type",
        workaround: "Compiler invariant: the value spec for this function is missing or not a function type. Check that the Sail definition typechecks (Lane 05) and file a reproducer if it does.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3514",
        title: "Function with empty clause list",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1250 (doc_fundef [])",
        trigger: "FD_function with empty function list",
        workaround: "Compiler invariant: a function definition arrived with no clauses. Check earlier passes (parser/Lane 17, rewrites/Lane 07) and file a reproducer.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3515",
        title: "Multi-clause function definition",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1253 (doc_fundef multi)",
        trigger: "FD_function with more than one clause :",
        workaround: "Merge the clauses into a single clause with an explicit match on the argument (rewrites/Lane 07 will eventually do this automatically).",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3516",
        title: "Type definition not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1342 (doc_typdef)",
        trigger: "Type definition <con> not translatable yet.",
        workaround: "Use variant/record/enum/abstract type definitions supported by Lane 30; bitfield and exotic definitions need Lane 30 support first.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3517",
        title: "Let-binding pattern not translatable",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1366 (doc_val)",
        trigger: "Pattern <con> <pat> not translatable yet.",
        workaround: "Bind top-level lets with a plain identifier (optionally type-ascribed) or an implicit-kid-compatible var pattern; destructure complex values inside the body instead.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::BackendLimit,
    },
    DiagEntry {
        code: "E3518",
        title: "Unexpected pattern/type combination",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:200 (untuple_args_pat)",
        trigger: "Unexpected pattern/type combination",
        workaround: "Compiler invariant: function argument patterns do not match the declared argument types. Check that the definition typechecks (Lane 05) and file a reproducer if it does.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3519",
        title: "Unhandled definition in printer dispatch",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1411 (doc_defs_rec)",
        trigger: "this case of doc_defs_rec should be unreachable",
        workaround: "Compiler invariant: a non-function/non-let definition reached the printer dispatch. File it with the Sail source as reproducer (Lanes 12/38 track dispatch coverage).",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3520",
        title: "Unhandled definition in import walk",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1444 (collect_imports_rec)",
        trigger: "this case of collect_imports_rec should be unreachable",
        workaround: "Compiler invariant in the import-splitting walk (Lane 11/12). File it with the Sail source as reproducer.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3521",
        title: "Import file-stack underflow on end pragma",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1658 (collect_import_files_aux)",
        trigger: "should not be reachable",
        workaround: "Compiler invariant: unbalanced include/file pragmas emptied the file stack. Check that every include_start/file_start has a matching end pragma, then file a reproducer.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3522",
        title: "Import file-stack underflow on definition",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1666 (collect_import_files_aux)",
        trigger: "should not be reachable",
        workaround: "Compiler invariant: a definition was seen with an empty file stack. Same triage as E3521 — check pragma balance, then file a reproducer.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
    DiagEntry {
        code: "E3523",
        title: "Instantiation dependencies not a single block",
        pass: Pass::LeanPrinter,
        ocaml_site: "pretty_print_lean.ml:1709 (pp_ast_lean)",
        trigger: "expected a single block of instantiation defs",
        workaround: "Compiler invariant in callgraph partitioning (Lane 08): instantiation definitions did not form one block. File it with the Sail source as reproducer.",
        lane: "WORK_STATE.md",
        kind: ErrorKind::Internal,
    },
];

/// Look up a catalog row by code string (ASCII case-insensitive).
#[must_use]
pub fn explain_str(s: &str) -> Option<&'static DiagEntry> {
    ALL.iter().find(|e| e.code.eq_ignore_ascii_case(s))
}

/// Render the `--explain <code>` output for one catalog row.
#[must_use]
pub fn format_explain(entry: &DiagEntry) -> String {
    let kind = match entry.kind {
        ErrorKind::BackendLimit => "known backend limitation (workaround exists)",
        ErrorKind::Internal => "internal compiler error (please report)",
    };
    format!(
        "error[{code}]: {title}\n\n  site:      {site}\n  pass:      {pass}\n  kind:      {kind}\n  trigger:   {trigger}\n\n  workaround:\n    {workaround}\n\n  see: {lane}\n",
        code = entry.code,
        title = entry.title,
        site = entry.ocaml_site,
        pass = entry.pass,
        kind = kind,
        trigger = entry.trigger,
        workaround = entry.workaround,
        lane = entry.lane,
    )
}

/// Render the one-line-per-row `--explain` index table.
#[must_use]
pub fn explain_table() -> String {
    let mut out = String::from("code   | title                                      | site\n");
    out.push_str("-------+--------------------------------------------+----------------------------------------\n");
    for e in ALL {
        out.push_str(&format!(
            "{:<6} | {:<42} | {}\n",
            e.code, e.title, e.ocaml_site
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_are_unique_and_wellformed() {
        let mut seen = HashSet::new();
        for e in ALL {
            assert!(e.code.starts_with("E35"), "unexpected code {}", e.code);
            assert!(seen.insert(e.code), "duplicate code {}", e.code);
        }
    }

    #[test]
    fn explain_str_roundtrips_all_codes() {
        for e in ALL {
            assert_eq!(explain_str(e.code), Some(e));
            assert_eq!(
                explain_str(&e.code.to_ascii_lowercase()),
                Some(e),
                "case-insensitive lookup failed for {}",
                e.code
            );
            assert_eq!(e.diag_code().entry().code, e.code);
            assert_eq!(e.code.parse(), Ok(e.diag_code()));
            assert_eq!(DiagCode::parse_code(e.code), Ok(e.diag_code()));
        }
        assert_eq!(explain_str("E9999"), None);
        assert_eq!("E9999".parse::<DiagCode>(), Err(UnknownDiagCode));
    }

    #[test]
    fn format_explain_mentions_code_and_workaround() {
        for e in ALL {
            let text = format_explain(e);
            assert!(
                text.contains(e.code),
                "explain for {} hides its code",
                e.code
            );
            assert!(
                text.contains(e.workaround),
                "explain for {} hides its workaround",
                e.code
            );
            assert!(
                text.contains(e.lane),
                "explain for {} hides its lane",
                e.code
            );
        }
    }
}
