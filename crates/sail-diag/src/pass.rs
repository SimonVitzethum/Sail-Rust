#![forbid(unsafe_code)]
//! Compiler passes that can emit a [`crate::SailError`].
//!
//! Every error carries the pass that raised it so `--explain` output and
//! exit-code mapping can point at the responsible lane spec.

use std::fmt;
use std::str::FromStr;

/// Error returned when parsing a [`Pass`] name fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParsePassError;

impl fmt::Display for ParsePassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown pass name")
    }
}

impl std::error::Error for ParsePassError {}

/// Pipeline stage that produced an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pass {
    /// Byte stream / token recovery (Lane 16).
    Lexer,
    /// Grammar / Menhir port (Lane 17).
    Parser,
    /// Name resolution (Lane 03).
    NameResolution,
    /// NExp / constraint solving (Lanes 04, 06).
    ConstraintCheck,
    /// Typechecker (Lane 05).
    Typecheck,
    /// Desugaring rewrites (Lane 07).
    Rewrites,
    /// Callgraph + effects (Lane 08).
    Effects,
    /// Monomorphisation / ANF lowering (Lane 09).
    Lowering,
    /// Lean pretty-printer (`pretty_print_lean.ml` port, Lane 10).
    LeanPrinter,
    /// Output sharding / incremental (Lane 11).
    Sharding,
    /// Lake project generation (Lane 12).
    LakeGen,
    /// CLI driver / anything not covered above (Lane 38).
    Driver,
}

impl Pass {
    /// Stable kebab-case name used in rendered diagnostics.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Pass::Lexer => "lexer",
            Pass::Parser => "parser",
            Pass::NameResolution => "name-resolution",
            Pass::ConstraintCheck => "constraint-check",
            Pass::Typecheck => "typecheck",
            Pass::Rewrites => "rewrites",
            Pass::Effects => "effects",
            Pass::Lowering => "lowering",
            Pass::LeanPrinter => "lean-printer",
            Pass::Sharding => "sharding",
            Pass::LakeGen => "lake-gen",
            Pass::Driver => "driver",
        }
    }

    /// Parse [`Pass::as_str`] output back; [`ParsePassError`] on unknown input.
    pub fn parse_name(s: &str) -> Result<Pass, ParsePassError> {
        s.parse()
    }
}

impl FromStr for Pass {
    type Err = ParsePassError;

    fn from_str(s: &str) -> Result<Pass, ParsePassError> {
        match s {
            "lexer" => Ok(Pass::Lexer),
            "parser" => Ok(Pass::Parser),
            "name-resolution" => Ok(Pass::NameResolution),
            "constraint-check" => Ok(Pass::ConstraintCheck),
            "typecheck" => Ok(Pass::Typecheck),
            "rewrites" => Ok(Pass::Rewrites),
            "effects" => Ok(Pass::Effects),
            "lowering" => Ok(Pass::Lowering),
            "lean-printer" => Ok(Pass::LeanPrinter),
            "sharding" => Ok(Pass::Sharding),
            "lake-gen" => Ok(Pass::LakeGen),
            "driver" => Ok(Pass::Driver),
            _ => Err(ParsePassError),
        }
    }
}

impl fmt::Display for Pass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_names_roundtrip() {
        let passes = [
            Pass::Lexer,
            Pass::Parser,
            Pass::NameResolution,
            Pass::ConstraintCheck,
            Pass::Typecheck,
            Pass::Rewrites,
            Pass::Effects,
            Pass::Lowering,
            Pass::LeanPrinter,
            Pass::Sharding,
            Pass::LakeGen,
            Pass::Driver,
        ];
        for p in passes {
            assert_eq!(p.as_str().parse(), Ok(p));
            assert_eq!(Pass::parse_name(p.as_str()), Ok(p));
            assert_eq!(p.to_string(), p.as_str());
        }
        assert_eq!("nope".parse::<Pass>(), Err(ParsePassError));
    }
}
