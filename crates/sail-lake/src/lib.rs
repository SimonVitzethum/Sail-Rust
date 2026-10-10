//! Lake project generator for the Sail Lean backend.
//!
//! Pure-Rust port of `start_lean_output` / `output_support_lib` /
//! `create_lake_project` from
//! `sail/src/sail_lean_backend/sail_plugin_lean.ml`.
//!
//! The generator only renders text: it never touches the filesystem
//! itself (callers join [`LakeProject::layout`] paths onto their output
//! directory and write the files). All rendering is total, allocation
//! only via [`String`], no `unsafe`.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt::Write as _;

/// Default Lean toolchain pinned exactly like the OCaml backend
/// (`lean_version = "lean4:v4.29.0"`, written as `leanprover/<version>`).
pub const DEFAULT_LEAN_TOOLCHAIN_VERSION: &str = "lean4:v4.29.0";

/// Default `lean-sail` support-library revision (`lib_default_rev = "v6"`).
pub const DEFAULT_LEAN_SAIL_REV: &str = "v6";

/// Default `lean-sail` git URL (`lib_default_git`).
pub const DEFAULT_LEAN_SAIL_GIT: &str = "https://github.com/rems-project/lean-sail";

/// Pinned mathlib revision, only emitted when `real_numbers` is set
/// (`mathlib_version = "v4.29.0"`).
pub const DEFAULT_MATHLIB_REV: &str = "v4.29.0";

/// mathlib git URL used by the OCaml backend.
pub const MATHLIB_GIT: &str = "https://github.com/leanprover-community/mathlib4-nightly-testing";

/// `decreasing_trivial` must only ever come from this module; the
/// generator never emits `sorry` (see [`NO_SORRY_NOTE`]).
pub const ARM_EXTRAS_MODULE: &str = "Sail.ArmExtras";

/// Marker documenting the no-`sorry` policy of this crate.
pub const NO_SORRY_NOTE: &str = "sail-lake never emits `sorry`; `decreasing_trivial` is only used via `Sail.ArmExtras`";

/// Concurrency interface version selecting the `Sail` interface module
/// and the `Specialization*` file, mirroring
/// `Preprocess.have_symbol "CONCURRENCY_INTERFACE_V2"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InterfaceVersion {
    /// `ConcurrencyInterfaceV1` + `SpecializationV1.lean`.
    #[default]
    V1,
    /// `ArchSem` + `SpecializationArchSem.lean`.
    V2,
}

impl InterfaceVersion {
    /// Lean module inside `Sail.*` implementing the interface.
    #[must_use]
    pub fn module_name(self) -> &'static str {
        match self {
            InterfaceVersion::V1 => "ConcurrencyInterfaceV1",
            InterfaceVersion::V2 => "ArchSem",
        }
    }

    /// Static-library file providing the `Specialization` shim.
    #[must_use]
    pub fn specialization_file(self) -> &'static str {
        match self {
            InterfaceVersion::V1 => "SpecializationV1.lean",
            InterfaceVersion::V2 => "SpecializationArchSem.lean",
        }
    }
}

/// Options controlling Lake project rendering.
///
/// Field defaults mirror the OCaml flag defaults: no real numbers, no
/// executable, support library from git at [`DEFAULT_LEAN_SAIL_REV`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LakeOptions {
    /// Raw (lowercase/underscore) project name, e.g. `armv9`.
    pub out_name: String,
    /// Concurrency interface version.
    pub interface: InterfaceVersion,
    /// `--lean-real-numbers`: pull mathlib into `lakefile.toml`.
    pub real_numbers: bool,
    /// `--lean-executable` (only effective when a `main` exists).
    pub executable: bool,
    /// `--lean-noncomputable`: prefix every prelude with
    /// `noncomputable section`.
    pub noncomputable: bool,
    /// Pinned toolchain *without* the `leanprover/` prefix.
    pub toolchain_version: String,
    /// mathlib rev used when [`LakeOptions::real_numbers`] is set.
    pub mathlib_rev: String,
    /// `--lean-lib-repo`: overrides [`DEFAULT_LEAN_SAIL_GIT`].
    pub lib_git: Option<String>,
    /// `--lean-lib-path`: local path dependency (mutually exclusive
    /// with `lib_git`).
    pub lib_path: Option<String>,
    /// `--lean-lib-rev`: overrides [`DEFAULT_LEAN_SAIL_REV`].
    pub lib_rev: Option<String>,
    /// Additionally `import Sail.ArmExtras` in function files so that
    /// `decreasing_trivial` resolves from there. Off by default to stay
    /// byte-compatible with the OCaml backend.
    pub with_arm_extras: bool,
}

