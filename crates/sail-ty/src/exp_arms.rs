#![forbid(unsafe_code)]
//! Arm-helper plans for the canonical expression rules missing from
//! `canonical_body.rs` (Lane-19 audit O-5).
//!
//! Each planner is a pure function of the rule inputs (field names, arm
//! count, ...) and returns the exact ordered checking premises of the
//! matching OCaml rule in `type_check.ml` (sail2 branch,
//! `src/lib/type_check.ml`). The Main-Agent (Lane-16 owns
//! `canonical_body.rs`) turns a [`Plan`] into `Task` pushes 1:1; the
//! per-variant `INSERT` doc comments carry the exact insertion text with
//! anchors in the current `canonical_body.rs`.
//!
//! Pattern binding itself (`bind_pat`, `bind_mpat`, exhaustiveness) stays
//! Lane-09 (`pat.rs`): arm scopes that need pattern-bound locals use
//! [`ScopeOp::PatArm`], which the insertion resolves via the Lane-09 API.
//! Record field payloads resolve via the Lane-08 `Program::accessor`
//! table (commit `9a3b6a0`); the planners only name the lookup
//! ([`TypRef::RecordField`]) so this file compiles without that table.
//!
//! OCaml rule index (all in `check_exp` unless noted):
//! - `E_struct` check ~L2218, infer ~L3774 (`E_struct`)
//! - `E_struct_update` check ~L2199, infer ~L3823
//! - `E_field` infer ~L3748 (check falls through to infer+subtype)
//! - `E_match` check ~L2176 (infer unsupported)
//! - `E_vector_append` check ~L2262, infer ~L4019 (both rewrite to `append`)
//! - `E_exit` check ~L2391 (infer unsupported)
//! - `E_config` check ~L2365 (infer unsupported; any expected type fits)
//! - `E_ref` infer ~L4090 (check falls through to infer+subtype)
//! - `E_throw` check ~L2394 (infer unsupported)
//! - `E_try` check ~L2196 (infer unsupported)
//! - `E_assert` infer ~L4039 (check falls through to infer+subtype)
//! - termination measure `check_termination_measure` ~L4829
//! - mapping application check ~L2299, infer ~L3850 (`_forwards` first)

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
    /// The enclosing function return type (`returning` task field).
    Returning,
    /// A builtin with no arguments: `bool`, `unit`, `string`, `int`.
    Builtin(&'static str),
    /// The exception type (`exc` in OCaml; `register exc` payload).
    Exc,
    /// The inferred type of a previously inferred sibling child.
    Inferred(&'static str),
    /// The field type of the record carried by another [`TypRef`],
    /// resolved through the record accessor table (Lane-08
    /// `Program::accessor`): unify the record telescope with the base
    /// type, then take the field payload. Carries the runtime field name.
    RecordField { base: Box<TypRef>, field: String },
    /// `option(Expected)`: only for `match` under the `mapping_match`
    /// attribute (OCaml `check_exp` E_match arm).
    OptionOfExpected,
    /// `vector(len, elem)` / `bitvector(len)` built from a literal length
    /// and an element type: used by the vector-append fallback only to
    /// describe the `append` call shape, never constructed here.
    AppendCall,
}

/// What to do with one sub-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push a `check(child) <= T` task.
    CheckAgainst(TypRef),
    /// Synthesize the child type (`infer` work stack in `canonical_body`).
    Infer,
    /// `E_field` projection: infer the record child, then resolve
    /// `(record, field)` through the accessor table. The insertion
    /// reports "not a struct" when the inferred type carries no record.
    FieldProject {
        record_child: &'static str,
        field: String,
    },
}

/// The scope a step runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeOp {
    /// Same scope as the parent task.
    Same,
    /// `scope.branch(condition, negated)`: boolean-fact child scope
    /// (kept only when the condition type is `atom_bool`).
    Branch { negated: bool },
    /// Fresh child scope holding the Lane-09 pattern bindings of arm
    /// `arm`; when `with_guard_fact` the checked guard constraint is
    /// added exactly like the `if` branches.
    PatArm { arm: usize, with_guard_fact: bool },
}

/// One ordered premise of a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// OCaml-level child name (`scrutinee`, `cond`, `msg`, ...).
    pub child: &'static str,
    pub action: Action,
    pub scope: ScopeOp,
}

