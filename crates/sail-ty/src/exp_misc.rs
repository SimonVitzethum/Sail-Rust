#![forbid(unsafe_code)]
//! Kleine `Exp`-Arme (Lane 27, exklusiv neu).
//!
//! Reine Entscheidungshelfer fuer die sechs kleinen `Exp`-Arme aus
//! OCaml `type_check.ml` (sail2-Branch, `src/lib/type_check.ml`), deren
//! Plaene in [`crate::exp_arms`] stehen:
//!
//! | Arm | OCaml | Modus |
//! | --- | --- | --- |
//! | `E_field` | `infer_exp` ~L3748 | infer-only (check faellt auf infer+Subtyp zurueck) |
//! | `E_ref` | `infer_exp` ~L4090 | infer-only (check faellt auf infer+Subtyp zurueck) |
//! | `E_config` | `check_exp` ~L2365 | check-only, passt zu jedem erwarteten Typ |
//! | `E_exit` | `check_exp` ~L2391 | check-only, Payload `<= unit`, divergierendes Ergebnis |
//! | `E_throw` | `check_exp` ~L2394 | check-only, Payload `<= exc`, divergierendes Ergebnis |
//! | `E_assert` | `infer_exp` ~L4039 | infer-only (check faellt auf infer+Subtyp zurueck) |
//! | `E_vector_append` | `check_exp` ~L2262 / `infer_exp` ~L4019 | beide Modi, Rewrite zu `append` |
//!
//! Das Modul ist bewusst rein (`MiniTyp`/`&str`/`bool` rein und raus, kein
//! `Program`-Zugriff), damit es ohne Aenderung an `program.rs`,
//! `canonical_body.rs` oder `pat.rs` nutzbar und testbar ist — diese drei
//! Dateien bleiben unveraendert (nur gelesen). Die Verdrahtung erfolgt ueber
//! die [`HOOK_NOTE`]-Insert-Specs.
//!
//! Modellierung: Die Lane-79-`MiniTyp`-Teilmenge kennt keine
//! Record-/Register-Koepfe (Record-Payloads = Lane 08 `Program::accessor`,
//! Register-Typen = `register(payload)`-Konstruktor); darum nimmt die
//! Feldprojektion einen `Named`-Kopf als Record-Kandidaten und die
//! Ref-Pruefung eine vom Aufrufer klassifizierte [`RefBinding`]. Das ist kein
//! Schatten-Typsystem: Sobald die kanonische Pruefung die Arme elaboriert,
//! uebersetzt der Hook ([`HOOK_NOTE`]) die kanonische `Type`-Ansicht genau in
//! diese Obligationen.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::nexp::{MiniTyp, Nexp};

/// Hook-Notiz fuer die spaetere `canonical_body`-Verdrahtung: `program.rs`,
/// `canonical_body.rs` und `pat.rs` bleiben unveraendert (nur gelesen).
/// Die spaetere Verdrahtung ruft die Helfer dieses Moduls an den
/// `ExpKind`-Stellen mit expandierter kanonischer Ansicht auf (Details in
/// den einzelnen Insert-Specs unten).
pub const HOOK_NOTE: &str =
    "canonical_body: Synonyme expandieren, dann exp_misc-Klassifikation aufrufen; \
     Program-Lookups (accessor/register) bleiben an der Call-Site";

// ---------------------------------------------------------------------------
// Modus-Tabelle
// ---------------------------------------------------------------------------

/// Kleiner `Exp`-Arm dieses Moduls (genau die Lane-27-Arme).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MiscArm {
    /// `E_field` (`infer_exp` ~L3748).
    Field,
    /// `E_ref` (`infer_exp` ~L4090).
    Ref,
    /// `E_config` (`check_exp` ~L2365).
    Config,
    /// `E_exit` (`check_exp` ~L2391).
    Exit,
    /// `E_throw` (`check_exp` ~L2394).
    Throw,
    /// `E_assert` (`infer_exp` ~L4039).
    Assert,
    /// `E_vector_append` (`check_exp` ~L2262 / `infer_exp` ~L4019).
    VectorAppend,
}

/// Bidirektionaler Modus eines [`MiscArm`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BidirectionalMode {
    /// Nur `run` (`check_exp`); kein Inferenzplan.
    CheckOnly,
    /// Nur `infer` (`Synthesis::Enter`); check faellt auf infer+Subtyp zurueck.
    InferOnly,
    /// Beide Modi mit identischer Klassifikation.
    Both,
}

