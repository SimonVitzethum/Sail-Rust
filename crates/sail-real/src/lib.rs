//! Real-number support for the Sail Lean backend.
//!
//! Pure-Rust port of the `--lean-real-numbers` handling in
//! `sail/src/sail_lean_backend/sail_plugin_lean.ml` plus the `L_real`
//! literal rendering in
//! `sail/src/sail_lean_backend/pretty_print_lean.ml`:
//!
//! ```ocaml
//! (* sail_plugin_lean.ml *)
//! let real_numbers_file =
//!   if !opt_lean_real_numbers then "/src/sail_lean_backend/Sail/Real.lean"
//!   else "/src/sail_lean_backend/Sail/FakeReal.lean"
//!
//! (* pretty_print_lean.ml *)
//! | L_real r -> utf8string (Q.to_string (Util.Rational.from_rocq r))
//! ```
//!
//! Design rules for this crate:
//!
//! * **Default is `FakeReal`.** Without the flag no mathlib download is
//!   triggered: the generated project copies `FakeReal.lean`
//!   (`abbrev real := Float`) and the `lakefile.toml` contains no
//!   `mathlib` require.
//! * **`--lean-real-numbers` is only a `lakefile` require flag.** With
//!   the flag the project copies `Real.lean` (`abbrev real := ℝ`, needs
//!   mathlib) and appends the mathlib `[[require]]` block.
//! * **`L_real` literals are rational strings.** Rendering matches
//!   `Q.to_string`: `num` alone when the denominator is 1, otherwise
//!   `num/den`, always normalized (sign on the numerator, gcd reduced).
//!
//! The mathlib URL / revision constants intentionally mirror the
//! Lane-12 `sail-lake` crate (`MATHLIB_GIT`, `DEFAULT_MATHLIB_REV`);
//! `sail-real` takes no dependency on it so this crate stays usable
//! standalone. Where Lane-12 types are available, use
//! [`RealOptions::from_flag`] with
//! `LakeOptions::real_numbers` / `LakeOptions::mathlib_rev`.
//!
//! Rendering is total, allocation only via [`String`], no `unsafe`.

#![forbid(unsafe_code)]

/// mathlib git URL used by the OCaml backend (mirrors Lane-12 `sail-lake::MATHLIB_GIT`).
pub const MATHLIB_GIT: &str = "https://github.com/leanprover-community/mathlib4-nightly-testing";

/// Pinned mathlib revision, only emitted when real numbers are enabled
/// (mirrors Lane-12 `sail-lake::DEFAULT_MATHLIB_REV` and OCaml `mathlib_version`).
pub const DEFAULT_MATHLIB_REV: &str = "v4.29.0";

/// Lean module copied for the default (no-mathlib) path.
pub const FAKE_REAL_MODULE: &str = "FakeReal";

/// Lean module copied for the `--lean-real-numbers` path.
pub const REAL_MODULE: &str = "Real";

/// Source path suffix selected without the flag (OCaml `real_numbers_file`, else-branch).
pub const FAKE_REAL_SUFFIX: &str = "/src/sail_lean_backend/Sail/FakeReal.lean";

/// Source path suffix selected with the flag (OCaml `real_numbers_file`, then-branch).
pub const REAL_SUFFIX: &str = "/src/sail_lean_backend/Sail/Real.lean";

/// Extern names provided by both `FakeReal.lean` and `Real.lean`
/// (kept in sync with the reference files; `undefined_real` included).
pub const REAL_EXTERNS: &[&str] = &[
    "neg_real",
    "mult_real",
    "sub_real",
    "add_real",
    "div_real",
    "sqrt_real",
    "abs_real",
    "round_down",
    "round_up",
    "to_real",
    "eq_real",
    "lt_real",
    "gt_real",
    "lteq_real",
    "gteq_real",
    "real_power",
    "print_real",
    "prerr_real",
    "random_real",
    "undefined_real",
];

/// Which real implementation the generated Lean project uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RealMode {
    /// Default: `FakeReal` (`abbrev real := Float`), no mathlib.
    #[default]
    Fake,
    /// `--lean-real-numbers`: mathlib `Real.lean` (`abbrev real := ℝ`).
    Mathlib,
}

