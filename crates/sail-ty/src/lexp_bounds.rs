#![forbid(unsafe_code)]
//! Vector LHS bounds (Lane 07, exklusiv neu).
//!
//! Regelbeschreibung (Spiegel von OCaml `type_check.ml::infer_lexp`,
//! Faelle `LE_vector` / `LE_vector_range`):
//!
//! - `LE_vector (v_lexp, index)`:
//!   Wurzel rekursiv per `infer_lexp` bestimmen (muss `mutable`-Local oder
//!   `Register` sein, sonst Fehler), Synonyme expandieren, dann:
//!   - `vector('len, 'elem)`: Indexausdruck vollstaendig per `infer_exp` +
//!     `bind_numeric` pruefen (muss `int`-artig sein), danach
//!     `0 <= index /\ index < len` beweisen; Ergebnis-Typ ist `'elem`.
//!   - `bitvector('len)`: dito, Ergebnis-Typ ist `bit`.
//!   - Bitfield-Wurzel mit `E_id`-Feld: kein Bounds-Check hier, sondern
//!     Delegation an den `bitfield`-Slice (ausserhalb dieser Lane).
//!   - Sonst: `"Invalid vector assignment l-expression"`.
//! - `LE_vector_range (v_lexp, e1, e2)`:
//!   Wurzel muss nach Synonym-Expansion `bitvector('len)` sein (sonst
//!   `"Cannot assign slice of non vector type"`). Beide Indexausdruecke
//!   vollstaendig per `infer_exp` + `bind_numeric` pruefen, dann
//!   ordnungsabhaengig (`Env.get_default_order`, kein impliziter Fallback —
//!   OCaml wirft ohne gesetzte Default-Order):
//!   - `inc`: `slice_len = (e2 - e1) + 1`,
//!     `check = 0 <= e1 /\ e1 <= e2 /\ e2 < len`.
//!   - `dec`: `slice_len = (e1 - e2) + 1`,
//!     `check = 0 <= e2 /\ e2 <= e1 /\ e1 < len`.
//!   Ergebnis-Typ ist `bitvector(slice_len)`.
//! - `e1`/`e2` sind positionsgebunden (erste/zweite geschriebene Grenze,
//!   `v[e1 .. e2]`): `lo`/`hi` sind keine semantischen Min/Max-Namen, die
//!   Ordnung legt fest, welche Seite unten liegt.
//! - RHS-Inferenz darf Index-Fehler nicht umgehen: `check_assign` prueft die
//!   LExp-Seite (Wurzel + Index-Typen + Bounds-Beweis) zuerst; schlaegt sie
//!   fehl, faellt das ganze Assign, unabhaengig vom RHS-Ergebnis (OCaml
//!   verkettet beide Fehler via `err_because`, die LExp-Seite geht nie
//!   verloren).
//!
//! Beweisverpflichtungen (Z3 hinter dem Trait, Regel `Z-TraitOnly`):
//! Alle Bounds werden als [`NConstraint`] gebaut
//! ([`index_obligation`], [`slice_obligation`]) und nur ueber den
//! [`Solver`](crate::solver::Solver)-Trait bewiesen. `Proved` heisst
//! Uebernahme, `Refuted`/`Unknown` heissen
//! [`TypeErrorKind::ConstraintNotProved`](crate::error::TypeErrorKind::ConstraintNotProved)
//! — `Unknown` ist immer sound (Regel `Z-UnknownSound`), es wird nie ein
//! Beweis fabriziert. Z3 plugt Lane 06/80 hinter denselben Trait; hier
//! entscheiden die Tests mit [`StubSolver`](crate::solver::StubSolver)
//! (konstante Faltung + syntaktische Reflexivitaet).
//!
//! Modellierung: Die Lane-79-`MiniTyp`-Teilmenge kennt keinen
//! `vector(n, elem)`-Kopf (Vektor-Expansionsregeln = Lane 02, `XFullExp`),
//! darum traegt [`VectorRoot`] Laenge + Element explizit statt sie aus
//! `MiniTyp` zu parsen. Das ist kein Schatten-Typsystem: sobald Lane 02
//! `E_vector_access`/`E_vector_subrange` elaboriert, wird der Hook
//! ([`HOOK_NOTE`]) die kanonische `vector`/`bitvector`-Applikation genau in
//! diese Obligationen uebersetzen.
//!
//! Hook-Punkt: `canonical_body.rs`/`program.rs` bleiben unveraendert (nur
//! gelesen). Die spaetere Verdrahtung ruft [`check_vector_index`] bzw.
//! [`check_vector_slice`] an der `Assign`-Stelle mit expandierter Wurzel,
//! geprueften Index-Nexps und `Env.get_default_order`-Ordnung auf.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::nexp::{MiniOrder, MiniTyp, NConstraint, Nexp};
use crate::solver::{ProveResult, Solver};

