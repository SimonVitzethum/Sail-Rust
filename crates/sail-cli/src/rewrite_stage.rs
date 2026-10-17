//! Rewrite stage: run the typed rewrite pipeline over emitted [`TypedDef`]s.
//!
//! Lane 32 consumes what Lane 17 emits: [`Checked::defs`](crate::typecheck::Checked)
//! (checked definitions with their type/constraint payloads) are reconstructed
//! into real [`sail_ast::Ast`]s and rewritten with
//! [`TypedPipeline::lean_implemented`](sail_rewrite::pipeline::TypedPipeline::lean_implemented),
//! i.e. exactly the passes that have a canonical-path (`TypedPass`)
//! implementation, in Lean (`lean_rewrites`) order.
//!
//! Reconstruction is per [`TypedDef`](sail_ty::TypedDef): the definition body
//! is reloaded from the frontend warm layer at the def's manifest position and
//! wrapped as a single-definition [`Ast`](sail_ast::Ast) together with its
//! definition-local interner. Whole-program windows are deliberately *not*
//! built here: [`Symbol`](sail_ast::Symbol) handles are indices into
//! per-definition [`SimpleInterner`](sail_ast::SimpleInterner) namespaces, so
//! concatenating bodies under one interner would need a symbol-remapping
//! merge (a later lane; the interner docs note "merge stays a later lane").
//! Consequence, stated honestly: passes that compare definitions against each
//! other (e.g. `remove_duplicate_valspecs`) observe single-definition windows
//! in this stage and degrade to local behaviour. [`Rewritten::passes`]
//! records the exact pass order that ran per definition, so the degradation
//! is observable rather than silent.
//!
//! No stub success: [`Rewritten::require_lean_emit`] keeps failing explicitly
//! (exit status 1) because Lean body emission does not exist yet. Rewritten
//! definitions are therefore never reported as a successful model build.
//! Byte-bounded workers persist rewritten definitions through the frontend
//! flat-snapshot storage path before their input credits are released. The outcome
//! retains content IDs and labels, never whole-program AST bodies.
//! Whole-program passes still need a separate integration step; persisting
//! their local output does not make single-definition windows equivalent.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufRead, BufReader};

use sail_ast::Ast;
#[cfg(test)]
use sail_ast::{Def, SimpleInterner};
use sail_ooc::{ContentId, WarmLayer};
use sail_rewrite::pipeline::TypedPipeline;
use sail_shard::{Scheduler, SchedulerConfig};
use sail_ty::{TypedDef, TypedDefKind};

use crate::args::{CliError, EXIT_ERROR};
use crate::frontend::{load_definition, store_snapshot, Descriptor, FrontendProgram};
use crate::typecheck::Checked;

fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}

/// Disk reference to one rewritten definition for the downstream emitter.
///
/// `position` is the source-order manifest index the body was reloaded from;
/// `name`/`kind` are the [`TypedDef`] emission labels, kept so logs can be
/// correlated without re-resolving symbols.
#[derive(Debug)]
pub struct RewrittenDef {
    pub position: usize,
    pub name: String,
    pub kind: TypedDefKind,
    pub contents: Vec<ContentId>,
}

/// Outcome of running the typed rewrite pipeline over emitted [`TypedDef`]s.
///
/// `definitions`/`expressions` are derived observations (`items.len()` and
/// the summed [`TypedDef::expressions`](sail_ty::TypedDef) payload); they are
/// never a success criterion — see [`Rewritten::require_lean_emit`].
/// `items` is in source order; each item ran the full
/// [`Rewritten::passes`] order over its own single-definition window.
#[derive(Debug)]
pub struct Rewritten {
    pub definitions: usize,
    pub expressions: usize,
    pub passes: Vec<&'static str>,
    pub items: Vec<RewrittenDef>,
}

impl Rewritten {
    /// Whether no definition was rewritten.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many rewritten items carry this emission kind.
    #[must_use]
    pub fn count_by_kind(&self, kind: TypedDefKind) -> usize {
        self.items.iter().filter(|item| item.kind == kind).count()
    }

    /// Explicit error path for the missing Lean-emission stage (exit 1,
    /// never success-by-rewrite). The CLI keeps failing with "compilation
    /// incomplete" until a downstream emitter consumes [`Self::items`] and
    /// this returns `Ok`.
    pub fn require_lean_emit(&self) -> Result<(), CliError> {
        let outputs: usize = self.items.iter().map(|item| item.contents.len()).sum();
        let functions = self.count_by_kind(TypedDefKind::Function);
        let first = if self.is_empty() {
            "<empty>"
        } else {
            &self.items[0].name
        };
        Err(error(format!(
            "sail-rs: compilation incomplete: lean-emit stage \"lean-emit\" missing \
             (no Lean body emission); {} rewritten TypedDefs through {} passes ({}) \
             are not a successful model build; {outputs} output definitions, \
             {functions} source functions, first source `{first}`",
            self.definitions,
            self.passes.len(),
            self.passes.join(", "),
        )))
    }
}

