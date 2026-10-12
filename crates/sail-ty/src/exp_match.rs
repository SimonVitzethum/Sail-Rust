#![forbid(unsafe_code)]
//! Check-only plan helpers for `match` and `try` (Lane-26).
//!
//! Pure planners returning the exact ordered checking premises of the OCaml
//! rules `check_exp E_match ~L2176` and `check_exp E_try ~L2196`
//! (`sail2` branch, `src/lib/type_check.ml`). The Main-Agent (owns
//! `canonical_body.rs`) turns a [`Plan`] into `Task` pushes 1:1; the
//! per-function `INSERT` doc comments carry the exact insertion text with
//! anchors in the current `canonical_body.rs`.
//!
//! Pattern binding itself (`bind_pat`, `bind_mpat`) and exhaustiveness
//! (`Pattern_completeness`) stay Lane-09 (`pat.rs`): this file never binds a
//! pattern and never decides completeness. Both are referenced only through
//! the [`MatchCompletenessOracle`] trait, and the [`SideCond::Completeness`]
//! premise names the owed oracle call. Arm scopes that need pattern-bound
//! locals use [`ScopeOp::PatArm`], mirroring the `exp_arms.rs` convention in
//! the main worktree (read-only reference; this worktree at `a972e3a` has no
//! `exp_arms.rs`, so the convention is restated here self-contained).
//!
//! Both rules are check-only: OCaml inference for `E_match`/`E_try` is
//! unsupported and must keep failing. There is intentionally no infer plan.

/// Check mode marker. Both planners return [`Mode::Check`]; the variant
/// exists so a future infer attempt fails at the type level, not silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Check,
}

/// A type the insertion must check a child against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypRef {
    /// The incoming expected type of the `run` task.
    Expected,
    /// A builtin with no arguments: only `"bool"` is used here
    /// (guard obligations).
    Builtin(&'static str),
    /// `option(Expected)`: only for `match` under the `mapping_match`
    /// attribute (OCaml `check_exp` E_match arm: the scrutinee of a
    /// mapping application is checked against the option of the result).
    OptionOfExpected,
    /// The exception type (`exc` in OCaml): `try` arms bind against it.
    Exc,
}

/// What to do with one sub-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push a `check(child) <= T` task.
    CheckAgainst(TypRef),
    /// Synthesize the child type (`infer` in `canonical_body`).
    Infer,
}

/// The scope a step runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeOp {
    /// Same scope as the parent task.
    Same,
    /// Fresh child scope holding the Lane-09 pattern bindings of arm `arm`;
    /// when `with_guard_fact` the checked guard constraint is added exactly
    /// like the `if` branches (`scope.branch(condition, false)` shape).
    /// The insertion resolves this via the Lane-09 API; this file only
    /// names the scope.
    PatArm { arm: usize, with_guard_fact: bool },
}

/// One ordered premise of a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// OCaml-level child name (`scrutinee`, `guard`, `body`, `exp`).
    pub child: &'static str,
    pub action: Action,
    pub scope: ScopeOp,
}

impl Step {
    fn check(child: &'static str, against: TypRef, scope: ScopeOp) -> Self {
        Self {
            child,
            action: Action::CheckAgainst(against),
            scope,
        }
    }

    fn infer(child: &'static str) -> Self {
        Self {
            child,
            action: Action::Infer,
            scope: ScopeOp::Same,
        }
    }
}

/// A side condition the insertion must enforce besides pushing tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideCond {
    /// Drop arms whose pattern+guard prove `false`, then annotate the
    /// surviving match `complete`/`incomplete` via the Lane-09
    /// pattern-completeness oracle ([`MatchCompletenessOracle`]).
    /// Pure oracle call: this file never implements it.
    Completeness,
}

/// One check-only rule, exactly as the insertion must execute it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// OCaml source reference, e.g. `"check_exp E_match ~L2176"`.
    pub ocaml: &'static str,
    pub mode: Mode,
    pub steps: Vec<Step>,
    pub side: Vec<SideCond>,
}

/// Pattern/completeness oracle owned by Lane-09 (`pat.rs`).
///
/// The `canonical_body.rs` insertion calls these; this module only names
/// them so the plan stays pure and the ownership boundary stays explicit.
/// No implementation lives here by design (Lane-26 constraint).
pub trait MatchCompletenessOracle {
    /// Pattern scope for arm `arm` against the given scrutinee type name
    /// (`"scrutinee"` for `match`, `"exc"` for `try`). Returns `None` when
    /// the arm environment proves `false` (arm is dropped).
    /// The concrete scope and type representations belong to the checker.
    fn arm_scope(&self, arm: usize) -> Option<()>;
    /// Annotate the surviving match `complete`/`incomplete` after all arms
    /// are planned.
    fn check_complete(&self) -> ();
}