impl Default for LakeOptions {
    fn default() -> Self {
        Self {
            out_name: "out".to_string(),
            interface: InterfaceVersion::V1,
            real_numbers: false,
            executable: false,
            noncomputable: false,
            toolchain_version: DEFAULT_LEAN_TOOLCHAIN_VERSION.to_string(),
            mathlib_rev: DEFAULT_MATHLIB_REV.to_string(),
            lib_git: None,
            lib_path: None,
            lib_rev: None,
            with_arm_extras: false,
        }
    }
}

impl LakeOptions {
    /// New options for project `out_name` with OCaml-compatible defaults.
    pub fn new(out_name: impl Into<String>) -> Self {
        Self {
            out_name: out_name.into(),
            ..Self::default()
        }
    }

    /// Validate mutually exclusive support-library sources.
    pub fn validate(&self) -> Result<(), String> {
        if self.lib_git.is_some() && self.lib_path.is_some() {
            return Err("--lean-lib-git and --lean-lib-path cannot be set simultaneously".to_string());
        }
        if self.out_name.is_empty() {
            return Err("out_name must not be empty".to_string());
        }
        Ok(())
    }

    /// `UpperCamelCase` project module, mirroring
    /// `Libsail.Util.to_upper_camel_case`.
    #[must_use]
    pub fn module_name(&self) -> String {
        to_upper_camel_case(&self.out_name)
    }

    /// Effective support-library revision.
    #[must_use]
    pub fn effective_lib_rev(&self) -> &str {
        self.lib_rev.as_deref().unwrap_or(DEFAULT_LEAN_SAIL_REV)
    }

    /// Effective support-library git URL.
    #[must_use]
    pub fn effective_lib_git(&self) -> &str {
        self.lib_git.as_deref().unwrap_or(DEFAULT_LEAN_SAIL_GIT)
    }
}

