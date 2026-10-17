//! Lean backend driver: whole-program tables, per-definition printing and
//! the project layout (`sail_plugin_lean.ml: output`).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use sail_ast::{Def, DefKind, SimpleInterner};
use sail_lake::{InterfaceVersion, LakeOptions};
use sail_leangen::ctx::{Ctx, Global, Oracle};
use sail_leangen::doc::{brk, chr, empty, hardline, separate, string, Doc};
use sail_leangen::info::GlobalInfo;
use sail_leangen::names::name_text;
use sail_leangen::pp_def::{doc_funcl, doc_typdef, doc_val};
use sail_leangen::pp_typ::doc_typ;
use sail_ty::program::Program;

use crate::args::{CliError, Options, EXIT_ERROR};
use crate::frontend::{load_definition, FrontendProgram};
use crate::typecheck::descriptors;

const DEFAULT_WIDTH: usize = 100;

fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}

/// `Env` queries answered from the whole-program tables and the solver.
struct BackendOracle<'a> {
    info: &'a GlobalInfo,
    program: &'a Program,
}

impl Oracle for BackendOracle<'_> {
    fn is_register(&self, name: &str) -> bool {
        self.info.is_register(name)
    }
    fn is_enum_member(&self, name: &str) -> bool {
        self.info.enum_members.contains(name)
    }
    fn is_union_ctor(&self, name: &str) -> bool {
        self.info.union_ctors.contains_key(name)
    }
    fn lean_extern(&self, name: &str) -> Option<String> {
        self.info.vals.get(name).and_then(|v| v.lean_extern.clone())
    }
    fn val_spec(&self, name: &str) -> Option<(sail_ast::TypSchm, Arc<SimpleInterner>)> {
        if let Some(v) = self.info.vals.get(name) {
            return Some((v.scheme.clone(), v.it.clone()));
        }
        // Functions with an inline signature declare their own scheme.
        self.program
            .value(name, usize::MAX - 1)
            .map(|scheme| (scheme.typschm.clone(), scheme.interner.clone()))
    }
    fn typ_head(&self, typ: &sail_ast::Typ, it: &SimpleInterner) -> Option<String> {
        self.program.typ_head(typ, it)
    }
    fn function_is_pure(&self, name: &str) -> bool {
        self.info.function_is_pure(name)
    }
    fn fun_args(&self, name: &str) -> Option<Vec<String>> {
        self.info.fun_args.get(name).cloned()
    }
    fn value_named(&self, name: &str) -> bool {
        self.info.defined_functions.contains(name)
            || self.info.is_register(name)
            || self.info.enum_members.contains(name)
            || self.info.union_ctors.contains_key(name)
            || self.info.global_lets.contains(name)
    }
    fn ctor_rename(&self, ctor: &str) -> Option<String> {
        self.info.ctor_rename(ctor).cloned()
    }
    fn provably_nneg(
        &self,
        quant: &[sail_ast::QuantItem],
        quant_it: &SimpleInterner,
        nexp: &sail_ast::NExp,
        nexp_it: &SimpleInterner,
    ) -> bool {
        self.program.prove_nonneg(quant, quant_it, nexp, nexp_it)
    }
}

fn is_file_pragma(def: &Def) -> bool {
    matches!(&def.kind, DefKind::Pragma { key, .. }
        if matches!(key.as_str(), "file_start" | "file_end" | "include_start" | "include_end"))
}

/// `doc_monad_abbrev ~version:1`.
fn doc_monad_abbrev(has_exception_type: bool) -> Doc {
    let exc = if has_exception_type {
        empty()
    } else {
        string("abbrev exception := Unit\n")
    };
    let monad = separate(
        &sail_leangen::doc::space(),
        [
            string("abbrev"),
            string("SailM"),
            string(":="),
            string("PreSailM RegisterType trivialChoiceSource exception"),
        ],
    );
    let monad_e = separate(
        &sail_leangen::doc::space(),
        [
            string("abbrev"),
            string("SailME"),
            string(":="),
            string("PreSailME RegisterType trivialChoiceSource exception"),
        ],
    )
    .cat(hardline())
    .cat(hardline());
    let docs: Vec<Doc> = [exc, monad, monad_e].into_iter().filter(|d| !d.is_empty()).collect();
    separate(&hardline(), docs)
}

