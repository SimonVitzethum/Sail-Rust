//! Streaming canonical declaration elaboration and bounded parallel body checks.
//!
//! Lane 17 (TypedDef emission, ported from the lane-12
//! `TypedProgramStatus::Missing` stand): the canonical check itself is real
//! (`sail_ty::Program::declare` in source order plus bounded parallel
//! `check_definition` bodies). Each checked definition now additionally emits
//! an elaborated [`sail_ty::TypedDef`] — checked definition with
//! type/constraint payload — instead of ending as bare expression counts.
//! `Checked::definitions`/`expressions` remain only as derived log
//! observations; they are never a success criterion.
//!
//! Downstream stages consuming the emitted program (rewrite/lean) still do
//! not exist, so [`Checked::require_typed_program`] keeps failing explicitly
//! with exit status 1 instead of letting emission pass as a successful build
//! (no stub success).
//!
//! OOC-window observations ([`TypecheckOoc`]) and profiler counters (via
//! [`ProfileCounters`]) are collected here so the profiler hook needs no
//! `sail-ty` change once canonical proof counters become public. The
//! scheduler runs on the Lane-22 byte budget (see marker below).
use crate::{
    args::{CliError, EXIT_ERROR},
    frontend::{load_definition, Descriptor, FrontendProgram},
    ooc::ProfileCounters,
};
use sail_ast::storage::drop_definition;
use sail_ooc::ContentId;
use sail_shard::{Scheduler, SchedulerConfig};
use sail_ty::program::Program;
use sail_ty::{TypedDef, TypedDefKind};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    sync::Arc,
};

/// OOC-window observations of one typecheck run: every blob is read,
/// checked, then dropped; bodies never stay resident.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypecheckOoc {
    /// Definitions streamed through the warm layer (declare + body passes).
    pub streamed: usize,
    /// Blob bytes streamed in total.
    pub streamed_bytes: u64,
    /// Peak live bytes retained in the warm layer (0 by construction: the
    /// disk-backed reader path retains nothing live).
    pub warm_live_peak: u64,
}

/// Status of the elaborated typed-program stage. The only current variant
/// is the explicit gap: expression counts are not a typed program and must
/// never be reported as a successful model build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedProgramStatus {
    /// No elaborated `TypedDef` pipeline exists yet.
    Missing {
        /// Pipeline stage, e.g. `"typed-program"`.
        stage: &'static str,
        /// Exactly one owning lane, e.g. `"lane-12"`.
        owner: &'static str,
    },
}

pub struct Checked {
    /// Derived observation: number of emitted definitions (== `defs.len()`).
    pub definitions: usize,
    /// Derived observation: checked expressions summed over emissions.
    pub expressions: usize,
    pub ooc: TypecheckOoc,
    pub typed: TypedProgramStatus,
    /// The emitted typed program in source order (sorted after parallel
    /// emission, which observes completion order).
    pub defs: Vec<TypedDef>,
    /// Echte Solver-Beweiszähler (Lane-35-Hook über `Program::smt_totals`).
    pub smt_calls: u64,
    pub smt_seconds: f64,
}

impl Checked {
    /// Explicit error path for the missing typed-program stage (exit 1,
    /// never success-by-emission). The CLI keeps failing with
    /// "compilation incomplete" until a downstream stage consumes
    /// [`Self::defs`] and this returns `Ok`.
    pub fn require_typed_program(&self) -> Result<(), CliError> {
        match &self.typed {
            TypedProgramStatus::Missing { stage, owner } => Err(error(format!(
                "sail-rs: compilation incomplete: typed-program stage \"{stage}\" missing \
                 (owner {owner}); {} emitted TypedDefs and {} checked expressions are not a successful model build",
                self.defs.len(), self.expressions,
            ))),
        }
    }

    /// Record one emitted definition. Counts are derived from the emission
    /// itself so they can never drift from the typed program.
    fn push(&mut self, def: TypedDef, bytes: u64) {
        self.expressions += def.expressions;
        self.definitions += 1;
        self.ooc.streamed += 1;
        self.ooc.streamed_bytes += bytes;
        self.defs.push(def);
    }

    /// How many emissions carry a type/constraint payload.
    #[must_use]
    pub fn with_scheme(&self) -> usize {
        self.defs.iter().filter(|def| def.has_scheme()).count()
    }