/// Convert `snake_case` / `kebab-case` / `lowercase` to `UpperCamelCase`,
/// mirroring `Libsail.Util.to_upper_camel_case`.
#[must_use]
pub fn to_upper_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == '.' || ch == ' ' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Contents of `lean-toolchain` (`leanprover/<version>`, no trailing newline,
/// exactly like the OCaml backend).
#[must_use]
pub fn render_lean_toolchain(options: &LakeOptions) -> String {
    format!("leanprover/{}", options.toolchain_version)
}

/// Contents of `.gitignore` (ignores Lake build artefacts).
#[must_use]
pub fn render_gitignore() -> String {
    "/.lake".to_string()
}

/// `file_prelude` from `sail_plugin_lean.ml`: heartbeat/recursion limits,
/// linter options, `open Sail[.Interface]`, optional `noncomputable
/// section`, and the target namespace.
#[must_use]
pub fn render_file_prelude(interface: InterfaceVersion, namespace: &str, noncomputable: bool) -> String {
    let noncomputable_line = if noncomputable { "noncomputable section\n" } else { "" };
    format!(
        "set_option maxHeartbeats 1_000_000_000\n\
         set_option maxRecDepth 1_000_000\n\
         set_option linter.unusedVariables false\n\
         set_option match.ignoreUnusedAlts true\n\
         \n\
         open Sail\n\
         open Sail.{iface}\n\
         \n\
         {nc}namespace {ns}\n\
         \n",
        iface = interface.module_name(),
        nc = noncomputable_line,
        ns = namespace,
    )
}

/// Header of `Defs.lean`: `import Sail`, `open PreSail`, then
/// [`render_file_prelude`].
#[must_use]
pub fn render_defs_header(options: &LakeOptions) -> String {
    let mut s = String::from("import Sail\nopen PreSail\n\n");
    s.push_str(&render_file_prelude(
        options.interface,
        &options.module_name(),
        options.noncomputable,
    ));
    s
}

/// Prelude of a function shard file (or the `<Camel>.lean` root file),
/// mirroring `print_function_file_prelude`.
///
/// `import_refs` lists sibling shard modules to import; when empty the
/// file imports `Sail`, `<Camel>.Defs`, plus `extra_imports` (e.g. the
/// copied `--lean-import-file` modules such as `Real`/`FakeReal` and the
/// `Specialization*` shim). When `with_arm_extras` is set,
/// `import Sail.ArmExtras` is appended so `decreasing_trivial` resolves
/// from there.
#[must_use]
pub fn render_function_file_prelude(
    options: &LakeOptions,
    import_refs: &[String],
    extra_imports: &[String],
) -> String {
    let camel = options.module_name();
    let mut s = String::with_capacity(256 + import_refs.len() * 32 + extra_imports.len() * 32);
    if import_refs.is_empty() {
        s.push_str("import Sail\n");
        let _ = writeln!(s, "import {camel}.Defs");
        for extra in extra_imports {
            let _ = writeln!(s, "import {camel}.{extra}");
        }
    } else {
        for n in import_refs {
            let _ = writeln!(s, "import {camel}.{n}");
        }
    }
    if options.with_arm_extras {
        let _ = writeln!(s, "import {ARM_EXTRAS_MODULE}");
    }
    s.push('\n');
    s.push_str(&render_file_prelude(options.interface, &camel, options.noncomputable));
    let _ = writeln!(s, "open {}\n", options.interface.module_name());
    s.push_str("namespace Functions\n\n");
    s
}

/// `[[require]]` block for the `Sail` support library, mirroring
/// `output_support_lib`. Returns `Err` when both `lib_git` and
/// `lib_path` are set.
pub fn render_support_lib_require(options: &LakeOptions) -> Result<String, String> {
    options.validate()?;
    let mut s = String::with_capacity(128);
    s.push_str("\n\n[[require]]\nname = \"Sail\"\n");
    match (&options.lib_git, &options.lib_path) {
        (Some(_), Some(_)) => {
            return Err("--lean-lib-git and --lean-lib-path cannot be set simultaneously".to_string());
        }
        (None, Some(path)) => {
            let _ = writeln!(s, "path = \"{path}\"");
        }
        (_, _) => {
            let _ = writeln!(
                s,
                "git = \"{}\"\nrev = \"{}\"",
                options.effective_lib_git(),
                options.effective_lib_rev()
            );
        }
    }
    Ok(s)
}

/// Contents of `lakefile.toml`, mirroring `create_lake_project`.
///
/// Layout: `name` / `defaultTargets` / `moreLeanArgs`, one `[[lean_lib]]`,
/// the weak name-check linter option, `moreLeancArgs`, the `Sail`
/// support-library require, optionally mathlib (only with
/// `real_numbers`), and optionally a `[[lean_exe]]` when `executable`.
pub fn render_lakefile(options: &LakeOptions) -> Result<String, String> {
    let camel = options.module_name();
    let mut s = format!(
        "name = \"{name}\"\ndefaultTargets = [\"{camel}\"]\nmoreLeanArgs = [\"--tstack=400000\"]\n\n[[lean_lib]]\nname = \"{camel}\"",
        name = options.out_name,
    );
    s.push_str("\nleanOptions.weak.linter.style.nameCheck = false");
    s.push_str("\nmoreLeancArgs= [\"-fbracket-depth=500\"]");
    s.push_str(&render_support_lib_require(options)?);
    if options.real_numbers {
        s.push_str("\n\n[[require]]\nname = \"mathlib\"\n");
        let _ = writeln!(s, "git = \"{MATHLIB_GIT}\"");
        let _ = writeln!(s, "rev = \"{}\"", options.mathlib_rev);
    }
    if options.executable {
        s.push_str("\n\n[[lean_exe]]\nname = \"run\"\n");
        let _ = writeln!(s, "root = \"{camel}\"");
    }
    Ok(s)
}

/// Contents of `lake-manifest.json`, mirroring `create_lake_project`.
#[must_use]
pub fn render_lake_manifest(options: &LakeOptions) -> String {
    format!(
        "{{\n \"version\": \"1.1.0\",\n \"packagesDir\": \".lake/packages\",\n \"packages\": [],\n \"name\": \"{name}\",\n \"lakeDir\": \".lake\"}}\n",
        name = options.out_name,
    )
}

/// A single register entry for the `Register` inductive template:
/// `(constructor_name, lean_type)`.
pub type RegisterEntry = (String, String);

/// `inductive Register` block, mirroring `register_enums`.
///
/// RAM-Diät (Lane 90): Zeilen per `write!` in einen vorreservierten
/// `String` statt `push_str(&format!(…))` pro Zeile (eine Allokation
/// statt zwei); Ausgabe identisch.
#[must_use]
pub fn render_register_inductive(registers: &[RegisterEntry]) -> String {
    let mut s = String::with_capacity(128 + registers.len() * 24);
    s.push_str("inductive Register : Type where\n");
    for (ctor, _) in registers {
        let _ = writeln!(s, "  | {ctor}");
    }
    s.push_str("  deriving DecidableEq, Hashable, Repr\nopen Register\n");
    s
}

/// `abbrev RegisterType` block, mirroring `type_enum`.
#[must_use]
pub fn render_register_type(registers: &[RegisterEntry]) -> String {
    let mut s = String::with_capacity(128 + registers.len() * 32);
    s.push_str("abbrev RegisterType : Register → Type\n");
    for (ctor, ty) in registers {
        let _ = writeln!(s, "  | .{ctor} => {ty}");
    }
    s
}

/// Empty-register fallback used when a model declares no registers.
#[must_use]
pub fn render_empty_registers() -> String {
    "abbrev Register := PEmpty\nabbrev RegisterType : Register -> Type := PEmpty.elim\n\n".to_string()
}

/// `Inhabited (RegisterRef ...)` instances, one per distinct register
/// type, mirroring `inhabit_enum_v1` / `inhabit_enum_v2`.
#[must_use]
pub fn render_register_inhabited(interface: InterfaceVersion, registers: &[RegisterEntry]) -> String {
    // Deduplicate by Lean type, keeping the first constructor per type.
    // `HashSet` statt O(n²)-Scan (`seen.iter().any(…)`); der `seen`-Vektor
    // hält die Erstauftretens-Reihenfolge, Ausgabe identisch.
    let mut seen_types: HashSet<&str> = HashSet::with_capacity(registers.len());
    let mut seen: Vec<(&str, &str)> = Vec::with_capacity(registers.len());
    for (ctor, ty) in registers {
        if seen_types.insert(ty.as_str()) {
            seen.push((ctor.as_str(), ty.as_str()));
        }
    }
    let mut s = String::with_capacity(seen.len() * 96);
    for (i, (ctor, ty)) in seen.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        match interface {
            InterfaceVersion::V1 => {
                let _ = write!(
                    s,
                    "instance : Inhabited (RegisterRef RegisterType {ty}) where\n  default := .Reg {ctor}"
                );
            }
            InterfaceVersion::V2 => {
                let _ = write!(
                    s,
                    "instance : Inhabited (RegisterRef {ty}) where\n  default := .Reg {ctor}"
                );
            }
        }
    }
    s
}

