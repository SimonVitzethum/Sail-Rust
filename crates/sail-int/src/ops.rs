#![forbid(unsafe_code)]
//! Lean extern-name operator table.
//!
//! Ports `op_of_id` (plus `unnop_of_id`) from
//! `sail/src/sail_lean_backend/pretty_print_lean.ml`: Sail extern names
//! such as `_lean_addi` map to Lean operators such as `+i`. Integer
//! arithmetic uses the `i`-suffixed forms (`+i`, `-i`, `*i`) from
//! `sail/lib/arith.sail`; the unsuffixed forms (`+`, `-`, `*`) are the
//! bitvector externs from `sail/lib/vector.sail`; `_lean_div` is `/`.

/// Classification of a Lean extern name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeanOp {
    /// Binary operator, e.g. `+i`.
    Binop(&'static str),
    /// Prefix operator, e.g. `2 ^`.
    Unop(&'static str),
    /// Not a known operator; emit as a regular application.
    NotOp,
}

impl LeanOp {
    /// `true` for [`LeanOp::Binop`].
    pub fn is_binop(self) -> bool {
        matches!(self, LeanOp::Binop(_))
    }

    /// `true` for [`LeanOp::Unop`].
    pub fn is_unop(self) -> bool {
        matches!(self, LeanOp::Unop(_))
    }
}

/// Map a Lean extern name (e.g. `Some("_lean_addi")`) to its operator.
///
/// `None` (no Lean extern) maps to [`LeanOp::NotOp`].
pub fn op_of_extern(lean_extern: Option<&str>) -> LeanOp {
    match lean_extern {
        Some("_lean_and") => LeanOp::Binop("&&"),
        Some("_lean_or") => LeanOp::Binop("||"),
        Some("_lean_beq") => LeanOp::Binop("=="),
        Some("_lean_bne") => LeanOp::Binop("!="),
        Some("_lean_not") => LeanOp::Unop("!"),
        Some("_lean_add") => LeanOp::Binop("+"),
        Some("_lean_addi") => LeanOp::Binop("+i"),
        Some("_lean_sub") => LeanOp::Binop("-"),
        Some("_lean_subi") => LeanOp::Binop("-i"),
        Some("_lean_mul") => LeanOp::Binop("*"),
        Some("_lean_muli") => LeanOp::Binop("*i"),
        Some("_lean_div") => LeanOp::Binop("/"),
        Some("_lean_app") => LeanOp::Binop("+++"),
        Some("_lean_bvand") => LeanOp::Binop("&&&"),
        Some("_lean_bvor") => LeanOp::Binop("|||"),
        Some("_lean_bvxor") => LeanOp::Binop("^^^"),
        Some("_lean_shiftl") => LeanOp::Binop("<<<"),
        Some("_lean_shiftr") => LeanOp::Binop(">>>"),
        Some("_lean_lt") => LeanOp::Binop("<b"),
        Some("_lean_ge") => LeanOp::Binop("≥b"),
        Some("_lean_le") => LeanOp::Binop("≤b"),
        Some("_lean_gt") => LeanOp::Binop(">b"),
        Some("_lean_pow2") => LeanOp::Unop("2 ^"),
        Some("_lean_pow2i") => LeanOp::Unop("2 ^i"),
        _ => LeanOp::NotOp,
    }
}

/// Prefix form for `_lean_pow2` (`unnop_of_id` in the backend).
pub fn unop_prefix(lean_extern: Option<&str>) -> Option<&'static str> {
    match lean_extern {
        Some("_lean_pow2") => Some("2 ^ "),
        _ => None,
    }
}

/// Render `(lhs op rhs)`; the backend always parenthesizes operator
/// applications.
pub fn render_binop(op: &str, lhs: &str, rhs: &str) -> String {
    format!("({lhs} {op} {rhs})")
}

/// Render `(op operand)` for prefix operators.
pub fn render_unop(op: &str, operand: &str) -> String {
    format!("({op} {operand})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_ops() {
        assert_eq!(op_of_extern(Some("_lean_addi")), LeanOp::Binop("+i"));
        assert_eq!(op_of_extern(Some("_lean_subi")), LeanOp::Binop("-i"));
        assert_eq!(op_of_extern(Some("_lean_muli")), LeanOp::Binop("*i"));
        assert_eq!(op_of_extern(Some("_lean_div")), LeanOp::Binop("/"));
    }

    #[test]
    fn nat_bitvec_ops() {
        assert_eq!(op_of_extern(Some("_lean_add")), LeanOp::Binop("+"));
        assert_eq!(op_of_extern(Some("_lean_sub")), LeanOp::Binop("-"));
        assert_eq!(op_of_extern(Some("_lean_mul")), LeanOp::Binop("*"));
    }

    #[test]
    fn unknown_is_not_op() {
        assert_eq!(op_of_extern(None), LeanOp::NotOp);
        assert_eq!(op_of_extern(Some("Int.tdiv")), LeanOp::NotOp);
        assert_eq!(op_of_extern(Some("_lean_pow2")), LeanOp::Unop("2 ^"));
        assert_eq!(op_of_extern(Some("_lean_pow2i")), LeanOp::Unop("2 ^i"));
        assert_eq!(unop_prefix(Some("_lean_pow2")), Some("2 ^ "));
        assert_eq!(unop_prefix(Some("_lean_addi")), None);
    }

    #[test]
    fn rendering_matches_backend() {
        // `loop.expected.lean`: `(res +i 1)`, `(m -i 1)`, `(res *i mult)`.
        assert_eq!(render_binop("+i", "res", "1"), "(res +i 1)");
        assert_eq!(render_binop("-i", "m", "1"), "(m -i 1)");
        assert_eq!(render_binop("*i", "res", "mult"), "(res *i mult)");
        assert_eq!(render_unop("2 ^", "n"), "(2 ^ n)");
    }
}
