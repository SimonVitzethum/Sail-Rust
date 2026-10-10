//! Lean executable stub for the pure-Rust Sail compiler.
//!
//! Pure-Rust port of `main_function_stub` plus the `--lean-executable`
//! gating from `sail/src/sail_lean_backend/pretty_print_lean.ml`
//! (lines ~1607-1621, `fix_id "main"` case, and the `pp_ast_lean`
//! `the_main_function_has_been_seen` wiring) and
//! `sail/src/sail_lean_backend/sail_plugin_lean.ml`
//! (`opt_lean_executable`, `create_lake_project`, `output`).
//!
//! The generator only renders text: it never touches the filesystem
//! itself. All rendering is total, allocation only via [`String`],
//! no `unsafe`.

#![forbid(unsafe_code)]

/// Sail source name of the program entry point.
///
/// The OCaml backend renames this identifier during pretty-printing
/// (`fix_id` maps `"main"` to [`LEAN_MAIN_NAME`] and sets
/// `the_main_function_has_been_seen`).
pub const SAIL_MAIN_NAME: &str = "main";

/// Lean name of the entry point after `fix_id` renaming.
pub const LEAN_MAIN_NAME: &str = "sail_main";

/// Name of the generated Lean `IO` entry point.
pub const LEAN_IO_MAIN_NAME: &str = "main";

/// Name of the support-library runner applied to the Sail entry point.
pub const MAIN_RUNNER_NAME: &str = "main_of_sail_main";

/// Name of the register-state initialiser chained before the entry
/// point when the model declares registers.
pub const MODEL_INIT_NAME: &str = "sail_model_init";

/// CLI flag name enabling executable output.
pub const LEAN_EXECUTABLE_FLAG: &str = "--lean-executable";

/// `[[lean_exe]]` target name emitted into `lakefile.toml`,
/// mirroring `create_lake_project`.
pub const LEAN_EXE_TARGET_NAME: &str = "run";

/// Returns `true` when a Sail function name is the program entry point
/// (i.e. must be renamed to [`LEAN_MAIN_NAME`]).
///
/// Mirrors the `"main"` case of `fix_id` in `pretty_print_lean.ml`.
#[must_use]
pub fn is_entry_main(name: &str) -> bool {
    name == SAIL_MAIN_NAME
}

/// Lean name for a Sail function: `"main"` becomes [`LEAN_MAIN_NAME`],
/// every other name passes through unchanged.
#[must_use]
pub fn lean_function_name(name: &str) -> &str {
    if is_entry_main(name) {
        LEAN_MAIN_NAME
    } else {
        name
    }
}

/// Returns `true` if any of the given Sail function names is the entry
/// point. This is the crate equivalent of dereferencing
/// `the_main_function_has_been_seen` after `pp_ast_lean`.
#[must_use]
pub fn detect_main_seen(names: &[&str]) -> bool {
    names.iter().any(|n| is_entry_main(n))
}

/// Renders the reference to the Sail entry point used inside the
/// `main_of_sail_main` call.
///
/// Mirrors `main_function_stub`'s `main_function` binding:
/// an effectful `main` is referenced directly as `sail_main`, while a
/// pure `main` is wrapped as `(lambda-unit arrow (pure (sail_main ())
/// : SailM Unit))` (the lambda/arrow are the Unicode characters used
/// by the OCaml backend).
#[must_use]
pub fn render_main_function_ref(main_is_effectful: bool) -> String {
    if main_is_effectful {
        LEAN_MAIN_NAME.to_string()
    } else {
        "(\u{03BB}() \u{21A6} (pure (sail_main ()) : SailM Unit))".to_string()
    }
}

/// Renders the call argument chained after `sail_model_init` when the
/// model declares registers, mirroring `main_call` in
/// `main_function_stub`:
///
/// * `has_registers == true`:
///   `(sail_model_init >=> <ref>)`
/// * otherwise: `<ref>`
#[must_use]
pub fn render_main_call(main_is_effectful: bool, has_registers: bool) -> String {
    let func = render_main_function_ref(main_is_effectful);
    if has_registers {
        format!("({MODEL_INIT_NAME} >=> {func})")
    } else {
        func
    }
}