/// `SailM` / `SailME` monad abbreviations plus the `exception` fallback,
/// mirroring `doc_monad_abbrev`. `has_exception_type` is true when the
/// model defines its own `exception` type.
#[must_use]
pub fn render_monad_abbrev(interface: InterfaceVersion, has_exception_type: bool) -> String {
    let mut s = String::new();
    if !has_exception_type {
        s.push_str("abbrev exception := Unit\n");
    }
    let (m, me) = match interface {
        InterfaceVersion::V1 => (
            "PreSailM RegisterType trivialChoiceSource exception",
            "PreSailME RegisterType trivialChoiceSource exception",
        ),
        InterfaceVersion::V2 => ("PreSailM exception", "PreSailME exception"),
    };
    if !s.is_empty() {
        s.push('\n');
    }
    s.push_str(&format!("abbrev SailM := {m}\nabbrev SailME := {me}\n"));
    s
}

/// `instance : Arch` template for concurrency interface V1, mirroring
/// `doc_instantiations_v1`. Each `Some(ty)` fills the corresponding
/// field; `None` leaves the field out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchFieldsV1 {
    pub pa_type: Option<String>,
    pub abort_type: Option<String>,
    pub translation_summary_type: Option<String>,
    pub trans_start_type: Option<String>,
    pub trans_end_type: Option<String>,
    pub fault_type: Option<String>,
    pub tlbi_type: Option<String>,
    pub cache_op_type: Option<String>,
    pub barrier_type: Option<String>,
    pub arch_ak_type: Option<String>,
    pub sys_reg_id_type: Option<String>,
}