/// Modus-Tabelle der Lane-27-Arme.
///
/// INSERT: kein Laufzeit-Code; die Tabelle pinnt nur, welcher `canonical_body`
/// -Arm existiert (check-only: nur `run`-Match; infer-only: nur
/// `Synthesis::Enter`; both: beide) und wo check auf infer+Subtyp zurueckfaellt.
#[must_use]
pub fn mode(arm: MiscArm) -> BidirectionalMode {
    match arm {
        MiscArm::Field | MiscArm::Ref | MiscArm::Assert => BidirectionalMode::InferOnly,
        MiscArm::Config | MiscArm::Exit | MiscArm::Throw => BidirectionalMode::CheckOnly,
        MiscArm::VectorAppend => BidirectionalMode::Both,
    }
}

// ---------------------------------------------------------------------------
// E_field (infer ~L3748)
// ---------------------------------------------------------------------------

/// Record-Kopf eines inferierten Feld-Basis-Typs.
///
/// Gibt genau dann `Some((name, args))` zurueck, wenn `base` ein benannter
/// Kopf ist (`MiniTyp::Named`, d.h. OCaml `Typ_id`/`Typ_app` nach
/// `expand_synonyms`): nur dann lohnt der Accessor-Lookup
/// (`Program::accessor`, Lane 08). Primitive (`int`/`bool`/`bit`/`bits`),
/// `range`, Tupel und Funktionen sind keine Records.
#[must_use]
pub fn field_record(base: &MiniTyp) -> Option<(&str, &[Nexp])> {
    match base {
        MiniTyp::Named { name, args } => Some((name.as_str(), args.as_slice())),
        MiniTyp::Int
        | MiniTyp::Bool
        | MiniTyp::Bit
        | MiniTyp::Bits(_)
        | MiniTyp::Range(..)
        | MiniTyp::Fun(..)
        | MiniTyp::Tuple(_) => None,
    }
}

/// Prueft die inferierte Basis einer `E_field`-Projektion (OCaml `infer_exp`
/// ~L3748, `InferredMustBeRecord`).
///
/// Liefert den Record-Kopf fuer den Accessor-Lookup; sonst
/// `"accessed type is not a struct"` als `Mismatch` (erwartet `record`).
///
/// INSERT (infer `Synthesis::Enter`, neben dem `ExpKind::App`-Arm):
/// ```ignore
/// ExpKind::Field { record, field } => {
///     let base = self.infer(record, &scope)?;
///     // Mini-Ebene: mini_base aus base uebersetzen, dann:
///     let (rec, _args) = crate::exp_misc::check_field_base(&mini_base, exp.span)?;
///     let scheme = self.program.accessor(rec, field_name, self.position)
///         .cloned().ok_or_else(|| TypeError::unbound_var(exp.span, field_name.to_owned()))?;
///     // Record-Teleskop gegen base instantiieren, Ergebnis ist der Feldtyp.
/// }
/// ```
/// Check-Modus existiert nicht: check faellt auf infer+Subtyp zurueck
/// ([`mode`] == [`BidirectionalMode::InferOnly`]).
pub fn check_field_base(base: &MiniTyp, span: Span) -> TypeResult<(&str, &[Nexp])> {
    field_record(base).ok_or_else(|| {
        TypeError::mismatch(
            span,
            "record".to_owned(),
            format!("accessed type is not a struct, it has type {base}"),
        )
    })
}

// ---------------------------------------------------------------------------
// E_ref (infer ~L4090)
// ---------------------------------------------------------------------------

/// Bindungsart eines `E_ref`-Namens (OCaml `Env.is_register`, ~L4090).
///
/// Der Aufrufer klassifiziert ueber `Scope`/`Program`: Nur
/// [`RefBinding::Register`] traegt die Payload; Locals qualifizieren sich
/// nie — auch dann nicht, wenn der Name im Scope gebunden ist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefBinding {
    /// Deklariertes Register mit Payload-Typ (`Env.get_register`).
    Register(MiniTyp),
    /// Im Scope gebundenes Local: nie ein Register.
    Local,
    /// Weder Local noch Register noch Enum: ungebunden.
    Unbound,
}

/// Prueft `E_ref` (OCaml `infer_exp` ~L4090, `RefMustBeRegister`).
///
/// Liefert die Register-Payload; die Insert-Spec wickelt sie in den
/// `register(payload)`-Konstruktor (`register_typ`). Locals werden mit
/// `RegisterError` abgewiesen (kein Local-Fallback), Unbounds mit
/// `unbound_var`.
///
/// INSERT (infer `Synthesis::Enter`):
/// ```ignore
/// ExpKind::Ref(name) => {
///     let name_text = self.name(name)?;
///     let binding = if let Some(scheme) = self.program.register(name_text, self.position) {
///         crate::exp_misc::RefBinding::Register(payload_of(scheme))
///     } else if scope.get(name_text).is_some() {
///         crate::exp_misc::RefBinding::Local
///     } else {
///         crate::exp_misc::RefBinding::Unbound
///     };
///     let payload = crate::exp_misc::check_ref(name_text, &binding, exp.span)?;
///     self.register_type(payload) // register(payload)-Konstruktor
/// }
/// ```
/// Check-Modus existiert nicht: check faellt auf infer+Subtyp zurueck
/// ([`mode`] == [`BidirectionalMode::InferOnly`]).
pub fn check_ref(name: &str, binding: &RefBinding, span: Span) -> TypeResult<MiniTyp> {
    match binding {
        RefBinding::Register(payload) => Ok(payload.clone()),
        RefBinding::Local => Err(TypeError {
            span,
            kind: TypeErrorKind::RegisterError(format!(
                "cannot take a reference to local '{name}' (not a register)"
            )),
        }),
        RefBinding::Unbound => Err(TypeError::unbound_var(span, name.to_owned())),
    }
}