/// Hook-Notiz fuer die spaetere `canonical_body`-Verdrahtung (Lane 02):
/// `Assign`-Elaboration expandiert die Wurzel-Synonyme, prueft alle
/// Indexausdruecke per `infer`/`bind_numeric`, holt die Default-Order und
/// ruft dann [`check_vector_index`] / [`check_vector_slice`] auf, bevor der
/// RHS gegen den hier bestimmten Element-/Slice-Typ gecheckt wird.
pub const HOOK_NOTE: &str =
    "canonical_body Assign: Wurzel expandieren, Indexausdruecke inferieren, \
     Default-Order holen, check_vector_index/check_vector_slice zuerst, dann RHS checken";

/// Vektor-Wurzel nach Synonym-Expansion (OCaml: `Env.expand_synonyms` vor
/// dem `Typ_app`-Match auf `"vector"` / `"bitvector"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorRoot {
    /// `vector('len, 'elem)`: zwei Applikations-Argumente (Nexp + Typ).
    Vector { len: Nexp, elem: MiniTyp },
    /// `bitvector('len)`: ein Applikations-Argument (Nexp).
    Bitvector { len: Nexp },
}

/// Bindungsart der `LE_id`-Wurzel (OCaml: `Env.lookup_id` in `infer_lexp`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootBinding {
    /// `Local (Mutable, typ)`.
    Mutable(VectorRoot),
    /// `Register typ`.
    Register(VectorRoot),
    /// `Local (Immutable, _)` oder `Enum _`: nicht beschreibbar.
    Immutable,
    /// `Unbound _`: in `infer_lexp` darf kein neuer Bezeichner entstehen.
    Unbound,
}

/// Prueft, dass die LExp-Wurzel beschreibbar ist (mutable-Local oder
/// Register). Gibt die expandierte [`VectorRoot`] zurueck.
pub fn check_root(name: &str, binding: &RootBinding, span: Span) -> TypeResult<VectorRoot> {
    match binding {
        RootBinding::Mutable(root) | RootBinding::Register(root) => Ok(root.clone()),
        RootBinding::Immutable => Err(TypeError {
            span,
            kind: TypeErrorKind::RegisterError(format!(
                "cannot modify let-bound constant or enumeration constructor '{name}'"
            )),
        }),
        RootBinding::Unbound => Err(TypeError::unbound_var(
            span,
            format!("cannot create a new identifier in this l-expression '{name}'"),
        )),
    }
}

/// Bounds-Obligation fuer `LE_vector`: `0 <= index /\ index < len`.
#[must_use]
pub fn index_obligation(index: &Nexp, len: &Nexp) -> NConstraint {
    NConstraint::And(
        Box::new(NConstraint::Ge(
            Box::new(index.clone()),
            Box::new(Nexp::Const(0)),
        )),
        Box::new(NConstraint::Lt(
            Box::new(index.clone()),
            Box::new(len.clone()),
        )),
    )
}

/// Ordnungsabhaengige Slice-Obligation fuer `LE_vector_range`.
/// `first`/`second` sind die erste/zweite geschriebene Grenze (`v[e1 .. e2]`).
/// Zurueck: `(slice_len, check)`.
/// - `inc`: `slice_len = (second - first) + 1`,
///   `check = 0 <= first /\ first <= second /\ second < len`.
/// - `dec`: `slice_len = (first - second) + 1`,
///   `check = 0 <= second /\ second <= first /\ first < len`.
#[must_use]
pub fn slice_obligation(
    first: &Nexp,
    second: &Nexp,
    len: &Nexp,
    order: MiniOrder,
) -> (Nexp, NConstraint) {
    let zero = Nexp::Const(0);
    match order {
        MiniOrder::Inc => {
            let slice_len = Nexp::Add(
                Box::new(Nexp::Sub(
                    Box::new(second.clone()),
                    Box::new(first.clone()),
                )),
                Box::new(Nexp::Const(1)),
            );
            let check = NConstraint::And(
                Box::new(NConstraint::And(
                    Box::new(NConstraint::Ge(
                        Box::new(first.clone()),
                        Box::new(zero),
                    )),
                    Box::new(NConstraint::Le(
                        Box::new(first.clone()),
                        Box::new(second.clone()),
                    )),
                )),
                Box::new(NConstraint::Lt(
                    Box::new(second.clone()),
                    Box::new(len.clone()),
                )),
            );
            (slice_len, check)
        }
        MiniOrder::Dec => {
            let slice_len = Nexp::Add(
                Box::new(Nexp::Sub(
                    Box::new(first.clone()),
                    Box::new(second.clone()),
                )),
                Box::new(Nexp::Const(1)),
            );
            let check = NConstraint::And(
                Box::new(NConstraint::And(
                    Box::new(NConstraint::Ge(
                        Box::new(second.clone()),
                        Box::new(zero),
                    )),
                    Box::new(NConstraint::Le(
                        Box::new(second.clone()),
                        Box::new(first.clone()),
                    )),
                )),
                Box::new(NConstraint::Lt(
                    Box::new(first.clone()),
                    Box::new(len.clone()),
                )),
            );
            (slice_len, check)
        }
    }
}

