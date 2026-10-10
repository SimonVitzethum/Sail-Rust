//! `mapping_patterns` port (`sail/src/lib/mappings.ml`).
//!
//! After `realize_mappings`, mapping *applications* can still occur as
//! patterns, e.g. `mapping.sail` line 50/66:
//!
//! ```sail
//! mapping hex_bits_2 : bits(2) <-> string = { hex_bits(2, s) <-> s }
//! ```
//!
//! A match arm whose pattern contains a mapping application
//!
//! ```sail
//! match x { hex_bits_2(ta @ sew) => e, ... }
//! ```
//!
//! is elaborated (see the header comment of `mappings.ml`) into a guarded
//! nested match:
//!
//! ```sail
//! let y = x in
//! match y {
//!   z if hex_bits_2_backwards_matches(z) =>
//!     match hex_bits_2_backwards(z) { (ta @ sew) => e, _ => ... },
//!   ... rest ...
//! }
//! ```
//!
//! This module ports the *detection* half faithfully (`extract_mapping_pats`,
//! `split_arms`, direction attributes, `mapping_forwards` /
//! `mapping_forwards_matches` naming) and represents the elaborated shape as
//! an [`ElabPlan`] the Lean printer sketch can consume. The full
//! option-wrapping / return-position variants are documented on
//! [`ElabPlan::describe`] rather than re-implemented: they belong to the
//! type-checked rewrite, which needs the Sail type checker (out of scope for
//! this lane; the plan carries everything the printer needs).

#![forbid(unsafe_code)]

use crate::ast::{Exp, Pat};
use std::fmt;

/// Mapping direction from the `forwards` / `backwards` pattern attribute
/// (`mapping_direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forwards,
    Backwards,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Forwards => "forwards",
            Direction::Backwards => "backwards",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse the direction attribute list of a mapping pattern.
///
/// Mirrors `mapping_direction`: exactly one of `forwards` / `backwards` must
/// be present.
pub fn direction_from_attr(attrs: &[String]) -> Option<Direction> {
    let fwd = attrs.iter().any(|a| a == "forwards");
    let bwd = attrs.iter().any(|a| a == "backwards");
    match (fwd, bwd) {
        (true, false) => Some(Direction::Forwards),
        (false, true) => Some(Direction::Backwards),
        _ => None,
    }
}

/// `mapping_forwards` / `mapping_backwards` helper name (`mapping_function`).
pub fn mapping_call(mapping: &str, direction: Direction) -> String {
    format!("{mapping}_{}", direction.as_str())
}

/// `mapping_forwards_matches` / `mapping_backwards_matches` guard name
/// (`mapping_guard`).
pub fn mapping_guard(mapping: &str, direction: Direction) -> String {
    format!("{mapping}_{}_matches", direction.as_str())
}

/// One extracted mapping pattern: the fresh identifier that replaced it plus
/// the original pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMapping {
    pub subst_id: String,
    pub mapping: String,
    pub direction: Direction,
    pub subpat: Pat,
}

/// Result of extracting mapping patterns from one arm pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub pat: Pat,
    pub mappings: Vec<ExtractedMapping>,
}

/// Fresh-identifier supply (`name_gen`).
#[derive(Debug)]
pub struct Fresh {
    prefix: String,
    counter: usize,
}

impl Fresh {
    pub fn new(prefix: impl Into<String>) -> Self {
        Fresh {
            prefix: prefix.into(),
            counter: 0,
        }
    }

    pub fn fresh(&mut self) -> String {
        let id = format!("{}{}#", self.prefix, self.counter);
        self.counter += 1;
        id
    }
}

