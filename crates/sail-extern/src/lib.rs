#![forbid(unsafe_code)]
//! `extern "lean"` FFI mappings (lane 25).
//!
//! Port of the OCaml Lean backend reference
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml`):
//! - `op_of_id` (lines ~652-680): `_lean_*` pseudo identifiers map to
//!   Lean infix/prefix operators instead of plain function calls.
//! - `doc_defs_rec` extern handling: a `DEF_fundef` with a `lean` extern
//!   mapping emits **no** `def` (call sites use the mapping instead);
//!   a `DEF_type` whose name is listed via `--lean-extern-type` is skipped.
//! - A function that is extern (has a `val` spec without a body) but has
//!   **no** `lean` mapping is a hard error, never a silent `sorry`.
//!   The error suggests providing the mapping via `ArmExtras`.
//!
//! Design: dependency-free, `std`-only, `Send + Sync` data types so later
//! lanes can share the table across threads. Peak RAM is O(one call).

use std::collections::BTreeSet;
use std::fmt;

/// 1-based source position for hard errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub col: u32,
}

impl Span {
    /// Create a span.
    #[must_use]
    pub const fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Failure modes. Missing externs are always hard errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Function is extern but carries no `lean` mapping.
    /// `suggestion` always points at `ArmExtras`.
    MissingExtern {
        /// Sail function name, e.g. `my_device_read`.
        sail_name: String,
        /// Where the call / spec was seen.
        span: Span,
        /// Full help text (contains `ArmExtras`).
        suggestion: String,
    },
    /// Wrong arity for an operator mapping, or other unsupported shape.
    Unsupported {
        /// Human-readable reason.
        msg: String,
        /// Source position.
        span: Span,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExtern {
                sail_name,
                span,
                suggestion,
            } => write!(
                f,
                "missing `lean` extern mapping for `{sail_name}` at {span}: {suggestion}"
            ),
            Self::Unsupported { msg, span } => {
                write!(f, "cannot translate extern call at {span}: {msg}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Operator class of a `lean` extern string.
///
/// Mirrors OCaml `` `Binop op | `Unop op | `NotOp `` from `op_of_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    /// Infix binary operator, e.g. `&&`, `+i`, `<<<`, `≤b`.
    BinOp(&'static str),
    /// Prefix unary operator, e.g. `!`, `2 ^`, `2 ^i`.
    UnOp(&'static str),
    /// Not an operator: emit as a plain Lean function application.
    NotOp,
}

impl Op {
    /// `true` for [`Op::BinOp`].
    #[must_use]
    pub const fn is_binop(self) -> bool {
        matches!(self, Self::BinOp(_))
    }

    /// `true` for [`Op::UnOp`].
    #[must_use]
    pub const fn is_unop(self) -> bool {
        matches!(self, Self::UnOp(_))
    }
}

/// Map a `lean` extern string to its operator class.
///
/// Direct port of `op_of_id` in `pretty_print_lean.ml` (lines ~652-680):
/// `_lean_and` → `&&`, `_lean_or` → `||`, `_lean_beq` → `==`,
/// `_lean_bne` → `!=`, `_lean_not` → `!`, `_lean_add` → `+`,
/// `_lean_addi` → `+i`, `_lean_sub` → `-`, `_lean_subi` → `-i`,
/// `_lean_mul` → `*`, `_lean_muli` → `*i`, `_lean_div` → `/`,
/// `_lean_app` → `+++`, `_lean_bvand` → `&&&`, `_lean_bvor` → `|||`,
/// `_lean_bvxor` → `^^^`, `_lean_shiftl` → `<<<`, `_lean_shiftr` → `>>>`,
/// `_lean_lt` → `<b`, `_lean_ge` → `≥b`, `_lean_le` → `≤b`,
/// `_lean_gt` → `>b`, `_lean_pow2` → `2 ^`, `_lean_pow2i` → `2 ^i`;
/// anything else → [`Op::NotOp`].
#[must_use]
pub fn op_of_extern(code: &str) -> Op {
    match code {
        "_lean_and" => Op::BinOp("&&"),
        "_lean_or" => Op::BinOp("||"),
        "_lean_beq" => Op::BinOp("=="),
        "_lean_bne" => Op::BinOp("!="),
        "_lean_not" => Op::UnOp("!"),
        "_lean_add" => Op::BinOp("+"),
        "_lean_addi" => Op::BinOp("+i"),
        "_lean_sub" => Op::BinOp("-"),
        "_lean_subi" => Op::BinOp("-i"),
        "_lean_mul" => Op::BinOp("*"),
        "_lean_muli" => Op::BinOp("*i"),
        "_lean_div" => Op::BinOp("/"),
        "_lean_app" => Op::BinOp("+++"),
        "_lean_bvand" => Op::BinOp("&&&"),
        "_lean_bvor" => Op::BinOp("|||"),
        "_lean_bvxor" => Op::BinOp("^^^"),
        "_lean_shiftl" => Op::BinOp("<<<"),
        "_lean_shiftr" => Op::BinOp(">>>"),
        "_lean_lt" => Op::BinOp("<b"),
        "_lean_ge" => Op::BinOp("≥b"),
        "_lean_le" => Op::BinOp("≤b"),
        "_lean_gt" => Op::BinOp(">b"),
        "_lean_pow2" => Op::UnOp("2 ^"),
        "_lean_pow2i" => Op::UnOp("2 ^i"),
        _ => Op::NotOp,
    }
}

/// Render `(lhs op rhs)` for a binary operator (always parenthesised,
/// matching the OCaml `parens res` behaviour).
#[must_use]
pub fn render_binop(op: &str, lhs: &str, rhs: &str) -> String {
    format!("({lhs} {op} {rhs})")
}

/// Render `(op arg)` for a unary operator (always parenthesised).
#[must_use]
pub fn render_unop(op: &str, arg: &str) -> String {
    format!("({op} {arg})")
}

/// Render a plain Lean application `(F a b ...)` with surrounding parens.
#[must_use]
pub fn render_direct_call(lean_fn: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("({lean_fn})")
    } else {
        format!("({} {})", lean_fn, args.join(" "))
    }
}

/// Render one call through the extern table.
///
/// - Operator externs require exact arity (2 for binop, 1 for unop),
///   otherwise [`Error::Unsupported`].
/// - Any other `lean` string is emitted as a direct call.
/// - `None` (no `lean` mapping for an extern function) is a hard
///   [`Error::MissingExtern`] pointing at `ArmExtras`.
///
/// `sail_name` is used only for the error path; `lean_code` is the value
/// of the `lean:` key in the Sail `val` spec, if present.
pub fn render_extern_call(
    sail_name: &str,
    lean_code: Option<&str>,
    args: &[&str],
    span: Span,
) -> Result<String, Error> {
    let code = match lean_code {
        Some(c) => c,
        None => return Err(missing_extern_error(sail_name, span)),
    };
    match op_of_extern(code) {
        Op::BinOp(op) => {
            if args.len() != 2 {
                return Err(Error::Unsupported {
                    msg: format!(
                        "operator extern `{code}` for `{sail_name}` needs 2 arguments, got {}",
                        args.len()
                    ),
                    span,
                });
            }
            Ok(render_binop(op, args[0], args[1]))
        }
        Op::UnOp(op) => {
            if args.len() != 1 {
                return Err(Error::Unsupported {
                    msg: format!(
                        "operator extern `{code}` for `{sail_name}` needs 1 argument, got {}",
                        args.len()
                    ),
                    span,
                });
            }
            Ok(render_unop(op, args[0]))
        }
        Op::NotOp => Ok(render_direct_call(code, args)),
    }
}

/// Build the hard error for a missing `lean` extern mapping.
///
/// The suggestion always names `ArmExtras` so model authors know where to
/// provide hand-written shims, and mentions `--lean-extern-type` for types.
#[must_use]
pub fn missing_extern_error(sail_name: &str, span: Span) -> Error {
    Error::MissingExtern {
        sail_name: sail_name.to_owned(),
        span,
        suggestion: format!(
            "add `lean: \"...\"` to the `val {sail_name}` spec, or provide `{sail_name}` in ArmExtras \
             (e.g. `ArmExtras.lean`); use `--lean-extern-type=TypeName` to skip a hand-provided type"
        ),
    }
}