impl RealMode {
    /// Select from the `--lean-real-numbers` flag value.
    #[must_use]
    pub fn from_flag(real_numbers: bool) -> Self {
        if real_numbers { Self::Mathlib } else { Self::Fake }
    }

    /// Lean module name copied into the project (`FakeReal` / `Real`).
    #[must_use]
    pub fn module_name(self) -> &'static str {
        match self {
            Self::Fake => FAKE_REAL_MODULE,
            Self::Mathlib => REAL_MODULE,
        }
    }

    /// Source suffix inside the Sail tree (OCaml `real_numbers_file`).
    #[must_use]
    pub fn import_suffix(self) -> &'static str {
        match self {
            Self::Fake => FAKE_REAL_SUFFIX,
            Self::Mathlib => REAL_SUFFIX,
        }
    }

    /// Whether this mode requires the mathlib `[[require]]` block.
    #[must_use]
    pub fn needs_mathlib(self) -> bool {
        matches!(self, Self::Mathlib)
    }
}

/// Flag-level options for real-number support.
///
/// Compatible with Lane-12 `sail-lake::LakeOptions`: pass
/// `real_numbers` / `mathlib_rev` through [`RealOptions::from_flag`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealOptions {
    /// Value of `--lean-real-numbers`.
    pub real_numbers: bool,
    /// mathlib rev used when [`RealOptions::real_numbers`] is set.
    pub mathlib_rev: String,
}

impl Default for RealOptions {
    fn default() -> Self {
        Self {
            real_numbers: false,
            mathlib_rev: DEFAULT_MATHLIB_REV.to_string(),
        }
    }
}

impl RealOptions {
    /// Build from a flag plus an explicit mathlib revision.
    #[must_use]
    pub fn from_flag(real_numbers: bool, mathlib_rev: impl Into<String>) -> Self {
        Self { real_numbers, mathlib_rev: mathlib_rev.into() }
    }

    /// Active [`RealMode`] for these options.
    #[must_use]
    pub fn mode(&self) -> RealMode {
        RealMode::from_flag(self.real_numbers)
    }

    /// mathlib `[[require]]` block, or `None` on the default FakeReal path.
    ///
    /// Byte-shape mirrors Lane-12 `sail-lake::render_lakefile`'s mathlib
    /// block (leading blank line, trailing newline per line).
    #[must_use]
    pub fn require_block(&self) -> Option<String> {
        if self.real_numbers {
            Some(render_mathlib_require(&self.mathlib_rev))
        } else {
            None
        }
    }
}

/// mathlib `[[require]]` block for `lakefile.toml`.
///
/// Only call when the `--lean-real-numbers` flag is set; the default
/// FakeReal path must not emit this (otherwise `lake update` would
/// download mathlib).
#[must_use]
pub fn render_mathlib_require(mathlib_rev: &str) -> String {
    let mut s = String::from("\n\n[[require]]\nname = \"mathlib\"\n");
    s.push_str(&format!("git = \"{MATHLIB_GIT}\"\n"));
    s.push_str(&format!("rev = \"{mathlib_rev}\"\n"));
    s
}

/// Whether a rendered `lakefile.toml` pulls mathlib.
#[must_use]
pub fn lakefile_has_mathlib(lakefile: &str) -> bool {
    lakefile.contains("name = \"mathlib\"")
}

/// Normalized rational `L_real` literal.
///
/// Invariant: `den > 0`, `gcd(|num|, den) == 1` (with `num == 0` stored
/// as `0/1`). Rendering matches OCaml `Q.to_string`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RealLiteral {
    /// Signed numerator.
    pub num: i128,
    /// Positive denominator.
    pub den: i128,
}

