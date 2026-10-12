#![forbid(unsafe_code)]
//! Struct-arm plan helpers for the canonical expression rules missing from
//! `canonical_body.rs` (Lane-25).
//!
//! Each planner is a pure function of the rule inputs (field names only)
//! and returns the exact ordered checking premises of the matching OCaml
//! rule in `type_check.ml` (sail2 branch, `src/lib/type_check.ml`). The
//! Main-Agent (owns `canonical_body.rs`) turns a [`Plan`] into `Task`
//! pushes 1:1; the per-variant `INSERT` doc comments carry the exact
//! insertion text with anchors in the current `canonical_body.rs`.
//!
//! Field payload types resolve exclusively through the record accessor
//! contract — `Program::accessor(record, field, position)` (Lane-08,
//! OCaml `Env.get_accessor`): unify the record telescope with the base
//! type, then take the field payload. The planners only *name* that
//! lookup ([`TypRef::RecordField`]), so this file compiles without the
//! accessor table and keeps no field table of its own.
//!
//! OCaml rule index (all in `check_exp`/`infer_exp`):
//! - `E_struct` check ~L2218, infer ~L3774
//! - `E_struct_update` check ~L2199, infer ~L3823

/// Check mode vs inference mode of one [`Plan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Check,
    Infer,
}

/// A type the insertion must check a child against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypRef {
    /// The incoming expected type of the `run` task.
    Expected,
    /// The inferred type of a previously inferred sibling child.
    Inferred(&'static str),
    /// The field type of the record carried by another [`TypRef`],
    /// resolved through the record accessor table (Lane-08
    /// `Program::accessor`): unify the record telescope with the base
    /// type, then take the field payload. Carries the runtime field name.
    RecordField { base: Box<TypRef>, field: String },
}

/// What to do with one sub-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push a `check(child) <= T` task.
    CheckAgainst(TypRef),
    /// Synthesize the child type (`infer` work stack in `canonical_body`).
    Infer,
}

/// One ordered premise of a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// OCaml-level child name (`record`, `field_value`, ...).
    pub child: &'static str,
    pub action: Action,
}

impl Step {
    fn check(child: &'static str, against: TypRef) -> Self {
        Self {
            child,
            action: Action::CheckAgainst(against),
        }
    }

    fn infer(child: &'static str) -> Self {
        Self {
            child,
            action: Action::Infer,
        }
    }
}

/// A side condition the insertion must enforce besides pushing tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideCond {
    /// Expand synonyms of [`TypRef::Expected`]; it must be a declared
    /// record, else "not a record".
    ExpectedMustBeRecord,
    /// Inferred record base must be a declared record, else
    /// "accessed type is not a struct".
    InferredMustBeRecord { child: &'static str },
    /// `SN_id` must equal the expected record; `SN_anon` never infers
    /// (and never checks against a named record).
    StructNameMustMatch,
    /// Every record field must be present exactly once; missing fields
    /// are a hard error (OCaml keeps a `record_fields` set per literal).
    /// Struct *updates* carry no coverage obligation (partial updates
    /// are legal), so update plans never list this condition.
    FullFieldCoverage,
}

/// One missing canonical rule, exactly as the owner of
/// `canonical_body.rs` must insert it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// OCaml source reference, e.g. `"check_exp E_struct ~L2218"`.
    pub ocaml: &'static str,
    pub mode: Mode,
    pub steps: Vec<Step>,
    pub side: Vec<SideCond>,
}

// ---------------------------------------------------------------------------
// E_struct
// ---------------------------------------------------------------------------

/// Check-mode `E_struct` (OCaml `check_exp` ~L2218).
///
/// INSERT (`Checker::run` match, before the trailing `_ =>` fallback):
/// ```ignore
/// ExpKind::Struct { name, fields } => {
///     // ExpectedMustBeRecord: resolve synonyms of `expected`; the head
///     // must be a declared record constructor, else "not a record".
///     // StructNameMustMatch: `StructName::Id(n)` must equal the
///     // expected record name; `StructName::Anon` is a mismatch here
///     // (infer rejects anon outright).
///     for fexp in fields.iter().rev() {
///         // FullFieldCoverage: unknown field or duplicate => error;
///         // after the loop, missing fields => error.
///         // Field type via the accessor contract:
///         // `self.program.accessor(record, field, self.position)` is the
///         // `Fn(record) -> field` scheme; unify its telescope with
///         // `expected`, take the field payload.
///         tasks.push(Task { exp: &fexp.value, expected: field_typ,
///             scope: scope.clone(), returning: returning.clone() });
///     }
/// }
/// ```
pub fn check_struct(field_names: &[&str]) -> Plan {
    Plan {
        ocaml: "check_exp E_struct ~L2218",
        mode: Mode::Check,
        steps: field_names
            .iter()
            .map(|field| {
                Step::check(
                    "field_value",
                    TypRef::RecordField {
                        base: Box::new(TypRef::Expected),
                        field: (*field).to_owned(),
                    },
                )
            })
            .collect(),
        side: vec![
            SideCond::ExpectedMustBeRecord,
            SideCond::StructNameMustMatch,
            SideCond::FullFieldCoverage,
        ],
    }
}