/// Replace mapping applications in a pattern with fresh identifiers
/// (`extract_mapping_pats`).
///
/// `is_mapping` answers `Env.is_mapping`; `direction_of` answers the
/// `forwards` / `backwards` attribute of a mapping application site.
pub fn extract_mapping_pats(
    pat: &Pat,
    is_mapping: &impl Fn(&str) -> bool,
    direction_of: &impl Fn(&str) -> Direction,
    fresh: &mut Fresh,
) -> Extracted {
    match pat {
        Pat::App(f, args) if is_mapping(f) => {
            let id = fresh.fresh();
            let direction = direction_of(f);
            Extracted {
                pat: Pat::Var(id.clone()),
                mappings: vec![ExtractedMapping {
                    subst_id: id,
                    mapping: f.clone(),
                    direction,
                    subpat: Pat::App(
                        f.clone(),
                        args.iter()
                            .map(|a| {
                                extract_mapping_pats(a, is_mapping, direction_of, fresh)
                                    .pat
                            })
                            .collect(),
                    ),
                }],
            }
        }
        Pat::App(f, args) => {
            let mut mappings = Vec::new();
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                let e = extract_mapping_pats(a, is_mapping, direction_of, fresh);
                out.push(e.pat);
                mappings.extend(e.mappings);
            }
            Extracted {
                pat: Pat::App(f.clone(), out),
                mappings,
            }
        }
        Pat::Tuple(ps) => {
            let mut mappings = Vec::new();
            let mut out = Vec::with_capacity(ps.len());
            for p in ps {
                let e = extract_mapping_pats(p, is_mapping, direction_of, fresh);
                out.push(e.pat);
                mappings.extend(e.mappings);
            }
            Extracted {
                pat: Pat::Tuple(out),
                mappings,
            }
        }
        leaf => Extracted {
            pat: leaf.clone(),
            mappings: Vec::new(),
        },
    }
}

/// A match arm before elaboration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pat: Pat,
    pub guard: Option<Exp>,
    pub body: Exp,
}

/// Split an arm list at the first arm containing a mapping pattern
/// (`split_arms`): `(before, Some((subst_arm, mappings, after)))` or
/// `(all, None)` when no arm uses a mapping.
pub fn split_arms(
    arms: &[MatchArm],
    is_mapping: &impl Fn(&str) -> bool,
    direction_of: &impl Fn(&str) -> Direction,
    fresh: &mut Fresh,
) -> (Vec<MatchArm>, Option<(MatchArm, Vec<ExtractedMapping>, Vec<MatchArm>)>) {
    let mut before = Vec::new();
    for (i, arm) in arms.iter().enumerate() {
        let extracted = extract_mapping_pats(&arm.pat, is_mapping, direction_of, fresh);
        if extracted.mappings.is_empty() {
            before.push(arm.clone());
        } else {
            let subst_arm = MatchArm {
                pat: extracted.pat,
                guard: arm.guard.clone(),
                body: arm.body.clone(),
            };
            let after = arms[i + 1..].to_vec();
            return (before, Some((subst_arm, extracted.mappings, after)));
        }
    }
    (before, None)
}

/// Elaboration plan for one mapping-pattern arm: the guard calls and the
/// nested-match scrutinees the printer must emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElabPlan {
    /// `(guard_fn, subst_id)` pairs, conjoined with `&&`.
    pub guards: Vec<(String, String)>,
    /// `(call_fn, subst_id)` scrutinees for the nested match.
    pub calls: Vec<(String, String)>,
    /// The inner pattern the nested match scrutinizes.
    pub inner_pat: Pat,
}

impl ElabPlan {
    /// Build the plan for the extracted mappings of one arm
    /// (`mappings_match` core: guard `mapping_*_matches(z)` + scrutinee
    /// `mapping_*(z)` per extracted mapping).
    pub fn for_mappings(mappings: &[ExtractedMapping]) -> Self {
        let guards = mappings
            .iter()
            .map(|m| (mapping_guard(&m.mapping, m.direction), m.subst_id.clone()))
            .collect();
        let calls = mappings
            .iter()
            .map(|m| (mapping_call(&m.mapping, m.direction), m.subst_id.clone()))
            .collect();
        let inner_pat = match mappings
            .iter()
            .map(|m| m.subpat.clone())
            .collect::<Vec<_>>()
            .as_slice()
        {
            [single] => single.clone(),
            pats => Pat::Tuple(pats.to_vec()),
        };
        ElabPlan {
            guards,
            calls,
            inner_pat,
        }
    }

