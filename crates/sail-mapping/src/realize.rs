//! `realize_mappings` port (`sail/src/lib/rewrites.ml`,
//! `rewrite_ast_realize_mappings`).
//!
//! A mapping
//!
//! ```sail
//! val size_bits : word_width <-> bits(2)
//! mapping size_bits = { BYTE <-> 0b00, ... }
//! ```
//!
//! is elaborated into four functions:
//!
//! - `size_bits_forwards : word_width -> bits(2)`
//! - `size_bits_backwards : bits(2) -> word_width`
//! - `size_bits_forwards_matches : word_width -> bool`
//! - `size_bits_backwards_matches : bits(2) -> bool`
//!
//! Rules (mirroring the OCaml):
//!
//! - `realize_mpexps forwards mpexp1 mpexp2`: the pattern side is `mpexp1`
//!   (forwards) or `mpexp2` (backwards); the body is `exp_of_mpat` of the
//!   other side. Guards travel with their pattern side.
//! - `MCL_forwards` arms only appear in the forwards functions,
//!   `MCL_backwards` only in the backwards functions, `MCL_bidir` in both.
//! - The `_matches` variants replace every body with `true` and append a
//!   wildcard `false` arm (`realize_bool_mapcl` + `wildcard`).
//! - The last arm of each direction carries `is_last` (`mapping_last`).
//! - Functions that already exist are not regenerated (`has_def` check).
//! - A bidirectional `val` spec is split into four function specs.
//! - `overload` entries naming mappings expand to the
//!   `_forwards`/`_backwards` pair.

#![forbid(unsafe_code)]

use crate::ast::{Arm, Exp, FunDef, MappingClause, MappingDef, Pat, ValSpec, exp_of_mpat, pat_of_mpat};
use std::collections::HashSet;

/// Elaborate one bidirectional clause arm for the given direction.
///
/// Mirrors `realize_mpexps`: pattern from the `forwards` side, body built
/// from the opposite side.
fn realize_bidir_arm(
    forwards: bool,
    left: &crate::ast::Mpat,
    left_guard: &Option<Exp>,
    right: &crate::ast::Mpat,
    right_guard: &Option<Exp>,
) -> Arm {
    if forwards {
        Arm {
            pat: pat_of_mpat(left),
            guard: left_guard.clone(),
            body: exp_of_mpat(right),
            is_last: false,
        }
    } else {
        Arm {
            pat: pat_of_mpat(right),
            guard: right_guard.clone(),
            body: exp_of_mpat(left),
            is_last: false,
        }
    }
}

/// Realize one clause for one direction (`realize_mapcl`).
fn realize_clause_arms(forwards: bool, clause: &MappingClause) -> Vec<Arm> {
    match clause {
        MappingClause::Bidir {
            left,
            left_guard,
            right,
            right_guard,
        } => vec![realize_bidir_arm(
            forwards,
            left,
            left_guard,
            right,
            right_guard,
        )],
        MappingClause::Forwards { arm } => {
            if forwards {
                vec![arm.clone()]
            } else {
                vec![]
            }
        }
        MappingClause::Backwards { arm } => {
            if forwards {
                vec![]
            } else {
                vec![arm.clone()]
            }
        }
    }
}

/// Replace an arm body with `true` (`true_pexp`).
fn true_arm(arm: &Arm) -> Arm {
    Arm {
        pat: arm.pat.clone(),
        guard: arm.guard.clone(),
        body: Exp::Bool(true),
        is_last: arm.is_last,
    }
}

/// Mark the final arm of a direction (`Util.map_last` + `annotate_pat ~last`).
fn mark_last(mut arms: Vec<Arm>) -> Vec<Arm> {
    if let Some(last) = arms.last_mut() {
        last.is_last = true;
    }
    arms
}

/// The four functions generated from one mapping definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizedMapping {
    pub forwards: Option<FunDef>,
    pub backwards: Option<FunDef>,
    pub forwards_matches: Option<FunDef>,
    pub backwards_matches: Option<FunDef>,
}