fn camel(name: &str) -> String {
    sail_lake::to_upper_camel_case(name)
}

/// Run the Lean backend over the checked program.
pub fn run(
    frontend: &FrontendProgram,
    program: &Program,
    opts: &Options,
) -> Result<(), CliError> {
    let out_name = opts.output_prefix.clone().unwrap_or_else(|| "out".to_owned());
    let out_camel = camel(&out_name);
    let width = opts.lean_line_width.map_or(DEFAULT_WIDTH, |w| w as usize);

    // Pass 1: whole-program tables.
    let mut info = GlobalInfo::new();
    let mut has_exception_type = false;
    let mut registers: Vec<sail_leangen::synth::RegDecl> = Vec::new();
    for (pos, descriptor) in descriptors(frontend)?.enumerate() {
        let descriptor = descriptor?;
        let content = sail_ooc::ContentId::from_hex(&descriptor.id)
            .ok_or_else(|| error("invalid AST content ID"))?;
        let (mut def, interner) = load_definition(&frontend.warm, &content)?;
        let interner = Arc::new(interner);
        if let DefKind::Type(td) = &def.kind {
            if let sail_ast::TypeDefKind::Enum { name, .. }
            | sail_ast::TypeDefKind::Variant { name, .. }
            | sail_ast::TypeDefKind::Record { name, .. }
            | sail_ast::TypeDefKind::Abbrev { name, .. } = &td.kind
            {
                if name_text(name, &interner) == "exception" {
                    has_exception_type = true;
                }
            }
        }
        info.observe(&def, &interner);
        if matches!(def.kind, DefKind::Register(_)) {
            let has_init = matches!(&def.kind, DefKind::Register(spec) if spec.init.is_some());
            let init_it = if has_init {
                program
                    .elaborate_definition(&mut def, interner.clone(), pos)
                    .map_err(|e| error(format!("sail-rs: lean backend: register initialiser: {e:?}")))?
            } else {
                (*interner).clone()
            };
            if let DefKind::Register(spec) = &mut def.kind {
                registers.push(sail_leangen::synth::RegDecl {
                    name: name_text(&spec.name, &init_it),
                    typ: spec.typ.clone(),
                    it: Arc::new(init_it),
                    init: spec.init.take(),
                });
            }
            sail_ast::storage::drop_definition(def);
        } else {
            sail_ast::storage::drop_definition(def);
        }
    }
    let extras = sail_leangen::synth::init_defs(&registers);
    for (def, it) in &extras {
        info.observe(def, &Arc::new(it.clone()));
    }
    info.finish();

    let oracle = BackendOracle { info: &info, program };
    let mut global = Global::new(&oracle, &out_camel);
    global.extern_types = opts.lean_extern_types.clone();
    global.non_beq_types = opts.lean_non_beq_types.clone();
    global.partial_functions = opts.lean_partial_functions.clone();
    global.noncomputable_functions = opts.lean_noncomputable_functions.clone();

    // Pass 2: elaborate, rewrite, print.
    let mut types_text = String::new();
    let mut funcs_text = String::new();
    for (position, descriptor) in descriptors(frontend)?.enumerate() {
        let descriptor = descriptor?;
        let content = sail_ooc::ContentId::from_hex(&descriptor.id)
            .ok_or_else(|| error("invalid AST content ID"))?;
        let (mut def, interner) = load_definition(&frontend.warm, &content)?;
        let interner = Arc::new(interner);
        let result = prepare_def(&oracle, program, &mut def, interner, Some((position, &descriptor))).and_then(|it| {
            print_def(&mut global, &def, &it, width, &mut types_text, &mut funcs_text)
        });
        sail_ast::storage::drop_definition(def);
        result.map_err(|m| error(format!("sail-rs: lean backend: {m}")))?;
    }
    for (mut def, it) in extras {
        let result = prepare_def(&oracle, program, &mut def, Arc::new(it), None).and_then(|it| {
            print_def(&mut global, &def, &it, width, &mut types_text, &mut funcs_text)
        });
        result.map_err(|m| error(format!("sail-rs: lean backend: {m}")))?;
    }

    let registers_doc = if info.registers.is_empty() {
        "abbrev Register := PEmpty\nabbrev RegisterType : Register -> Type := PEmpty.elim\n\n".to_owned()
    } else {
        return Err(error("sail-rs: lean backend: registers are not supported yet"));
    };

    let mut options = LakeOptions::new(out_name.clone());
    options.noncomputable = opts.lean_noncomputable;
    options.real_numbers = opts.lean_real_numbers;
    options.executable = opts.lean_executable;
    options.lib_path = opts.lean_lib_path.clone();
    options.lib_git = opts.lean_lib_repo.clone();
    options.lib_rev = opts.lean_lib_rev.clone();
    options.interface = InterfaceVersion::V1;

    let instantiations = separate(
        &hardline(),
        [empty(), empty(), doc_monad_abbrev(has_exception_type), empty(), empty()],
    );
    let mut defs_file = sail_lake::render_defs_header(&options);
    defs_file.push_str(&types_text);
    defs_file.push_str(&registers_doc);
    defs_file.push_str(&instantiations.render(width));

    // `open` lines: types opened by their definitions (descending order).
    let mut opens = String::new();
    for id in global.opens.borrow().iter().rev() {
        opens.push_str(&format!("open {id}\n"));
    }
    if !opens.is_empty() {
        opens.push('\n');
    }
    let extra_imports = vec!["SpecializationV1".to_owned(), "FakeReal".to_owned()];
    let mut funcs_file = sail_lake::render_function_file_prelude(&options, &[], &extra_imports);
    funcs_file.push_str(&opens);
    funcs_file.push_str(&funcs_text);
    funcs_file.push_str(&format!("end {out_camel}.Functions\n"));

    write_project(opts, &options, &out_name, &out_camel, &defs_file, &funcs_file)
}