// ---------------------------------------------------------------------------
// E_match (check-only, OCaml `check_exp` ~L2176)
// ---------------------------------------------------------------------------

/// Check-mode `E_match` plan.
///
/// - Scrutinee: `infer` normally; under the `mapping_match` attribute
///   `check <= option(Expected)`.
/// - Per arm `i`: optional guard `check <= bool` in `PatArm{i,false}`,
///   body `check <= Expected` in `PatArm{i, with_guard_fact}` where
///   `with_guard_fact` is exactly `has_guard[i]`.
/// - [`SideCond::Completeness`] names the owed Lane-09 oracle call.
///
/// `has_guard[i]` reports whether arm `i` carries a `when` guard
/// (`PExpKind::PatWhen` vs `PExpKind::PatExp`).
///
/// INSERT (run match, before the trailing `_ =>` fallback):
/// ```ignore
/// ExpKind::Match { scrutinee, arms } => {
///     // mapping_match attribute => check scrutinee <= option(Expected),
///     // else infer it.
///     let scrut = if has_mapping_match_attr(exp) {
///         tasks.push(Task { exp: scrutinee,
///             expected: self.option_of(&expected, exp.span)?,
///             scope: scope.clone(), returning: returning.clone() });
///         self.infer(scrutinee, &scope)?
///     } else {
///         self.infer(scrutinee, &scope)?
///     };
///     for (i, arm) in arms.iter().enumerate().rev() {
///         // Lane-09 oracle: pattern scope for arm i against scrut type;
///         // skip arm when scope proves false.
///         let (pat, guard, body) = match &arm.kind {
///             PExpKind::PatExp { pat, body } => (pat, None, body),
///             PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
///         };
///         // Body: Task { exp: body, expected, scope: PatArm{i, with_guard_fact} }
///         // Guard (if any): Task { exp: guard, expected: bool,
///         //     scope: PatArm{i,false} }
///     }
///     // Completeness: annotate complete/incomplete via Lane-09 oracle.
/// }
/// ```
#[must_use]
pub fn check_match(has_guard: &[bool], mapping_match: bool) -> Plan {
    let scrutinee = if mapping_match {
        Step::check("scrutinee", TypRef::OptionOfExpected, ScopeOp::Same)
    } else {
        Step::infer("scrutinee")
    };
    let mut steps = vec![scrutinee];
    for (arm, guard) in has_guard.iter().enumerate() {
        if *guard {
            steps.push(Step::check(
                "guard",
                TypRef::Builtin("bool"),
                ScopeOp::PatArm {
                    arm,
                    with_guard_fact: false,
                },
            ));
        }
        steps.push(Step::check(
            "body",
            TypRef::Expected,
            ScopeOp::PatArm {
                arm,
                with_guard_fact: *guard,
            },
        ));
    }
    Plan {
        ocaml: "check_exp E_match ~L2176",
        mode: Mode::Check,
        steps,
        side: vec![SideCond::Completeness],
    }
}

// ---------------------------------------------------------------------------
// E_try (check-only, OCaml `check_exp` ~L2196)
// ---------------------------------------------------------------------------

/// Check-mode `E_try` plan.
///
/// - Body `check <= Expected` in the same scope.
/// - Every arm binds against `exc` (no scrutinee inference) and checks
///   exactly like a `match` arm: optional guard `<= bool` in
///   `PatArm{i,false}`, body `<= Expected` in `PatArm{i, with_guard_fact}`.
/// - Impossible arms are dropped like `match` arms; the same Lane-09
///   oracle call applies ([`SideCond::Completeness`]).
///
/// INSERT (run match, before the trailing `_ =>` fallback):
/// ```ignore
/// ExpKind::Try { exp, arms } => {
///     tasks.push(Task { exp, expected: expected.clone(),
///         scope: scope.clone(), returning: returning.clone() });
///     for (i, arm) in arms.iter().enumerate().rev() {
///         // Lane-09 oracle: pattern scope for arm i against exc type;
///         // skip arm when scope proves false.
///         let (pat, guard, body) = match &arm.kind {
///             PExpKind::PatExp { pat, body } => (pat, None, body),
///             PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
///         };
///         // Body: Task { exp: body, expected, scope: PatArm{i, with_guard_fact} }
///         // Guard (if any): Task { exp: guard, expected: bool,
///         //     scope: PatArm{i,false} }
///     }
/// }
/// ```
#[must_use]
pub fn check_try(has_guard: &[bool]) -> Plan {
    let mut steps = vec![Step::check("exp", TypRef::Expected, ScopeOp::Same)];
    for (arm, guard) in has_guard.iter().enumerate() {
        if *guard {
            steps.push(Step::check(
                "guard",
                TypRef::Builtin("bool"),
                ScopeOp::PatArm {
                    arm,
                    with_guard_fact: false,
                },
            ));
        }
        steps.push(Step::check(
            "body",
            TypRef::Expected,
            ScopeOp::PatArm {
                arm,
                with_guard_fact: *guard,
            },
        ));
    }
    Plan {
        ocaml: "check_exp E_try ~L2196",
        mode: Mode::Check,
        steps,
        side: vec![SideCond::Completeness],
    }
}