    /// Profiler hook. Canonical proof counters come from the program's
    /// solver pool (`Program::smt_totals`, Lane-35); the OOC-window counts
    /// are real observations of this run.
    #[must_use]
    pub fn profile_counters(&self) -> ProfileCounters {
        ProfileCounters {
            smt_calls: self.smt_calls,
            smt_seconds: self.smt_seconds,
            ooc_streamed: self.ooc.streamed as u64,
            ooc_streamed_bytes: self.ooc.streamed_bytes,
            ooc_evicted: 0,
        }
    }
}
fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}
fn type_error(frontend: &FrontendProgram, phase: &str, e: sail_ty::TypeError) -> CliError {
    let source = frontend
        .source_paths
        .get(e.span.file.0 as usize)
        .map(String::as_str)
        .unwrap_or("<unknown source>");
    error(format!("sail-rs: {source}: {phase} typecheck: {e}"))
}
fn descriptors(
    frontend: &FrontendProgram,
) -> Result<impl Iterator<Item = Result<Descriptor, CliError>>, CliError> {
    let file = File::open(&frontend.manifest).map_err(|e| CliError::fatal(e.to_string()))?;
    Ok(BufReader::new(file).lines().map(|line| {
        let line = line.map_err(|e| CliError::fatal(e.to_string()))?;
        serde_json::from_str(&line).map_err(|e| CliError::fatal(e.to_string()))
    }))
}
fn id(descriptor: &Descriptor) -> Result<ContentId, CliError> {
    ContentId::from_hex(&descriptor.id).ok_or_else(|| CliError::fatal("invalid AST content ID"))
}
/// Blob size of a spooled definition (OOC-window accounting only; the
/// definition itself is loaded separately and fails first if missing).
fn blob_bytes(frontend: &FrontendProgram, id: &ContentId) -> Result<u64, CliError> {
    std::fs::metadata(frontend.warm.blob_path(id))
        .map(|m| m.len())
        .map_err(|e| CliError::fatal(e.to_string()))
}
pub fn run(frontend: &FrontendProgram) -> Result<Checked, CliError> {
    let mut program = Program::new();
    let mut ooc = TypecheckOoc::default();
    // Header dependencies are elaborated in source order. Their histories are
    // frozen once; a body lookup is a binary search at its own source position.
    for (position, descriptor) in descriptors(frontend)?.enumerate() {
        let descriptor = descriptor?;
        if !descriptor.declaration {
            continue;
        }
        let content = id(&descriptor)?;
        let (def, interner) = load_definition(&frontend.warm, &content)?;
        let bytes = blob_bytes(frontend, &content)?;
        let interner = Arc::new(interner);
        let result = if matches!(def.kind, sail_ast::DefKind::Let { .. }) {
            program
                .infer_let(&def, interner.clone(), position)
                .and_then(|inferred| program.declare_with(&def, interner, position, &inferred))
        } else {
            program.declare(&def, interner, position)
        };
        drop_definition(def);
        result.map_err(|e| type_error(frontend, "declaration", e))?;
        ooc.streamed += 1;
        ooc.streamed_bytes += bytes;
    }
    // Lane-22 byte budget (start): Scheduler-Credits in Bytes statt
    // Definitionszahl (Audit O-8). Budget aus SAIL_TYPECHECK_MEMORY, Default
    // 32 MiB. Gewicht = AST-Blobgroesse auf Platte (Stat, kein Laden);
    // Count-Deckel als Safety fuer winzige Items. Grosse Einzeldefinition
    // laeuft allein im leeren Fenster (kein Verhungern), muss aber selbst in
    // RAM passen. Lane-17-Hinweis: dieser Block ist absichtlich abgegrenzt.
    let byte_budget = sail_ooc::byte_budget_from_env(
        sail_ooc::TYPECHECK_BUDGET_ENV,
        sail_ooc::DEFAULT_TYPECHECK_BUDGET_BYTES,
    )
    .map_err(CliError::fatal)?;
    eprintln!(
        "sail-rs: typecheck byte budget {} ({} AST bytes spooled, {} workers)",
        sail_ooc::format_bytes(byte_budget),
        frontend.summary.ast_bytes,
        frontend.summary.workers,
    );
    let scheduler = Scheduler::new(SchedulerConfig::new_byte_budget(
        byte_budget,
        sail_ooc::TYPECHECK_COUNT_CEILING,
        frontend.summary.workers,
    ));
    let weigh = |(_position, descriptor): &(usize, Result<Descriptor, CliError>)| -> u64 {
        match descriptor {
            Err(_) => 1,
            Ok(descriptor) => ContentId::from_hex(&descriptor.id)
                .map(|content| {
                    blob_bytes(frontend, &content).unwrap_or(sail_ooc::UNKNOWN_BLOB_BYTES)
                })
                .unwrap_or(1)
                .max(1),
        }
    };
    let check_from_file: u32 = std::env::var("SAIL_RS_CHECK_FROM_FILE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut checked = Checked {
        definitions: 0,
        expressions: 0,
        ooc,
        typed: TypedProgramStatus::Missing {
            stage: "typed-program",
            owner: "lane-17",
        },
        defs: Vec::new(),
        smt_calls: 0,
        smt_seconds: 0.0,
    };
    scheduler.run_streaming_weighed(
        descriptors(frontend)?.enumerate(),
        weigh,
        |(position, descriptor)| {
            let descriptor = descriptor?;
            let content = id(&descriptor)?;
            let (def, interner) = load_definition(&frontend.warm, &content)?;
            let bytes = blob_bytes(frontend, &content)?;
            let interner = Arc::new(interner);
            // Development aid: `SAIL_RS_CHECK_FROM_FILE=n` declares everything but
            // checks only bodies from file index `n` on (re-checking a later
            // file after a fix without paying for the whole model).
            let result = if def.span.file.0 < check_from_file {
                Ok(0)
            } else {
                program.check_definition(&def, interner.clone(), position)
            };
            // Emit before the storage-path drop: snapshot the frozen
            // declaration-table scheme; only the small payload is retained.
            let emitted = match &result {
                Ok(expressions) => Some(TypedDef::checked(
                    &def,
                    &interner,
                    position,
                    *expressions,
                    &program,
                )),
                Err(_) => None,
            };
            drop_definition(def);
            result
                .map(|_| (emitted.expect("checked definition emits"), bytes))
                .map_err(|e| type_error(frontend, "body", e))
        },
        |_, result| {
            let (def, bytes) = result?;
            checked.push(def, bytes);
            Ok::<_, CliError>(())
        },
    )?;
    // Lane-22 byte budget (end)
    checked.ooc.warm_live_peak = frontend.warm.live_bytes();
    TypedDef::sort_by_position(&mut checked.defs);
    let (smt_calls, smt_seconds) = program.smt_totals();
    checked.smt_calls = smt_calls;
    checked.smt_seconds = smt_seconds;
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted() -> Checked {
        Checked {
            definitions: 4096,
            expressions: 1_052_672,
            ooc: TypecheckOoc {
                streamed: 4096,
                streamed_bytes: 200 << 20,
                warm_live_peak: 0,
            },
            typed: TypedProgramStatus::Missing {
                stage: "typed-program",
                owner: "lane-12",
            },
            defs: Vec::new(),
            smt_calls: 0,
            smt_seconds: 0.0,
        }
    }

    fn emitted() -> Checked {
        let mut defs = Vec::new();
        for position in 0..3 {
            defs.push(TypedDef::without_scheme(
                format!("d{position}"),
                TypedDefKind::Other,
                position,
                position * 10,
            ));
        }
        Checked {
            definitions: defs.len(),
            expressions: defs.iter().map(|def| def.expressions).sum(),
            ooc: TypecheckOoc {
                streamed: 3,
                streamed_bytes: 200 << 20,
                warm_live_peak: 0,
            },
            typed: TypedProgramStatus::Missing {
                stage: "typed-program",
                owner: "lane-17",
            },
            defs,
            smt_calls: 0,
            smt_seconds: 0.0,
        }
    }

    #[test]
    fn emission_never_passes_as_typed_program() {
        let checked = emitted();
        assert_eq!(checked.definitions, checked.defs.len());
        let err = checked
            .require_typed_program()
            .expect_err("emission is not a successful build");
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("typed-program"), "{err}");
        assert!(err.message.contains("lane-17"), "{err:?}");
        assert!(err.message.contains("3 emitted TypedDefs"), "{err}");
        assert!(
            err.message.contains("not a successful model build"),
            "{err}"
        );
    }

    #[test]
    fn counts_are_derived_from_emission() {
        let mut checked = Checked {
            definitions: 0,
            expressions: 0,
            ooc: TypecheckOoc::default(),
            typed: TypedProgramStatus::Missing {
                stage: "typed-program",
                owner: "lane-17",
            },
            defs: Vec::new(),
            smt_calls: 0,
            smt_seconds: 0.0,
        };
        for position in 0..4 {
            checked.push(
                TypedDef::without_scheme(
                    format!("d{position}"),
                    TypedDefKind::Function,
                    position,
                    position + 1,
                ),
                100,
            );
        }
        assert_eq!(checked.definitions, checked.defs.len());
        assert_eq!(
            checked.expressions,
            checked
                .defs
                .iter()
                .map(|def| def.expressions)
                .sum::<usize>()
        );
        assert_eq!(checked.ooc.streamed, 4);
        assert_eq!(checked.ooc.streamed_bytes, 400);
        assert_eq!(checked.with_scheme(), 0);
    }

    #[test]
    fn ooc_window_starts_empty() {
        let ooc = TypecheckOoc::default();
        assert_eq!(ooc.streamed, 0);
        assert_eq!(ooc.streamed_bytes, 0);
        assert_eq!(ooc.warm_live_peak, 0);
    }

    #[test]
    fn profiler_counters_carry_real_ooc_counts_and_zero_smt() {
        // Canonical proof counters are crate-internal in `sail-ty`; the
        // profiler hook reports them as honestly-zero until they become
        // public, while OOC-window counts are real observations.
        let counters = counted().profile_counters();
        assert_eq!(counters.smt_calls, 0);
        assert_eq!(counters.ooc_streamed, 4096);
        assert_eq!(counters.ooc_streamed_bytes, 200 << 20);
        assert!(counters.log_fields().contains("ooc_streamed=4096"));
    }
}