/// Check an extern function declaration: `Ok(())` when a body exists
/// (`is_defined`) or a `lean` mapping is present; otherwise the hard
/// [`Error::MissingExtern`].
///
/// Mirrors `should_print_function_def` / `doc_defs_rec`: without a `lean`
/// mapping the OCaml backend would attempt to print a body that does not
/// exist, so we fail early with an actionable error instead.
pub fn check_extern_fundef(
    sail_name: &str,
    lean_code: Option<&str>,
    is_defined: bool,
    span: Span,
) -> Result<(), Error> {
    if is_defined || lean_code.is_some() {
        Ok(())
    } else {
        Err(missing_extern_error(sail_name, span))
    }
}

/// Whether to emit a `def` for a function: `false` when it has a `lean`
/// extern mapping (call sites inline the mapping instead).
///
/// Port of `should_print_function_def` /
/// `doc_defs_rec`'s `Env.is_extern ... "lean"` branch.
#[must_use]
pub const fn should_emit_fundef(has_lean_extern: bool) -> bool {
    !has_lean_extern
}

/// Set of type names passed via repeated `--lean-extern-type=Name` flags.
///
/// Port of `opt_extern_types` in `pretty_print_lean.ml`: a typedef whose
/// name is in this set is skipped (`doc_defs_rec` emits nothing for it)
/// because the type is hand-provided (e.g. by `ArmExtras` or the Sail
/// support library).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternTypes {
    names: BTreeSet<String>,
}

