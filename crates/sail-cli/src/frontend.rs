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
use sail_ast::storage::{DefinitionSnapshot, drop_definition};
use sail_ooc::{ContentId, WarmLayer};
use sail_parse::preprocess::Preprocessor;
use sail_parse::{Def as SurfaceDef, ParserContext, PragmaArg, RealParser};
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
        // /tmp is commonly tmpfs. With swap disabled its contents cannot be
        // evicted, defeating out-of-core storage even after all ASTs are freed.
        let root = if let Some(path) = std::env::var_os("SAIL_RS_SPOOL_DIR") {
            PathBuf::from(path)
        } else if let Some(path) = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from).filter(|path| path.is_absolute()) {
            path.join("sail-rs/spool")
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".cache/sail-rs/spool")
        } else {
            std::env::current_dir().map_err(|e| CliError::fatal(e.to_string()))?
                .join("target/sail-rs-spool")
        };
        fs::create_dir_all(&root).map_err(|e| CliError::fatal(format!("cannot create spool {}: {e}", root.display())))?;
        let root = root.canonicalize().map_err(|e| CliError::fatal(e.to_string()))?;
        #[cfg(target_os = "linux")]
        {
            let mounts = fs::read_to_string("/proc/self/mountinfo")
                .map_err(|e| CliError::fatal(format!("cannot check spool filesystem: {e}")))?;
            match spool_filesystem(&root, &mounts) {
                Some("tmpfs" | "ramfs" | "devtmpfs" | "hugetlbfs") => return Err(error(format!(
                    "out-of-core spool {} is on a RAM filesystem; set SAIL_RS_SPOOL_DIR to a disk-backed directory", root.display()))),
                None => return Err(CliError::fatal("cannot determine spool filesystem")),
                Some(_) => {},
            }
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = root.join(format!(
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

/// Resolve mount boundaries by path component, including bind mounts and
/// mountinfo's escaped spaces/backslashes. Paths are canonicalized first.
#[cfg(target_os = "linux")]
fn spool_filesystem<'a>(path: &Path, mounts: &'a str) -> Option<&'a str> {
    let mut best = None;
    let mut length = 0;
    for line in mounts.lines() {
        let Some((mount, filesystem)) = line.split_once(" - ") else { continue };
        let Some(point) = mount.split_whitespace().nth(4) else { continue };
        let point = point.replace("\\040", " ").replace("\\011", "\t")
            .replace("\\012", "\n").replace("\\134", "\\");
        if point.len() >= length && path.starts_with(Path::new(&point)) {
            length = point.len();
            best = filesystem.split_whitespace().next();
        }
    }
    best
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
    declaration: bool,
}
struct Job {
    record: Record,
    parser: RealParser,
    parsed: Option<sail_ast::Def>,
}
struct Jobs {
    lines: std::io::Lines<BufReader<File>>,
    context: ParserContext,
    trace: bool,
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
            let mut parser = RealParser::with_context(self.context.clone());
            // Declarations affect subsequent jobs. Convert just these in
            // source order and reuse their AST in the worker; ordinary bodies
            // remain parallel and never enter the context.
            let parsed = if record.declaration {
                if self.trace { eprintln!("sail-rs: declare {}:{}", record.path, record.span); }
                let def = parser.parse_def(&record.source, record.span)
                    .map_err(|e| error(format!("{}: {e}", record.path)))?;
                self.context = parser.context();
                Some(def)
            } else { None };
            Ok(Job { record, parser, parsed })
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

/// The standard library uses these backend declarations in unguarded code.
/// Apply them to the same options that later emission will receive, and keep
/// the directive in the AST. Other $option forms remain explicit errors until
/// their compilation-order effects can be supported.
fn apply_library_option(opts: &mut Options, command: &str) -> Result<(), String> {
    let mut words = command.split_whitespace();
    let flag = words.next().ok_or("empty $option directive")?;
    if !flag.starts_with('-') {
        return Err("$option requires a flag, not an input filename".into());
    }
    let name = words.next().ok_or("$option extern type requires a name")?;
    if words.next().is_some() {
        return Err("unsupported $option syntax: expected one extern-type declaration".into());
    }
    let flag = flag.trim_start_matches('-').replace('_', "-");
    match flag.as_str() {
        "lean-extern-type" => opts.lean_extern_types.push(name.into()),
        "lem-extern-type" | "coq-extern-type" => opts.passthrough_value.push((flag, name.into())),
        _ => return Err(format!("unsupported $option directive: {command}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn spool_mount_detection_respects_components_and_escaped_paths() {
        let mounts = "1 0 8:1 / / rw - ext4 /dev/disk rw\n\
2 1 0:2 / /tmp rw - tmpfs tmpfs rw\n\
3 1 8:2 / /data\\040disk rw - ext4 /dev/other rw\n\
4 3 0:3 / /data\\040disk/cache rw - tmpfs tmpfs rw\n\
5 1 8:1 /disk/subdir /bind rw - ext4 /dev/disk rw\n";
        for (path, expected) in [
            ("/tmp/work", "tmpfs"), ("/tmp2/work", "ext4"),
            ("/data disk/work", "ext4"), ("/data disk/cache/work", "tmpfs"),
            ("/bind/work", "ext4"),
        ] {
            assert_eq!(spool_filesystem(Path::new(path), mounts), Some(expected), "{path}");
        }
        assert_eq!(spool_filesystem(Path::new("/tmp"), "malformed\n"), None);
    }

    #[test]
    fn profiler_felder_tragen_ooc_fenster() {
        let summary = FrontendSummary {
            files: 1,
            definitions: 4096,
            ast_bytes: 200 << 20,
            workers: 16,
            window_peak_defs: 1,
            window_peak_bytes: 1 << 20,
            warm_live_peak: 0,
        };
        let fields = summary.profile_fields();
        assert!(fields.contains("defs=4096"), "{fields}");
        assert!(fields.contains("window_peak_defs=1"), "{fields}");
        assert!(fields.contains("warm_live_peak=0"), "{fields}");
        assert!(fields.contains("workers=16"), "{fields}");
    }

    #[test]
    fn library_extern_options_extend_the_effective_compiler_configuration() {
        let mut opts = Options::default();
        opts.lean_extern_types.push("Existing".into());
        for command in ["-lem_extern_type result", "-coq_extern_type result", "--lean-extern-type result"] {
            apply_library_option(&mut opts, command).unwrap();
        }
        assert_eq!(opts.lean_extern_types, ["Existing", "result"]);
        assert_eq!(opts.passthrough_value.len(), 2);
        for command in ["", "-lean_extern_type", "-lean_extern_type result other.sail", "-D FEATURE", "input.sail x"] {
            assert!(apply_library_option(&mut opts, command).is_err(), "{command}");
        }
    }
}

#[derive(Debug)]
pub struct FrontendSummary {
    pub files: usize,
    pub definitions: usize,
    pub ast_bytes: u64,
    pub workers: usize,
    /// Geordnete Consumer-Spitze: höchstens eine Definition gleichzeitig
    /// materialisiert (1 sobald ≥ 1 Definition, sonst 0).
    pub window_peak_defs: usize,
    /// Größter einzelner AST-Blob im Fenster (muss samt
    /// Zwischenrepräsentation in RAM passen).
    pub window_peak_bytes: u64,
    /// Peak live gehaltener Warm-Bytes (0 per Konstruktion: `store`/`reader`
    /// halten nichts live; Bodies werden nach Verarbeitung verworfen).
    pub warm_live_peak: u64,
}

impl FrontendSummary {
    /// Stabile `key=value`-Felder für Profiler-/Verbose-Zeilen (Lane 12).
    #[must_use]
    pub fn profile_fields(&self) -> String {
        format!(
            "defs={} ast_bytes={} window_peak_defs={} window_peak_bytes={} warm_live_peak={} workers={}",
            self.definitions,
            self.ast_bytes,
            self.window_peak_defs,
            self.window_peak_bytes,
            self.warm_live_peak,
            self.workers,
        )
    }
}

pub struct FrontendProgram {
    pub summary: FrontendSummary,
    pub warm: WarmLayer,
    pub manifest: PathBuf,
    pub source_paths: Vec<String>,
    _scratch: Scratch,
}

#[derive(Serialize, Deserialize)]
pub struct Descriptor {
    pub id: String,
    pub declaration: bool,
}

pub fn load_definition(warm: &WarmLayer, id: &ContentId) -> Result<(sail_ast::Def, sail_ast::SimpleInterner), CliError> {
    let reader = warm.reader(id).map_err(|e| CliError::fatal(e.to_string()))?
        .ok_or_else(|| CliError::fatal(format!("missing AST blob {id}")))?;
    let (snapshot, interner) = DefinitionSnapshot::read(reader)
        .map_err(|e| CliError::fatal(format!("AST blob {id}: {e}")))?;
    Ok((snapshot.into_definition(), interner))
}

/// Persist one flat definition and its symbols without caching the bytes.
/// Callers retain the snapshot for iterative disposal on success and failure.
pub fn store_snapshot(
    warm: &WarmLayer,
    snapshot: &DefinitionSnapshot,
    interner: &sail_ast::SimpleInterner,
) -> Result<(ContentId, u64), CliError> {
    let mut bytes = Vec::new();
    snapshot.write(&mut bytes, interner)
        .map_err(|e| CliError::fatal(e.to_string()))?;
    let id = ContentId::of_domain_parts("sail-rs/flat-ast-v1", &[&bytes]);
    warm.store(&id, &bytes)
        .map_err(|e| CliError::fatal(e.to_string()))?;
    Ok((id, bytes.len() as u64))
}

/// Convert all active input definitions without retaining whole-program bodies.
/// Any lexical, include, grammar, conversion or linkage gap stops the pipeline.
/// Successful conversion is deliberately not reported as successful checking.
pub fn run(opts: &mut Options) -> Result<FrontendProgram, CliError> {
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
    if opts.verbose >= 2 { eprintln!("sail-rs: disk spool {}", scratch.0.display()); }
    let external = std::env::var_os("SAIL_RS_LIBDIR").map(PathBuf::from);
    let manifest_path = scratch.0.join("definitions.jsonl");
    let mut manifest =
        BufWriter::new(File::create(&manifest_path).map_err(|e| CliError::fatal(e.to_string()))?);
    let mut files = BTreeMap::new();
    let mut preprocessor = Preprocessor::new(opts.lean.then(|| "lean".into()));
    for symbol in &opts.defines {
        preprocessor.define(symbol.clone());
    }
    let no_core = opts.no_core;
    let inputs = opts.inputs.clone();
    let mut declared_vals: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut default_inc = false;
    let mut emit = |path: &str, source: &str, def: sail_parse::SpannedDef| -> Result<(), String> {
        let id = FileId(u32::try_from(files.len()).map_err(|_| "too many input files")?);
        let id = *files.entry(path.to_owned()).or_insert(id);
        let mut span = def.span;
        span.file = id;
        let pragma = match &def.def {
            SurfaceDef::Pragma { name, arg } => {
                let (PragmaArg::Line(arg) | PragmaArg::Structured(arg)) = arg;
                if name == "option" {
                    apply_library_option(opts, arg)?;
                }
                Some((name.clone(), arg.clone()))
            }
            _ => None,
        };
        let declaration = matches!(&def.def,
            SurfaceDef::Type { .. } | SurfaceDef::Fixity { .. });
        let record = Record {
            path: path.into(),
            span,
            pragma,
            declaration,
            source: source
                .get(span.start as usize..span.end as usize)
                .ok_or("invalid definition source span")?
                .into(),
        };
        serde_json::to_writer(&mut manifest, &record).map_err(|e| e.to_string())?;
        manifest.write_all(b"\n").map_err(|e| e.to_string())?;
        // OCaml `generate_enum_number_conversions`: every enum gets
        // `<E>_of_num` and `num_of_<E>` unless suppressed or already declared.
        match &def.def {
            SurfaceDef::Val { name, .. } => {
                declared_vals.insert(name.clone());
            }
            SurfaceDef::Type { name, keyword, body_src, .. } if keyword == "enum" => {
                let prefix = source.get(..span.start as usize).unwrap_or("");
                let attributes = prefix.rsplit(['}', ';', '\n']).next().unwrap_or("");
                let suppressed = record.source.contains("no_enum_number_conversions")
                    || attributes.contains("no_enum_number_conversions");
                if !suppressed {
                    for text in enum_number_conversions(name, body_src, &declared_vals) {
                        let synthetic = Record {
                            path: record.path.clone(),
                            span: record.span,
                            pragma: None,
                            declaration: false,
                            source: text,
                        };
                        serde_json::to_writer(&mut manifest, &synthetic).map_err(|e| e.to_string())?;
                        manifest.write_all(b"\n").map_err(|e| e.to_string())?;
                    }
                }
            }
            SurfaceDef::Default { kind, value } if kind == "Order" => {
                default_inc = value == "inc";
            }
            SurfaceDef::Type { name, keyword, head_src, body_src } if keyword == "bitfield" => {
                for text in bitfield_accessors(name, head_src, body_src, default_inc)? {
                    let synthetic = Record {
                        path: record.path.clone(),
                        span: record.span,
                        pragma: None,
                        declaration: false,
                        source: text,
                    };
                    serde_json::to_writer(&mut manifest, &synthetic).map_err(|e| e.to_string())?;
                    manifest.write_all(b"\n").map_err(|e| e.to_string())?;
                }
            }
            _ => {}
        }
        Ok(())
    };
    if !no_core {
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
    for input in &inputs {
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
        context: ParserContext::default(),
        trace: opts.verbose >= 2,
    };
    let warm = WarmLayer::open(scratch.0.join("ast")).with_max_live_bytes(0);
    let scheduler = Scheduler::new(SchedulerConfig::new(2, workers));
    let mut resolver = Resolver::new();
    let mut definitions = 0;
    let mut ast_bytes = 0;
    // OOC-Fenster (Lane 12): der geordnete Consumer materialisiert höchstens
    // eine Definition gleichzeitig; nur die Blob-Spitze wird beobachtet.
    let mut window_peak_bytes = 0;
    // Descriptors also stay on disk. Later compiler phases can stream the
    // (AST, local interner) blobs without loading previous bodies into RAM.
    let mut snapshots = BufWriter::new(
        File::create(scratch.0.join("ast.jsonl")).map_err(|e| CliError::fatal(e.to_string()))?,
    );
    scheduler.run_streaming(
        jobs,
        |job| {
            let Job { record, mut parser, parsed } = job?;
            let def = if let Some(def) = parsed { def }
            else if let Some((key, payload)) = record.pragma {
                Node::new(NodeId::DUMMY, record.span, DefKind::Pragma { key, payload })
            } else {
                if opts.verbose >= 2 { eprintln!("sail-rs: convert {}:{}", record.path, record.span); }
                parser
                    .parse_def(&record.source, record.span)
                    .map_err(|e| error(format!("{}: {e}", record.path)))?
            };
            let interner = parser.into_interner();
            let declaration = matches!(
                &def.kind,
                DefKind::Type(_)
                    | DefKind::Val(_)
                    | DefKind::Let { .. }
                    | DefKind::Register(_)
                    | DefKind::Constraint(_)
                    | DefKind::Outcome { .. }
                    | DefKind::Overload { .. }
                    | DefKind::Default(_)
                    | DefKind::Scattered(_)
                    | DefKind::Impl(_)
                    | DefKind::Measure { .. }
                    | DefKind::LoopMeasures { .. }
                    | DefKind::Instantiation { .. }
                    | DefKind::Mapdef { .. }
                    | DefKind::Fundef {
                        annot: sail_ast::TAnnotOpt::Some { .. },
                        ..
                    }
            );
            if opts.verbose >= 2 { eprintln!("sail-rs: spool {}:{}", record.path, record.span); }
            let snapshot = DefinitionSnapshot::new(def);
            let (id, bytes) = store_snapshot(&warm, &snapshot, &interner)?;
            // Queued results hold disk references, never completed AST bodies.
            Ok::<_, CliError>((id, bytes, declaration))
        },
        |_, result| {
            let (id, bytes, declaration) = result?;
            let (def, interner) = load_definition(&warm, &id)?;
            resolver.add_definition(&def, &interner);
            drop_definition(def);
            serde_json::to_writer(&mut snapshots, &Descriptor { id: id.to_hex(), declaration })
                .map_err(|e| CliError::fatal(e.to_string()))?;
            snapshots.write_all(b"\n").map_err(|e| CliError::fatal(e.to_string()))?;
            definitions += 1;
            ast_bytes += bytes;
            window_peak_bytes = window_peak_bytes.max(bytes);
            Ok::<_, CliError>(())
        },
    )?;
    snapshots
        .flush()
        .map_err(|e| CliError::fatal(e.to_string()))?;
    let resolved = resolver.finish();
    for warning in resolved.warnings.iter().take(20) {
        eprintln!("sail-rs: warning: {warning}");
    }
    if resolved.warnings.len() > 20 {
        eprintln!("sail-rs: {} further resolution warnings", resolved.warnings.len() - 20);
    }
    if let Some(first) = resolved.errors.first() {
        if opts.verbose >= 2 {
            for diagnostic in resolved.errors.iter().skip(1).take(19) {
                eprintln!("sail-rs: name resolution: {diagnostic}");
            }
        }
        return Err(error(format!(
            "frontend name resolution: {first} ({} error(s))",
            resolved.errors.len()
        )));
    }
    let summary = FrontendSummary {
        files: files.len(),
        definitions,
        ast_bytes,
        workers,
        window_peak_defs: usize::from(definitions > 0),
        window_peak_bytes,
        warm_live_peak: warm.live_bytes(),
    };
    let mut source_paths = vec![String::new(); files.len()];
    for (path, id) in files { source_paths[id.0 as usize] = path; }
    Ok(FrontendProgram { summary, warm, manifest: scratch.0.join("ast.jsonl"), source_paths, _scratch: scratch })
}

/// Remove comments and `$[..]` attributes from raw definition text.
fn strip_noise(body: &str) -> String {
    let mut text = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('*')) => {
                chars.next();
                let mut previous = ' ';
                for c in chars.by_ref() {
                    if previous == '*' && c == '/' {
                        break;
                    }
                    previous = c;
                }
                text.push(' ');
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                text.push(' ');
            }
            ('$', Some('[')) => {
                let mut depth = 0;
                for c in chars.by_ref() {
                    match c {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                text.push(' ');
            }
            _ => text.push(c),
        }
    }
    text
}

/// Member names of an enum body (`{A, B}`, `A | B`, `{A -> 1, B -> 2}`),
/// ignoring comments and attributes.
fn enum_members(body: &str) -> Vec<String> {
    let text = strip_noise(body);
    let text = text.trim();
    let text = text.strip_prefix('{').and_then(|t| t.rsplit_once('}').map(|(inner, _)| inner)).unwrap_or(text);
    text.split([',', '|'])
        .filter_map(|part| {
            let part = part.split("->").next().unwrap_or("").split('=').next().unwrap_or("").trim();
            let ident: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '\'')
                .collect();
            (!ident.is_empty()).then_some(ident)
        })
        .collect()
}

/// A bitfield index range part: `hi .. lo` or a single index.
fn bit_range(text: &str) -> Option<(String, String)> {
    match text.split_once("..") {
        Some((hi, lo)) => Some((hi.trim().to_owned(), lo.trim().to_owned())),
        None => {
            let index = text.trim();
            (!index.is_empty()).then(|| (index.to_owned(), index.to_owned()))
        }
    }
}

fn slice_width(hi: &str, lo: &str) -> String {
    match (hi.parse::<i64>(), lo.parse::<i64>()) {
        (Ok(hi), Ok(lo)) => (hi - lo + 1).to_string(),
        _ => format!("(({hi}) - ({lo}) + 1)"),
    }
}

/// Source text of the definitions OCaml's `Bitfield.macro` generates for a
/// `bitfield T : bits(N) = { f : hi..lo, .. }`: constructor, and per field a
/// getter, updater, register setter and the `_mod_`/`update_` overloads.
fn bitfield_accessors(name: &str, head: &str, body: &str, inc: bool) -> Result<Vec<String>, String> {
    let typ = head.split_once(':').map(|(_, typ)| typ.trim()).ok_or("bitfield without type")?;
    let size = typ
        .strip_prefix("bits(")
        .or_else(|| typ.strip_prefix("bitvector("))
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| format!("bitfield type {typ} is not a constant-width bitvector"))?
        .trim()
        .to_owned();
    let text = strip_noise(body);
    let inner = text
        .trim()
        .strip_prefix('{')
        .and_then(|t| t.rsplit_once('}').map(|(inner, _)| inner))
        .ok_or("bitfield body without braces")?;
    let mut fields: Vec<(String, Vec<(String, String)>)> = vec![(
        "bits".to_owned(),
        vec![(format!("(({size}) - 1)"), "0".to_owned())],
    )];
    if let Ok(size) = size.parse::<i64>() {
        fields[0].1[0].0 = (size - 1).to_string();
    }
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in inner.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (field, ranges) = part.split_once(':').ok_or_else(|| format!("bitfield field {part}"))?;
        let ranges = ranges.trim();
        let ungrouped = ranges.replace(['(', ')'], "");
        let ranges = if ungrouped
            .split(['@', '.'])
            .filter(|piece| !piece.trim().is_empty())
            .all(|piece| piece.trim().parse::<i64>().is_ok())
        {
            ungrouped.as_str()
        } else {
            ranges
        };
        let ranges: Vec<(String, String)> = ranges
            .split('@')
            .map(bit_range)
            .collect::<Option<_>>()
            .ok_or_else(|| format!("bitfield range {part}"))?;
        fields.push((field.trim().to_owned(), ranges));
    }
    let mut out = vec![
        format!("val Mk_{name} : bits({size}) -> {name}"),
        format!("function Mk_{name} v = struct {{ bits = v }}"),
    ];
    for (field, ranges) in fields {
        let widths: Vec<String> = ranges.iter().map(|(hi, lo)| slice_width(hi, lo)).collect();
        let width = if widths.iter().all(|w| w.parse::<i64>().is_ok()) {
            widths.iter().map(|w| w.parse::<i64>().unwrap()).sum::<i64>().to_string()
        } else {
            format!("({})", widths.join(" + "))
        };
        let get_id = format!("_get_{name}_{field}");
        let update_id = format!("_update_{name}_{field}");
        let set_id = format!("_set_{name}_{field}");
        let get_body = ranges
            .iter()
            .map(|(hi, lo)| format!("v.bits[{hi} .. {lo}]"))
            .collect::<Vec<_>>()
            .join(" @ ");
        let mut update = "v.bits".to_owned();
        if ranges.len() == 1 {
            let (hi, lo) = &ranges[0];
            update = format!("[{update} with {hi} .. {lo} = x]");
        } else {
            let total: i64 = widths
                .iter()
                .map(|w| w.parse::<i64>())
                .sum::<Result<i64, _>>()
                .map_err(|_| format!("bitfield concat field {field} needs constant widths"))?;
            let mut at = if inc { 0 } else { total - 1 };
            for ((hi, lo), width) in ranges.iter().zip(&widths) {
                let width: i64 = width.parse().unwrap();
                let next = if inc { at + width } else { at - width };
                let (first, last) = if inc { (at, next - 1) } else { (at, next + 1) };
                update = format!("[{update} with {hi} .. {lo} = x[{first} .. {last}]]");
                at = next;
            }
        }
        out.push(format!("val {get_id} : {name} -> bits({width})"));
        out.push(format!("function {get_id} v = {get_body}"));
        out.push(format!("val {update_id} : ({name}, bits({width})) -> {name}"));
        out.push(format!("function {update_id} (v, x) = {{ v with bits = {update} }}"));
        out.push(format!("overload update_{field} = {{{update_id}}}"));
        out.push(format!("val {set_id} : (register({name}), bits({width})) -> unit"));
        out.push(format!(
            "function {set_id} (r_ref, v) = {{ let r = __deref(r_ref); (*r_ref) = {update_id}(r, v) }}"
        ));
        out.push(format!("overload _mod_{field} = {{{get_id}, {set_id}}}"));
    }
    Ok(out)
}

/// Source text of the generated enum/number conversion definitions.
fn enum_number_conversions(
    name: &str,
    body: &str,
    declared: &std::collections::HashSet<String>,
) -> Vec<String> {
    let members = enum_members(body);
    if members.is_empty() {
        return Vec::new();
    }
    let last = members.len() - 1;
    let to_enum = format!("{name}_of_num");
    let from_enum = format!("num_of_{name}");
    let mut out = Vec::new();
    if !declared.contains(&to_enum) {
        let arms: Vec<String> = members
            .iter()
            .enumerate()
            .map(|(n, member)| {
                if n == last { format!("_ => {member}") } else { format!("{n} => {member}") }
            })
            .collect();
        out.push(format!(
            "val {to_enum} : forall 'e, 0 <= 'e <= {last}. int('e) -> {name}"
        ));
        out.push(format!(
            "function {to_enum} arg_ = match arg_ {{ {} }}",
            arms.join(", ")
        ));
    }
    if !declared.contains(&from_enum) {
        let arms: Vec<String> = members
            .iter()
            .enumerate()
            .map(|(n, member)| format!("{member} => {n}"))
            .collect();
        out.push(format!(
            "val {from_enum} : {name} -> {{'e, 0 <= 'e <= {last}. int('e)}}"
        ));
        out.push(format!(
            "function {from_enum} arg_ = match arg_ {{ {} }}",
            arms.join(", ")
        ));
    }
    out
}
