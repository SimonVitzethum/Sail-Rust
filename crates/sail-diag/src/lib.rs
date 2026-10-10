#![forbid(unsafe_code)]
//! Shared Sail diagnostics (Lane 35): one error type for every lane.
//!
//! - [`SailError`] is the only error shape lanes report: `{ span, pass,
//!   code, message, hint }` plus an optional Sail source [`Span`] snippet.
//! - [`DiagCode`] / [`DiagEntry`] / [`ALL`] is the `--explain` catalog
//!   covering every current `failwith` / `unreachable` site in
//!   `sail/src/sail_lean_backend/pretty_print_lean.ml` (23 sites,
//!   `E3501`–`E3523`). Each row carries the OCaml site, the message
//!   pattern, a workaround hint, and the owning lane spec.
//! - [`ExitCode`] maps diagnostics to process exit codes at the CLI edge.
//! - [`Span`] / [`FileId`] are minimal local types, field-compatible with
//!   `sail-ast` / `sail-lex`, so a later lane can unify them.
//!
//! Re-export-friendly by design: other lanes just `use sail_diag::{...}`.
//! This crate has zero dependencies so every lane can depend on it.

mod code;
mod error;
mod exit;
mod pass;
mod span;

pub use code::{
    explain_str, explain_table, format_explain, DiagCode, DiagEntry, ErrorKind, UnknownDiagCode,
    ALL,
};
pub use error::SailError;
pub use exit::ExitCode;
pub use pass::{ParsePassError, Pass};
pub use span::{FileId, Span};