/// Manifest descriptors must line up 1:1 with the emitted [`TypedDef`]s:
/// [`TypedDef::position`](sail_ty::TypedDef) is the descriptor index that
/// both the declaration and the body-check loops enumerated over.
fn ensure_alignment(descriptors: usize, defs: &[TypedDef]) -> Result<(), CliError> {
    if descriptors != defs.len() {
        return Err(CliError::fatal(format!(
            "rewrite stage: frontend manifest holds {descriptors} descriptors \
             but typecheck emitted {} TypedDefs; positions no longer align",
            defs.len(),
        )));
    }
    Ok(())
}

/// One input's source reference and byte charge. Bodies are loaded only after
/// the shared scheduler grants credits, so parallel workers keep the same
/// bounded input window as typechecking.
struct RewriteJob<'a> {
    checked: &'a TypedDef,
    content: ContentId,
    bytes: u64,
}

fn rewrite_jobs<'a, I>(
    warm: &WarmLayer,
    jobs: I,
    config: SchedulerConfig,
) -> Result<Rewritten, CliError>
where
    I: IntoIterator<Item = Result<RewriteJob<'a>, CliError>>,
    I::IntoIter: Send,
{
    let pipeline = TypedPipeline::lean_implemented();
    let mut rewritten = Rewritten {
        definitions: 0,
        expressions: 0,
        passes: pipeline.pass_names(),
        items: Vec::new(),
    };
    Scheduler::new(config).run_streaming_weighed(
        jobs,
        |job| job.as_ref().map_or(1, |job| job.bytes.max(1)),
        |job| {
            let job = job?;
            let (body, mut interner) = load_definition(warm, &job.content)?;
            let mut ast = Ast::new(vec![body]);
            pipeline.run(&mut ast, &mut interner);
            // Rewrites may synthesize helpers or split the input. Persist
            // every output in order; flatten first for iterative cleanup.
            let snapshots: Vec<_> = ast
                .defs
                .into_iter()
                .map(sail_ast::storage::DefinitionSnapshot::new)
                .collect();
            let mut contents = Vec::with_capacity(snapshots.len());
            for snapshot in &snapshots {
                let (content, _) = store_snapshot(warm, snapshot, &interner)?;
                contents.push(content);
            }
            Ok::<_, CliError>((
                RewrittenDef {
                    position: job.checked.position,
                    name: job.checked.name.clone(),
                    kind: job.checked.kind,
                    contents,
                },
                job.checked.expressions,
            ))
        },
        |_, result| {
            let (item, expressions) = result?;
            rewritten.expressions += expressions;
            rewritten.items.push(item);
            Ok::<_, CliError>(())
        },
    )?;
    // Completion order is deliberately independent of source order.
    rewritten.items.sort_by_key(|item| item.position);
    rewritten.definitions = rewritten.items.len();
    Ok(rewritten)
}

fn manifest_descriptors(frontend: &FrontendProgram) -> Result<Vec<Descriptor>, CliError> {
    let file = File::open(&frontend.manifest).map_err(|e| CliError::fatal(e.to_string()))?;
    BufReader::new(file)
        .lines()
        .map(|line| {
            let line = line.map_err(|e| CliError::fatal(e.to_string()))?;
            serde_json::from_str(&line).map_err(|e| CliError::fatal(e.to_string()))
        })
        .collect()
}

fn blob_id(descriptor: &Descriptor) -> Result<ContentId, CliError> {
    ContentId::from_hex(&descriptor.id)
        .ok_or_else(|| CliError::fatal("rewrite stage: invalid AST content ID"))
}

