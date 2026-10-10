#![forbid(unsafe_code)]
//! Int/Nat/Range/Atom mapping, integer operator table, and `E_for`
//! lowering for the Lean backend.
//!
//! Pure port of three fragments of
//! `sail/src/sail_lean_backend/pretty_print_lean.ml`:
//!
//! - type mapping (`doc_typ`): `int` -> `Int`, `nat` -> `Nat`,
//!   `atom(x)` -> `Nat`/`Int` via [`provably_nneg`],
//!   `range(low, _)` -> `Nat`/`Int` via [`provably_nneg`] on `low`;
//! - operator table (`op_of_id`): `_lean_addi` -> `+i`,
//!   `_lean_subi` -> `-i`, `_lean_muli` -> `*i`, `_lean_div` -> `/`,
//!   plus the bitvector/boolean entries of the same table;
//! - loop heads (`E_for` case): `Ord_inc` -> `foreach_Z_up`,
//!   `Ord_dec` -> `foreach_Z_down`, bracket `[from:to:step]i` with a
//!   negated step for `Ord_dec`, `Id.run` prefix for pure bodies.

pub mod conv;
pub mod loops;
pub mod nexp;
pub mod ops;
pub mod types;

pub use loops::{ForLoop, Order};
pub use nexp::{provably_neg_in, provably_nneg, provably_nneg_in, provably_nonpos_in, provably_pos_in, Facts, NExp};
pub use ops::{op_of_extern, render_binop, render_unop, unop_prefix, LeanOp};
pub use types::{IntKind, NumType};
