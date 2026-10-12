//! Numeric-expression identifier canonicalization, matching `nexp_ids` in
//! `sail/src/lib/rewrites.ml`: OCaml rewrites every `Nexp_id id` to
//! `Nexp_var (kid id)` so size expressions only mention type-level kids.
//!
//! Purely syntactic here: the string [`Shard`](crate::ast::Shard) carries no
//! `NExp` AST, so nexp positions are delimited by the size constructors the
//! string surface already uses — `sizeof(`, `bits(`, `bitvector(`, `atom(`
//! (the same markers [`crate::passes::undefined`](super::undefined)
//! understands). Every bare identifier token inside the balanced parens is
//! an `Nexp_id` and becomes kid form `'ident`, exactly like OCaml's
//! unconditional `Nexp_id` mapping; numeric literals and already-quoted
//! `'ident` kids are untouched, so the pass is deterministic and idempotent.
//! Markers require an adjacent `(` and a non-word char before them
//! (`mybits(n)` and `f(n)` are not nexp positions and stay verbatim);
//! unbalanced input is preserved best-effort up to the end of text.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"nexp_ids"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 21, zwischen
//!   `"guarded_pats"` (20) und `"split"` (22).
//! - Trait-Impl-Name: `NexpIds` (diese Datei).
//!   `passes/mod.rs`: `pub mod nexp_ids;` +
//!   `pub use nexp_ids::NexpIds;` ergaenzen.
//!   `pipeline.rs`: `"nexp_ids" => Some(Arc::new(NexpIds))` in
//!   `make_implemented` und den Namen in `is_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct NexpIds;

impl Pass for NexpIds {
    fn name(&self) -> &'static str {
        "nexp_ids"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            rewrite_body(&mut def.body);
        }
        shard
    }
}

/// Size constructors whose `(...)` arguments are nexp positions.
const MARKERS: &[&str] = &["sizeof(", "bits(", "bitvector(", "atom("];

fn rewrite_body(body: &mut [Stmt]) {
    for_each_text(body, &mut |text| {
        let rewritten = rewrite_text(text);
        if rewritten != *text {
            *text = rewritten;
        }
    });
}

fn for_each_text(body: &mut [Stmt], f: &mut dyn FnMut(&mut String)) {
    for stmt in body {
        match stmt {
            Stmt::Assign { lhs, rhs } => {
                f(lhs);
                f(rhs);
            }
            Stmt::TupleAssign { targets, rhs } => {
                for t in targets {
                    f(t);
                }
                f(rhs);
            }
            Stmt::LetTuple { bindings, rhs, body } => {
                for b in bindings {
                    f(b);
                }
                f(rhs);
                for_each_text(body, f);
            }
            Stmt::Undefined { target, ty } => {
                f(target);
                f(ty);
            }
            Stmt::Expr(src) | Stmt::Return(src) => f(src),
        }
    }
}

/// Rewrite bare identifiers in nexp positions to kid form (`n` -> `'n`).
///
/// Single scan with an explicit paren-depth counter: a marker enters depth 1
/// (nested markers re-trigger and are copied literally), `)` leaves one
/// level. Words at depth 0 are copied verbatim; at depth > 0 a bare
/// `[A-Za-z_][0-9A-Za-z_]*` word becomes `'word` unless already quoted.
fn rewrite_text(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut depth: usize = 0;
    let mut i = 0;
    // Last emitted char, for the `'`-quote and marker-boundary checks.
    let mut prev: Option<char> = None;
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    while i < bytes.len() {
        // `text` is only split at ASCII boundaries below, so slicing here
        // is safe; non-ASCII bytes fall through to the verbatim arm.
        let rest = &text[i..];
        if let Some(marker) = MARKERS.iter().find(|m| rest.starts_with(**m)) {
            let boundary = prev.map_or(true, |c| !is_word(c));
            if boundary {
                out.push_str(marker);
                prev = Some('(');
                depth += 1;
                i += marker.len();
                continue;
            }
        }
        // ASCII-only word scan: non-ASCII bytes always take the verbatim
        // arm below, so multi-byte UTF-8 sequences are never split.
        let b = bytes[i];
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if depth > 0 && prev != Some('\'') {
                out.push('\'');
            }
            out.push_str(&text[i..j]);
            prev = Some(text[i..j].chars().next_back().unwrap_or('_'));
            i = j;
            continue;
        }
        if b == b'(' && depth > 0 {
            depth += 1;
        } else if b == b')' && depth > 0 {
            depth -= 1;
        }
        // Verbatim copy of one UTF-8 scalar (or single byte, ASCII fast path
        // included: every special char above is ASCII).
        let len = text[i..].chars().next().map_or(1, |c| c.len_utf8());
        out.push_str(&text[i..i + len]);
        prev = text[i..i + len].chars().next();
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    #[test]
    fn rewrites_bare_ids_in_size_positions_to_kids() {
        assert_eq!(rewrite_text("sizeof(n)"), "sizeof('n)");
        assert_eq!(rewrite_text("sizeof((n + 1) * m)"), "sizeof(('n + 1) * 'm)");
        assert_eq!(rewrite_text("undefined_bitvector(sizeof(n))"), "undefined_bitvector(sizeof('n))");
        assert_eq!(rewrite_text("bits(n)"), "bits('n)");
        assert_eq!(rewrite_text("atom(len + 8)"), "atom('len + 8)");
    }

    #[test]
    fn leaves_literals_quoted_kids_and_other_calls_alone() {
        for src in [
            "sizeof(8)",
            "sizeof('n + 1)",
            "sizeof('n)",
            "int",
            "bool",
            "vector(4, int)",
            "mybits(n)",
            "f(n)",
            "n + 1",
        ] {
            assert_eq!(rewrite_text(src), src, "must preserve {src:?}");
        }
    }

    #[test]
    fn pass_rewrites_type_strings_and_is_idempotent() {
        let shard = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Undefined { target: "v".into(), ty: "bits(n)".into() },
                    Stmt::Assign { lhs: "x".into(), rhs: "sizeof(m) + foo(k)".into() },
                ],
            )],
        );
        let out = NexpIds.run(shard);
        assert_eq!(out.defs[0].body[0], Stmt::Undefined {
            target: "v".into(), ty: "bits('n)".into(),
        });
        assert_eq!(out.defs[0].body[1], Stmt::Assign {
            lhs: "x".into(), rhs: "sizeof('m) + foo(k)".into(),
        });
        let again = NexpIds.run(out.clone());
        assert_eq!(again, out, "pass must be idempotent");
    }
}