/// Elaborate (types on nodes) and rewrite one definition; returns its final
/// interner.
fn prepare_def(
    oracle: &BackendOracle<'_>,
    program: &Program,
    def: &mut Def,
    interner: Arc<SimpleInterner>,
    source: Option<(usize, &crate::frontend::Descriptor)>,
) -> Result<SimpleInterner, String> {
    let bodies = matches!(def.kind, DefKind::Fundef { .. } | DefKind::Let { .. });
    if !bodies {
        return Ok((*interner).clone());
    }
    let it = match source {
        Some((position, _)) => program
            .elaborate_definition(def, interner.clone(), position)
            .map_err(|e| format!("elaboration failed: {e:?}"))?,
        _ => (*interner).clone(),
    };
    if std::env::var_os("SAIL_RS_DEBUG_DEF").is_some() {
        eprintln!("elaborated:\n{}", sail_leangen::sailpp::def_to_string(&it, def));
    }
    let mut rw = sail_leangen::rw::Rw::new(it, oracle);
    let ret = match &def.kind {
        DefKind::Fundef { funcls, .. } => funcls.first().and_then(|f| {
            let name = name_text(&f.name, &rw.it);
            let (schm, from) = oracle.val_spec(&name)?;
            match &schm.typ.kind {
                sail_ast::TypKind::Fn { ret, .. } => {
                    Some(sail_ty::constraint_synonym::import_typ(ret, &from, &mut rw.it))
                }
                _ => None,
            }
        }),
        _ => None,
    };
    sail_leangen::rw::pipeline::rewrite_def(&mut rw, ret.as_ref(), def)?;
    if std::env::var_os("SAIL_RS_DEBUG_DEF").is_some() {
        eprintln!("rewritten:\n{}", sail_leangen::sailpp::def_to_string(&rw.it, def));
    }
    Ok(rw.it)
}