impl ExternTypes {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from any string iterator (one `--lean-extern-type` value each).
    pub fn from_names<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut out = Self::new();
        out.extend(iter);
        out
    }

    /// Record one `--lean-extern-type=Name` value. Empty values are ignored.
    pub fn add(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            self.names.insert(name.to_owned());
        }
    }

    /// Record many values.
    pub fn extend<I, S>(&mut self, iter: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for s in iter {
            self.add(&s.into());
        }
    }

    /// `true` when `name` was registered.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name.trim())
    }

    /// Number of registered names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// `true` when no names are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Iterate over registered names in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Whether to emit a typedef: `false` when listed via
    /// `--lean-extern-type` (type is hand-provided elsewhere).
    #[must_use]
    pub fn should_emit_typedef(&self, type_name: &str) -> bool {
        !self.contains(type_name)
    }
}
/// Parse one CLI occurrence `--lean-extern-type=Name` (or the bare `Name`
/// value after `--lean-extern-type`). Returns the type name, or `None`
/// for an empty value.
#[must_use]
pub fn parse_extern_type_flag(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_owned())
    }
}

/// Minimal model of a Sail `val` spec's backend mapping table.
///
/// `backends` holds `(backend, code)` pairs as written in
/// `val f = pure {lean: "...", _: "..."}`. Only the `lean` key matters here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValSpec {
    /// Sail-level function name.
    pub sail_name: String,
    /// `(backend, code)` mapping pairs.
    pub backends: Vec<(String, String)>,
}