    /// Human-readable sketch of the elaborated Sail code (the two variants
    /// from the `mappings.ml` header: option-wrapping vs. return position).
    pub fn describe(&self, pat: &Pat) -> String {
        let guards = self
            .guards
            .iter()
            .map(|(g, z)| format!("{g}({z})"))
            .collect::<Vec<_>>()
            .join(" && ");
        let calls = self
            .calls
            .iter()
            .map(|(c, z)| format!("{c}({z})"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "match y {{ {pat:?} if {guards} => match {calls} {{ inner => body, _ => fallback }}, ... rest ... }}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_test_mapping(id: &str) -> bool {
        matches!(id, "hex_bits" | "hex_bits_2" | "ta_flag")
    }

    fn backwards(_: &str) -> Direction {
        Direction::Backwards
    }

    /// `mapping.sail` line 66: `hex_bits_2(ta @ sew) <-> (ta, sew)` uses the
    /// mapping `hex_bits_2` as a pattern; the elaboration must guard on
    /// `hex_bits_2_backwards_matches` and scrutinize `hex_bits_2_backwards`.
    #[test]
    fn mapping_pattern_arm_produces_guard_and_call() {
        let arm = MatchArm {
            pat: Pat::App(
                "hex_bits_2".to_string(),
                vec![Pat::VecAppend(vec![
                    Pat::Var("ta".to_string()),
                    Pat::Var("sew".to_string()),
                ])],
            ),
            guard: None,
            body: Exp::Id("e".to_string()),
        };
        let mut fresh = Fresh::new("mapping");
        let (before, hit) = split_arms(&[arm], &is_test_mapping, &backwards, &mut fresh);
        assert!(before.is_empty());
        let (subst_arm, mappings, after) = hit.expect("mapping arm found");
        assert!(after.is_empty());
        assert_eq!(subst_arm.pat, Pat::Var("mapping0#".to_string()));
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].mapping, "hex_bits_2");
        assert_eq!(mappings[0].direction, Direction::Backwards);

        let plan = ElabPlan::for_mappings(&mappings);
        assert_eq!(
            plan.guards,
            vec![(
                "hex_bits_2_backwards_matches".to_string(),
                "mapping0#".to_string()
            )]
        );
        assert_eq!(
            plan.calls,
            vec![("hex_bits_2_backwards".to_string(), "mapping0#".to_string())]
        );
        assert_eq!(
            plan.inner_pat,
            Pat::App(
                "hex_bits_2".to_string(),
                vec![Pat::VecAppend(vec![
                    Pat::Var("ta".to_string()),
                    Pat::Var("sew".to_string())
                ])]
            )
        );
    }

    #[test]
    fn split_stops_at_first_mapping_arm() {
        let plain = MatchArm {
            pat: Pat::Lit("BYTE".to_string()),
            guard: None,
            body: Exp::Id("e1".to_string()),
        };
        let mapping_arm = MatchArm {
            pat: Pat::App("ta_flag".to_string(), vec![Pat::Var("sew".to_string())]),
            guard: None,
            body: Exp::Id("e2".to_string()),
        };
        let trailing = MatchArm {
            pat: Pat::Wild,
            guard: None,
            body: Exp::Id("e3".to_string()),
        };
        let mut fresh = Fresh::new("mapping");
        let (before, hit) = split_arms(
            &[plain.clone(), mapping_arm, trailing.clone()],
            &is_test_mapping,
            &backwards,
            &mut fresh,
        );
        assert_eq!(before, vec![plain]);
        let (_, mappings, after) = hit.expect("hit");
        assert_eq!(mappings.len(), 1);
        assert_eq!(after, vec![trailing]);
    }

    #[test]
    fn no_mapping_pattern_leaves_arms_untouched() {
        let arms = vec![MatchArm {
            pat: Pat::Lit("BYTE".to_string()),
            guard: None,
            body: Exp::Id("e".to_string()),
        }];
        let mut fresh = Fresh::new("mapping");
        let (before, hit) = split_arms(&arms, &is_test_mapping, &backwards, &mut fresh);
        assert_eq!(before, arms);
        assert!(hit.is_none());
    }

    #[test]
    fn direction_naming_matches_ocaml() {
        assert_eq!(mapping_call("hex_bits", Direction::Forwards), "hex_bits_forwards");
        assert_eq!(
            mapping_call("hex_bits", Direction::Backwards),
            "hex_bits_backwards"
        );
        assert_eq!(
            mapping_guard("hex_bits", Direction::Forwards),
            "hex_bits_forwards_matches"
        );
        assert_eq!(
            direction_from_attr(&["backwards".to_string()]),
            Some(Direction::Backwards)
        );
        assert_eq!(direction_from_attr(&[]), None);
        // Both / neither direction attributes are ill-formed.
        assert_eq!(
            direction_from_attr(&["forwards".to_string(), "backwards".to_string()]),
            None
        );
    }
}