fn print_def(
    global: &mut Global<'_>,
    def: &Def,
    it: &SimpleInterner,
    width: usize,
    types_text: &mut String,
    funcs_text: &mut String,
) -> Result<(), String> {
    let oracle = global.oracle;
    match &def.kind {
        DefKind::Type(td) => {
            let name = match &td.kind {
                sail_ast::TypeDefKind::Enum { name, .. }
                | sail_ast::TypeDefKind::Variant { name, .. }
                | sail_ast::TypeDefKind::Record { name, .. }
                | sail_ast::TypeDefKind::Abbrev { name, .. }
                | sail_ast::TypeDefKind::Abstract { name, .. }
                | sail_ast::TypeDefKind::Bitfield { name, .. } => name_text(name, it),
            };
            if global.extern_types.contains(&name) {
                return Ok(());
            }
            let ctx = Ctx::new(global, it);
            let doc = doc_typdef(&ctx, it, td)?;
            types_text.push_str(&doc.group().cat(brk(1)).cat(hardline()).render(width));
        }
        DefKind::Fundef { rec, funcls, .. } => {
            let [funcl] = funcls.as_slice() else {
                return Err(format!(
                    "function {} still has {} clauses after rewriting",
                    funcls.first().map_or_else(String::new, |f| name_text(&f.name, it)),
                    funcls.len()
                ));
            };
            let name = name_text(&funcl.name, it);
            if oracle.lean_extern(&name).is_some() {
                return Ok(());
            }
            let spec = oracle
                .val_spec(&name)
                .ok_or_else(|| format!("function {name} has no val specification"))?;
            let doc = doc_funcl(global, it, rec, funcl, &spec)?;
            funcs_text.push_str(&doc.group().cat(brk(1)).cat(hardline()).render(width));
        }
        DefKind::Let { pat, exp } => {
            let (rename, doc) = {
                let ctx = Ctx::new(global, it);
                doc_val(&ctx, it, pat, exp)?
            };
            if let Some((id, kid)) = rename {
                global.add_kid_id_rename(&id, &kid);
            }
            funcs_text.push_str(&doc.group().cat(brk(1)).cat(hardline()).render(width));
        }
        _ => {}
    }
    let _ = (chr, doc_typ, BTreeSet::<String>::new);
    Ok(())
}

fn write_project(
    opts: &Options,
    options: &LakeOptions,
    out_name: &str,
    out_camel: &str,
    defs_file: &str,
    funcs_file: &str,
) -> Result<(), CliError> {
    let base = opts.lean_output_dir.clone().unwrap_or_else(|| ".".to_owned());
    let project = Path::new(&base).join(out_name);
    if opts.lean_force_output && project.is_dir() {
        for entry in fs::read_dir(&project).map_err(|e| CliError::fatal(e.to_string()))? {
            let path = entry.map_err(|e| CliError::fatal(e.to_string()))?.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| CliError::fatal(e.to_string()))?;
            } else {
                fs::remove_file(&path).map_err(|e| CliError::fatal(e.to_string()))?;
            }
        }
    }
    let src = project.join(out_camel);
    fs::create_dir_all(&src).map_err(|e| CliError::fatal(e.to_string()))?;
    let write = |path: std::path::PathBuf, text: String| {
        fs::write(&path, text).map_err(|e| CliError::fatal(format!("{}: {e}", path.display())))
    };
    write(project.join(".gitignore"), sail_lake::render_gitignore())?;
    write(project.join("lean-toolchain"), sail_lake::render_lean_toolchain(options))?;
    write(
        project.join("lakefile.toml"),
        sail_lake::render_lakefile(options).map_err(error)?,
    )?;
    write(project.join("lake-manifest.json"), sail_lake::render_lake_manifest(options))?;
    write(src.join("Defs.lean"), defs_file.to_owned())?;
    write(project.join(format!("{out_camel}.lean")), funcs_file.to_owned())?;
    for (file, text) in [
        ("SpecializationV1.lean", sail_leangen::lean_support::SPECIALIZATION_V1),
        ("FakeReal.lean", sail_leangen::lean_support::FAKE_REAL),
    ] {
        write(src.join(file), text.replace("THE_MODULE_NAME", out_camel))?;
    }
    Ok(())
}