/// Element-Typ von `LE_vector`: `vector` -> Element, `bitvector` -> `bit`.
#[must_use]
pub fn element_typ(root: &VectorRoot) -> MiniTyp {
    match root {
        VectorRoot::Vector { elem, .. } => elem.clone(),
        VectorRoot::Bitvector { .. } => MiniTyp::Bit,
    }
}

/// Slice-Typ von `LE_vector_range`: `bitvector(slice_len)`.
#[must_use]
pub fn slice_typ(slice_len: Nexp) -> MiniTyp {
    MiniTyp::Bits(Box::new(slice_len))
}

/// Prueft, dass ein Indexausdruck `int`-artig ist (OCaml: `bind_numeric`
/// nach `infer_exp`). Akzeptiert `int` und `range(lo, hi)`; alles andere
/// (bool/bit/bits/...) ist ein Typfehler — der Indexausdruck ist damit
/// "vollstaendig geprueft", bevor irgendein Bounds-Beweis versucht wird.
pub fn check_index_typ(typ: &MiniTyp, span: Span) -> TypeResult<()> {
    match typ {
        MiniTyp::Int | MiniTyp::Range(..) => Ok(()),
        _ => Err(TypeError::mismatch(
            span,
            "int".to_owned(),
            format!("{typ}"),
        )),
    }
}

/// Beweist eine Bounds-Obligation ueber den [`Solver`]-Trait.
/// Nur `Proved` uebernimmt; `Refuted` und `Unknown` werden als
/// `ConstraintNotProved` gemeldet (`Unknown` bleibt sound).
pub fn prove_bounds<S: Solver>(
    solver: &S,
    obligation: &NConstraint,
    span: Span,
    what: &str,
) -> TypeResult<()> {
    match solver.prove(obligation) {
        ProveResult::Proved => Ok(()),
        ProveResult::Refuted | ProveResult::Unknown => Err(TypeError::constraint_not_proved(
            span,
            format!("{what}: {obligation}"),
        )),
    }
}

/// Extrahiert die Bitvektor-Laenge fuer `LE_vector_range`. Eine
/// `vector`-Wurzel ist hier ein Fehler (OCaml:
/// `"Cannot assign slice of non vector type"`).
pub fn check_slice_root_len(root: &VectorRoot, span: Span) -> TypeResult<Nexp> {
    match root {
        VectorRoot::Bitvector { len } => Ok(len.clone()),
        VectorRoot::Vector { .. } => Err(TypeError::mismatch(
            span,
            "bitvector".to_owned(),
            "vector".to_owned(),
        )),
    }
}

/// Vollstaendige `LE_vector`-Pruefung: Index-Typ pruefen, Bounds beweisen,
/// Element-Typ liefern. Die Reihenfolge ist load-bearing: Der Index-Typfehler
/// kommt immer vor dem Bounds-Beweis, und beide kommen vor jeder RHS-Pruefung
/// (siehe [`check_assign`]).
pub fn check_vector_index<S: Solver>(
    solver: &S,
    root: &VectorRoot,
    index: &Nexp,
    index_typ: &MiniTyp,
    span: Span,
) -> TypeResult<MiniTyp> {
    check_index_typ(index_typ, span)?;
    let len = match root {
        VectorRoot::Vector { len, .. } | VectorRoot::Bitvector { len } => len.clone(),
    };
    prove_bounds(solver, &index_obligation(index, &len), span, "vector index bounds")?;
    Ok(element_typ(root))
}

/// Vollstaendige `LE_vector_range`-Pruefung: beide Index-Typen pruefen,
/// ordnungsabhaengige Bounds beweisen, Slice-Typ liefern.
pub fn check_vector_slice<S: Solver>(
    solver: &S,
    root: &VectorRoot,
    first: &Nexp,
    first_typ: &MiniTyp,
    second: &Nexp,
    second_typ: &MiniTyp,
    order: MiniOrder,
    span: Span,
) -> TypeResult<MiniTyp> {
    let len = check_slice_root_len(root, span)?;
    check_index_typ(first_typ, span)?;
    check_index_typ(second_typ, span)?;
    let (slice_len, check) = slice_obligation(first, second, &len, order);
    prove_bounds(solver, &check, span, "vector slice bounds")?;
    Ok(slice_typ(slice_len))
}

/// Assign-Reihenfolge: Die LExp-Seite (`lexp`) wird zuerst ausgewertet; ihr
/// Fehler gewinnt immer — die RHS-Inferenz (auch wenn sie gelingt) kann einen
/// Index-Fehler nicht umgehen. Ist die LExp-Seite ok, entscheidet das
/// RHS-Ergebnis.
pub fn check_assign<T>(
    lexp: TypeResult<T>,
    rhs: TypeResult<()>,
    _span: Span,
) -> TypeResult<T> {
    match (lexp, rhs) {
        (Err(lexp_err), _) => Err(lexp_err),
        (Ok(target), Ok(())) => Ok(target),
        (Ok(_), Err(rhs_err)) => Err(rhs_err),
    }
}
