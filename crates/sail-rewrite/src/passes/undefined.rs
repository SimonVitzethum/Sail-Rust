//! `undefined` pass: eliminate `undefined`-typed placeholders.
//!
//! OCaml equivalent: `("undefined", [Bool_arg true])` in `lean_rewrites`
//! (`sail_plugin_lean.ml`). The OCaml rewrite fills in unspecified values so
//! the Lean backend never emits a bare `undefined`; here we lower
//! [`Stmt::Undefined`] to a concrete default assignment per declared type.
//!
//! Shard-local: each definition is rewritten independently, no cross-def state.

#![forbid(unsafe_code)]

use crate::ast::{DefKind, Shard, Stmt};
use crate::pass::Pass;

/// Eliminate [`Stmt::Undefined`] placeholders.
///
/// `let x : ty = undefined;` becomes `x = <default(ty)>;` where the default
/// is picked by [`default_for_ty`]. Non-`Undefined` statements pass through
/// unchanged. `Valspec` defs (no body semantics) are left untouched.
#[derive(Clone, Copy, Debug, Default)]
pub struct Undefined;

impl Pass for Undefined {
    fn name(&self) -> &'static str {
        "undefined"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            if def.kind == DefKind::Valspec {
                continue;
            }
            def.body = rewrite_body(std::mem::take(&mut def.body));
        }
        shard
    }
}

/// Rewrite one statement list, expanding `Undefined` in place.
fn rewrite_body(body: Vec<Stmt>) -> Vec<Stmt> {
    let mut out = Vec::with_capacity(body.len());
    for stmt in body {
        match stmt {
            Stmt::Undefined { target, ty } => {
                let rhs = default_for_ty(&ty);
                out.push(Stmt::Assign { lhs: target, rhs });
            }
            other => out.push(other),
        }
    }
    out
}

/// Pick a concrete default expression for a declared type.
///
/// Mirrors the spirit of the OCaml `undefined` rewrite (which inserts a
/// type-directed unspecified value): small inhabited defaults per type shape,
/// falling back to a `0`-literal-ish placeholder for unknown types.
#[must_use]
pub fn default_for_ty(ty: &str) -> String {
    let t = ty.trim().to_ascii_lowercase();
    if t == "bool" {
        return "false".to_string();
    }
    if t == "string" {
        return "\"\"".to_string();
    }
    if t == "real" {
        return "0.0".to_string();
    }
    if t.starts_with("bits(") || t.starts_with("bitvector") || t == "bits" {
        return "0b0".to_string();
    }
    if t.starts_with("vector(") || t.starts_with("list(") || t.ends_with(" list") {
        return "[]".to_string();
    }
    if t == "unit" || t == "()" {
        return "()".to_string();
    }
    // int, nat, integer, range(..), enum names, unknown: zero literal.
    "0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn shard_of(body: Vec<Stmt>) -> Shard {
        Shard::new(0, vec![Def::new("f", DefKind::Function, body)])
    }

    #[test]
    fn replaces_undefined_with_typed_default() {
        let p = Undefined;
        let out = p.run(shard_of(vec![
            Stmt::Undefined {
                target: "x".to_string(),
                ty: "bool".to_string(),
            },
            Stmt::Undefined {
                target: "n".to_string(),
                ty: "int".to_string(),
            },
        ]));
        assert_eq!(
            out.defs[0].body,
            vec![
                Stmt::Assign {
                    lhs: "x".to_string(),
                    rhs: "false".to_string()
                },
                Stmt::Assign {
                    lhs: "n".to_string(),
                    rhs: "0".to_string()
                },
            ]
        );
    }

    #[test]
    fn leaves_other_statements_and_valspecs_alone() {
        let p = Undefined;
        let shard = Shard::new(
            0,
            vec![
                Def::new(
                    "sig",
                    DefKind::Valspec,
                    vec![Stmt::Undefined {
                        target: "x".to_string(),
                        ty: "int".to_string(),
                    }],
                ),
                Def::new("f", DefKind::Function, vec![Stmt::Expr("skip".to_string())]),
            ],
        );
        let out = p.run(shard);
        // Valspec untouched (still Undefined), Expr untouched.
        assert!(matches!(out.defs[0].body[0], Stmt::Undefined { .. }));
        assert_eq!(out.defs[1].body, vec![Stmt::Expr("skip".to_string())]);
    }

    #[test]
    fn default_table() {
        assert_eq!(default_for_ty("bool"), "false");
        assert_eq!(default_for_ty("string"), "\"\"");
        assert_eq!(default_for_ty("bits(8)"), "0b0");
        assert_eq!(default_for_ty("vector(4, int)"), "[]");
        assert_eq!(default_for_ty("mystery_enum"), "0");
    }
}
