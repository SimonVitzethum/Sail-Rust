//! Build script for `sail-grammar`: compiles `src/sail.lalrpop` to the
//! LALRPOP tables consumed via `lalrpop_mod!(sail)` in `src/lib.rs`.
//!
//! Pure Rust (`#![forbid(unsafe_code)]`). Generation failures are reported
//! as a plain `[build-script]` error with the grammar path and the LALRPOP
//! cause — no `panic!`/`unwrap`/`expect`, so a remaining LALR conflict
//! shows up on the Test-Rig as an actionable message, not a backtrace.

#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=src/sail.lalrpop");
    if let Err(err) = lalrpop::process_root() {
        eprintln!("error: LALRPOP table generation for `sail-grammar` failed");
        eprintln!("       grammar: crates/sail-grammar/src/sail.lalrpop");
        eprintln!("       cause: {err}");
        eprintln!("help: sail.lalrpop must stay LALR(1)-conflict-free; see");
        eprintln!("      crates/sail-grammar/GRAMMAR.md (E3c/E4 designs) and fix the");
        eprintln!("      reported state instead of working around it.");
        std::process::exit(1);
    }
}
