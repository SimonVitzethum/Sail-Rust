//! `Defs.lean` / root-module generation from emission.
//!
//! While [`crate::LakeProject::layout`] renders the static project
//! skeleton (toolchain, lakefile, register templates), this module renders
//! the two definition-carrying files from actual emission output:
//!
//! - `<Camel>/Defs.lean`: [`render_defs_header`](crate::render_defs_header)
//!   + register section + emitted definitions in source order.
//! - `<Camel>.lean` (root): imports of the emitted shard modules plus the
//!   shared function-file prelude.
//!
//! Contract, mirroring the OCaml backend (`start_lean_output`):
//!
//! - **Module names**: derived from [`crate::LakeOptions::module_name`]
//!   (`UpperCamelCase`); every shard/extra import must be a valid single
//!   Lean module path segment ([`validate_module_segment`]).
//! - **Imports**: the root module lists the shard modules in caller order;
//!   `extra_imports` (e.g. copied `--lean-import-file` modules) are
//!   appended exactly like
//!   [`crate::render_function_file_prelude`] does.
//! - **Order**: emitted definitions are concatenated in stable
//!   [`EmittedDef::position`] order (the `TypedDef.position` source index:
//!   declaration order over the frontend manifest). [`sort_by_position`]
//!   is stable, so equal positions keep caller order.
//! - **Empty rejects stubs**: an empty emission, an empty def body, a
//!   duplicate def name, or a root module without shard imports is a hard
//!   `Err` — never a silently written stub file. There is deliberately
//!   **no** `sorry` fallback.
//!
//! The module only renders text and never touches the filesystem.
//! Rendering is total over validated input, allocation only via [`String`],
//! no `unsafe`.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::{render_defs_header, render_function_file_prelude, LakeOptions};

/// One emitted Lean definition with its source-order position.
///
/// `position` is the `TypedDef.position` source index (declaration order
/// over the frontend manifest, see `sail-cli/src/typecheck.rs`: the
/// `enumerate()` index over descriptors). `lean` holds the fully rendered
/// Lean source of this single definition (trailing newline optional).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedDef {
    /// Lean definition name (already sanitized by the printer).
    pub name: String,
    /// Rendered Lean source of this definition.
    pub lean: String,
    /// Source-order index; lower renders first.
    pub position: usize,
}

impl EmittedDef {
    /// New emitted definition.
    pub fn new(name: impl Into<String>, lean: impl Into<String>, position: usize) -> Self {
        Self {
            name: name.into(),
            lean: lean.into(),
            position,
        }
    }
}

/// Validate a single Lean module path segment (e.g. `Defs`, `Functions`,
/// `FakeReal`, `SpecializationV1`).
///
/// Non-empty, no path separators or dots, no whitespace; first character
/// alphabetic or `_`, the rest alphanumeric, `_` or `'`. Rejects `..`,
/// `/`, `.lean` suffixes and empty strings so a bad shard name can never
/// produce a broken `import` line or an escaping file path.
pub fn validate_module_segment(segment: &str) -> Result<(), String> {
    if segment.is_empty() {
        return Err("module segment must not be empty".to_string());
    }
    let mut chars = segment.chars();
    let first = chars.next().unwrap_or_default(); // checked non-empty above
    if !(first.is_alphabetic() || first == '_') {
        return Err(format!(
            "module segment {segment:?} must start with a letter or `_`"
        ));
    }
    if segment.contains('.') || segment.contains('/') || segment.contains('\\') {
        return Err(format!(
            "module segment {segment:?} must not contain `.` or path separators"
        ));
    }
    if segment.chars().any(char::is_whitespace) {
        return Err(format!(
            "module segment {segment:?} must not contain whitespace"
        ));
    }
    if !segment
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
    {
        return Err(format!("module segment {segment:?} has invalid characters"));
    }
    Ok(())
}

