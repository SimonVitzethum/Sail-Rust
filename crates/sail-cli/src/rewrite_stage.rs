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
//! Bodies stay small and disk-backed until this stage runs; the rewritten
//! items are retained resident for the (missing) downstream emitter and the
//! outcome reports that residency honestly instead of pretending the
//! streaming window survived.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufRead, BufReader};

use sail_ast::{Ast, Def, SimpleInterner};
use sail_ooc::ContentId;
use sail_rewrite::pipeline::TypedPipeline;
use sail_ty::{TypedDef, TypedDefKind};

use crate::args::{CliError, EXIT_ERROR};
use crate::frontend::{load_definition, Descriptor, FrontendProgram};
use crate::typecheck::Checked;

fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}

/// The Lean pass order this stage executes per definition.
///
/// By construction this is [`TypedPipeline::lean_implemented`] restricted to
/// the canonical-path implementations, in `lean_rewrites` order. The pipeline
/// is built from the same constructor inside [`rewrite_defs`], so the
/// recorded [`Rewritten::passes`] can never drift from what actually ran.
#[must_use]
pub fn pass_order() -> Vec<&'static str> {
    TypedPipeline::lean_implemented().pass_names()
}

/// One rewritten definition, passed through for the downstream emitter.
///
/// `position` is the source-order manifest index the body was reloaded from;
/// `name`/`kind` are the [`TypedDef`] emission labels, kept so logs can be
/// correlated without re-resolving symbols.
#[derive(Debug)]
pub struct RewrittenDef {
    pub position: usize,
    pub name: String,
    pub kind: TypedDefKind,
    pub def: Def,
    pub interner: SimpleInterner,
}

