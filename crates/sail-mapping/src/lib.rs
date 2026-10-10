//! Lane 20 — mapping-encdec.
//!
//! Pure-Rust port of the Sail mapping elaboration used by the Lean backend:
//!
//! - [`realize`]: `realize_mappings` from `sail/src/lib/rewrites.ml`
//!   (mapping definition `{ name, type, forward/backward clauses }`
//!   elaborated into four functions `_forwards`, `_backwards`,
//!   `_forwards_matches`, `_backwards_matches`).
//! - [`patterns`]: `mapping_patterns` from `sail/src/lib/mappings.ml`
//!   (mapping applications used as match patterns are elaborated into
//!   guarded nested matches on `mapping_forwards` / `mapping_backwards`
//!   plus the `_matches` guards).
//! - [`untranslatable`]: `untranslatable_mapping` from
//!   `sail/src/sail_lean_backend/pretty_print_lean.ml` (lines 1188–1214):
//!   `_forward`/`_backwards` bodies that bottom out in `exit` (string
//!   concatenation on the LHS, cf. Sail issue #260) are rendered as
//!   `throw Error.Exit`.
//! - [`encdec`]: sketch of the encdec/execute split — the realized
//!   mapping functions form the encode/decode (`encdec`) shard, everything
//!   else stays in the `execute` shard (cf. `("split", [String_arg
//!   "execute"])` in `sail_plugin_lean.ml`).
//!
//! Fixture: `sail/test/lean/mapping.sail` (71 lines) with expectations from
//! `sail/test/lean/mapping.expected.lean`. The fixture is *not* executed via
//! OCaml; its mappings and expected outcomes are encoded as Rust test cases.
//!
//! ```text
//! mapping.sail lines  9-14  size_bits   (bidir enum <-> bits)
//! mapping.sail lines 16-21  size_bits2  (typed bidir, same shape)
//! mapping.sail lines 23-29  size_bits3  (bidir + forwards/backwards-only clauses)
//! mapping.sail lines 31-34  ta_flag     (string <-> bits(1))
//! mapping.sail lines 50      hex_bits_2  (mapping application inside a clause)
//! mapping.sail lines 60-63  vtype_assembly   (string-append LHS -> forwards untranslatable)
//! mapping.sail lines 65-67  vtype_assembly2  (mapping-pattern LHS, translatable)
//! mapping.sail lines 69-71  vtype_assembly3  (string-append LHS -> forwards untranslatable)
//! ```

#![forbid(unsafe_code)]

pub mod ast;
pub mod conv;
pub mod encdec;
pub mod patterns;
pub mod realize;
pub mod untranslatable;

pub use ast::{Arm, Exp, FunDef, MappingClause, MappingDef, Mpat, Pat, ValSpec};
pub use encdec::{Shard, split_encdec_execute};
pub use patterns::{Direction, ElabPlan, direction_from_attr, mapping_call, mapping_guard};
pub use realize::{RealizedMapping, realize_mapping, realize_overload, realize_val_spec};
pub use untranslatable::{is_mapping_helper_exempt, render_funcl_body, untranslatable_mapping};