impl RealLiteral {
    /// Normalize `num/den`; returns `Err` when `den == 0`.
    pub fn new(num: i128, den: i128) -> Result<Self, String> {
        if den == 0 {
            return Err("real literal denominator must not be zero".to_string());
        }
        let (mut n, mut d) = (num, den);
        if d < 0 {
            n = n.checked_neg().ok_or_else(|| "real literal numerator overflow".to_string())?;
            d = d.checked_neg().ok_or_else(|| "real literal denominator overflow".to_string())?;
        }
        if n == 0 {
            return Ok(Self { num: 0, den: 1 });
        }
        let g = gcd(n.unsigned_abs(), d as u128);
        Ok(Self { num: n / g as i128, den: d / g as i128 })
    }

    /// Parse an `L_real` source form: `3`, `-3/2`, `1.5`, `-0.25`.
    ///
    /// Accepts an optional sign, digits with an optional `.` fraction,
    /// and an optional `/den` suffix. Whitespace around the parts is
    /// rejected (literals are single tokens).
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("empty real literal".to_string());
        }
        // Split off /den first.
        let (rat, den_part) = match t.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (t, None),
        };
        let (num, den) = match den_part {
            Some(d) => {
                let (scaled, scale) = parse_scaled_int(rat)?;
                let dv = parse_signed_den(d)?;
                let d = scale
                    .checked_mul(dv)
                    .ok_or_else(|| "real literal out of range".to_string())?;
                (scaled, d)
            }
            None => parse_scaled_int(rat)?,
        };
        Self::new(num, den)
    }

    /// Render as a Lean rational string (`Q.to_string` shape).
    #[must_use]
    pub fn render(self) -> String {
        render_real_parts(self.num, self.den)
    }
}

/// Render pre-normalized `num/den` in `Q.to_string` shape.
///
/// `den == 1` renders as just `num`, otherwise `num/den`.
#[must_use]
pub fn render_real_parts(num: i128, den: i128) -> String {
    if den == 1 { num.to_string() } else { format!("{num}/{den}") }
}

/// Render an `L_real` literal given as raw source text.
///
/// Parses (normalizing) then renders, so `"2.0"` becomes `"2"` and
/// `"-4/6"` becomes `"-2/3"`, exactly like the OCaml backend's
/// `Q.to_string (from_rocq r)` pipeline.
pub fn render_real_literal(text: &str) -> Result<String, String> {
    Ok(RealLiteral::parse(text)?.render())
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    if a == 0 { 1 } else { a }
}

/// Parse `"[-]digits[.digits]"` into `(scaled_num, scale)`.
fn parse_scaled_int(part: &str) -> Result<(i128, i128), String> {
    if part.is_empty() {
        return Err("empty real literal".to_string());
    }
    let (neg, rest) = match part.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, part.strip_prefix('+').unwrap_or(part)),
    };
    if rest.is_empty() || rest.chars().any(|c| !c.is_ascii_digit() && c != '.') {
        return Err(format!("invalid real literal `{part}`"));
    }
    if rest.chars().filter(|&c| c == '.').count() > 1 {
        return Err(format!("invalid real literal `{part}`"));
    }
    let (int_s, frac_s) = match rest.split_once('.') {
        Some((i, f)) => (i, f),
        None => (rest, ""),
    };
    if int_s.is_empty() && frac_s.is_empty() {
        return Err(format!("invalid real literal `{part}`"));
    }
    for s in [int_s, frac_s] {
        if !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("invalid real literal `{part}`"));
        }
    }
    let scale: i128 = 10i128.checked_pow(frac_s.len() as u32).ok_or_else(|| format!("real literal `{part}` out of range"))?;
    let int_v: i128 = if int_s.is_empty() { 0 } else { int_s.parse().map_err(|_| format!("real literal `{part}` out of range"))? };
    let frac_v: i128 = if frac_s.is_empty() { 0 } else { frac_s.parse().map_err(|_| format!("real literal `{part}` out of range"))? };
    let mut num = int_v.checked_mul(scale).and_then(|v| v.checked_add(frac_v)).ok_or_else(|| format!("real literal `{part}` out of range"))?;
    if neg {
        num = num.checked_neg().ok_or_else(|| format!("real literal `{part}` out of range"))?;
    }
    Ok((num, scale))
}