/// The scrutinee type a `try` arm binds against (oracle input name).
#[must_use]
pub const fn try_scrutinee() -> &'static str {
    "exc"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_plain_infers_scrutinee() {
        let plan = check_match(&[false], false);
        assert_eq!(plan.ocaml, "check_exp E_match ~L2176");
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(plan.steps[0], Step::infer("scrutinee"));
        assert!(plan.side.contains(&SideCond::Completeness));
    }

    #[test]
    fn match_mapping_attr_checks_option_of_expected() {
        let plan = check_match(&[false], true);
        assert_eq!(
            plan.steps[0],
            Step::check("scrutinee", TypRef::OptionOfExpected, ScopeOp::Same)
        );
    }

    #[test]
    fn match_guard_then_body_per_arm_with_guard_fact() {
        let plan = check_match(&[false, true], false);
        // scrutinee + arm0 body + arm1 guard + arm1 body
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(
            plan.steps[1],
            Step::check(
                "body",
                TypRef::Expected,
                ScopeOp::PatArm {
                    arm: 0,
                    with_guard_fact: false,
                }
            )
        );
        assert_eq!(
            plan.steps[2],
            Step::check(
                "guard",
                TypRef::Builtin("bool"),
                ScopeOp::PatArm {
                    arm: 1,
                    with_guard_fact: false,
                }
            )
        );
        assert_eq!(
            plan.steps[3],
            Step::check(
                "body",
                TypRef::Expected,
                ScopeOp::PatArm {
                    arm: 1,
                    with_guard_fact: true,
                }
            )
        );
    }

    #[test]
    fn match_empty_has_scrutinee_only() {
        let plan = check_match(&[], false);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0], Step::infer("scrutinee"));
    }

    #[test]
    fn try_checks_body_first_in_same_scope() {
        let plan = check_try(&[false]);
        assert_eq!(plan.ocaml, "check_exp E_try ~L2196");
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(
            plan.steps[0],
            Step::check("exp", TypRef::Expected, ScopeOp::Same)
        );
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.side.contains(&SideCond::Completeness));
    }

    #[test]
    fn try_arms_mirror_match_against_exc() {
        let plan = check_try(&[true]);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(
            plan.steps[1],
            Step::check(
                "guard",
                TypRef::Builtin("bool"),
                ScopeOp::PatArm {
                    arm: 0,
                    with_guard_fact: false,
                }
            )
        );
        assert_eq!(
            plan.steps[2],
            Step::check(
                "body",
                TypRef::Expected,
                ScopeOp::PatArm {
                    arm: 0,
                    with_guard_fact: true,
                }
            )
        );
        assert_eq!(try_scrutinee(), "exc");
    }

    #[test]
    fn try_arm_indices_track_match_convention() {
        let got = check_try(&[false, false]);
        let want = check_match(&[false, false], false);
        // Same per-arm tail (guard/body scopes), only the head differs:
        // try checks `exp <= Expected`, match infers the scrutinee.
        assert_eq!(&got.steps[1..], &want.steps[1..]);
        assert_eq!(got.side, want.side);
    }

    #[test]
    fn oracle_is_reference_only() {
        struct Stub {
            arms: usize,
        }
        impl MatchCompletenessOracle for Stub {
            fn arm_scope(&self, arm: usize) -> Option<()> {
                (arm < self.arms).then_some(())
            }
            fn check_complete(&self) -> () {}
        }
        let oracle = Stub { arms: 2 };
        assert_eq!(oracle.arm_scope(0), Some(()));
        assert_eq!(oracle.arm_scope(2), None);
        oracle.check_complete();
    }
}