impl ArchFieldsV1 {
    /// Render the `instance : Arch` block; empty when no fields are set.
    #[must_use]
    pub fn render(&self) -> String {
        // Direkt in den Ziel-`String` schreiben (RAM-Diät Lane 90): kein
        // `Vec<String>` + `join` mehr; Ausgabe identisch.
        let mut s = String::with_capacity(256);
        for (k, v) in [
            ("pa", &self.pa_type),
            ("abort", &self.abort_type),
            ("translation", &self.translation_summary_type),
            ("trans_start", &self.trans_start_type),
            ("trans_end", &self.trans_end_type),
            ("fault", &self.fault_type),
            ("tlb_op", &self.tlbi_type),
            ("cache_op", &self.cache_op_type),
            ("barrier", &self.barrier_type),
            ("arch_ak", &self.arch_ak_type),
            ("sys_reg_id", &self.sys_reg_id_type),
        ] {
            if let Some(t) = v {
                let _ = writeln!(s, "  {k} := {t}");
            }
        }
        if s.is_empty() {
            return String::new();
        }
        format!("instance : Arch where\n{s}")
    }
}

/// `instance : Arch` template for concurrency interface V2
/// (`ArchSem`), mirroring `doc_instantiations_v2`. Unset type fields
/// fall back to the OCaml defaults (`Unit`, `64`, `false`, `0`,
/// `Register`/`RegisterType`, `fun _ => false` for predicates).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchFieldsV2 {
    pub addr_size: Option<String>,
    pub addr_space: Option<String>,
    pub cheri: Option<String>,
    pub cap_size_log: Option<String>,
    pub register: Option<String>,
    pub register_type: Option<String>,
    pub mem_acc: Option<String>,
    pub mem_acc_predicates: Vec<(String, Option<String>)>,
    pub trans_start: Option<String>,
    pub trans_end: Option<String>,
    pub abort: Option<String>,
    pub barrier: Option<String>,
    pub cache_op: Option<String>,
    pub tlbi: Option<String>,
    pub exn: Option<String>,
    pub sys_reg_id: Option<String>,
}

impl ArchFieldsV2 {
    /// Predicate names emitted by the OCaml backend, in order.
    pub fn default_predicate_names() -> &'static [&'static str] {
        &[
            "mem_acc_is_explicit",
            "mem_acc_is_ifetch",
            "mem_acc_is_ttw",
            "mem_acc_is_relaxed",
            "mem_acc_is_rel_acq_rcpc",
            "mem_acc_is_rel_acq_rcsc",
            "mem_acc_is_standalone",
            "mem_acc_is_exclusive",
            "mem_acc_is_atomic_rmw",
        ]
    }

    /// Render the `@[reducible] instance : Arch` block.
    #[must_use]
    pub fn render(&self) -> String {
        // `&str`-View statt `to_string()` pro Feld, alles per `write!` in
        // einen vorreservierten `String` (RAM-Diät Lane 90); Ausgabe identisch.
        fn ty<'a>(v: &'a Option<String>, d: &'a str) -> &'a str {
            v.as_deref().unwrap_or(d)
        }
        let mut s = String::with_capacity(1024);
        let _ = writeln!(s, "  addr_size := {}", ty(&self.addr_size, "64"));
        let _ = writeln!(s, "  addr_space := {}", ty(&self.addr_space, "Unit"));
        let _ = writeln!(s, "  CHERI := {}", ty(&self.cheri, "false"));
        let _ = writeln!(s, "  cap_size_log := {}", ty(&self.cap_size_log, "0"));
        let _ = writeln!(s, "  register := {}", ty(&self.register, "Register"));
        let _ = writeln!(
            s,
            "  register_type := {}",
            ty(&self.register_type, "RegisterType")
        );
        let _ = writeln!(s, "  mem_acc := {}", ty(&self.mem_acc, "Unit"));
        for name in Self::default_predicate_names() {
            let val = self
                .mem_acc_predicates
                .iter()
                .find(|(n, _)| n == name)
                .and_then(|(_, v)| v.as_deref())
                .unwrap_or("fun _ => false");
            let _ = writeln!(s, "  {name} := {val}");
        }
        for (k, v, d) in [
            ("trans_start", &self.trans_start, "Unit"),
            ("trans_end", &self.trans_end, "Unit"),
            ("abort", &self.abort, "Unit"),
            ("barrier", &self.barrier, "Unit"),
            ("cache_op", &self.cache_op, "Unit"),
            ("tlbi", &self.tlbi, "Unit"),
            ("exn", &self.exn, "Unit"),
            ("sys_reg_id", &self.sys_reg_id, "Unit"),
        ] {
            let _ = writeln!(s, "  {k} := {}", ty(v, d));
        }
        format!("@[reducible]\ninstance : Arch where\n{s}")
    }
}

