//! Register-init synthesis, matching `add_register_init_function` in
//! `sail/src/lib/rewrites.ml`: the backend needs one function that runs
//! every register initializer, so the pass appends it to the definition list.
//!
//! String-[`Shard`](crate::ast::Shard) convention (documented, no guessing):
//! a [`DefKind::Register`](crate::ast::DefKind) def carries its initializer
//! as exactly one [`Stmt::Expr`](crate::ast::Stmt) body; any other body shape
//! (empty included) means "no initializer" and contributes nothing. The pass
//! appends
//! `Def::new("register_init", DefKind::Function, [Assign { lhs: reg, rhs:
//! init }, ..])` in shard order — unless no initialized register exists or a
//! `register_init` def is already present, in which case the shard is
//! returned unchanged (deterministic and idempotent, no duplicates on
//! re-runs). Shard-local by construction: only registers inside the same
//! shard are collected.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"add_register_init_function"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 27, zwischen
//!   `"top_sort_defs"` (26) und `"const_prop_mutrec"` (28).
//! - Trait-Impl-Name: `AddRegisterInitFunction` (diese Datei).
//!   `passes/mod.rs`: `pub mod add_register_init_function;` +
//!   `pub use add_register_init_function::AddRegisterInitFunction;`
//!   ergaenzen.
//!   `pipeline.rs`: `"add_register_init_function" =>
//!   Some(Arc::new(AddRegisterInitFunction))` in `make_implemented` und den
//!   Namen in `is_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use crate::ast::{Def, DefKind, Shard, Stmt};
use crate::pass::Pass;

/// Name of the synthesized initializer function (fixed, deterministic).
pub const REGISTER_INIT_FN: &str = "register_init";

#[derive(Clone, Copy, Debug, Default)]
pub struct AddRegisterInitFunction;

impl Pass for AddRegisterInitFunction {
    fn name(&self) -> &'static str {
        "add_register_init_function"
    }

    fn run(&self, mut shard: Shard) -> Shard {
        if shard.defs.iter().any(|d| d.name == REGISTER_INIT_FN) {
            return shard;
        }
        let mut inits = Vec::new();
        for def in &shard.defs {
            if def.kind == DefKind::Register {
                if let [Stmt::Expr(init)] = def.body.as_slice() {
                    inits.push((def.name.clone(), init.clone()));
                }
            }
        }
        if inits.is_empty() {
            return shard;
        }
        shard.defs.push(Def::new(
            REGISTER_INIT_FN,
            DefKind::Function,
            inits
                .into_iter()
                .map(|(lhs, rhs)| Stmt::Assign { lhs, rhs })
                .collect(),
        ));
        shard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(name: &str, init: Option<&str>) -> Def {
        Def::new(
            name,
            DefKind::Register,
            init.map(|e| vec![Stmt::Expr(e.into())]).unwrap_or_default(),
        )
    }

    #[test]
    fn appends_init_function_in_shard_order() {
        let shard = Shard::new(
            0,
            vec![
                Def::new("f", DefKind::Function, vec![Stmt::Expr("f()".into())]),
                register("r1", Some("0")),
                register("r2", None),
                register("r3", Some("undefined_bits(8)")),
            ],
        );
        let out = AddRegisterInitFunction.run(shard);
        assert_eq!(out.defs.len(), 5);
        let last = out.defs.last().expect("init fn appended");
        assert_eq!(last.name, REGISTER_INIT_FN);
        assert_eq!(last.kind, DefKind::Function);
        assert_eq!(last.body, vec![
            Stmt::Assign { lhs: "r1".into(), rhs: "0".into() },
            Stmt::Assign { lhs: "r3".into(), rhs: "undefined_bits(8)".into() },
        ]);
        // Idempotent: no second init function on re-run.
        let again = AddRegisterInitFunction.run(out.clone());
        assert_eq!(again, out);
    }

    #[test]
    fn leaves_shards_without_inits_or_with_init_fn_untouched() {
        let plain = Shard::new(0, vec![register("r", None)]);
        assert_eq!(AddRegisterInitFunction.run(plain.clone()), plain);
        assert_eq!(
            AddRegisterInitFunction.run(Shard::new(0, vec![])),
            Shard::new(0, vec![])
        );
        // A non-`Expr` register body carries no initializer by convention.
        let assign_reg = Shard::new(
            0,
            vec![Def::new(
                "r",
                DefKind::Register,
                vec![Stmt::Assign { lhs: "r".into(), rhs: "0".into() }],
            )],
        );
        assert_eq!(AddRegisterInitFunction.run(assign_reg.clone()), assign_reg);
        // Pre-existing init function is never duplicated.
        let existing = Shard::new(
            0,
            vec![
                register("r", Some("0")),
                Def::new(REGISTER_INIT_FN, DefKind::Function, vec![]),
            ],
        );
        assert_eq!(AddRegisterInitFunction.run(existing.clone()), existing);
    }
}