/// Outcome of running the typed rewrite pipeline over emitted [`TypedDef`]s.
///
/// `definitions`/`expressions` are derived observations (`items.len()` and
/// the summed [`TypedDef::expressions`](sail_ty::TypedDef) payload); they are
/// never a success criterion — see [`Rewritten::require_lean_emit`].
/// `items` is in the input emission order; each item ran the full
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
        Err(error(format!(
            "sail-rs: compilation incomplete: lean-emit stage \"lean-emit\" missing \
             (no Lean body emission); {} rewritten TypedDefs through {} passes ({}) \
             are not a successful model build",
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

/// Run [`TypedPipeline::lean_implemented`] over the emitted defs.
///
/// `loader` maps a [`TypedDef::position`](sail_ty::TypedDef) to its
/// (`Def`, definition-local interner) pair; [`run`] builds it from the
/// frontend warm layer, tests use an in-memory map. Each pair is wrapped as
/// a single-definition [`Ast`](sail_ast::Ast) and rewritten in place, in
/// emission order. A loader failure aborts the stage with the loader's
/// error — partial rewrites are never returned as success.
pub fn rewrite_defs(
    loader: &mut dyn FnMut(usize) -> Result<(Def, SimpleInterner), CliError>,
    defs: &[TypedDef],
) -> Result<Rewritten, CliError> {
    let pipeline = TypedPipeline::lean_implemented();
    let passes = pipeline.pass_names();
    let mut items = Vec::with_capacity(defs.len());
    let mut expressions = 0;
    for def in defs {
        let (mut body, mut interner) = loader(def.position)?;
        let mut ast = Ast::new(vec![body]);
        pipeline.run(&mut ast, &mut interner);
        debug_assert_eq!(ast.defs.len(), 1);
        body = ast
            .defs
            .pop()
            .expect("single-definition window keeps its def");
        expressions += def.expressions;
        items.push(RewrittenDef {
            position: def.position,
            name: def.name.clone(),
            kind: def.kind,
            def: body,
            interner,
        });
    }
    Ok(Rewritten {
        definitions: items.len(),
        expressions,
        passes,
        items,
    })
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
/// Bodies are reloaded from the frontend warm layer at their emission
/// positions (out-of-core until here) and each runs the full
/// [`pass_order`]. The returned [`Rewritten`] still fails
/// [`Rewritten::require_lean_emit`] until Lean body emission exists.
pub fn run(frontend: &FrontendProgram, checked: &Checked) -> Result<Rewritten, CliError> {
    let descriptors = manifest_descriptors(frontend)?;
    ensure_alignment(descriptors.len(), &checked.defs)?;
    let mut loader = |position: usize| {
        let descriptor = descriptors.get(position).ok_or_else(|| {
            CliError::fatal(format!(
                "rewrite stage: TypedDef position {position} outside {} manifest descriptors",
                descriptors.len(),
            ))
        })?;
        load_definition(&frontend.warm, &blob_id(descriptor)?)
    };
    rewrite_defs(&mut loader, &checked.defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::DefKind;
    use sail_rewrite::pipeline::{is_typed_implemented_pass, LEAN_REWRITES};

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

    fn memory_loader(
        bodies: &std::collections::BTreeMap<usize, (String, String)>,
    ) -> impl FnMut(usize) -> Result<(Def, SimpleInterner), CliError> + '_ {
        move |position| {
            bodies
                .get(&position)
                .map(|(key, _)| pragma_def(key))
                .ok_or_else(|| CliError::fatal(format!("missing test body {position}")))
        }
    }

    #[test]
    fn pass_order_is_the_implemented_lean_sequence() {
        let order = pass_order();
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
    fn rewrite_loads_by_position_and_preserves_emission_order() {
        let defs = vec![emitted("b", 1), emitted("a", 0)];
        let bodies = [(0, ("k0".to_string(), "v".to_string()))]
            .into_iter()
            .chain([(1, ("k1".to_string(), "v".to_string()))])
            .collect::<std::collections::BTreeMap<_, _>>();
        let rewritten = rewrite_defs(&mut memory_loader(&bodies), &defs).unwrap();
        // Emission order in, emission order out — even though positions are
        // visited out of order through the loader.
        assert_eq!(rewritten.definitions, 2);
        assert_eq!(rewritten.items[0].position, 1);
        assert_eq!(rewritten.items[1].position, 0);
        assert_eq!(rewritten.items[0].name, "b");
        assert_eq!(rewritten.expressions, 1 + 2);
        assert_eq!(rewritten.passes, pass_order());
        assert_eq!(rewritten.count_by_kind(TypedDefKind::Other), 2);
        assert_eq!(rewritten.count_by_kind(TypedDefKind::Function), 0);
        // Loader round-trip kept the right bodies: k1 belongs to position 1.
        for item in &rewritten.items {
            let DefKind::Pragma { key, .. } = &item.def.kind else {
                panic!("test bodies are pragmas")
            };
            assert_eq!(*key, format!("k{}", item.position));
        }
    }

    #[test]
    fn rewrite_empty_is_vacuous_but_still_incomplete() {
        let bodies = std::collections::BTreeMap::new();
        let rewritten = rewrite_defs(&mut memory_loader(&bodies), &[]).unwrap();
        assert!(rewritten.is_empty());
        assert_eq!(rewritten.definitions, 0);
        assert_eq!(rewritten.expressions, 0);
        assert_eq!(rewritten.passes, pass_order());
        let err = rewritten
            .require_lean_emit()
            .expect_err("rewrite is not a successful build");
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("lean-emit"), "{err}");
        assert!(
            err.message.contains("not a successful model build"),
            "{err}"
        );
    }

    #[test]
    fn loader_failure_aborts_without_partial_success() {
        let defs = vec![emitted("a", 0), emitted("b", 1)];
        let bodies = [(0, ("k0".to_string(), "v".to_string()))]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        let err = rewrite_defs(&mut memory_loader(&bodies), &defs)
            .expect_err("missing body must fail the stage");
        assert_eq!(err.exit_code, crate::args::EXIT_FATAL);
        assert!(err.message.contains('1'), "{err}");
    }

    #[test]
    fn require_lean_emit_reports_passes_and_counts() {
        let defs = vec![emitted("a", 0)];
        let bodies = [(0, ("k0".to_string(), "v".to_string()))]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        let rewritten = rewrite_defs(&mut memory_loader(&bodies), &defs).unwrap();
        let err = rewritten.require_lean_emit().expect_err("no stub success");
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("1 rewritten TypedDefs"), "{err}");
        assert!(
            err.message
                .contains(&format!("{} passes", rewritten.passes.len())),
            "{err}"
        );
        assert!(err.message.contains(&rewritten.passes.join(", ")), "{err}");
    }

    #[test]
    fn manifest_def_count_mismatch_is_fatal() {
        let defs = vec![emitted("a", 0)];
        let err = ensure_alignment(2, &defs).expect_err("counts must align 1:1");
        assert_eq!(err.exit_code, crate::args::EXIT_FATAL);
        assert!(err.message.contains("2 descriptors"), "{err}");
        assert!(err.message.contains("1 TypedDefs"), "{err}");
        ensure_alignment(1, &defs).expect("aligned counts pass");
    }
}
