//! Ordered preprocessing followed by bounded, parallel core AST conversion.
//!
//! Raw definitions and completed ASTs are spooled to disk. Only a bounded
//! number of definitions and global name/linkage summaries stay resident.
//! This is frontend infrastructure, not full Sail typechecking or Lean emission.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sail_ast::{DefKind, FileId, Node, NodeId, Span};
use sail_ooc::{ContentId, WarmLayer};
use sail_parse::infix::{Assoc, FixityEnv};
use sail_parse::preprocess::Preprocessor;
use sail_parse::{Def as SurfaceDef, PragmaArg, RealParser};
use sail_resolve::resolve::Resolver;
use sail_shard::{Scheduler, SchedulerConfig};
use serde::{Deserialize, Serialize};

use crate::args::{CliError, Options, EXIT_ERROR};

const LIB_PREFIX: &str = "<sail-lib>/";

fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, CliError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "sail-rs-frontend-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(CliError::fatal(format!(
                        "cannot create frontend spool: {e}"
                    )))
                }
            }
        }
        Err(CliError::fatal(
            "cannot allocate an isolated frontend spool",
        ))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    path: String,
    span: Span,
    source: String,
    pragma: Option<(String, String)>,
    fixity: Option<(String, u8, String)>,
}
struct Job {
    record: Record,
    fixities: FixityEnv,
}
struct Jobs {
    lines: std::io::Lines<BufReader<File>>,
    fixities: FixityEnv,
}
impl Iterator for Jobs {
    type Item = Result<Job, CliError>;
    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        Some((|| {
            let line =
                line.map_err(|e| CliError::fatal(format!("cannot read definition spool: {e}")))?;
            let record: Record = serde_json::from_str(&line)
                .map_err(|e| CliError::fatal(format!("invalid definition spool: {e}")))?;
            if let Some((assoc, level, name)) = &record.fixity {
                let assoc = match assoc.as_str() {
                    "infixl" => Assoc::Left,
                    "infixr" => Assoc::Right,
                    "infix" => Assoc::Non,
                    _ => {
                        return Err(error(format!(
                            "{}:{}: invalid fixity",
                            record.path, record.span
                        )))
                    }
                };
                self.fixities
                    .insert(name, assoc, *level)
                    .map_err(|e| error(e.to_string()))?;
            }
            Ok(Job {
                record,
                fixities: self.fixities.clone(),
            })
        })())
    }
}

fn library_source(name: &str, external: Option<&Path>) -> Result<(String, String), String> {
    if let Some(dir) = external {
        return file_source(&dir.join(name));
    }
    let file = sail_prelude::get(name).ok_or_else(|| format!("Sail library include <{name}> is unavailable; set SAIL_RS_LIBDIR to a compatible library directory"))?;
    Ok((format!("{LIB_PREFIX}{name}"), file.contents.into()))
}
fn file_source(path: &Path) -> Result<(String, String), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let path =
        fs::canonicalize(path).map_err(|e| format!("cannot resolve '{}': {e}", path.display()))?;
    Ok((path.to_string_lossy().into_owned(), source))
}
fn include_source(
    includer: &str,
    argument: &str,
    external: Option<&Path>,
) -> Result<Option<(String, String)>, String> {
    if let Some(name) = argument.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return library_source(name, external).map(Some);
    }
    if let Some(name) = argument.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        if let Some(parent) = includer.strip_prefix(LIB_PREFIX) {
            let path = Path::new(parent)
                .parent()
                .unwrap_or(Path::new(""))
                .join(name);
            return library_source(&path.to_string_lossy(), external).map(Some);
        }
        return file_source(
            &Path::new(includer)
                .parent()
                .unwrap_or(Path::new("."))
                .join(name),
        )
        .map(Some);
    }
    Err(format!(
        "invalid $include argument {argument:?}: expected quotes or angle brackets"
    ))
}

#[derive(Debug)]
pub struct FrontendSummary {
    pub files: usize,
    pub definitions: usize,
    pub ast_bytes: u64,
    pub workers: usize,
}