// ---------------------------------------------------------------------------
// E_config (check ~L2365)
// ---------------------------------------------------------------------------

/// Prueft `E_config` (OCaml `check_exp` ~L2365, `ConfigAcceptsAny`).
///
/// `%config`-Schluessel passen zu jedem erwarteten Typ ganz ohne Praemisse:
/// kein Task, kein Kompatibilitaets-Check. Immer `Ok(())`.
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Config(keys) => {
///     crate::exp_misc::check_config(keys)?;
///     // ConfigAcceptsAny: bewusst kein Task, kein Kompatibilitaets-Check.
/// }
/// ```
/// Kein Inferenzplan (`infer` bleibt `unsupported`).
pub fn check_config(_keys: &[String]) -> TypeResult<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// E_exit / E_throw (check ~L2391 / ~L2394)
// ---------------------------------------------------------------------------

/// Divergierender Arm: `E_exit` oder `E_throw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Diverging {
    /// `E_exit` (OCaml `check_exp` ~L2391).
    Exit,
    /// `E_throw` (OCaml `check_exp` ~L2394).
    Throw,
}

/// Payload-Obligation eines divergierenden Arms: der Builtin-Name, gegen den
/// das Payload-Kind zu pruefen ist (`exit`: `unit`, `throw`: `exc` =
/// `register exc`-Payload).
#[must_use]
pub fn diverging_payload_builtin(kind: Diverging) -> &'static str {
    match kind {
        Diverging::Exit => "unit",
        Diverging::Throw => "exc",
    }
}

/// Kind-Name des Payload-Kinds (`"payload"`, wie in [`crate::exp_arms`]).
#[must_use]
pub fn diverging_payload_child(kind: Diverging) -> &'static str {
    let _ = kind;
    "payload"
}

/// Pruef-Praemisse eines divergierenden Arms als `(child, builtin)`-Paar:
/// `("payload", "unit")` fuer `exit`, `("payload", "exc")` fuer `throw`.
/// Das Ergebnis bewohnt den erwarteten Typ ohne Kompatibilitaets-Check
/// (`DivergingResult`).
///
/// INSERT (run match):
/// ```ignore
/// ExpKind::Exit(child) => tasks.push(Task {
///     exp: child, expected: self.builtin(crate::exp_misc::diverging_payload_builtin(
///         crate::exp_misc::Diverging::Exit)), scope, returning,
/// }),
/// // DivergingResult: kein Kompatibilitaets-Check gegen `expected`.
/// ExpKind::Throw(child) => tasks.push(Task {
///     exp: child, expected: self.exc_type(exp.span)?, scope, returning,
/// }),
/// // DivergingResult: kein Kompatibilitaets-Check gegen `expected`.
/// ```
/// Kein Inferenzplan (`infer` bleibt `unsupported`).
#[must_use]
pub fn diverging_obligation(kind: Diverging) -> (&'static str, &'static str) {
    (
        diverging_payload_child(kind),
        diverging_payload_builtin(kind),
    )
}

// ---------------------------------------------------------------------------
// E_assert (infer ~L4039)
// ---------------------------------------------------------------------------

/// Builtin-Obligationen von `E_assert` (OCaml `infer_exp` ~L4039):
/// Test `<= bool`, Nachricht `<= string`, Ergebnis `unit`.
pub const ASSERT_COND_BUILTIN: &str = "bool";
/// Siehe [`ASSERT_COND_BUILTIN`].
pub const ASSERT_MSG_BUILTIN: &str = "string";
/// Siehe [`ASSERT_COND_BUILTIN`].
pub const ASSERT_RESULT_BUILTIN: &str = "unit";

/// `E_assert`-Praemissen als `[(child, builtin)]`: `[("cond", "bool"),
/// ("msg", "string")]` in OCaml-Reihenfolge (Test zuerst, vgl. `check_block`
/// ~L2638 und `infer_exp` ~L4039).
#[must_use]
pub fn assert_obligations() -> [(&'static str, &'static str); 2] {
    [("cond", ASSERT_COND_BUILTIN), ("msg", ASSERT_MSG_BUILTIN)]
}