/// One rendered file: project-relative path plus contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    /// Path relative to the project directory, using `/` separators.
    pub path: String,
    /// Full file contents.
    pub contents: String,
}

/// High-level project generator: options plus the shard layout and
/// register model needed to render every file.
#[derive(Debug, Clone)]
pub struct LakeProject {
    /// Global rendering options.
    pub options: LakeOptions,
    /// Extra shard modules (already `UpperCamelCase`) imported by the
    /// root file when there are no `import_refs`, e.g. copied
    /// `--lean-import-file` modules (`Real`/`FakeReal`,
    /// `Specialization*`).
    pub extra_imports: Vec<String>,
    /// Shard modules to `import` from the root `<Camel>.lean` file.
    pub root_import_refs: Vec<String>,
    /// Register model for the `Defs.lean` register/instance templates.
    pub registers: Vec<RegisterEntry>,
    /// Whether the model defines its own `exception` type.
    pub has_exception_type: bool,
    /// V1 arch fields (used when [`InterfaceVersion::V1`]).
    pub arch_v1: ArchFieldsV1,
    /// V2 arch fields (used when [`InterfaceVersion::V2`]).
    pub arch_v2: ArchFieldsV2,
}

impl LakeProject {
    /// New project with OCaml-compatible defaults for `out_name`.
    pub fn new(out_name: impl Into<String>) -> Self {
        Self {
            options: LakeOptions::new(out_name),
            extra_imports: Vec::new(),
            root_import_refs: Vec::new(),
            registers: Vec::new(),
            has_exception_type: false,
            arch_v1: ArchFieldsV1::default(),
            arch_v2: ArchFieldsV2::default(),
        }
    }

    /// Render the `Defs.lean` register/instance section for the
    /// configured interface version.
    #[must_use]
    pub fn render_register_section(&self) -> String {
        if self.registers.is_empty() {
            return render_empty_registers();
        }
        let mut s = render_register_inductive(&self.registers);
        s.push('\n');
        s.push_str(&render_register_type(&self.registers));
        s.push('\n');
        match self.options.interface {
            InterfaceVersion::V1 => {
                let arch = self.arch_v1.render();
                if !arch.is_empty() {
                    s.push_str(&arch);
                    s.push('\n');
                }
                s.push('\n');
                s.push_str(&render_monad_abbrev(InterfaceVersion::V1, self.has_exception_type));
                s.push_str("\n\n");
                s.push_str(&render_register_inhabited(InterfaceVersion::V1, &self.registers));
            }
            InterfaceVersion::V2 => {
                s.push_str(&self.arch_v2.render());
                s.push('\n');
                s.push_str(&render_monad_abbrev(InterfaceVersion::V2, self.has_exception_type));
                s.push_str("\n\n");
                s.push_str(&render_register_inhabited(InterfaceVersion::V2, &self.registers));
            }
        }
        s.push('\n');
        s
    }