impl Step {
    fn check(child: &'static str, against: TypRef) -> Self {
        Self {
            child,
            action: Action::CheckAgainst(against),
            scope: ScopeOp::Same,
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
    /// Expand synonyms of [`TypRef::Expected`] (or the inferred base
    /// type); it must be a declared record, else "not a record".
    ExpectedMustBeRecord,
    /// Inferred record base must be a declared record, else
    /// "accessed type is not a struct".
    InferredMustBeRecord { child: &'static str },
    /// `SN_id` must equal the expected record; `SN_anon` never infers.
    StructNameMustMatch,
    /// Every record field must be present exactly once; missing fields
    /// are a hard error (OCaml keeps a `record_fields` set per literal).
    FullFieldCoverage,
    /// Drop arms whose pattern+guard prove `false`, then annotate the
    /// surviving match `complete`/`incomplete` via the Lane-09
    /// pattern-completeness oracle.
    MatchCompleteness,
    /// `E_vector_append(v, [])` checks/infers `v` directly; otherwise
    /// rewrite to `append(v1, v2)` and re-dispatch (never a direct
    /// length proof here).
    VectorAppendRewrite,
    /// `exit`/`throw` inhabit the expected type (diverging result):
    /// no compatibility check between payload and expected.
    DivergingResult,
    /// `E_config` fits any expected type without touching it.
    ConfigAcceptsAny,
    /// `E_ref` requires a declared register; locals never qualify.
    RefMustBeRegister,
    /// Empty-string assert messages elaborate to the source location
    /// string (`assert_msg`); non-empty messages check against `string`.
    AssertMsgCoercion,
    /// Mapping application: try `<name>_forwards`, on `Type_error`
    /// retry `<name>_backwards`, else raise both errors (`Err_no_overloading`).
    MappingFallback { forwards: String, backwards: String },
    /// Termination measure: the pattern binds against the single
    /// argument type, or against the tuple of all argument types when
    /// the function takes several arguments; the body checks `<= int`.
    MeasureArityTuple,
}

/// One missing canonical rule, exactly as Lane-16 must insert it.
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
/// INSERT (run match, before the trailing `_ =>` fallback):
/// ```ignore
/// ExpKind::Struct { name, fields } => {
///     let record = match self.resolve(&expected)?.node().kind {
///         TypKind::Id(_) | TypKind::App { .. } => self.record_of(&expected, exp.span)?,
///         _ => return Err(mismatch(exp.span, &expected, &self.builtin("struct"))),
///     };
///     // StructNameMustMatch: SN_id == record, SN_anon rejected in check mode
///     // only via mismatch (infer rejects anon outright).
///     let mut seen = std::collections::BTreeSet::new();
///     for fexp in fields.iter().rev() {
///         // FullFieldCoverage: unknown field or duplicate => error;
///         // after the loop, missing fields => error.
///         let field_typ = self.accessor_field(&record, &fexp.field, &expected, exp.span)?;
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
pub fn infer_struct(field_names: &[&str]) -> Plan {
    Plan {
        ocaml: "infer_exp E_struct ~L3774",
        mode: Mode::Infer,
        steps: field_names
            .iter()
            .map(|_| Step::infer("field_value"))
            .collect(),
        side: vec![SideCond::StructNameMustMatch, SideCond::FullFieldCoverage],
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
/// INSERT (run match):
/// ```ignore
/// ExpKind::StructUpdate { record, fields } => {
///     let record_typ = self.resolve(&expected)?;
///     // ExpectedMustBeRecord.
///     tasks.push(Task { exp: record, expected: expected.clone(),
///         scope: scope.clone(), returning: returning.clone() });
///     for fexp in fields.iter().rev() {
///         let field_typ = self.accessor_field_of(&record_typ, &fexp.field, exp.span)?;
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

// ---------------------------------------------------------------------------
// E_field
// ---------------------------------------------------------------------------

/// Inference-mode `E_field` (OCaml `infer_exp` ~L3748). Check mode falls
/// through to infer+subtype, so no check plan exists.
///
/// INSERT (infer `Synthesis::Enter`, beside the `ExpKind::App` arm):
/// ```ignore
/// ExpKind::Field { record, field } => {
///     let base = self.infer(record, &scope)?;
///     let field_name = self.name(field)?;
///     // InferredMustBeRecord(record): expand synonyms; Id/App of a
///     // declared record => accessor call, else "not a struct".
///     let scheme = self.program.accessor(&record_name, field_name, self.position)
///         .cloned().ok_or_else(|| TypeError::unbound_var(exp.span, field_name.to_owned()))?;
///     // instantiate the record telescope against base, result is the field type.
/// }
/// ```
pub fn infer_field(field: &str) -> Plan {
    Plan {
        ocaml: "infer_exp E_field ~L3748",
        mode: Mode::Infer,
        steps: vec![Step {
            child: "record",
            action: Action::FieldProject {
                record_child: "record",
                field: field.to_owned(),
            },
            scope: ScopeOp::Same,
        }],
        side: vec![SideCond::InferredMustBeRecord { child: "record" }],
    }
}

// ---------------------------------------------------------------------------
// E_match
// ---------------------------------------------------------------------------

/// Check-mode `E_match` (OCaml `check_exp` ~L2176). No infer plan:
/// inference is unsupported and must keep failing.
///
/// Per-arm work is Lane-09 `check_case`: bind the pattern against the
/// scrutinee type, drop the arm when the environment proves `false`,
/// check an optional guard `<= bool`, check the body `<= Expected`
/// under the arm scope (+ guard fact).
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Match { scrutinee, arms } => {
///     // mapping_match attribute => check scrutinee <= option(Expected),
///     // else infer it (condition() helper covers the bool special-case;
///     // here any type infers).
///     let scrut = self.infer(scrutinee, &scope)?;
///     for (i, arm) in arms.iter().enumerate().rev() {
///         // Lane-09: pattern scope for arm i against scrut type;
///         // skip arm when scope proves false (DropImpossibleArms).
///         // Guard (if any): Task { exp: guard, expected: bool, scope: PatArm{i,false} }
///         // Body: Task { exp: body, expected, scope: PatArm{i, with_guard_fact} }
///     }
///     // MatchCompleteness: annotate complete/incomplete via Lane-09 oracle.
/// }
/// ```
pub fn check_match(arms: usize, mapping_match_attr: bool) -> Plan {
    let scrutinee = if mapping_match_attr {
        Step::check("scrutinee", TypRef::OptionOfExpected)
    } else {
        Step::infer("scrutinee")
    };
    let mut steps = vec![scrutinee];
    for arm in 0..arms {
        steps.push(Step {
            child: "guard",
            action: Action::CheckAgainst(TypRef::Builtin("bool")),
            scope: ScopeOp::PatArm {
                arm,
                with_guard_fact: false,
            },
        });
        steps.push(Step {
            child: "body",
            action: Action::CheckAgainst(TypRef::Expected),
            scope: ScopeOp::PatArm {
                arm,
                with_guard_fact: true,
            },
        });
    }
    Plan {
        ocaml: "check_exp E_match ~L2176",
        mode: Mode::Check,
        steps,
        side: vec![SideCond::MatchCompleteness],
    }
}

// ---------------------------------------------------------------------------
// E_vector_append
// ---------------------------------------------------------------------------

/// `E_vector_append` (OCaml `check_exp` ~L2262 / `infer_exp` ~L4019):
/// empty right literal is the identity, otherwise the whole append
/// re-dispatches as `append(v1, v2)`.
///
/// INSERT (run match AND infer Enter — same shape, mode differs):
/// ```ignore
/// // check mode:
/// ExpKind::VectorAppend anaerBox { first, second } => {
///     if matches!(&second.kind, ExpKind::Vector(items) if items.is_empty()) {
///         tasks.push(Task { exp: first, expected, scope, returning });
///     } else {
///         // rewrite to App { func: "append", args: [first, second] }
///         // and re-dispatch through the App arm (needs a synthetic Exp node).
///     }
/// }
/// // infer mode: same, but infer(first) / infer(App{append}).
/// ```
pub fn check_vector_append() -> Plan {
    Plan {
        ocaml: "check_exp E_vector_append ~L2262",
        mode: Mode::Check,
        steps: vec![
            Step::check("first", TypRef::Expected),
            Step::check("second", TypRef::AppendCall),
        ],
        side: vec![SideCond::VectorAppendRewrite],
    }
}

/// Inference twin of [`check_vector_append`].
pub fn infer_vector_append() -> Plan {
    Plan {
        ocaml: "infer_exp E_vector_append ~L4019",
        mode: Mode::Infer,
        steps: vec![Step::infer("first"), Step::infer("second")],
        side: vec![SideCond::VectorAppendRewrite],
    }
}

// ---------------------------------------------------------------------------
// E_exit / E_throw
// ---------------------------------------------------------------------------

/// Check-mode `E_exit` (OCaml `check_exp` ~L2391): payload `<= unit`,
/// the exit itself inhabits the expected type. No infer plan.
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Exit(child) => tasks.push(Task {
///     exp: child, expected: self.builtin("unit"), scope, returning,
/// }),
/// // DivergingResult: no compatibility check against `expected`.
/// ```
pub fn check_exit() -> Plan {
    Plan {
        ocaml: "check_exp E_exit ~L2391",
        mode: Mode::Check,
        steps: vec![Step::check("payload", TypRef::Builtin("unit"))],
        side: vec![SideCond::DivergingResult],
    }
}

/// Check-mode `E_throw` (OCaml `check_exp` ~L2394): payload `<= exc`,
/// the throw itself inhabits the expected type. No infer plan.
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Throw(child) => tasks.push(Task {
///     exp: child, expected: self.exc_type(exp.span)?, scope, returning,
/// }),
/// // DivergingResult: no compatibility check against `expected`.
/// ```
pub fn check_throw() -> Plan {
    Plan {
        ocaml: "check_exp E_throw ~L2394",
        mode: Mode::Check,
        steps: vec![Step::check("payload", TypRef::Exc)],
        side: vec![SideCond::DivergingResult],
    }
}

// ---------------------------------------------------------------------------
// E_config
// ---------------------------------------------------------------------------

/// Check-mode `E_config` (OCaml `check_exp` ~L2365): `%config` keys fit
/// any expected type with no premise at all. No infer plan.
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Config(_) => {}
/// // ConfigAcceptsAny: intentionally no task and no compatibility check.
/// ```
pub fn check_config() -> Plan {
    Plan {
        ocaml: "check_exp E_config ~L2365",
        mode: Mode::Check,
        steps: Vec::new(),
        side: vec![SideCond::ConfigAcceptsAny],
    }
}

// ---------------------------------------------------------------------------
// E_ref
// ---------------------------------------------------------------------------

/// Inference-mode `E_ref` (OCaml `infer_exp` ~L4090): the name must be a
/// declared register; the result is `register(payload)`. Check mode
/// falls through to infer+subtype, so no check plan exists.
///
/// INSERT (infer `Synthesis::Enter`):
/// ```ignore
/// ExpKind::Ref(name) => {
///     let name_text = self.name(name)?;
///     let scheme = self.program.register(name_text, self.position)
///         .cloned().ok_or_else(|| TypeError::unbound_var(exp.span, name_text.to_owned()))?;
///     // RefMustBeRegister: no local/env fallback.
///     self.register_type(scheme) // register(payload) constructor
/// }
/// ```
pub fn infer_ref() -> Plan {
    Plan {
        ocaml: "infer_exp E_ref ~L4090",
        mode: Mode::Infer,
        steps: Vec::new(),
        side: vec![SideCond::RefMustBeRegister],
    }
}

// ---------------------------------------------------------------------------
// E_try
// ---------------------------------------------------------------------------

/// Check-mode `E_try` (OCaml `check_exp` ~L2196): the body checks
/// against the expected type, every arm binds against `exc` and checks
/// its body against the expected type. Impossible arms are dropped
/// like `match` arms. No infer plan.
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Try { exp, arms } => {
///     tasks.push(Task { exp, expected: expected.clone(), scope: scope.clone(),
///         returning: returning.clone() });
///     for (i, arm) in arms.iter().enumerate().rev() {
///         // Lane-09: pattern scope for arm i against exc type;
///         // skip arm when scope proves false.
///         // Guard (if any): Task { exp: guard, expected: bool, scope: PatArm{i,false} }
///         // Body: Task { exp: body, expected, scope: PatArm{i, with_guard_fact} }
///     }
/// }
/// ```
pub fn check_try(arms: usize) -> Plan {
    let mut steps = vec![Step::check("exp", TypRef::Expected)];
    for arm in 0..arms {
        steps.push(Step {
            child: "guard",
            action: Action::CheckAgainst(TypRef::Builtin("bool")),
            scope: ScopeOp::PatArm {
                arm,
                with_guard_fact: false,
            },
        });
        steps.push(Step {
            child: "body",
            action: Action::CheckAgainst(TypRef::Expected),
            scope: ScopeOp::PatArm {
                arm,
                with_guard_fact: true,
            },
        });
    }
    Plan {
        ocaml: "check_exp E_try ~L2196",
        mode: Mode::Check,
        steps,
        side: vec![SideCond::MatchCompleteness],
    }
}

// ---------------------------------------------------------------------------
// E_assert
// ---------------------------------------------------------------------------

/// Inference-mode `E_assert` (OCaml `infer_exp` ~L4039): test `<=
/// bool`, message `<= string` (empty literal coerced to the source
/// location), result `unit`. Check mode falls through to
/// infer+subtype, so no check plan exists. The in-block assertion
/// reading (adding the asserted constraint to the scope, dead-code
/// elimination after provable `false`) lives in `check_block`
/// (~L2638) and is out of scope for the expression arm.
///
/// INSERT (infer `Synthesis::Enter`):
/// ```ignore
/// ExpKind::Assert { cond, msg } => {
///     // desugar first: empty-string msg => location-string literal.
///     tasks.push(Task { exp: msg, expected: self.builtin("string"), ... });
///     tasks.push(Task { exp: cond, expected: self.builtin("bool"), ... });
///     // result type: unit.
/// }
/// ```
/// NOTE: `run` is task-stack based while `infer` synthesizes inline;
/// the Lane-16 insertion must stage cond/msg through the same
/// `Synthesis::Enter` queue the `Vector`/`Call` synthesis uses, then
/// yield `unit`.
pub fn infer_assert() -> Plan {
    Plan {
        ocaml: "infer_exp E_assert ~L4039",
        mode: Mode::Infer,
        steps: vec![
            Step::check("cond", TypRef::Builtin("bool")),
            Step::check("msg", TypRef::Builtin("string")),
        ],
        side: vec![SideCond::AssertMsgCoercion],
    }
}

// ---------------------------------------------------------------------------
// Termination measure
// ---------------------------------------------------------------------------

/// Termination-measure typing (OCaml `check_termination_measure`
/// ~L4829, wired into `check_fundef` for `Rec_measure` ~L4960 and into
/// `DEF_measure` ~L5616).
///
/// The measure pattern binds against the single argument type, or
/// against the tuple of all argument types for multi-argument
/// functions; the measure body checks `<= int` in the clause
/// environment extended with the pattern bindings.
///
/// INSERT (new `Checker::measure` called from `function()` when
/// `RecOpt::Measure { pat, exp }` is present, replacing the current
/// `unsupported(def.span, "termination-measure typing")` early return):
/// ```ignore
/// fn measure(&mut self, pat: &Pat, exp: &Exp, args: &[Type], scope: &Scope) -> TypeResult<()> {
///     let bound = if args.len() == 1 { args[0].clone() } else { tuple_of(args) };
///     let mut child = Scope { locals: BTreeMap::new(), facts: Vec::new(),
///         parent: Some(scope_as_arc) };
///     self.pattern(pat, bound, false, &mut child)?;
///     // Task { exp, expected: builtin("int"), scope: Arc::new(child), returning: int }
/// }
/// ```
/// The `function()` dispatcher keeps checking every clause body as
/// today; the measure runs once per definition in the first clause's
/// scope (OCaml checks it in `funcl_env`, shared across clauses).
pub fn check_measure(_multi_arg: bool) -> Plan {
    Plan {
        ocaml: "check_termination_measure ~L4829",
        mode: Mode::Check,
        steps: vec![
            Step {
                child: "measure_pat",
                action: Action::CheckAgainst(TypRef::Expected),
                scope: ScopeOp::Same,
            },
            Step::check("measure_exp", TypRef::Builtin("int")),
        ],
        side: vec![SideCond::MeasureArityTuple],
    }
}

/// The argument shape a termination measure binds against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureArity {
    Single,
    Tuple,
}

/// Companion to [`check_measure`]: which binding shape the insertion
/// must build. Kept separate so tests pin the rule without overloading
/// [`Plan`] with definition-level inputs.
#[must_use]
pub fn measure_arity(arg_count: usize) -> MeasureArity {
    if arg_count == 1 {
        MeasureArity::Single
    } else {
        MeasureArity::Tuple
    }
}

// ---------------------------------------------------------------------------
// Mapping expansion
// ---------------------------------------------------------------------------

/// Mapping application (OCaml `check_exp` mapping arm ~L2299 and
/// `infer_exp` mapping arm ~L3850).
///
/// `f(xs)` with `f` a declared mapping rewrites to
/// `<f>_forwards(xs)`; on `Type_error` it retries
/// `<f>_backwards(xs)`; when both fail the insertion raises both
/// errors (`Err_no_overloading`). This is a re-dispatch, not a direct
/// task: the insertion synthesizes the rewritten `App` node and feeds
/// it back through the `App` arm (check mode threads `Some(expected)`
/// as the return context, infer mode threads `None`).
///
/// INSERT (top of the `ExpKind::App` arms in both `run` and `infer`,
/// before overload resolution):
/// ```ignore
/// if let Some((forwards, backwards)) = self.program.mapping_expansion(func_name, self.position) {
///     // try forwards App; on TypeError retry backwards App;
///     // on double failure return the combined overload error.
/// }
/// ```
/// Requires the Lane-19 `Program::mappings` table (`program.rs`):
/// `is_mapping` gates the rewrite, `mapping_expansion` supplies the
/// generated clause-function names.
pub fn mapping_app(mapping: &str) -> Plan {
    Plan {
        ocaml: "mapping application check ~L2299 / infer ~L3850",
        mode: Mode::Check,
        steps: vec![Step::check("arg", TypRef::AppendCall)],
        side: vec![SideCond::MappingFallback {
            forwards: format!("{mapping}_forwards"),
            backwards: format!("{mapping}_backwards"),
        }],
    }
}

/// Inference twin of [`mapping_app`]: identical fallback order, return
/// context `None` instead of `Some(expected)`.
pub fn infer_mapping_app(mapping: &str) -> Plan {
    let mut plan = mapping_app(mapping);
    plan.ocaml = "infer_exp mapping application ~L3850";
    plan.mode = Mode::Infer;
    plan
}

/// Canonical generated clause-function names for a mapping
/// (mirrors the OCaml `mk_id (name ^ "_forwards")` convention).
#[must_use]
pub fn mapping_expansion_names(mapping: &str) -> (String, String) {
    (
        format!("{mapping}_forwards"),
        format!("{mapping}_backwards"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_check_pins_ocaml_premises() {
        let plan = check_struct(&["x", "y"]);
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(plan.steps.len(), 2);
        for (step, field) in plan.steps.iter().zip(["x", "y"]) {
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
    fn struct_infer_has_no_expected_premise() {
        let plan = infer_struct(&["x"]);
        assert_eq!(plan.mode, Mode::Infer);
        assert_eq!(plan.steps, vec![Step::infer("field_value")]);
        assert!(!plan
            .steps
            .iter()
            .any(|s| matches!(s.action, Action::CheckAgainst(_))));
    }

    #[test]
    fn struct_update_checks_base_first_without_coverage() {
        let plan = check_struct_update(&["f"]);
        assert_eq!(plan.steps[0], Step::check("record", TypRef::Expected));
        assert_eq!(plan.steps.len(), 2);
        assert!(!plan.side.contains(&SideCond::FullFieldCoverage));
        let infer = infer_struct_update(&["f"]);
        assert_eq!(infer.steps[0], Step::infer("record"));
        assert_eq!(
            infer.steps[1].action,
            Action::CheckAgainst(TypRef::RecordField {
                base: Box::new(TypRef::Inferred("record")),
                field: "f".to_owned(),
            })
        );
    }

    #[test]
    fn field_is_infer_only_projection() {
        let plan = infer_field("f");
        assert_eq!(plan.mode, Mode::Infer);
        assert_eq!(
            plan.steps,
            vec![Step {
                child: "record",
                action: Action::FieldProject {
                    record_child: "record",
                    field: "f".to_owned(),
                },
                scope: ScopeOp::Same,
            }]
        );
    }

    #[test]
    fn match_checks_guard_then_body_per_arm() {
        let plan = check_match(2, false);
        assert_eq!(plan.mode, Mode::Check);
        assert_eq!(plan.steps[0], Step::infer("scrutinee"));
        assert_eq!(plan.steps.len(), 1 + 2 * 2);
        assert_eq!(
            plan.steps[1].scope,
            ScopeOp::PatArm {
                arm: 0,
                with_guard_fact: false,
            }
        );
        assert_eq!(
            plan.steps[2],
            Step {
                child: "body",
                action: Action::CheckAgainst(TypRef::Expected),
                scope: ScopeOp::PatArm {
                    arm: 0,
                    with_guard_fact: true,
                },
            }
        );
        assert!(plan.side.contains(&SideCond::MatchCompleteness));
    }

    #[test]
    fn match_mapping_attr_checks_option_of_expected() {
        let plan = check_match(1, true);
        assert_eq!(
            plan.steps[0],
            Step::check("scrutinee", TypRef::OptionOfExpected)
        );
    }

    #[test]
    fn vector_append_rewrite_in_both_modes() {
        assert!(check_vector_append()
            .side
            .contains(&SideCond::VectorAppendRewrite));
        assert!(infer_vector_append()
            .side
            .contains(&SideCond::VectorAppendRewrite));
        assert_eq!(check_vector_append().mode, Mode::Check);
        assert_eq!(infer_vector_append().mode, Mode::Infer);
    }

    #[test]
    fn exit_and_throw_diverge_with_payload_obligations() {
        assert_eq!(
            check_exit().steps,
            vec![Step::check("payload", TypRef::Builtin("unit"))]
        );
        assert_eq!(
            check_throw().steps,
            vec![Step::check("payload", TypRef::Exc)]
        );
        for plan in [check_exit(), check_throw()] {
            assert!(plan.side.contains(&SideCond::DivergingResult));
        }
    }

    #[test]
    fn config_has_no_premise_and_accepts_any_type() {
        let plan = check_config();
        assert!(plan.steps.is_empty());
        assert!(plan.side.contains(&SideCond::ConfigAcceptsAny));
    }

    #[test]
    fn ref_requires_register() {
        let plan = infer_ref();
        assert_eq!(plan.mode, Mode::Infer);
        assert!(plan.steps.is_empty());
        assert!(plan.side.contains(&SideCond::RefMustBeRegister));
    }

    #[test]
    fn try_checks_body_then_arms_against_expected() {
        let plan = check_try(1);
        assert_eq!(plan.steps[0], Step::check("exp", TypRef::Expected));
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(
            plan.steps[2].scope,
            ScopeOp::PatArm {
                arm: 0,
                with_guard_fact: true,
            }
        );
    }

    #[test]
    fn assert_checks_bool_then_string_with_coercion() {
        let plan = infer_assert();
        assert_eq!(
            plan.steps,
            vec![
                Step::check("cond", TypRef::Builtin("bool")),
                Step::check("msg", TypRef::Builtin("string")),
            ]
        );
        assert!(plan.side.contains(&SideCond::AssertMsgCoercion));
    }

    #[test]
    fn measure_binds_single_or_tuple_and_checks_int() {
        let plan = check_measure(false);
        assert_eq!(
            plan.steps[1],
            Step::check("measure_exp", TypRef::Builtin("int"))
        );
        assert!(plan.side.contains(&SideCond::MeasureArityTuple));
        assert_eq!(measure_arity(1), MeasureArity::Single);
        assert_eq!(measure_arity(0), MeasureArity::Tuple);
        assert_eq!(measure_arity(3), MeasureArity::Tuple);
    }

    #[test]
    fn mapping_fallback_order_is_forwards_first() {
        let plan = mapping_app("decode");
        assert_eq!(
            plan.side,
            vec![SideCond::MappingFallback {
                forwards: "decode_forwards".to_owned(),
                backwards: "decode_backwards".to_owned(),
            }]
        );
        assert_eq!(
            mapping_expansion_names("decode"),
            ("decode_forwards".to_owned(), "decode_backwards".to_owned())
        );
        let infer = infer_mapping_app("decode");
        assert_eq!(infer.mode, Mode::Infer);
        assert_eq!(infer.side, plan.side);
    }
}
