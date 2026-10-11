//! Type-directed undefined values, following `undefined_of_typ` in OCaml
//! `sail/src/lib/ast_util.ml`. Undefined is a helper call, not an arbitrary
//! literal: zero may violate ranges and a one-bit literal has the wrong width.
//! Unsupported type applications remain explicit until a typed AST is used.

#![forbid(unsafe_code)]

use crate::ast::{DefKind, Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct Undefined;

impl Pass for Undefined {
    fn name(&self) -> &'static str { "undefined" }
    fn run(&self, mut shard: Shard) -> Shard {
        for def in &mut shard.defs {
            if def.kind != DefKind::Valspec { rewrite_body(&mut def.body); }
        }
        shard
    }
}

fn rewrite_body(body: &mut [Stmt]) {
    for stmt in body {
        match stmt {
            Stmt::Undefined { target, ty } => {
                if let Some(rhs) = default_for_ty(ty) {
                    *stmt = Stmt::Assign { lhs: std::mem::take(target), rhs };
                }
            }
            Stmt::LetTuple { body, .. } => rewrite_body(body),
            _ => {}
        }
    }
}

/// Render the OCaml helper for types representable without a type environment.
/// Names are case sensitive. `None` preserves the unresolved placeholder;
/// guessing a value for an unsupported type would silently change semantics.
#[must_use]
pub fn default_for_ty(ty: &str) -> Option<String> {
    let t = ty.trim();
    if !t.is_empty() && t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && t.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return Some(format!("undefined_{t}()"));
    }
    // Lean's undefined rewrite enables machine words. Keep symbolic widths
    // via sizeof, exactly as undefined_of_typ_args does for numeric arguments.
    for name in ["bits", "bitvector", "atom"] {
        if let Some(arg) = t.strip_prefix(name).map(str::trim)
            .and_then(|s| s.strip_prefix('(')).and_then(|s| s.strip_suffix(')'))
        {
            if arg.trim().is_empty() || arg.contains(',') { return None; }
            return Some(if name == "atom" {
                format!("sizeof({})", arg.trim())
            } else {
                format!("undefined_bitvector(sizeof({}))", arg.trim())
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    #[test]
    fn calls_type_directed_helpers_instead_of_guessing_literals() {
        assert_eq!(default_for_ty("bool").as_deref(), Some("undefined_bool()"));
        assert_eq!(default_for_ty("string").as_deref(), Some("undefined_string()"));
        assert_eq!(default_for_ty("MyEnum").as_deref(), Some("undefined_MyEnum()"));
        assert_eq!(default_for_ty("bits(8)").as_deref(), Some("undefined_bitvector(sizeof(8))"));
        assert_eq!(default_for_ty("bits('n + 1)").as_deref(), Some("undefined_bitvector(sizeof('n + 1))"));
        assert_eq!(default_for_ty("atom(5)").as_deref(), Some("sizeof(5)"));
        assert_eq!(default_for_ty("vector(4, int)"), None);
        assert_eq!(default_for_ty("range(1, 10)"), None);
        assert_eq!(default_for_ty("int -> int"), None);
    }

    #[test]
    fn preserves_unsupported_types_and_valspecs_and_visits_scopes() {
        let unknown = Stmt::Undefined { target: "v".into(), ty: "vector(4, int)".into() };
        let known = Stmt::Undefined { target: "x".into(), ty: "int".into() };
        let shard = Shard::new(0, vec![
            Def::new("sig", DefKind::Valspec, vec![known.clone()]),
            Def::new("f", DefKind::Function, vec![unknown.clone(), Stmt::LetTuple {
                bindings: vec![], rhs: "effect()".into(), body: vec![known.clone()],
            }]),
        ]);
        let out = Undefined.run(shard);
        assert_eq!(out.defs[0].body, vec![known]);
        assert_eq!(out.defs[1].body[0], unknown);
        let Stmt::LetTuple { body, .. } = &out.defs[1].body[1] else { panic!("scope") };
        assert_eq!(body, &[Stmt::Assign { lhs: "x".into(), rhs: "undefined_int()".into() }]);
    }
}