    /// Render every file of the Lake project as `(path, contents)`
    /// pairs, mirroring the file layout created by `start_lean_output`:
    /// `.gitignore`, `lean-toolchain`, `lakefile.toml`,
    /// `lake-manifest.json`, `<Camel>/Defs.lean` (header + register
    /// templates), and the `<Camel>.lean` root file.
    pub fn layout(&self) -> Result<Vec<GeneratedFile>, String> {
        self.options.validate()?;
        let camel = self.options.module_name();
        let mut files = vec![
            GeneratedFile {
                path: ".gitignore".to_string(),
                contents: render_gitignore(),
            },
            GeneratedFile {
                path: "lean-toolchain".to_string(),
                contents: render_lean_toolchain(&self.options),
            },
            GeneratedFile {
                path: "lakefile.toml".to_string(),
                contents: render_lakefile(&self.options)?,
            },
            GeneratedFile {
                path: "lake-manifest.json".to_string(),
                contents: render_lake_manifest(&self.options),
            },
        ];
        let mut defs = render_defs_header(&self.options);
        defs.push_str(&self.render_register_section());
        files.push(GeneratedFile {
            path: format!("{camel}/Defs.lean"),
            contents: defs,
        });
        files.push(GeneratedFile {
            path: format!("{camel}.lean"),
            contents: render_function_file_prelude(&self.options, &self.root_import_refs, &self.extra_imports),
        });
        Ok(files)
    }