impl ValSpec {
    /// Create a spec.
    #[must_use]
    pub fn new(sail_name: &str, backends: Vec<(&str, &str)>) -> Self {
        Self {
            sail_name: sail_name.to_owned(),
            backends: backends
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }

    /// The `lean:` mapping code, if present.
    #[must_use]
    pub fn lean_mapping(&self) -> Option<&str> {
        self.backends
            .iter()
            .find(|(k, _)| k == "lean")
            .map(|(_, v)| v.as_str())
    }

    /// `true` when a `lean` mapping exists.
    #[must_use]
    pub fn has_lean_mapping(&self) -> bool {
        self.lean_mapping().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAN: Span = Span::new(1, 1);

    #[test]
    fn full_op_table_matches_ocaml_reference() {
        let cases: &[(&str, Op)] = &[
            ("_lean_and", Op::BinOp("&&")),
            ("_lean_or", Op::BinOp("||")),
            ("_lean_beq", Op::BinOp("==")),
            ("_lean_bne", Op::BinOp("!=")),
            ("_lean_not", Op::UnOp("!")),
            ("_lean_add", Op::BinOp("+")),
            ("_lean_addi", Op::BinOp("+i")),
            ("_lean_sub", Op::BinOp("-")),
            ("_lean_subi", Op::BinOp("-i")),
            ("_lean_mul", Op::BinOp("*")),
            ("_lean_muli", Op::BinOp("*i")),
            ("_lean_div", Op::BinOp("/")),
            ("_lean_app", Op::BinOp("+++")),
            ("_lean_bvand", Op::BinOp("&&&")),
            ("_lean_bvor", Op::BinOp("|||")),
            ("_lean_bvxor", Op::BinOp("^^^")),
            ("_lean_shiftl", Op::BinOp("<<<")),
            ("_lean_shiftr", Op::BinOp(">>>")),
            ("_lean_lt", Op::BinOp("<b")),
            ("_lean_ge", Op::BinOp("≥b")),
            ("_lean_le", Op::BinOp("≤b")),
            ("_lean_gt", Op::BinOp(">b")),
            ("_lean_pow2", Op::UnOp("2 ^")),
            ("_lean_pow2i", Op::UnOp("2 ^i")),
        ];
        for (code, want) in cases {
            assert_eq!(op_of_extern(code), *want, "op table entry {code}");
        }
        // Direct (non-operator) externs from the Sail libs.
        for direct in [
            "Int.shiftl",
            "Int.tdiv",
            "Neg.neg",
            "BitVec.slice",
            "Vector.length",
            "String.append",
            "Max.max",
        ] {
            assert_eq!(op_of_extern(direct), Op::NotOp, "{direct} is NotOp");
        }
    }

    #[test]
    fn int_bool_externs_render_as_operators_like_expected_lean() {
        // Shapes taken from test/lean/extern.sail -> extern.expected.lean:
        // `add_int(5,4)` with lean `_lean_addi` becomes `(5 +i 4)`, etc.
        let cases: &[(&str, &str, &[&str], &str)] = &[
            ("add_int", "_lean_addi", &["5", "4"], "(5 +i 4)"),
            ("sub_int", "_lean_subi", &["5", "(Neg.neg 4)"], "(5 -i (Neg.neg 4))"),
            ("mult_int", "_lean_muli", &["5", "4"], "(5 *i 4)"),
            ("and_bool", "_lean_and", &["true", "false"], "(true && false)"),
            ("or_bool", "_lean_or", &["true", "false"], "(true || false)"),
            ("not_bool", "_lean_not", &["true"], "(! true)"),
            ("eq_int", "_lean_beq", &["5", "4"], "(5 == 4)"),
            ("eq_bool", "_lean_beq", &["true", "false"], "(true == false)"),
            ("neq_int", "_lean_bne", &["5", "4"], "(5 != 4)"),
            ("lt_int", "_lean_lt", &["5", "4"], "(5 <b 4)"),
            ("gt_int", "_lean_gt", &["5", "4"], "(5 >b 4)"),
            ("lteq_int", "_lean_le", &["5", "4"], "(5 ≤b 4)"),
            ("gteq_int", "_lean_ge", &["5", "4"], "(5 ≥b 4)"),
            ("pow2", "_lean_pow2i", &["n"], "(2 ^i n)"),
            ("add_vec", "_lean_add", &["a", "b"], "(a + b)"),
            ("sub_vec", "_lean_sub", &["a", "b"], "(a - b)"),
            ("and_vec", "_lean_bvand", &["a", "b"], "(a &&& b)"),
            ("or_vec", "_lean_bvor", &["a", "b"], "(a ||| b)"),
            ("xor_vec", "_lean_bvxor", &["a", "b"], "(a ^^^ b)"),
            ("shl", "_lean_shiftl", &["a", "i"], "(a <<< i)"),
            ("shr", "_lean_shiftr", &["a", "i"], "(a >>> i)"),
            ("concat", "_lean_app", &["a", "b"], "(a +++ b)"),
            ("div_nat", "_lean_div", &["a", "b"], "(a / b)"),
        ];
        for (sail, code, args, want) in cases {
            assert_eq!(
                render_extern_call(sail, Some(code), args, SPAN).unwrap(),
                (*want).to_owned(),
                "call {sail} via {code}"
            );
        }
    }

    #[test]
    fn bitvec_externs_render_as_direct_calls_like_expected_lean() {
        // Shapes taken from test/lean/extern_bitvec.sail -> expected.lean.
        let cases: &[(&str, &str, &[&str], &str)] = &[
            (
                "replicate_bits",
                "BitVec.replicateBits",
                &["0x1234#16", "4"],
                "(BitVec.replicateBits 0x1234#16 4)",
            ),
            ("slice", "BitVec.slice", &["x", "2", "4"], "(BitVec.slice x 2 4)"),
            ("length", "Vector.length", &["x"], "(Vector.length x)"),
            (
                "count_leading_zeros",
                "BitVec.countLeadingZeros",
                &["0x00FF0FF0#32"],
                "(BitVec.countLeadingZeros 0x00FF0FF0#32)",
            ),
            (
                "count_trailing_zeros",
                "BitVec.countTrailingZeros",
                &["0x00FF0FF0#32"],
                "(BitVec.countTrailingZeros 0x00FF0FF0#32)",
            ),
            (
                "sail_arith_shiftright",
                "BitVec.sshiftRight",
                &["0xDEADBEEF#32", "4"],
                "(BitVec.sshiftRight 0xDEADBEEF#32 4)",
            ),
            (
                "_shl32",
                "Int.shiftl",
                &["32", "1"],
                "(Int.shiftl 32 1)",
            ),
            ("tdiv_int", "Int.tdiv", &["5", "4"], "(Int.tdiv 5 4)"),
            ("negate_int", "Neg.neg", &["5"], "(Neg.neg 5)"),
            ("concat_str", "HAppend.hAppend", &["\"a\"", "\"b\""], "(HAppend.hAppend \"a\" \"b\")"),
            ("hex_str", "Int.toHex", &["123"], "(Int.toHex 123)"),
            ("max_int", "Max.max", &["5", "4"], "(Max.max 5 4)"),
        ];
        for (sail, code, args, want) in cases {
            assert_eq!(
                render_extern_call(sail, Some(code), args, SPAN).unwrap(),
                (*want).to_owned(),
                "call {sail} via {code}"
            );
        }
    }

    #[test]
    fn extern_types_flag_skips_typedef() {
        let mut t = ExternTypes::new();
        assert!(t.is_empty());
        assert!(t.should_emit_typedef("MyType"));
        t.add("MyType");
        t.add("  "); // empty values are ignored
        assert!(!t.should_emit_typedef("MyType"));
        assert!(t.should_emit_typedef("OtherType"));
        assert_eq!(t.len(), 1);

        let u = ExternTypes::from_names(["A", "B"]);
        assert!(!u.should_emit_typedef("A"));
        assert!(!u.should_emit_typedef("B"));
        assert!(u.should_emit_typedef("C"));
        assert_eq!(
            u.iter().collect::<Vec<_>>(),
            vec!["A", "B"],
            "sorted iteration"
        );

        assert_eq!(parse_extern_type_flag("ArmRegs"), Some("ArmRegs".to_owned()));
        assert_eq!(parse_extern_type_flag("  "), None);
    }

    #[test]
    fn fundef_emission_skips_lean_externs() {
        assert!(should_emit_fundef(false));
        assert!(!should_emit_fundef(true));
        // End-to-end with a ValSpec: extern without body is skipped iff
        // a lean mapping exists; otherwise it is a hard error.
        let with = ValSpec::new("add_int", vec![("lean", "_lean_addi")]);
        assert!(!should_emit_fundef(with.has_lean_mapping()));
        assert!(check_extern_fundef("add_int", with.lean_mapping(), false, SPAN).is_ok());

        let without = ValSpec::new("my_device_read", vec![("_", "my_device_read")]);
        assert!(should_emit_fundef(without.has_lean_mapping()));
        let err = check_extern_fundef("my_device_read", without.lean_mapping(), false, SPAN)
            .expect_err("missing lean mapping must fail");
        match &err {
            Error::MissingExtern { suggestion, .. } => {
                assert!(suggestion.contains("ArmExtras"), "suggest ArmExtras: {err}");
            }
            other => panic!("wrong error kind: {other:?}"),
        }
        // A locally defined function never needs a mapping.
        assert!(check_extern_fundef("local_fn", None, true, SPAN).is_ok());
    }

    #[test]
    fn missing_extern_is_hard_error_with_arm_extras_hint() {
        let err = render_extern_call("my_device_read", None, &["x"], Span::new(3, 7))
            .expect_err("must be a hard error");
        let msg = err.to_string();
        assert!(msg.contains("my_device_read"), "{msg}");
        assert!(msg.contains("3:7"), "{msg}");
        assert!(msg.contains("ArmExtras"), "{msg}");
        assert!(msg.contains("--lean-extern-type"), "{msg}");
        assert!(!msg.to_lowercase().contains("sorry"), "never suggest sorry: {msg}");
    }

    #[test]
    fn operator_arity_mismatch_is_unsupported_not_sorry() {
        let err = render_extern_call("and_bool", Some("_lean_and"), &["only_one"], SPAN)
            .expect_err("binop needs 2 args");
        assert!(matches!(err, Error::Unsupported { .. }));
        let err = render_extern_call("not_bool", Some("_lean_not"), &["a", "b"], SPAN)
            .expect_err("unop needs 1 arg");
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn val_spec_lean_mapping_lookup() {
        let s = ValSpec::new(
            "string_take",
            vec![("coq", "string_take"), ("lean", "String.takeStr")],
        );
        assert_eq!(s.lean_mapping(), Some("String.takeStr"));
        let s2 = ValSpec::new("no_lean", vec![("_", "no_lean")]);
        assert_eq!(s2.lean_mapping(), None);
    }
}
