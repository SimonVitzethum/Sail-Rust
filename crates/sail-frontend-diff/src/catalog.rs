#![forbid(unsafe_code)]
//! Case catalog: the `sail/test/lean/*.sail` fixtures pinned with tiers,
//! focus, and notes (Sail 0.20.3, Oct 2026: 36 files).
//!
//! Tiers mirror the `sail-oracle` suite cap (`DEFAULT_MAX_FILE_BYTES =
//! 8192`): files above the cap are [`Tier::Nightly`]. [`Tier::Smoke`] is the
//! fast subset for pre-commit / low-RAM gates. Byte sizes were read off the
//! checkout; the `catalog_sizes_current` test pins them so fixture drift
//! (upstream renames, regenerated files) fails loudly instead of silently
//! changing coverage.

/// Execution tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Fast gate: tiny files covering the core def kinds.
    Smoke,
    /// Default suite: everything at or below the suite size cap.
    Standard,
    /// Above the cap (ARM-scale): nightly-only, like `sail-oracle`.
    Nightly,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Standard => "standard",
            Self::Nightly => "nightly",
        }
    }
}

/// What the harness checks for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focus {
    /// Parse-tree comparison + type-judgment sampling.
    ParseAndTy,
    /// Parse-tree comparison only (e.g. oracle typecheck is *expected* to
    /// fail, so there is no `--ddump-tc-ast` to sample).
    ParseOnly,
}

/// One fixture case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Case {
    /// File stem in `sail/test/lean` (e.g. `trivial` for `trivial.sail`).
    pub stem: &'static str,
    /// Fixture size in bytes (pinned; see module docs).
    pub bytes: u64,
    pub tier: Tier,
    pub focus: Focus,
    /// Why this tier/focus; exercised constructs.
    pub notes: &'static str,
}

/// Suite size cap shared with `sail-oracle` (files above are nightly-only).
pub const MAX_FILE_BYTES: u64 = 8192;

/// All 36 `sail/test/lean` fixtures, sorted by stem.
pub const CASES: &[Case] = &[
    Case { stem: "SailTinyArm", bytes: 18171, tier: Tier::Nightly, focus: Focus::ParseAndTy, notes: "tiny ARM model; above suite cap" },
    Case { stem: "SailTinyArmV2", bytes: 36899, tier: Tier::Nightly, focus: Focus::ParseAndTy, notes: "tiny ARMv2 model; above suite cap" },
    Case { stem: "append", bytes: 543, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "list append; recursion + polymorphism" },
    Case { stem: "atom_bool", bytes: 61, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "minimal val/function pair" },
    Case { stem: "bitfield", bytes: 168, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "bitfield type decl" },
    Case { stem: "bitvec_operation", bytes: 1992, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "bitvector ops; solver-heavy schemes" },
    Case { stem: "early_return", bytes: 3865, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "early return / exceptions" },
    Case { stem: "enum", bytes: 64, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "minimal enum decl" },
    Case { stem: "errors", bytes: 226, tier: Tier::Standard, focus: Focus::ParseOnly, notes: "oracle typecheck EXPECTED to fail; tc-dump absent" },
    Case { stem: "extern", bytes: 3185, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "extern decls; needs extern config" },
    Case { stem: "extern_bitvec", bytes: 750, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "extern bitvector decls" },
    Case { stem: "guardedmatch", bytes: 606, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "guarded match clauses; has sibling .json" },
    Case { stem: "implicit", bytes: 435, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "implicit arguments / typquant" },
    Case { stem: "ite", bytes: 438, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "if-then-else typing" },
    Case { stem: "let", bytes: 582, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "top-level let + bitvector lets" },
    Case { stem: "loop", bytes: 3017, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "for/while loops + measures" },
    Case { stem: "map_tactics", bytes: 15724, tier: Tier::Nightly, focus: Focus::ParseAndTy, notes: "large mapping file; above suite cap" },
    Case { stem: "mapping", bytes: 1795, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "mapping/enum bidirectional decls" },
    Case { stem: "match", bytes: 321, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "pattern match arms" },
    Case { stem: "match_bv", bytes: 1091, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "bitvector pattern match" },
    Case { stem: "option", bytes: 297, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "option type + union ctors" },
    Case { stem: "range", bytes: 449, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "range constraints / nexp" },
    Case { stem: "register_vector", bytes: 1771, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "register vector decls" },
    Case { stem: "registers", bytes: 450, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "register decls across int/nat/bool/bit" },
    Case { stem: "riscv_duopod", bytes: 6698, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "largest standard file; near cap" },
    Case { stem: "string", bytes: 325, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "string literals / print_effect" },
    Case { stem: "struct", bytes: 1364, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "struct decls + accessors" },
    Case { stem: "struct_of_enum", bytes: 102, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "struct-of-enum pattern" },
    Case { stem: "termination", bytes: 637, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "termination measures" },
    Case { stem: "trivial", bytes: 49, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "smallest file; single function" },
    Case { stem: "tuples", bytes: 236, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "tuple types/exps" },
    Case { stem: "type_kid", bytes: 95, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "kind variables (K_int)" },
    Case { stem: "typedef", bytes: 492, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "type abbreviations" },
    Case { stem: "typquant", bytes: 1598, tier: Tier::Standard, focus: Focus::ParseAndTy, notes: "forall/constraint schemes; prime ty-sample target" },
    Case { stem: "undefined", bytes: 287, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "undefined / dwt semantics" },
    Case { stem: "union", bytes: 468, tier: Tier::Smoke, focus: Focus::ParseAndTy, notes: "union decl + ctors" },
];

/// Find a case by file stem.
pub fn find(stem: &str) -> Option<&'static Case> {
    CASES.iter().find(|c| c.stem == stem)
}

/// All cases of one tier, in catalog order.
pub fn by_tier(tier: Tier) -> Vec<&'static Case> {
    CASES.iter().filter(|c| c.tier == tier).collect()
}

/// Total fixture bytes for one tier (RAM-planning input for the Rig run).
pub fn tier_bytes(tier: Tier) -> u64 {
    by_tier(tier).iter().map(|c| c.bytes).sum()
}
