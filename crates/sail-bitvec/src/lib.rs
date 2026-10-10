#![forbid(unsafe_code)]
//! Bitvector semantics for the pure-Rust Sail compiler (lane 22).
//!
//! Pure-Rust port of the bitvector parts of the OCaml Sail→Lean backend
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml`, lines ~309-313,
//! ~397-416, ~520-586, ~638-641, ~652-678, ~965-973):
//!
//! * [`expr`]: `E_vector` → `(BitVec.join1 [...])` for bitvector targets,
//!   `#v[...]` (reversed) otherwise; `E_vector_append` → `(x +++ y)`;
//! * [`pat`]: `P_vector` → `0b...` or concatenated `in_vector` parts,
//!   `P_vector_concat` → `[a,b,...]` (`doc_vector_concat` equivalent);
//! * [`lit`]: `L_bin` / `L_hex` rendering with and without `#w` suffix;
//! * [`bitfield`]: `bitfield.sail` ranges → `extractLsb` / `updateSubrange`;
//! * [`ops`]: every `bitvec_operation.sail` primitive with its Lean body.
//!
//! Width agreement is always checked ([`error`]); mismatches are reported
//! with the offending [`span`](span::Span) instead of being silently
//! truncated or padded.

pub mod bitfield;
pub mod error;
pub mod expr;
pub mod lit;
pub mod ops;
pub mod pat;
pub mod span;

pub use bitfield::{BitfieldDecl, Field, cr_type_fixture};
pub use error::{BitvecError, ErrorKind};
pub use expr::{Elem, render_e_lit, render_e_vector_append, render_e_vector_bitvec, render_e_vector_generic};
pub use lit::{Bit, Lit, check_lit_width};
pub use ops::BitvecOp;
pub use pat::{
    Pat, check_concat_width, is_bitvector_pat, match_keyword, render_pat, render_vector_concat,
};
pub use span::{FileId, Span};
