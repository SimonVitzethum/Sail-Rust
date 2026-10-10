#![forbid(unsafe_code)]
//! Sail bitvector primitives from `bitvec_operation.sail` as a coded fixture.
//!
//! Each [`BitvecOp`] pairs a Sail function (`sail_sign_extend`, `append`,
//! `x + y`, ...) with the exact Lean body the backend emits
//! (`bitvec_operation.expected.lean`, lines ~154-208). This pins the
//! operator mapping (`op_of_id`: `_lean_app` → `+++`, `_lean_bvand` → `&&&`,
//! ...) without depending on a parser or typechecker lane.

use core::fmt;

/// One Sail-level bitvector operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BitvecOp {
    Eq,
    Neq,
    Len,
    SignExtend,
    ZeroExtend,
    Truncate,
    TruncateLsb,
    Append,
    Add,
    Sub,
    Not,
    And,
    Or,
    Xor,
    Unsigned,
    Signed,
    Access,
    PlusInt,
}

impl BitvecOp {
    /// Sail function name in `bitvec_operation.sail`.
    #[must_use]
    pub fn sail_name(self) -> &'static str {
        match self {
            BitvecOp::Eq => "bitvector_eq",
            BitvecOp::Neq => "bitvector_neq",
            BitvecOp::Len => "bitvector_len",
            BitvecOp::SignExtend => "bitvector_sign_extend",
            BitvecOp::ZeroExtend => "bitvector_zero_extend",
            BitvecOp::Truncate => "bitvector_truncate",
            BitvecOp::TruncateLsb => "bitvector_truncateLSB",
            BitvecOp::Append => "bitvector_append",
            BitvecOp::Add => "bitvector_add",
            BitvecOp::Sub => "bitvector_sub",
            BitvecOp::Not => "bitvector_not",
            BitvecOp::And => "bitvector_and",
            BitvecOp::Or => "bitvector_or",
            BitvecOp::Xor => "bitvector_xor",
            BitvecOp::Unsigned => "bitvector_unsigned",
            BitvecOp::Signed => "bitvector_signed",
            BitvecOp::Access => "bitvector_access'",
            BitvecOp::PlusInt => "bitvector_plus_int",
        }
    }

    /// All operations covered by `bitvec_operation.sail`, in file order.
    pub const ALL: [BitvecOp; 18] = [
        BitvecOp::Eq,
        BitvecOp::Neq,
        BitvecOp::Len,
        BitvecOp::SignExtend,
        BitvecOp::ZeroExtend,
        BitvecOp::Truncate,
        BitvecOp::TruncateLsb,
        BitvecOp::Append,
        BitvecOp::Add,
        BitvecOp::Sub,
        BitvecOp::Not,
        BitvecOp::And,
        BitvecOp::Or,
        BitvecOp::Xor,
        BitvecOp::Unsigned,
        BitvecOp::Signed,
        BitvecOp::Access,
        BitvecOp::PlusInt,
    ];

    /// Lean body for fully applied operands, exactly as emitted in
    /// `bitvec_operation.expected.lean`.
    #[must_use]
    pub fn lean_body(self, args: &[&str]) -> String {
        match self {
            BitvecOp::Eq => format!("({} == {})", args[0], args[1]),
            BitvecOp::Neq => format!("({} != {})", args[0], args[1]),
            BitvecOp::Len => format!("(Sail.BitVec.length {})", args[0]),
            BitvecOp::SignExtend => format!("(Sail.BitVec.signExtend {} {})", args[0], args[1]),
            BitvecOp::ZeroExtend => format!("(Sail.BitVec.zeroExtend {} {})", args[0], args[1]),
            BitvecOp::Truncate => format!("(Sail.BitVec.truncate {} {})", args[0], args[1]),
            BitvecOp::TruncateLsb => format!("(Sail.BitVec.truncateLsb {} {})", args[0], args[1]),
            BitvecOp::Append => format!("({} +++ {})", args[0], args[1]),
            BitvecOp::Add => format!("({} + {})", args[0], args[1]),
            BitvecOp::Sub => format!("({} - {})", args[0], args[1]),
            BitvecOp::Not => format!("(Complement.complement {})", args[0]),
            BitvecOp::And => format!("({} &&& {})", args[0], args[1]),
            BitvecOp::Or => format!("({} ||| {})", args[0], args[1]),
            BitvecOp::Xor => format!("({} ^^^ {})", args[0], args[1]),
            BitvecOp::Unsigned => format!("(BitVec.toNatInt {})", args[0]),
            BitvecOp::Signed => format!("(BitVec.toInt {})", args[0]),
            BitvecOp::Access => format!("(BitVec.access {} {})", args[0], args[1]),
            BitvecOp::PlusInt => format!("(BitVec.addInt {} {})", args[0], args[1]),
        }
    }
}

impl fmt::Display for BitvecOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sail_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eighteen_ops_covered_in_order() {
        assert_eq!(BitvecOp::ALL.len(), 18);
        assert_eq!(BitvecOp::ALL[0], BitvecOp::Eq);
        assert_eq!(BitvecOp::ALL[17], BitvecOp::PlusInt);
    }

    #[test]
    fn lean_bodies_match_expected_lean() {
        // bitvec_operation.expected.lean, lines 154-208
        let cases = [
            (BitvecOp::Eq, vec!["x", "y"], "(x == y)"),
            (BitvecOp::Neq, vec!["x", "y"], "(x != y)"),
            (BitvecOp::Len, vec!["x"], "(Sail.BitVec.length x)"),
            (
                BitvecOp::SignExtend,
                vec!["x", "32"],
                "(Sail.BitVec.signExtend x 32)",
            ),
            (
                BitvecOp::ZeroExtend,
                vec!["x", "32"],
                "(Sail.BitVec.zeroExtend x 32)",
            ),
            (
                BitvecOp::Truncate,
                vec!["x", "16"],
                "(Sail.BitVec.truncate x 16)",
            ),
            (
                BitvecOp::TruncateLsb,
                vec!["x", "16"],
                "(Sail.BitVec.truncateLsb x 16)",
            ),
            (BitvecOp::Append, vec!["x", "y"], "(x +++ y)"),
            (BitvecOp::Add, vec!["x", "y"], "(x + y)"),
            (BitvecOp::Sub, vec!["x", "y"], "(x - y)"),
            (BitvecOp::Not, vec!["x"], "(Complement.complement x)"),
            (BitvecOp::And, vec!["x", "y"], "(x &&& y)"),
            (BitvecOp::Or, vec!["x", "y"], "(x ||| y)"),
            (BitvecOp::Xor, vec!["x", "y"], "(x ^^^ y)"),
            (BitvecOp::Unsigned, vec!["x"], "(BitVec.toNatInt x)"),
            (BitvecOp::Signed, vec!["x"], "(BitVec.toInt x)"),
            (BitvecOp::Access, vec!["x", "i"], "(BitVec.access x i)"),
            (BitvecOp::PlusInt, vec!["x", "i"], "(BitVec.addInt x i)"),
        ];
        for (op, args, expected) in cases {
            assert_eq!(op.lean_body(&args), expected, "{op}");
        }
    }
}