/// Nachrichtenseite von `E_assert` nach OCaml `assert_msg` (~L1977):
/// Ein leeres String-Literal wird zuerst zum Quellort-String desugart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertMsg {
    /// Leeres String-Literal: durch den Location-String ersetzen
    /// (`L_string (short_loc_to_string l)`), dann `<= string` pruefen.
    Location,
    /// Alles andere (inkl. Nicht-Literale): direkt `<= string` pruefen.
    Keep,
}

/// Klassifiziert die Assert-Nachricht nach OCaml `assert_msg`.
///
/// `msg_text` ist der String-Payload genau dann, wenn `msg` ein
/// `E_lit(L_string)` ist (der Aufrufer loest das Literal ueber den Interner;
/// `None` = kein String-Literal). Nur `Some("")` desugart zum
/// Location-String; alles andere bleibt unveraendert.
///
/// INSERT (infer `Synthesis::Enter`; Staging ueber dieselbe
/// `Synthesis::Enter`-Queue wie `Vector`/`Call`, Ergebnis `unit`):
/// ```ignore
/// ExpKind::Assert { cond, msg } => {
///     let msg_text: Option<&str> = match &msg.kind {
///         ExpKind::Lit(lit) => match &lit.kind {
///             LitKind::String(sym) => Some(self.it.resolve(*sym).unwrap_or("?")),
///             _ => None,
///         },
///         _ => None,
///     };
///     let msg = match crate::exp_misc::assert_msg_action(msg_text) {
///         crate::exp_misc::AssertMsg::Location => location_string_literal(msg.span),
///         crate::exp_misc::AssertMsg::Keep => msg.clone(),
///     };
///     // tasks: msg <= string, cond <= bool (diese Reihenfolge einreihen,
///     // Ergebnis: unit). In-Block-Constraint (Scope-Fakt, Dead-Code-Elim
///     // nach beweisbarem `false`) gehoert `check_block` (~L2638), nicht hier.
/// }
/// ```
/// Check-Modus existiert nicht: check faellt auf infer+Subtyp zurueck
/// ([`mode`] == [`BidirectionalMode::InferOnly`]).
#[must_use]
pub fn assert_msg_action(msg_text: Option<&str>) -> AssertMsg {
    match msg_text {
        Some("") => AssertMsg::Location,
        _ => AssertMsg::Keep,
    }
}

// ---------------------------------------------------------------------------
// E_vector_append (check ~L2262 / infer ~L4019)
// ---------------------------------------------------------------------------

/// `append`-Funktionsname des `E_vector_append`-Rewrites
/// (OCaml `mk_id "append"`).
pub const APPEND_FUN: &str = "append";

/// Klassifikation von `E_vector_append` (beide Modi identisch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorAppendAction {
    /// Rechts ist das leere Vektor-Literal (`E_vector []`): Identitaet —
    /// `first` direkt checken/inferieren, kein Rewrite.
    Identity,
    /// Sonst: als `append(first, second)`-App neu dispatchen (Rewrite +
    /// Re-Dispatch durch den `App`-Arm, nie direkter Laengenbeweis hier).
    RewriteAppend,
}

/// Klassifiziert `E_vector_append (first, second)`.
///
/// `second_is_empty_vector` ist genau dann wahr, wenn `second` die Form
/// `E_aux (E_vector [], _)` hat (der Aufrufer matcht die AST-Form; hier nur
/// das `bool` herein, damit das Modul AST-frei bleibt). Modus-unabhaengig:
/// check (~L2262) und infer (~L4019) verzweigen identisch.
///
/// INSERT (run match UND infer `Synthesis::Enter` — gleiche Form, Modus
/// unterscheidet sich nur im Dispatch):
/// ```ignore
/// // check-Modus:
/// ExpKind::VectorAppend(first, second) => {
///     match crate::exp_misc::classify_vector_append(
///         matches!(&second.kind, ExpKind::Vector(items) if items.is_empty())) {
///         crate::exp_misc::VectorAppendAction::Identity => {
///             tasks.push(Task { exp: first, expected, scope, returning });
///         }
///         crate::exp_misc::VectorAppendAction::RewriteAppend => {
///             // synthetischer App-Knoten { func: "append", args: [first, second] }
///             // (Name via crate::exp_misc::APPEND_FUN), Re-Dispatch durch den App-Arm.
///         }
///     }
/// }
/// // infer-Modus: identisch, aber infer(first) / infer(App{append}).
/// ```
#[must_use]
pub fn classify_vector_append(second_is_empty_vector: bool) -> VectorAppendAction {
    if second_is_empty_vector {
        VectorAppendAction::Identity
    } else {
        VectorAppendAction::RewriteAppend
    }
}