    /// Snapshot of the whole project layout as a single `String` for
    /// golden tests: `=== <path> ===` headers followed by contents.
    pub fn layout_snapshot(&self) -> Result<String, String> {
        let layout = self.layout()?;
        let mut snap =
            String::with_capacity(layout.iter().map(|f| f.path.len() + f.contents.len() + 16).sum());
        for f in &layout {
            let _ = writeln!(snap, "=== {} ===\n{}", f.path, f.contents);
        }
        Ok(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_project() -> LakeProject {
        let mut p = LakeProject::new("test_model");
        p.extra_imports = vec!["FakeReal".to_string(), "SpecializationArchSem".to_string()];
        p.registers = vec![
            ("x1".to_string(), "(BitVec 64)".to_string()),
            ("x2".to_string(), "(BitVec 64)".to_string()),
            ("v0".to_string(), "(BitVec 128)".to_string()),
        ];
        p.options.interface = InterfaceVersion::V2;
        p
    }

    #[test]
    fn toolchain_pins_default() {
        let opts = LakeOptions::new("m");
        assert_eq!(render_lean_toolchain(&opts), "leanprover/lean4:v4.29.0");
    }

    #[test]
    fn file_prelude_has_limits() {
        let pre = render_file_prelude(InterfaceVersion::V2, "TestModel", false);
        assert!(pre.contains("set_option maxHeartbeats 1_000_000_000"));
        assert!(pre.contains("set_option maxRecDepth 1_000_000"));
        assert!(pre.contains("open Sail.ArchSem"));
        assert!(pre.contains("namespace TestModel"));
    }

    #[test]
    fn lakefile_defaults_to_sail_rev_v6_without_mathlib() {
        let opts = LakeOptions::new("mymodel");
        let lf = render_lakefile(&opts).unwrap();
        assert!(lf.contains("name = \"mymodel\""));
        assert!(lf.contains("defaultTargets = [\"Mymodel\"]"));
        assert!(lf.contains("--tstack=400000"));
        assert!(lf.contains("name = \"Sail\""));
        assert!(lf.contains("https://github.com/rems-project/lean-sail"));
        assert!(lf.contains("rev = \"v6\""));
        assert!(!lf.contains("mathlib"));
        assert!(lf.contains("linter.style.nameCheck = false"));
        assert!(lf.contains("-fbracket-depth=500"));
    }

    #[test]
    fn lakefile_with_real_numbers_pulls_mathlib() {
        let mut opts = LakeOptions::new("m");
        opts.real_numbers = true;
        let lf = render_lakefile(&opts).unwrap();
        assert!(lf.contains("name = \"mathlib\""));
        assert!(lf.contains("mathlib4-nightly-testing"));
        assert!(lf.contains("rev = \"v4.29.0\""));
    }

    #[test]
    fn lakefile_with_executable_adds_lean_exe() {
        let mut opts = LakeOptions::new("m");
        opts.executable = true;
        let lf = render_lakefile(&opts).unwrap();
        assert!(lf.contains("[[lean_exe]]"));
        assert!(lf.contains("root = \"M\""));
    }

    #[test]
    fn lib_path_and_git_conflict() {
        let mut opts = LakeOptions::new("m");
        opts.lib_git = Some("https://example.com/sail".to_string());
        opts.lib_path = Some("/tmp/sail".to_string());
        assert!(render_lakefile(&opts).is_err());
    }

    #[test]
    fn register_templates_dedupe_types() {
        let regs = vec![
            ("x1".to_string(), "(BitVec 64)".to_string()),
            ("x2".to_string(), "(BitVec 64)".to_string()),
        ];
        let ind = render_register_inductive(&regs);
        assert!(ind.contains("inductive Register"));
        assert!(ind.contains("| x1"));
        let inhabited = render_register_inhabited(InterfaceVersion::V2, &regs);
        // One instance per distinct type, not per register.
        assert_eq!(inhabited.matches("instance : Inhabited").count(), 1);
    }

    #[test]
    fn inhabited_dedup_keeps_first_ctor_in_order() {
        // Lane 90: HashSet-Dedup statt O(n²)-Scan — erster Ctor pro Typ
        // gewinnt, Erstauftretens-Reihenfolge bleibt.
        let regs = vec![
            ("x1".to_string(), "(BitVec 64)".to_string()),
            ("v0".to_string(), "(BitVec 128)".to_string()),
            ("x2".to_string(), "(BitVec 64)".to_string()),
        ];
        let inhabited = render_register_inhabited(InterfaceVersion::V2, &regs);
        assert_eq!(inhabited.matches("instance : Inhabited").count(), 2);
        assert!(inhabited.contains(".Reg x1"));
        assert!(inhabited.contains(".Reg v0"));
        assert!(!inhabited.contains(".Reg x2"));
        assert!(inhabited.find(".Reg x1").unwrap() < inhabited.find(".Reg v0").unwrap());
        let v1 = render_register_inhabited(InterfaceVersion::V1, &regs);
        assert_eq!(v1.matches("instance : Inhabited").count(), 2);
        assert!(v1.contains("RegisterRef RegisterType (BitVec 64)"));
    }

    #[test]
    fn arch_v2_renders_all_fields() {
        let arch = ArchFieldsV2::default().render();
        assert!(arch.contains("@[reducible]"));
        assert!(arch.contains("instance : Arch where"));
        assert!(arch.contains("register := Register"));
        assert!(arch.contains("mem_acc_is_explicit := fun _ => false"));
    }

    #[test]
    fn generator_never_emits_sorry() {
        let snap = small_project().layout_snapshot().unwrap();
        assert!(!snap.contains("sorry"));
        // Reference the policy constant so it stays linked to the test.
        assert!(NO_SORRY_NOTE.contains("sorry"));
    }

    #[test]
    fn layout_snapshot_is_stable() {
        let snap = small_project().layout_snapshot().unwrap();
        for path in [".gitignore", "lean-toolchain", "lakefile.toml", "lake-manifest.json", "TestModel/Defs.lean", "TestModel.lean"] {
            assert!(snap.contains(&format!("=== {path} ===")), "missing {path}");
        }
        // Golden content: update deliberately, never blindly.
        assert!(snap.contains("leanprover/lean4:v4.29.0"));
        assert!(snap.contains("rev = \"v6\""));
        assert!(snap.contains("maxHeartbeats"));
        assert!(snap.contains("inductive Register"));
        assert!(snap.contains("namespace Functions"));
        insta_check(&snap);
    }

    /// Minimal inline snapshot guard: the full layout hash is pinned so
    /// accidental template drift fails loudly. Refresh by running with
    /// `BLESS=1`.
    fn insta_check(snap: &str) {
        // wyhash-style FNV-1a over the snapshot; cheap and stable.
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in snap.bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let got = format!("{hash:016x}");
        let want = snapshot_hash();
        if std::env::var("BLESS").is_ok() {
            std::eprintln!("BLESS snapshot hash: {got}");
            return;
        }
        assert_eq!(got, want, "layout snapshot drifted; inspect diff with BLESS=1");
    }

    fn snapshot_hash() -> String {
        // Computed from the current templates; see `BLESS=1` output.
        // Placeholder replaced on first green run below.
        EXPECTED_SNAPSHOT_HASH.to_string()
    }

    const EXPECTED_SNAPSHOT_HASH: &str = "0548491546d4278b";
}
