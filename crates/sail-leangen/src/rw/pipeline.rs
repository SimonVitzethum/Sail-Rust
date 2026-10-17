#![forbid(unsafe_code)]
//! The per-definition part of `lean_rewrites`, in the OCaml order.

use sail_ast::{Def, Exp, Typ};

use super::early_return::early_return_body;
use super::lower::{
    exp_lift_assign, internal_lets, remove_blocks, remove_e_assign, remove_superfluous_letbinds,
    remove_superfluous_returns, bit_lists_to_lits,
};
use super::util::{map_def_bodies, Rw};

/// Rewrite one function/let body. `ret` is the declared return type.
pub fn rewrite_body(rw: &mut Rw<'_>, ret: Option<&Typ>, e: Exp) -> Result<Exp, String> {
    let e = exp_lift_assign(rw, e)?;
    let e = early_return_body(rw, ret, e);
    let e = remove_blocks(rw, e);
    let e = remove_e_assign(rw, e)?;
    let e = internal_lets(rw, e);
    let e = remove_superfluous_letbinds(rw, e);
    let e = remove_superfluous_returns(rw, e);
    let e = bit_lists_to_lits(rw, e);
    Ok(e)
}

/// Rewrite every body of `def` in place.
pub fn rewrite_def(rw: &mut Rw<'_>, ret: Option<&Typ>, def: &mut Def) -> Result<(), String> {
    let mut failure = None;
    map_def_bodies(def, &mut |e| match rewrite_body(rw, ret, e.clone()) {
        Ok(out) => out,
        Err(m) => {
            failure.get_or_insert(m);
            e
        }
    });
    match failure {
        Some(m) => Err(m),
        None => Ok(()),
    }
}
