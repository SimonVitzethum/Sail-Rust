#![forbid(unsafe_code)]
//! Differential test harness: OCaml `sail` as oracle vs. Rust `sail` output.
//!
//! Lane 13 skeleton. The oracle side (running the OCaml `sail` binary on
//! `sail/test/lean/*.sail`) is fully implemented; the Rust-compiler side is a
//! stub until later lanes produce output. See `oracle::RustBackend`.

pub mod diff;
pub mod normalize;
pub mod oracle;
pub mod report;