/// Renders the `def main` stub, byte-compatible with
/// `main_function_stub` in `pretty_print_lean.ml`:
///
/// ```lean
/// def main (_ : List String) : IO UInt32 := do
///   main_of_sail_main default <call>
/// ```
///
/// The stub ends with a single trailing newline, matching the OCaml
/// `separate hardline [...; empty]` layout.
#[must_use]
pub fn render_main_stub(main_is_effectful: bool, has_registers: bool) -> String {
    let call = render_main_call(main_is_effectful, has_registers);
    format!("def main (_ : List String) : IO UInt32 := do\n  {MAIN_RUNNER_NAME} default {call}\n")
}

/// Configuration for executable output.
///
/// `lean_executable` mirrors `opt_lean_executable`
/// (`--lean-executable`); it defaults to `false` so the default output
/// stays a library without `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ExeOptions {
    /// Value of `--lean-executable`.
    pub lean_executable: bool,
}

impl ExeOptions {
    /// New options with the given flag value.
    #[must_use]
    pub fn new(lean_executable: bool) -> Self {
        Self { lean_executable }
    }

    /// Library default: no executable output.
    #[must_use]
    pub fn lib() -> Self {
        Self {
            lean_executable: false,
        }
    }

    /// Executable output requested.
    #[must_use]
    pub fn exe() -> Self {
        Self {
            lean_executable: true,
        }
    }
}

/// Returns `true` when the `[[lean_exe]]` `run`-target must be emitted
/// into `lakefile.toml`.
///
/// Mirrors `create_lake_project ctx (executable && !opt_lean_executable)`
/// in `sail_plugin_lean.ml`, where `executable` is the
/// `the_main_function_has_been_seen` return value of `pp_ast_lean`:
/// the target is only emitted when the flag is set **and** a `main`
/// was seen. The default therefore stays a library.
#[must_use]
pub fn should_emit_lean_exe(options: ExeOptions, main_seen: bool) -> bool {
    options.lean_executable && main_seen
}

/// Returns `true` when the `def main` stub must be appended to the
/// function file. Uses the same gate as [`should_emit_lean_exe`]: only
/// with the flag set and a `main` present; otherwise the output stays
/// a library without `main`.
#[must_use]
pub fn should_emit_main_stub(options: ExeOptions, main_seen: bool) -> bool {
    should_emit_lean_exe(options, main_seen)
}

/// Renders the stub when the gate is open, or `None` when the output
/// stays a library without `main`.
#[must_use]
pub fn render_main_stub_gated(
    options: ExeOptions,
    main_seen: bool,
    main_is_effectful: bool,
    has_registers: bool,
) -> Option<String> {
    if should_emit_main_stub(options, main_seen) {
        Some(render_main_stub(main_is_effectful, has_registers))
    } else {
        None
    }
}