/// Inference-mode `E_struct` (OCaml `infer_exp` ~L3774): every field is
/// inferred, the telescope is unified field by field, missing
/// instantiations are hard errors.
///
/// INSERT (`Checker::infer`, `Synthesis::Enter` match beside the
/// `ExpKind::App` arm, plus a new `Synthesis::Struct` queue state that
/// infers each field value and unifies it against the accessor payload):
/// ```ignore
/// ExpKind::Struct { name, fields } => {
///     // StructNameMustMatch: `StructName::Anon` never infers; `Id(n)`
///     // must name a declared record.
///     // FullFieldCoverage: same duplicate/missing discipline as check.
///     work.push(Synthesis::Struct(exp, fields.len()));
///     work.extend(fields.iter().rev().map(Synthesis::Enter));
///     continue;
/// }
/// ```
pub fn infer_struct(field_names: &[&str]) -> Plan {
    Plan {
        ocaml: "infer_exp E_struct ~L3774",
        mode: Mode::Infer,
        steps: field_names
            .iter()
            .map(|_| Step::infer("field_value"))
            .collect(),
        side: vec![
            SideCond::StructNameMustMatch,
            SideCond::FullFieldCoverage,
        ],
    }
}

// ---------------------------------------------------------------------------
// E_struct_update
// ---------------------------------------------------------------------------

/// Check-mode `E_struct_update` (OCaml `check_exp` ~L2199): the base
/// record checks against the expected record type, then each update
/// field checks against its accessor payload. No coverage obligation
/// (partial updates are legal).
///
/// INSERT (`Checker::run` match, before the trailing `_ =>` fallback):
/// ```ignore
/// ExpKind::StructUpdate { record, fields } => {
///     // ExpectedMustBeRecord: resolve synonyms of `expected`; the head
///     // must be a declared record constructor, else "not a record".
///     tasks.push(Task { exp: record, expected: expected.clone(),
///         scope: scope.clone(), returning: returning.clone() });
///     for fexp in fields.iter().rev() {
///         // Field type via the accessor contract, telescope unified
///         // with `expected` (same as E_struct check).
///         tasks.push(Task { exp: &fexp.value, expected: field_typ,
///             scope: scope.clone(), returning: returning.clone() });
///     }
/// }
/// ```
pub fn check_struct_update(field_names: &[&str]) -> Plan {
    let mut steps = vec![Step::check("record", TypRef::Expected)];
    steps.extend(field_names.iter().map(|field| {
        Step::check(
            "field_value",
            TypRef::RecordField {
                base: Box::new(TypRef::Expected),
                field: (*field).to_owned(),
            },
        )
    }));
    Plan {
        ocaml: "check_exp E_struct_update ~L2199",
        mode: Mode::Check,
        steps,
        side: vec![SideCond::ExpectedMustBeRecord],
    }
}