/// Parse the `/den` suffix, allowing an explicit sign (`4/-6` == `-2/3`).
fn parse_signed_den(part: &str) -> Result<i128, String> {
    let digits = part.strip_prefix('+').unwrap_or(part);
    let (neg, digits) = match digits.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, digits),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid real denominator `{part}`"));
    }
    let mut v: i128 = digits.parse().map_err(|_| format!("real denominator `{part}` out of range"))?;
    if v <= 0 {
        return Err(format!("real denominator `{part}` must be positive"));
    }
    if neg {
        v = v.checked_neg().ok_or_else(|| format!("real denominator `{part}` out of range"))?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_fakereal_without_mathlib() {
        assert_eq!(RealMode::default(), RealMode::Fake);
        assert_eq!(RealMode::default().module_name(), "FakeReal");
        assert!(!RealMode::default().needs_mathlib());
        let opts = RealOptions::default();
        assert!(!opts.real_numbers);
        assert_eq!(opts.require_block(), None);
    }

    #[test]
    fn flag_selects_module_and_suffix() {
        assert_eq!(RealMode::from_flag(false).module_name(), FAKE_REAL_MODULE);
        assert_eq!(RealMode::from_flag(true).module_name(), REAL_MODULE);
        assert_eq!(RealMode::from_flag(false).import_suffix(), FAKE_REAL_SUFFIX);
        assert_eq!(RealMode::from_flag(true).import_suffix(), REAL_SUFFIX);
        assert!(RealMode::from_flag(true).needs_mathlib());
    }

    #[test]
    fn mathlib_require_block_shape() {
        let block = render_mathlib_require(DEFAULT_MATHLIB_REV);
        assert!(block.contains("name = \"mathlib\""));
        assert!(block.contains(MATHLIB_GIT));
        assert!(block.contains(&format!("rev = \"{DEFAULT_MATHLIB_REV}\"")));
        assert!(lakefile_has_mathlib(&block));
        assert!(!lakefile_has_mathlib("name = \"out\"\n[[lean_lib]]\n"));
    }

    #[test]
    fn real_options_flag_rendering() {
        let off = RealOptions::default();
        assert_eq!(off.mode(), RealMode::Fake);
        assert_eq!(off.require_block(), None);
        let on = RealOptions::from_flag(true, DEFAULT_MATHLIB_REV);
        assert_eq!(on.mode(), RealMode::Mathlib);
        let block = on.require_block().expect("flag set");
        assert!(lakefile_has_mathlib(&block));
        assert!(block.contains(DEFAULT_MATHLIB_REV));
    }

    #[test]
    fn literal_roundtrip_rational() {
        for (input, expected) in [
            ("3/2", "3/2"),
            ("3", "3"),
            ("-3", "-3"),
            ("0", "0"),
            ("1.5", "3/2"),
            ("2.0", "2"),
            ("-0.25", "-1/4"),
            ("0.5", "1/2"),
            ("-4/6", "-2/3"),
            ("4/-6", "-2/3"),
            ("10/4", "5/2"),
            ("100/10", "10"),
            ("0.0", "0"),
        ] {
            let lit = RealLiteral::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(lit.render(), expected, "input {input}");
            // Rendered form re-parses to the same value (roundtrip).
            let again = RealLiteral::parse(&lit.render()).expect("re-parse");
            assert_eq!(again, lit, "roundtrip {input}");
        }
    }

    #[test]
    fn render_real_literal_helper_normalizes() {
        assert_eq!(render_real_literal("1.50").unwrap(), "3/2");
        assert_eq!(render_real_literal("7/1").unwrap(), "7");
        assert_eq!(render_real_literal("-0").unwrap(), "0");
    }

    #[test]
    fn literal_errors() {
        for bad in ["", "abc", "1/0", "1/", "/2", "1.2.3", "1 /2", "--3", "1/a"] {
            assert!(RealLiteral::parse(bad).is_err(), "expected error for `{bad}`");
        }
        assert!(RealLiteral::new(1, 0).is_err());
    }

    #[test]
    fn real_externs_cover_reference_files() {
        for name in ["add_real", "sqrt_real", "round_down", "to_real", "eq_real", "real_power", "undefined_real"] {
            assert!(REAL_EXTERNS.contains(&name), "missing {name}");
        }
    }
}