/// Renders the `[[lean_exe]]` fragment appended to `lakefile.toml` by
/// `create_lake_project`, byte-identical to the OCaml backend and to
/// `sail-lake`'s `render_lakefile` executable branch:
///
/// ```toml
/// [[lean_exe]]
/// name = "run"
/// root = "<Module>"
/// ```
///
/// Callers that render through `sail-lake` should instead set
/// `LakeOptions.executable = should_emit_lean_exe(options, main_seen)`;
/// this helper exists for callers that append to a manually rendered
/// lakefile. `module_name` is the `UpperCamelCase` project module.
#[must_use]
pub fn render_lean_exe_fragment(module_name: &str) -> String {
    format!("\n\n[[lean_exe]]\nname = \"{LEAN_EXE_TARGET_NAME}\"\nroot = \"{module_name}\"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forbidden Lean placeholder must never be emitted. Built from
    /// character codes so the source itself stays free of it.
    fn forbidden_placeholder() -> String {
        String::from_iter(['s', 'o', 'r', 'r', 'y'].iter())
    }

    #[test]
    fn entry_detection_renames_only_main() {
        assert!(is_entry_main("main"));
        assert!(!is_entry_main("sail_main"));
        assert!(!is_entry_main("Main"));
        assert!(!is_entry_main(""));
        assert_eq!(lean_function_name("main"), "sail_main");
        assert_eq!(lean_function_name("foo"), "foo");
    }

    #[test]
    fn detect_main_seen_scans_function_names() {
        assert!(detect_main_seen(&["init", "main"]));
        assert!(!detect_main_seen(&["init", "step"]));
        assert!(!detect_main_seen(&[]));
    }

    #[test]
    fn effectful_main_ref_is_direct() {
        assert_eq!(render_main_function_ref(true), "sail_main");
    }

    #[test]
    fn pure_main_ref_wraps_in_lambda() {
        let func = render_main_function_ref(false);
        assert!(func.contains("pure (sail_main ()) : SailM Unit"));
        assert!(func.starts_with('('));
    }

    #[test]
    fn main_call_chains_model_init_only_with_registers() {
        let with_regs = render_main_call(true, true);
        assert_eq!(with_regs, "(sail_model_init >=> sail_main)");
        let without_regs = render_main_call(true, false);
        assert_eq!(without_regs, "sail_main");
        let pure_with_regs = render_main_call(false, true);
        assert!(pure_with_regs.starts_with("(sail_model_init >=> ("));
        assert!(pure_with_regs.contains("pure (sail_main ()) : SailM Unit"));
    }

    #[test]
    fn stub_matches_ocaml_shape() {
        let stub = render_main_stub(true, true);
        assert_eq!(
            stub,
            "def main (_ : List String) : IO UInt32 := do\n  main_of_sail_main default (sail_model_init >=> sail_main)\n"
        );
        let stub_pure = render_main_stub(false, false);
        assert!(stub_pure.starts_with("def main (_ : List String) : IO UInt32 := do\n  main_of_sail_main default "));
        assert!(stub_pure.contains("pure (sail_main ()) : SailM Unit"));
        assert!(!stub_pure.contains("sail_model_init"));
        assert!(!stub.contains(&forbidden_placeholder()));
        assert!(!stub_pure.contains(&forbidden_placeholder()));
    }

    #[test]
    fn default_stays_lib_without_main() {
        let lib = ExeOptions::lib();
        assert!(!lib.lean_executable);
        assert_eq!(ExeOptions::default(), lib);
        // Flag off: no stub and no exe target even when main exists.
        assert!(!should_emit_lean_exe(lib, true));
        assert!(!should_emit_main_stub(lib, true));
        assert_eq!(render_main_stub_gated(lib, true, true, true), None);
        // Flag on but no main: still a library.
        assert!(!should_emit_lean_exe(ExeOptions::exe(), false));
        assert_eq!(render_main_stub_gated(ExeOptions::exe(), false, true, true), None);
    }

    #[test]
    fn flag_plus_main_emits_stub_and_exe() {
        let exe = ExeOptions::exe();
        assert!(should_emit_lean_exe(exe, true));
        assert!(should_emit_main_stub(exe, true));
        let stub = render_main_stub_gated(exe, true, true, false).unwrap();
        assert!(stub.contains("main_of_sail_main default sail_main"));
    }

    #[test]
    fn lean_exe_fragment_is_byte_stable() {
        let frag = render_lean_exe_fragment("TestModel");
        assert_eq!(frag, "\n\n[[lean_exe]]\nname = \"run\"\nroot = \"TestModel\"\n");
        assert!(!frag.contains(&forbidden_placeholder()));
    }

    #[test]
    fn small_model_with_flag_yields_run_target_via_sail_lake() {
        // Docking check against the real sail-lake interface: the lane-34
        // gate feeds `LakeOptions.executable`, and sail-lake renders the
        // identical `[[lean_exe]]` fragment.
        let options = ExeOptions::exe();
        let main_seen = true;
        let mut lake = sail_lake::LakeOptions::new("tiny");
        lake.executable = should_emit_lean_exe(options, main_seen);
        assert!(lake.executable);
        let lakefile = sail_lake::render_lakefile(&lake).unwrap();
        assert!(lakefile.contains(&render_lean_exe_fragment(&lake.module_name())));
        assert!(lakefile.contains("name = \"run\""));
        assert!(!lakefile.contains(&forbidden_placeholder()));
    }

    #[test]
    fn small_model_without_flag_stays_lib_via_sail_lake() {
        let mut lake = sail_lake::LakeOptions::new("tiny");
        lake.executable = should_emit_lean_exe(ExeOptions::lib(), true);
        assert!(!lake.executable);
        let lakefile = sail_lake::render_lakefile(&lake).unwrap();
        assert!(!lakefile.contains("[[lean_exe]]"));
        assert!(render_main_stub_gated(ExeOptions::lib(), true, true, true).is_none());
    }
}