/// Rewrite every emitted [`TypedDef`](sail_ty::TypedDef) of `checked`.
///
/// Bodies are reloaded from the frontend warm layer within a byte-bounded
/// parallel window, and each runs the implemented Lean pass sequence. The returned [`Rewritten`] still fails
/// [`Rewritten::require_lean_emit`] until Lean body emission exists.
pub fn run(frontend: &FrontendProgram, checked: &Checked) -> Result<Rewritten, CliError> {
    let descriptors = manifest_descriptors(frontend)?;
    ensure_alignment(descriptors.len(), &checked.defs)?;
    let budget = sail_ooc::byte_budget_from_env(
        "SAIL_REWRITE_MEMORY",
        sail_ooc::DEFAULT_TYPECHECK_BUDGET_BYTES,
    )
    .map_err(CliError::fatal)?;
    eprintln!(
        "sail-rs: rewrite byte budget {} ({} workers)",
        sail_ooc::format_bytes(budget),
        frontend.summary.workers
    );
    let jobs = checked.defs.iter().map(|checked| {
        let descriptor = descriptors.get(checked.position).ok_or_else(|| {
            CliError::fatal(format!(
                "rewrite stage: TypedDef position {} outside {} manifest descriptors",
                checked.position,
                descriptors.len(),
            ))
        })?;
        let content = blob_id(descriptor)?;
        let bytes = std::fs::metadata(frontend.warm.blob_path(&content))
            .map_err(|e| CliError::fatal(e.to_string()))?
            .len();
        Ok(RewriteJob {
            checked,
            content,
            bytes,
        })
    });
    rewrite_jobs(
        &frontend.warm,
        jobs,
        SchedulerConfig::new_byte_budget(
            budget,
            sail_ooc::TYPECHECK_COUNT_CEILING,
            frontend.summary.workers,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::DefKind;
    use sail_rewrite::pipeline::{is_typed_implemented_pass, LEAN_REWRITES};

    struct TestStore(WarmLayer);

    impl TestStore {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::current_dir()
                .unwrap()
                .join("target/rewrite-stage-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            Self(WarmLayer::open(dir).with_max_live_bytes(0))
        }
    }

    impl Drop for TestStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.dir());
        }
    }

    fn pragma_def(key: &str) -> (Def, SimpleInterner) {
        (
            Def::dummy(DefKind::Pragma {
                key: key.to_string(),
                payload: "v".to_string(),
            }),
            SimpleInterner::new(),
        )
    }

    fn emitted(name: &str, position: usize) -> TypedDef {
        TypedDef::without_scheme(name.to_owned(), TypedDefKind::Other, position, position + 1)
    }

    struct Source {
        checked: TypedDef,
        content: ContentId,
        bytes: u64,
    }

    fn source(store: &TestStore, key: &str, position: usize, size: usize) -> Source {
        let (mut body, mut interner) = pragma_def(key);
        let symbol = sail_ast::Interner::intern(&mut interner, "local_symbol");
        assert_eq!(
            sail_ast::Interner::resolve(&interner, symbol),
            Some("local_symbol")
        );
        let DefKind::Pragma { payload, .. } = &mut body.kind else {
            panic!()
        };
        *payload = "x".repeat(size);
        let snapshot = sail_ast::storage::DefinitionSnapshot::new(body);
        let (content, bytes) = store_snapshot(&store.0, &snapshot, &interner).unwrap();
        Source {
            checked: emitted(key, position),
            content,
            bytes,
        }
    }

    fn run_sources(
        store: &TestStore,
        sources: &[Source],
        workers: usize,
        budget: u64,
    ) -> Result<Rewritten, CliError> {
        rewrite_jobs(
            &store.0,
            sources.iter().map(|source| {
                Ok(RewriteJob {
                    checked: &source.checked,
                    content: source.content,
                    bytes: source.bytes,
                })
            }),
            SchedulerConfig::new_byte_budget(budget, 1024, workers),
        )
    }

    const BUDGET: u64 = 32 * 1024 * 1024;

    #[test]
    fn pass_order_is_the_implemented_lean_sequence() {
        let order = TypedPipeline::lean_implemented().pass_names();
        assert_eq!(order, TypedPipeline::lean_implemented().pass_names());
        assert!(
            !order.is_empty(),
            "lean_implemented must wire at least one real pass"
        );
        for name in &order {
            assert!(
                is_typed_implemented_pass(name),
                "{name} must have a canonical-path implementation, no stubs"
            );
        }
        // Every implemented pass keeps its relative Lean (`lean_rewrites`)
        // position: indices into LEAN_REWRITES must be non-decreasing, so a
        // reordered pipeline fails here instead of silently changing
        // rewrite semantics.
        let mut previous = 0;
        for name in &order {
            let index = LEAN_REWRITES
                .iter()
                .position(|lean| lean == name)
                .unwrap_or_else(|| panic!("{name} not in LEAN_REWRITES"));
            assert!(index >= previous, "{name} moved before Lean order");
            previous = index;
        }
    }

    #[test]
    fn rewrite_preserves_helpers_symbols_and_source_order() {
        let store = TestStore::new();
        let sources = [source(&store, "b", 1, 1), source(&store, "a", 0, 1)];
        let rewritten = run_sources(&store, &sources, 4, BUDGET).unwrap();
        assert_eq!(rewritten.definitions, 2);
        assert_eq!(rewritten.items[0].position, 0);
        assert_eq!(rewritten.items[1].position, 1);
        assert_eq!(rewritten.items[0].name, "a");
        assert_eq!(rewritten.expressions, 3);
        assert_eq!(
            rewritten.passes,
            TypedPipeline::lean_implemented().pass_names()
        );
        assert_eq!(rewritten.count_by_kind(TypedDefKind::Other), 2);
        for item in &rewritten.items {
            assert!(
                item.contents.len() > 1,
                "generated helpers must be retained"
            );
            let mut found_original = false;
            for content in &item.contents {
                let (body, interner) = load_definition(&store.0, content).unwrap();
                assert_eq!(
                    sail_ast::Interner::resolve(&interner, sail_ast::Symbol(0)),
                    Some("local_symbol")
                );
                if let DefKind::Pragma { key, .. } = &body.kind {
                    assert_eq!(key, &item.name);
                    found_original = true;
                }
                sail_ast::storage::drop_definition(body);
            }
            assert!(found_original);
        }
        assert_eq!(store.0.live_bytes(), 0);
    }

    #[test]
    fn sequential_and_parallel_rewrites_are_identical() {
        let store = TestStore::new();
        let sources: Vec<_> = (0..32)
            .map(|i| source(&store, &format!("k{i}"), i, 1024))
            .collect();
        let single = run_sources(&store, &sources, 1, BUDGET).unwrap();
        let parallel = run_sources(&store, &sources, 4, BUDGET).unwrap();
        for (first, second) in single.items.iter().zip(&parallel.items) {
            assert_eq!(first.position, second.position);
            assert_eq!(first.contents, second.contents);
        }
        assert_eq!(single.expressions, parallel.expressions);
    }

    #[test]
    fn rewrite_empty_is_vacuous_but_still_incomplete() {
        let store = TestStore::new();
        let rewritten = run_sources(&store, &[], 4, BUDGET).unwrap();
        assert!(rewritten.is_empty());
        let error = rewritten.require_lean_emit().expect_err("no stub success");
        assert_eq!(error.exit_code, crate::args::EXIT_ERROR);
        assert!(error.message.contains("lean-emit"));
    }

    #[test]
    fn missing_blob_aborts_without_partial_success() {
        let store = TestStore::new();
        let sources = [source(&store, "a", 0, 1), source(&store, "b", 1, 1)];
        std::fs::remove_file(store.0.blob_path(&sources[1].content)).unwrap();
        let error = run_sources(&store, &sources, 4, BUDGET).unwrap_err();
        assert_eq!(error.exit_code, crate::args::EXIT_FATAL);
        assert!(error.message.contains("missing AST blob"));
    }

    #[test]
    fn failed_persistence_aborts_the_stage() {
        let store = TestStore::new();
        let sources = [source(&store, "a", 0, 1)];
        let first = run_sources(&store, &sources, 1, BUDGET).unwrap();
        let generated = first.items[0]
            .contents
            .iter()
            .find(|id| **id != sources[0].content)
            .unwrap();
        let path = store.0.blob_path(generated);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let error = run_sources(&store, &sources, 4, BUDGET).unwrap_err();
        assert_eq!(error.exit_code, crate::args::EXIT_FATAL);
    }

    #[test]
    fn rewritten_bodies_larger_than_test_memory_stay_on_disk() {
        let store = TestStore::new();
        let sources: Vec<_> = (0..160)
            .map(|i| source(&store, &format!("large_{i}"), i, 1024 * 1024))
            .collect();
        let rewritten = run_sources(&store, &sources, 4, 4 * 1024 * 1024).unwrap();
        assert_eq!(rewritten.definitions, sources.len());
        assert_eq!(store.0.live_bytes(), 0);
        let bytes: u64 = rewritten
            .items
            .iter()
            .flat_map(|item| &item.contents)
            .map(|content| std::fs::metadata(store.0.blob_path(content)).unwrap().len())
            .sum();
        assert!(
            bytes > 128 * 1024 * 1024,
            "workload must exceed the 128 MiB test limit"
        );
        let item = rewritten.items.last().unwrap();
        let mut found = false;
        for content in &item.contents {
            let (body, _) = load_definition(&store.0, content).unwrap();
            if let DefKind::Pragma { key, payload } = &body.kind {
                assert_eq!(key, "large_159");
                assert_eq!(payload.len(), 1024 * 1024);
                found = true;
            }
            sail_ast::storage::drop_definition(body);
        }
        assert!(found);
    }

    #[test]
    fn require_lean_emit_reports_passes_and_counts() {
        let store = TestStore::new();
        let sources = [source(&store, "a", 0, 1)];
        let rewritten = run_sources(&store, &sources, 2, BUDGET).unwrap();
        let error = rewritten.require_lean_emit().unwrap_err();
        assert!(error.message.contains("1 rewritten TypedDefs"));
        assert!(error.message.contains(&rewritten.passes.join(", ")));
    }

    #[test]
    fn manifest_def_count_mismatch_is_fatal() {
        let defs = vec![emitted("a", 0)];
        let error = ensure_alignment(2, &defs).unwrap_err();
        assert_eq!(error.exit_code, crate::args::EXIT_FATAL);
        ensure_alignment(1, &defs).unwrap();
    }
}