/// Convert all active input definitions without retaining whole-program bodies.
/// Any lexical, include, grammar, conversion or linkage gap stops the pipeline.
/// Successful conversion is deliberately not reported as successful checking.
pub fn run(opts: &Options) -> Result<FrontendSummary, CliError> {
    let workers = match std::env::var("SAIL_RS_THREADS") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| error("SAIL_RS_THREADS must be a positive integer"))?,
        Err(std::env::VarError::NotPresent) => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(16),
        Err(e) => return Err(error(format!("SAIL_RS_THREADS: {e}"))),
    };
    let scratch = Scratch::new()?;
    let external = std::env::var_os("SAIL_RS_LIBDIR").map(PathBuf::from);
    let manifest_path = scratch.0.join("definitions.jsonl");
    let mut manifest =
        BufWriter::new(File::create(&manifest_path).map_err(|e| CliError::fatal(e.to_string()))?);
    let mut files = BTreeMap::new();
    let mut preprocessor = Preprocessor::new(opts.lean.then(|| "lean".into()));
    for symbol in &opts.defines {
        preprocessor.define(symbol.clone());
    }
    let mut emit = |path: &str, source: &str, def: sail_parse::SpannedDef| -> Result<(), String> {
        let id = FileId(u32::try_from(files.len()).map_err(|_| "too many input files")?);
        let id = *files.entry(path.to_owned()).or_insert(id);
        let mut span = def.span;
        span.file = id;
        let pragma = match &def.def {
            SurfaceDef::Pragma { name, arg } => {
                if name == "option" {
                    return Err("$option directives require compiler option elaboration, which is not implemented".into());
                }
                let (PragmaArg::Line(arg) | PragmaArg::Structured(arg)) = arg;
                Some((name.clone(), arg.clone()))
            }
            _ => None,
        };
        let fixity = match &def.def {
            SurfaceDef::Fixity { prec, level, name } => Some((
                prec.clone(),
                level.parse::<u8>().map_err(|e| e.to_string())?,
                name.clone(),
            )),
            _ => None,
        };
        let record = Record {
            path: path.into(),
            span,
            pragma,
            fixity,
            source: source
                .get(span.start as usize..span.end as usize)
                .ok_or("invalid definition source span")?
                .into(),
        };
        serde_json::to_writer(&mut manifest, &record).map_err(|e| e.to_string())?;
        manifest.write_all(b"\n").map_err(|e| e.to_string())
    };
    if !opts.no_core {
        let (path, source) =
            library_source(sail_prelude::CORE_LIBRARY.name, external.as_deref()).map_err(error)?;
        preprocessor
            .process(
                &path,
                &source,
                |path, arg| include_source(path, arg, external.as_deref()),
                &mut emit,
            )
            .map_err(|e| error(e.to_string()))?;
    }
    for input in &opts.inputs {
        let (path, source) = file_source(Path::new(input)).map_err(CliError::fatal)?;
        preprocessor
            .process(
                &path,
                &source,
                |path, arg| include_source(path, arg, external.as_deref()),
                &mut emit,
            )
            .map_err(|e| error(e.to_string()))?;
    }
    drop(emit);
    manifest
        .flush()
        .map_err(|e| CliError::fatal(e.to_string()))?;
    drop(manifest);

    let jobs = Jobs {
        lines: BufReader::new(
            File::open(&manifest_path).map_err(|e| CliError::fatal(e.to_string()))?,
        )
        .lines(),
        fixities: FixityEnv::sail_defaults(),
    };
    let warm = WarmLayer::open(scratch.0.join("ast")).with_max_live_bytes(0);
    let scheduler = Scheduler::new(SchedulerConfig::new(2, workers));
    let mut resolver = Resolver::new();
    let mut definitions = 0;
    let mut ast_bytes = 0;
    // Descriptors also stay on disk. Later compiler phases can stream the
    // (AST, local interner) blobs without loading previous bodies into RAM.
    let mut snapshots = BufWriter::new(
        File::create(scratch.0.join("ast.jsonl")).map_err(|e| CliError::fatal(e.to_string()))?,
    );
    scheduler.run_streaming(
        jobs,
        |job| {
            let Job { record, fixities } = job?;
            let mut parser = RealParser::with_fixities(fixities);
            let def = if let Some((key, payload)) = record.pragma {
                Node::new(NodeId::DUMMY, record.span, DefKind::Pragma { key, payload })
            } else {
                parser
                    .parse_def(&record.source, record.span)
                    .map_err(|e| error(format!("{}: {e}", record.path)))?
            };
            let interner = parser.into_interner();
            let bytes = serde_json::to_vec(&(&def, &interner))
                .map_err(|e| CliError::fatal(e.to_string()))?;
            let id = ContentId::of_domain_parts("sail-rs/core-ast-v1", &[&bytes]);
            warm.store(&id, &bytes)
                .map_err(|e| CliError::fatal(e.to_string()))?;
            Ok::<_, CliError>((def, interner, id, bytes.len() as u64))
        },
        |_, result| {
            let (def, interner, id, bytes) = result?;
            resolver.add_definition(&def, &interner);
            writeln!(snapshots, "{id}").map_err(|e| CliError::fatal(e.to_string()))?;
            definitions += 1;
            ast_bytes += bytes;
            Ok::<_, CliError>(())
        },
    )?;
    snapshots
        .flush()
        .map_err(|e| CliError::fatal(e.to_string()))?;
    let resolved = resolver.finish();
    if let Some(first) = resolved.errors.first() {
        return Err(error(format!(
            "frontend name resolution: {first} ({} error(s))",
            resolved.errors.len()
        )));
    }
    Ok(FrontendSummary {
        files: files.len(),
        definitions,
        ast_bytes,
        workers,
    })
}