/// Inference-mode `E_struct_update` (OCaml `infer_exp` ~L3823): infer
/// the base, require a record, check each update against the accessor
/// payload unified with the inferred base type.
///
/// INSERT (`Checker::infer`, `Synthesis::Enter` match beside the
/// `ExpKind::App` arm, plus a new `Synthesis::StructUpdate` queue state
/// that infers the base first and stages each update field as a
/// `check <= accessor-payload` task):
/// ```ignore
/// ExpKind::StructUpdate { record, fields } => {
///     // InferredMustBeRecord(record): expand synonyms of the inferred
///     // base; Id/App of a declared record => accessor call, else
///     // "accessed type is not a struct".
///     work.push(Synthesis::StructUpdate(exp, fields.len()));
///     work.push(Synthesis::Enter(record));
///     continue;
/// }
/// ```
pub fn infer_struct_update(field_names: &[&str]) -> Plan {
    let mut steps = vec![Step::infer("record")];
    steps.extend(field_names.iter().map(|field| {
        Step::check(
            "field_value",
            TypRef::RecordField {
                base: Box::new(TypRef::Inferred("record")),
                field: (*field).to_owned(),
            },
        )
    }));
    Plan {
        ocaml: "infer_exp E_struct_update ~L3823",
        mode: Mode::Infer,
        steps,
        side: vec![SideCond::InferredMustBeRecord { child: "record" }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_check_pins_ocaml_premises() {
        let plan = check_struct(&["x", "y"]);
        assert_eq!(plan.ocaml, "check_exp E_struct ~L2218");
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(plan.steps.len(), 2);
        for (step, field) in plan.steps.iter().zip(["x", "y"]) {
            assert_eq!(step.child, "field_value");
            assert_eq!(
                step.action,
                Action::CheckAgainst(TypRef::RecordField {
                    base: Box::new(TypRef::Expected),
                    field: field.to_owned(),
                })
            );
        }
        assert!(plan.side.contains(&SideCond::ExpectedMustBeRecord));
        assert!(plan.side.contains(&SideCond::StructNameMustMatch));
        assert!(plan.side.contains(&SideCond::FullFieldCoverage));
    }

    #[test]
    fn struct_check_empty_still_constrains() {
        // An empty literal still resolves the expected record and owes
        // full coverage (missing fields error unless the record is empty).
        let plan = check_struct(&[]);
        assert!(plan.steps.is_empty());
        assert_eq!(
            plan.side,
            vec![
                SideCond::ExpectedMustBeRecord,
                SideCond::StructNameMustMatch,
                SideCond::FullFieldCoverage,
            ]
        );
    }

    #[test]
    fn struct_infer_has_no_expected_premise() {
        let plan = infer_struct(&["x"]);
        assert_eq!(plan.ocaml, "infer_exp E_struct ~L3774");
        assert_eq!(plan.mode, Mode::Infer);
        assert_eq!(plan.steps, vec![Step::infer("field_value")]);
        assert!(
            !plan
                .steps
                .iter()
                .any(|s| matches!(s.action, Action::CheckAgainst(_)))
        );
        assert!(plan.side.contains(&SideCond::StructNameMustMatch));
        assert!(plan.side.contains(&SideCond::FullFieldCoverage));
        assert!(!plan.side.contains(&SideCond::ExpectedMustBeRecord));
    }

    #[test]
    fn struct_update_checks_base_first_without_coverage() {
        let plan = check_struct_update(&["f"]);
        assert_eq!(plan.ocaml, "check_exp E_struct_update ~L2199");
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(plan.steps[0], Step::check("record", TypRef::Expected));
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(
            plan.steps[1].action,
            Action::CheckAgainst(TypRef::RecordField {
                base: Box::new(TypRef::Expected),
                field: "f".to_owned(),
            })
        );
        assert!(plan.side.contains(&SideCond::ExpectedMustBeRecord));
        assert!(!plan.side.contains(&SideCond::FullFieldCoverage));
        assert!(!plan.side.contains(&SideCond::StructNameMustMatch));
    }

    #[test]
    fn struct_update_infer_binds_fields_to_inferred_base() {
        let plan = infer_struct_update(&["f", "g"]);
        assert_eq!(plan.ocaml, "infer_exp E_struct_update ~L3823");
        assert_eq!(plan.mode, Mode::Infer);
        assert_eq!(plan.steps[0], Step::infer("record"));
        assert_eq!(plan.steps.len(), 3);
        for (step, field) in plan.steps[1..].iter().zip(["f", "g"]) {
            assert_eq!(step.child, "field_value");
            assert_eq!(
                step.action,
                Action::CheckAgainst(TypRef::RecordField {
                    base: Box::new(TypRef::Inferred("record")),
                    field: field.to_owned(),
                })
            );
        }
        assert_eq!(
            plan.side,
            vec![SideCond::InferredMustBeRecord { child: "record" }]
        );
    }

    #[test]
    fn field_types_never_name_a_table() {
        // Every field premise must go through the accessor contract
        // (RecordField), never a direct/owned field table entry.
        for plan in [
            check_struct(&["x"]),
            check_struct_update(&["x"]),
            infer_struct_update(&["x"]),
        ] {
            assert!(plan.steps.iter().any(|step| matches!(
                step.action,
                Action::CheckAgainst(TypRef::RecordField { .. })
            )));
        }
    }
}