impl RealizedMapping {
    /// All generated functions in definition order.
    pub fn all(&self) -> Vec<&FunDef> {
        [
            &self.forwards,
            &self.backwards,
            &self.forwards_matches,
            &self.backwards_matches,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Realize a mapping definition (`realize_mapdef`).
///
/// `existing` holds already-defined function names; any generated function
/// whose name is present is skipped and returned as `None` (`has_def`).
pub fn realize_mapping(def: &MappingDef, existing: &HashSet<String>) -> RealizedMapping {
    let fwd_arms = mark_last(
        def.clauses
            .iter()
            .flat_map(|c| realize_clause_arms(true, c))
            .collect(),
    );
    let bwd_arms = mark_last(
        def.clauses
            .iter()
            .flat_map(|c| realize_clause_arms(false, c))
            .collect(),
    );
    let fwd_matches_arms = {
        let mut arms: Vec<Arm> = fwd_arms.iter().map(true_arm).collect();
        arms.push(Arm::plain(Pat::Wild, Exp::Bool(false)));
        arms
    };
    let bwd_matches_arms = {
        let mut arms: Vec<Arm> = bwd_arms.iter().map(true_arm).collect();
        arms.push(Arm::plain(Pat::Wild, Exp::Bool(false)));
        arms
    };

    let mk = |name: String, arg_ty: String, ret_ty: String, arms: Vec<Arm>| -> Option<FunDef> {
        if existing.contains(&name) {
            None
        } else {
            Some(FunDef {
                name,
                arg_ty,
                ret_ty,
                arms,
            })
        }
    };

    RealizedMapping {
        forwards: mk(
            def.forwards_id(),
            def.typ1.clone(),
            def.typ2.clone(),
            fwd_arms,
        ),
        backwards: mk(
            def.backwards_id(),
            def.typ2.clone(),
            def.typ1.clone(),
            bwd_arms,
        ),
        forwards_matches: mk(
            def.forwards_matches_id(),
            def.typ1.clone(),
            "bool".to_string(),
            fwd_matches_arms,
        ),
        backwards_matches: mk(
            def.backwards_matches_id(),
            def.typ2.clone(),
            "bool".to_string(),
            bwd_matches_arms,
        ),
    }
}

/// Split a bidirectional `val` spec into the four function specs
/// (`realize_val_spec`).
///
/// `val m : t1 <-> t2` becomes `m_forwards : t1 -> t2`,
/// `m_backwards : t2 -> t1`, `m_forwards_matches : t1 -> bool`,
/// `m_backwards_matches : t2 -> bool`.
pub fn realize_val_spec(name: &str, typ1: &str, typ2: &str) -> Vec<ValSpec> {
    vec![
        ValSpec {
            name: format!("{name}_forwards"),
            arg_ty: typ1.to_string(),
            ret_ty: typ2.to_string(),
        },
        ValSpec {
            name: format!("{name}_backwards"),
            arg_ty: typ2.to_string(),
            ret_ty: typ1.to_string(),
        },
        ValSpec {
            name: format!("{name}_forwards_matches"),
            arg_ty: typ1.to_string(),
            ret_ty: "bool".to_string(),
        },
        ValSpec {
            name: format!("{name}_backwards_matches"),
            arg_ty: typ2.to_string(),
            ret_ty: "bool".to_string(),
        },
    ]
}

/// Expand an `overload` clause: mapping entries become the
/// `_forwards`/`_backwards` pair, plain functions stay as-is.
///
/// `is_mapping` answers `Env.is_mapping` for an overload entry.
pub fn realize_overload(
    overloads: &[String],
    is_mapping: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut out = Vec::new();
    for overload in overloads {
        if is_mapping(overload) {
            out.push(format!("{overload}_forwards"));
            out.push(format!("{overload}_backwards"));
        } else {
            out.push(overload.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Mpat;

    /// `mapping.sail` lines 9-14: `mapping size_bits`.
    fn size_bits_def() -> MappingDef {
        MappingDef {
            name: "size_bits".to_string(),
            typ1: "word_width".to_string(),
            typ2: "bits(2)".to_string(),
            clauses: vec![
                MappingClause::Bidir {
                    left: Mpat::Lit("BYTE".to_string()),
                    left_guard: None,
                    right: Mpat::Lit("0b00".to_string()),
                    right_guard: None,
                },
                MappingClause::Bidir {
                    left: Mpat::Lit("HALF".to_string()),
                    left_guard: None,
                    right: Mpat::Lit("0b01".to_string()),
                    right_guard: None,
                },
                MappingClause::Bidir {
                    left: Mpat::Lit("WORD".to_string()),
                    left_guard: None,
                    right: Mpat::Lit("0b10".to_string()),
                    right_guard: None,
                },
                MappingClause::Bidir {
                    left: Mpat::Lit("DOUBLE".to_string()),
                    left_guard: None,
                    right: Mpat::Lit("0b11".to_string()),
                    right_guard: None,
                },
            ],
        }
    }

    #[test]
    fn bidir_mapping_realizes_four_functions() {
        let def = size_bits_def();
        let r = realize_mapping(&def, &HashSet::new());

        let fwd = r.forwards.expect("forwards generated");
        assert_eq!(fwd.name, "size_bits_forwards");
        assert_eq!(fwd.arg_ty, "word_width");
        assert_eq!(fwd.ret_ty, "bits(2)");
        assert_eq!(fwd.arms.len(), 4);
        assert_eq!(fwd.arms[0].pat, Pat::Lit("BYTE".to_string()));
        assert_eq!(fwd.arms[0].body, Exp::Lit("0b00".to_string()));
        // `mapping_last` lands on the final clause of each direction.
        assert!(fwd.arms[3].is_last);
        assert!(!fwd.arms[2].is_last);

        let bwd = r.backwards.expect("backwards generated");
        assert_eq!(bwd.name, "size_bits_backwards");
        assert_eq!(bwd.arg_ty, "bits(2)");
        assert_eq!(bwd.ret_ty, "word_width");
        assert_eq!(bwd.arms.len(), 4);
        assert_eq!(bwd.arms[0].pat, Pat::Lit("0b00".to_string()));
        assert_eq!(bwd.arms[0].body, Exp::Lit("BYTE".to_string()));
        assert!(bwd.arms[3].is_last);

        // `_matches` variants: bodies become `true`, wildcard `false` appended.
        // Matches `mapping.expected.lean` lines 191-205.
        let fwd_m = r.forwards_matches.expect("forwards_matches generated");
        assert_eq!(fwd_m.arms.len(), 5);
        assert!(fwd_m.arms[..4].iter().all(|a| a.body == Exp::Bool(true)));
        assert_eq!(fwd_m.arms[4].pat, Pat::Wild);
        assert_eq!(fwd_m.arms[4].body, Exp::Bool(false));

        let bwd_m = r.backwards_matches.expect("backwards_matches generated");
        assert_eq!(bwd_m.arms.len(), 5);
        assert!(bwd_m.arms[..4].iter().all(|a| a.body == Exp::Bool(true)));
        assert_eq!(bwd_m.arms[4].body, Exp::Bool(false));
    }

    /// `mapping.sail` lines 23-29: `mapping size_bits3` mixes bidir clauses
    /// with one `forwards`-only and one `backwards`-only clause.
    #[test]
    fn direction_only_clauses_are_filtered() {
        let mut def = size_bits_def();
        def.name = "size_bits3".to_string();
        def.clauses.truncate(3);
        def.clauses.push(MappingClause::Forwards {
            arm: Arm::plain(
                Pat::Lit("DOUBLE".to_string()),
                Exp::Lit("0b11".to_string()),
            ),
        });
        def.clauses.push(MappingClause::Backwards {
            arm: Arm::plain(
                Pat::Lit("0b11".to_string()),
                Exp::Lit("DOUBLE".to_string()),
            ),
        });

        let r = realize_mapping(&def, &HashSet::new());
        let fwd = r.forwards.expect("forwards");
        let bwd = r.backwards.expect("backwards");
        // 3 bidir + 1 direction-only each; the opposite-only clause is absent.
        assert_eq!(fwd.arms.len(), 4);
        assert_eq!(bwd.arms.len(), 4);
        assert_eq!(fwd.arms[3].pat, Pat::Lit("DOUBLE".to_string()));
        assert_eq!(fwd.arms[3].body, Exp::Lit("0b11".to_string()));
        assert_eq!(bwd.arms[3].pat, Pat::Lit("0b11".to_string()));
        assert_eq!(bwd.arms[3].body, Exp::Lit("DOUBLE".to_string()));
        // Matches `mapping.expected.lean` lines 237-265: both directions
        // still cover all four enum cases.
        assert_eq!(r.forwards_matches.expect("fwd matches").arms.len(), 5);
        assert_eq!(r.backwards_matches.expect("bwd matches").arms.len(), 5);
    }

    /// `mapping.sail` line 7: `val size_bits : word_width <-> bits(2)`.
    #[test]
    fn bidir_val_spec_splits_into_four() {
        let specs = realize_val_spec("size_bits", "word_width", "bits(2)");
        assert_eq!(specs.len(), 4);
        assert_eq!(
            specs[0],
            ValSpec {
                name: "size_bits_forwards".to_string(),
                arg_ty: "word_width".to_string(),
                ret_ty: "bits(2)".to_string(),
            }
        );
        assert_eq!(
            specs[1],
            ValSpec {
                name: "size_bits_backwards".to_string(),
                arg_ty: "bits(2)".to_string(),
                ret_ty: "word_width".to_string(),
            }
        );
        assert_eq!(specs[2].name, "size_bits_forwards_matches");
        assert_eq!(specs[2].ret_ty, "bool");
        assert_eq!(specs[3].name, "size_bits_backwards_matches");
        assert_eq!(specs[3].ret_ty, "bool");
    }

    #[test]
    fn existing_definitions_are_not_regenerated() {
        let def = size_bits_def();
        let existing: HashSet<String> = ["size_bits_forwards".to_string()].into_iter().collect();
        let r = realize_mapping(&def, &existing);
        assert!(r.forwards.is_none(), "has_def: skip existing");
        assert!(r.backwards.is_some());
        assert!(r.forwards_matches.is_some());
        assert!(r.backwards_matches.is_some());
    }

    #[test]
    fn overload_entries_expand_mappings() {
        let out = realize_overload(&["size_bits".to_string(), "add".to_string()], |id| {
            id == "size_bits"
        });
        assert_eq!(
            out,
            vec![
                "size_bits_forwards".to_string(),
                "size_bits_backwards".to_string(),
                "add".to_string(),
            ]
        );
    }

    /// `mapping.sail` line 62: guarded clause
    /// `ta_flag(sew) ^ ta_flag(ta) <-> (ta, sew) when eq_bit(...)`.
    /// Guards travel with their pattern side.
    #[test]
    fn guards_travel_with_pattern_side() {
        let guard = Exp::App(
            "eq_bit".to_string(),
            vec![Exp::Id("sew0".to_string()), Exp::Id("bitone".to_string())],
        );
        let def = MappingDef {
            name: "vtype_assembly".to_string(),
            typ1: "string".to_string(),
            typ2: "(bits(1), bits(1))".to_string(),
            clauses: vec![MappingClause::Bidir {
                left: Mpat::StrAppend(vec![
                    Mpat::App("ta_flag".to_string(), vec![Mpat::Var("sew".to_string())]),
                    Mpat::App("ta_flag".to_string(), vec![Mpat::Var("ta".to_string())]),
                ]),
                left_guard: Some(guard.clone()),
                right: Mpat::Tuple(vec![
                    Mpat::Var("ta".to_string()),
                    Mpat::Var("sew".to_string()),
                ]),
                right_guard: None,
            }],
        };
        let r = realize_mapping(&def, &HashSet::new());
        let fwd = r.forwards.expect("forwards");
        assert_eq!(fwd.arms.len(), 1);
        assert_eq!(fwd.arms[0].guard, Some(guard));
        let bwd = r.backwards.expect("backwards");
        assert_eq!(bwd.arms.len(), 1);
        assert_eq!(bwd.arms[0].guard, None);
        assert_eq!(
            bwd.arms[0].pat,
            Pat::Tuple(vec![
                Pat::Var("ta".to_string()),
                Pat::Var("sew".to_string())
            ])
        );
    }
}
