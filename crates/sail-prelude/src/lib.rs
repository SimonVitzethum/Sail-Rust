#![forbid(unsafe_code)]
//! Embedded Sail prelude library.
//!
//! Vendored copy of `sail/lib/*.sail` from the OCaml Sail compiler,
//! embedded at compile time via `include_str!` / `include_bytes!`
//! with paths relative to this crate (no absolute-path hardcoding).
//!
//! Pinned Sail version: see [`SAIL_VERSION`] (read from
//! `sail/dune-project` at vendoring time).

/// Sail upstream version this prelude was vendored from.
///
/// Read from `sail/dune-project` (`(version 0.20.3)`).
pub const SAIL_VERSION: &str = "0.20.3";

/// One vendored prelude file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreludeFile {
    /// File name, e.g. `"prelude.sail"`.
    pub name: &'static str,
    /// UTF-8 source text (via `include_str!`).
    pub contents: &'static str,
    /// Raw bytes (via `include_bytes!`).
    pub bytes: &'static [u8],
}

macro_rules! prelude_file {
    ($name:literal) => {
        PreludeFile {
            name: $name,
            contents: include_str!(concat!("../sail-lib/", $name)),
            bytes: include_bytes!(concat!("../sail-lib/", $name)),
        }
    };
}

/// Inventory of all vendored `sail/lib/*.sail` files (26 entries).
pub static PRELUDE_FILES: &[PreludeFile] = &[
    prelude_file!("arith.sail"),
    prelude_file!("asl_compat.sail"),
    prelude_file!("concurrency_interface.sail"),
    prelude_file!("dec_bits.sail"),
    prelude_file!("elf.sail"),
    prelude_file!("exception.sail"),
    prelude_file!("exception_basic.sail"),
    prelude_file!("exception_result.sail"),
    prelude_file!("float.sail"),
    prelude_file!("flow.sail"),
    prelude_file!("generic_equality.sail"),
    prelude_file!("hex_bits.sail"),
    prelude_file!("hex_bits_signed.sail"),
    prelude_file!("isla.sail"),
    prelude_file!("mapping.sail"),
    prelude_file!("mono_rewrites.sail"),
    prelude_file!("option.sail"),
    prelude_file!("prelude.sail"),
    prelude_file!("real.sail"),
    prelude_file!("result.sail"),
    prelude_file!("reverse_endianness.sail"),
    prelude_file!("smt.sail"),
    prelude_file!("string.sail"),
    prelude_file!("vector.sail"),
    prelude_file!("vector_dec.sail"),
    prelude_file!("vector_inc.sail"),
];

/// Implicit builtins loaded before model inputs unless `--no-core` is set,
/// matching OCaml `Frontend.load_files` and `Corelib_sail.path`.
pub static CORE_LIBRARY: PreludeFile = prelude_file!("core/corelib.sail");

/// File names in [`PRELUDE_FILES`] order.
pub fn names() -> impl Iterator<Item = &'static str> {
    PRELUDE_FILES.iter().map(|f| f.name)
}

/// Look up a vendored prelude file by file name.
pub fn get(name: &str) -> Option<&'static PreludeFile> {
    PRELUDE_FILES.iter().find(|f| f.name == name)
        .or_else(|| (name == CORE_LIBRARY.name).then_some(&CORE_LIBRARY))
}

/// Minimal empty-model skeleton: a stub Sail model that only pulls in
/// the prelude and defines an empty `main`. Used by tests and, later,
/// by compiler lanes as the smallest "model" input.
pub const EMPTY_MODEL_SKELETON: &str = "$include <prelude.sail>\n\nval main : unit -> unit\nfunction main() = ()\n";

/// Returns the empty-model skeleton source.
pub fn empty_model() -> &'static str {
    EMPTY_MODEL_SKELETON
}

pub mod corpus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_pinned() {
        assert_eq!(SAIL_VERSION, "0.20.3");
    }

    #[test]
    fn empty_model_skeleton_is_present() {
        assert!(!EMPTY_MODEL_SKELETON.is_empty());
        assert!(EMPTY_MODEL_SKELETON.contains("prelude.sail"));
        assert!(!empty_model().is_empty());
    }
}