/// Project-relative file path of the definitions module (`<Camel>/Defs.lean`).
#[must_use]
pub fn defs_module_file(camel: &str) -> String {
    format!("{camel}/Defs.lean")
}

/// Project-relative file path of the root module (`<Camel>.lean`).
#[must_use]
pub fn root_module_file(camel: &str) -> String {
    format!("{camel}.lean")
}

/// Lean import path of one shard (`<Camel>.<Shard>`).
#[must_use]
pub fn shard_import(camel: &str, shard: &str) -> String {
    format!("{camel}.{shard}")
}

/// Sort emitted definitions into source order (stable by `position`).
///
/// Stable: definitions sharing a position keep their caller order
/// (same-telescope elaboration order within one source definition).
pub fn sort_by_position(defs: &mut [EmittedDef]) {
    defs.sort_by_key(|d| d.position);
}

/// Validate an emission: non-empty, no empty names/bodies, no duplicate
/// names. Returns `Err` instead of letting callers write stub files.
pub fn check_emission(defs: &[EmittedDef]) -> Result<(), String> {
    if defs.is_empty() {
        return Err("emission is empty: refusing to write stub Defs.lean".to_string());
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(defs.len());
    for d in defs {
        if d.name.trim().is_empty() {
            return Err(format!(
                "emitted def at position {} has an empty name",
                d.position
            ));
        }
        if d.lean.trim().is_empty() {
            return Err(format!(
                "emitted def `{}` is empty: refusing to write stub",
                d.name
            ));
        }
        if !seen.insert(d.name.as_str()) {
            return Err(format!("duplicate emitted def `{}`", d.name));
        }
    }
    Ok(())
}

/// Validate shard module imports for the root module: non-empty list (an
/// import-less root would be a stub) plus per-segment validation.
pub fn check_shard_modules(shard_modules: &[String]) -> Result<(), String> {
    if shard_modules.is_empty() {
        return Err("no shard modules: refusing to write stub root module".to_string());
    }
    for m in shard_modules {
        validate_module_segment(m)?;
    }
    Ok(())
}

/// Render concatenated definition bodies in stable source order.
///
/// Each body is separated by exactly one blank line and the result ends
/// with a newline. Validates via [`check_emission`] first.
pub fn render_defs_body(defs: &[EmittedDef]) -> Result<String, String> {
    check_emission(defs)?;
    let mut sorted: Vec<&EmittedDef> = defs.iter().collect();
    sorted.sort_by_key(|d| d.position);
    let mut s = String::with_capacity(sorted.iter().map(|d| d.lean.len() + 2).sum());
    for d in sorted {
        s.push_str(d.lean.trim_end());
        s.push_str("\n\n");
    }
    Ok(s)
}

/// Render the full `<Camel>/Defs.lean` from emission: header + register
/// section + definition bodies in stable source order.
///
/// `register_section` is e.g. [`crate::LakeProject::render_register_section`]
/// output. A blank line separates it from the first body. Rejects empty
/// emissions via [`check_emission`].
#[must_use = "rendered text must be used or tested"]
pub fn render_defs_lean(
    options: &LakeOptions,
    register_section: &str,
    defs: &[EmittedDef],
) -> Result<String, String> {
    let body = render_defs_body(defs)?;
    let mut s = render_defs_header(options);
    if !register_section.is_empty() {
        s.push_str(register_section);
        if !register_section.ends_with('\n') {
            s.push('\n');
        }
        if !register_section.ends_with("\n\n") {
            s.push('\n');
        }
    }
    s.push_str(&body);
    Ok(s)
}

/// Render the full `<Camel>.lean` root module importing the emitted shard
/// modules (in caller order) plus `extra_imports`.
///
/// Delegates the actual prelude rendering to
/// [`crate::render_function_file_prelude`] (byte-compatible with the
/// static skeleton); this wrapper only enforces the emission contract:
/// non-empty validated shard list, validated extra imports — never a
/// stub root.
///
/// Note: [`crate::render_function_file_prelude`] renders `extra_imports`
/// only when `import_refs` is empty, so both lists are merged into one
/// ordered `import_refs` here (`import <Camel>.<Shard>` spelling is
/// identical in both paths).
#[must_use = "rendered text must be used or tested"]
pub fn render_root_module(
    options: &LakeOptions,
    shard_modules: &[String],
    extra_imports: &[String],
) -> Result<String, String> {
    check_shard_modules(shard_modules)?;
    for extra in extra_imports {
        validate_module_segment(extra)?;
    }
    let mut refs: Vec<String> = Vec::with_capacity(shard_modules.len() + extra_imports.len());
    refs.extend(shard_modules.iter().cloned());
    refs.extend(extra_imports.iter().cloned());
    Ok(render_function_file_prelude(options, &refs, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InterfaceVersion;

    fn opts() -> LakeOptions {
        LakeOptions::new("tiny_arm")
    }

    fn emission() -> Vec<EmittedDef> {
        vec![
            EmittedDef::new("c_third", "def c_third : Nat := 3\n", 30),
            EmittedDef::new("a_first", "def a_first : Nat := 1\n", 4),
            EmittedDef::new("b_second", "def b_second : Nat := 2\n", 17),
        ]
    }

    #[test]
    fn bodies_render_in_position_order() {
        let body = render_defs_body(&emission()).unwrap();
        let (a, b, c) = (
            body.find("a_first").unwrap(),
            body.find("b_second").unwrap(),
            body.find("c_third").unwrap(),
        );
        assert!(a < b && b < c, "position order violated:\n{body}");
        assert!(body.ends_with('\n'));
        assert!(!body.contains("sorry"));
    }

    #[test]
    fn equal_positions_keep_caller_order() {
        let defs = vec![
            EmittedDef::new("first", "def first : Nat := 1\n", 7),
            EmittedDef::new("second", "def second : Nat := 2\n", 7),
        ];
        let body = render_defs_body(&defs).unwrap();
        assert!(body.find("first").unwrap() < body.find("second").unwrap());
    }

    #[test]
    fn sort_by_position_is_stable() {
        let mut defs = emission();
        sort_by_position(&mut defs);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["a_first", "b_second", "c_third"]);
    }

    #[test]
    fn empty_emission_is_rejected() {
        let err = render_defs_body(&[]).unwrap_err();
        assert!(err.contains("empty"), "{err}");
        assert!(render_defs_lean(&opts(), "", &[]).is_err());
    }

    #[test]
    fn empty_body_is_rejected_not_stubbed() {
        let defs = vec![EmittedDef::new("hole", "  \n ", 0)];
        let err = render_defs_body(&defs).unwrap_err();
        assert!(err.contains("hole"), "{err}");
        assert!(err.contains("refusing"), "{err}");
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let defs = vec![
            EmittedDef::new("dup", "def dup : Nat := 1\n", 1),
            EmittedDef::new("dup", "def dup : Nat := 2\n", 2),
        ];
        let err = render_defs_body(&defs).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn empty_names_are_rejected() {
        let defs = vec![EmittedDef::new("  ", "def x : Nat := 1\n", 0)];
        assert!(render_defs_body(&defs).is_err());
    }

    #[test]
    fn defs_lean_has_header_registers_and_ordered_bodies() {
        let options = opts();
        let regs = "abbrev Register := PEmpty\n";
        let out = render_defs_lean(&options, regs, &emission()).unwrap();
        assert!(out.starts_with("import Sail\nopen PreSail\n"));
        assert!(out.contains("namespace TinyArm"));
        assert!(out.contains(regs.trim_end()));
        let (a, b, c) = (
            out.find("a_first").unwrap(),
            out.find("b_second").unwrap(),
            out.find("c_third").unwrap(),
        );
        assert!(a < b && b < c);
        assert!(out.find("namespace TinyArm").unwrap() < a);
        assert!(!out.contains("sorry"));
    }

    #[test]
    fn defs_lean_without_register_section_still_separates() {
        let out = render_defs_lean(&opts(), "", &emission()).unwrap();
        assert!(out.contains("namespace TinyArm"));
        assert!(out.contains("def a_first"));
    }

    #[test]
    fn root_module_imports_shards_in_order() {
        let shards = vec!["Defs".to_string(), "Functions".to_string()];
        let out = render_root_module(&opts(), &shards, &[]).unwrap();
        let (d, f) = (
            out.find("import TinyArm.Defs").unwrap(),
            out.find("import TinyArm.Functions").unwrap(),
        );
        assert!(d < f, "import order violated:\n{out}");
        assert!(out.contains("namespace Functions"));
        assert!(!out.contains("sorry"));
    }

    #[test]
    fn root_module_appends_extra_imports() {
        let shards = vec!["Defs".to_string()];
        let extras = vec!["FakeReal".to_string()];
        let out = render_root_module(&opts(), &shards, &extras).unwrap();
        assert!(out.contains("import TinyArm.Defs"));
        assert!(out.contains("import TinyArm.FakeReal"));
    }

    #[test]
    fn root_module_without_shards_is_rejected() {
        let err = render_root_module(&opts(), &[], &[]).unwrap_err();
        assert!(err.contains("refusing"), "{err}");
    }

    #[test]
    fn invalid_module_segments_are_rejected() {
        for bad in ["", "a/b", "A.B", "has space", "0abc", "-x", "x/y.lean"] {
            assert!(
                validate_module_segment(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        for good in [
            "Defs",
            "Functions",
            "FakeReal",
            "SpecializationV1",
            "_priv",
            "A'B",
        ] {
            assert!(
                validate_module_segment(good).is_ok(),
                "{good:?} should be accepted"
            );
        }
        // Rejected segments also fail root rendering.
        assert!(render_root_module(&opts(), &["A.B".to_string()], &[])
            .unwrap_err()
            .contains("must not contain"));
        assert!(
            render_root_module(&opts(), &["Defs".to_string()], &["../evil".to_string()]).is_err()
        );
    }

    #[test]
    fn module_file_and_import_helpers() {
        assert_eq!(defs_module_file("TinyArm"), "TinyArm/Defs.lean");
        assert_eq!(root_module_file("TinyArm"), "TinyArm.lean");
        assert_eq!(shard_import("TinyArm", "Defs"), "TinyArm.Defs");
        // Camel derivation stays the single module-name source.
        assert_eq!(defs_module_file(&opts().module_name()), "TinyArm/Defs.lean");
    }

    #[test]
    fn v1_and_noncomputable_options_flow_through() {
        let mut options = LakeOptions::new("m");
        options.interface = InterfaceVersion::V1;
        options.noncomputable = true;
        let out = render_defs_lean(&options, "", &emission()).unwrap();
        assert!(out.contains("open Sail.ConcurrencyInterfaceV1"));
        assert!(out.contains("noncomputable section"));
        let root = render_root_module(&options, &["Defs".to_string()], &[]).unwrap();
        assert!(root.contains("open Sail.ConcurrencyInterfaceV1"));
    }

    #[test]
    fn rendered_chunks_never_emit_sorry() {
        let defs_lean = render_defs_lean(&opts(), "abbrev R := Unit\n", &emission()).unwrap();
        let root = render_root_module(&opts(), &["Defs".to_string()], &[]).unwrap();
        for (what, haystack) in [("Defs.lean", &defs_lean), ("root", &root)] {
            assert!(
                !haystack.contains("sorry"),
                "{what} must not emit `sorry`:\n{haystack}"
            );
        }
        // Keep the policy constant linked so it cannot drift silently.
        assert!(crate::NO_SORRY_NOTE.contains("sorry"));
        // `write!` is actually exercised (no dead import on strict lints).
        let mut probe = String::new();
        let _ = write!(probe, "{}", "ok");
        assert_eq!(probe, "ok");
    }
}
