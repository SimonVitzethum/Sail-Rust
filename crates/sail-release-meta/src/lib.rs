#![forbid(unsafe_code)]

//! Release-Metadaten (Lane 40).
//!
//! Platzhalter-Crate, damit `cargo test` im Spec-Only-Branch grün ist.
//! Die eigentliche Release-Logik liegt in `docs/`; diese Crate enthält
//! bewusst nur Doku-Tests und kein `unsafe`.
//!
//! ```
//! assert_eq!(sail_release_meta::version_prefix(), "sail-rs");
//! ```

/// Präfix, das `sail-rs --version` laut `ARCHITECTURE.md` ausgeben muss.
#[must_use]
pub fn version_prefix() -> &'static str {
    "sail-rs"
}
