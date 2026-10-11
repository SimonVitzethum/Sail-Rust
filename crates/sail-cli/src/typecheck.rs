//! Streaming canonical declaration elaboration and bounded parallel body checks.
use crate::{
    args::{CliError, EXIT_ERROR},
    frontend::{load_definition, Descriptor, FrontendProgram},
};
use sail_ast::storage::drop_definition;
use sail_ooc::ContentId;
use sail_shard::{Scheduler, SchedulerConfig};
use sail_ty::program::Program;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    sync::Arc,
};

pub struct Checked {
    pub definitions: usize,
    pub expressions: usize,
}
fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
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
pub fn run(frontend: &FrontendProgram) -> Result<Checked, CliError> {
    let mut program = Program::new();
    // Header dependencies are elaborated in source order. Their histories are
    // frozen once; a body lookup is a binary search at its own source position.
    for (position, descriptor) in descriptors(frontend)?.enumerate() {
        let descriptor = descriptor?;
        if !descriptor.declaration {
            continue;
        }
        let (def, interner) = load_definition(&frontend.warm, &id(&descriptor)?)?;
        let result = program.declare(&def, Arc::new(interner), position);
        drop_definition(def);
        result.map_err(|e| error(format!("sail-rs: declaration typecheck: {e}")))?;
    }
    let scheduler = Scheduler::new(SchedulerConfig::new(2, frontend.summary.workers));
    let mut checked = Checked {
        definitions: 0,
        expressions: 0,
    };
    scheduler.run_streaming(
        descriptors(frontend)?.enumerate(),
        |(position, descriptor)| {
            let descriptor = descriptor?;
            let (def, interner) = load_definition(&frontend.warm, &id(&descriptor)?)?;
            let result = program.check_definition(&def, Arc::new(interner), position);
            drop_definition(def);
            result.map_err(|e| error(format!("sail-rs: body typecheck: {e}")))
        },
        |_, result| {
            checked.expressions += result?;
            checked.definitions += 1;
            Ok::<_, CliError>(())
        },
    )?;
    Ok(checked)
}
