//! Vektor-Primitive `vector_*#` (ARM-Gate-Blocker `to_bytes_le`).
//!
//! Gezielte Normalisierung genau der vier internen OCaml-Primitiven, die der
//! Parser fuer Index-/Subrange-Zugriffe erzeugt (siehe
//! `crates/sail-parse/src/convert.rs`):
//!
//! | interner Name (`#`)          | Arity | normalisierter Overload      | OCaml-Regel (`type_check.ml`)                          |
//! |------------------------------|-------|------------------------------|--------------------------------------------------------|
//! | `vector_access#`             | 2     | `vector_access`              | erst Overload versuchen, bei Fehlschlag Bitfield-Fallback (Z. 3900) |
//! | `vector_subrange#`           | 3     | `vector_subrange`            | direkter Overload-Aufruf (Z. 3913)                     |
//! | `vector_update#`             | 3     | `vector_update`              | via `infer_vector_update`: Kette sammeln, Bitfield oder Overload-Faltung (Z. 3915, 4102) |
//! | `vector_update_subrange#`    | 4     | `vector_update_subrange`     | direkter Overload-Aufruf (Z. 3916)                     |
//!
//! Es wird bewusst **kein** generelles `#`-Strippen vorgenommen: `classify`
//! und `normalize` akzeptieren ausschliesslich die vier exakten Namen oben.
//! `vector_access`, `slice`, `foo#` oder `vector_access##` ergeben `None`.
//!
//! `to_bytes_le`-Fall (`sail/lib/vector.sail`, Z. 475ff, `_DEFAULT_DEC`):
//! ```sail
//! function to_bytes_le(n, b) = {
//!     var res = vector_init(n, sail_zeros(8));
//!     foreach (i from 0 to (n - 1)) {
//!         res[i] = b[(8 * i + 7) .. (8 * i)]
//!     };
//!     res
//! }
//! ```
//! - Die rechte Seite `b[(8 * i + 7) .. (8 * i)]` ist ein
//!   `vector_subrange#`-Aufruf mit 3 Argumenten und normalisiert zu
//!   `vector_subrange`.
//! - Die linke Seite `res[i] = ...` ist **kein** App-Aufruf, sondern ein
//!   `LExpKind::Vector`-L-Wert. Dafuer gelten die `infer_lexp`-Regeln
//!   (`type_check.ml`, ab Z. 3575): Wurzel muss mutable/Register sein,
//!   Indexausdruck wird vollstaendig geprueft, Bounds `0 <= index < length`
//!   muessen per Solver bewiesen werden; Subranges ordnungsabhaengig
//!   (`dec`: `0 <= lo <= hi < n`, `inc`: `0 <= hi <= lo < n`). Die
//!   L-Wert-Typisierung selbst gehoert Lane-07 (`lexp_bounds.rs`); hier wird
//!   nur dokumentiert, dass `res[i]` nicht durch `normalize` laeuft.
//!
//! Dieses Modul ist bewusst rein (`&str`/`usize` rein und raus, kein
//! `Program`-Zugriff), damit es ohne Aenderung an `program.rs` nutzbar und
//! testbar ist. Die Verdrahtung in `canonical_body.rs` erfolgt ueber
//! [`resolve_app`] (siehe Insert-Spec im Lane-16-Bericht).

/// Genau eine der vier internen Vektor-Primitiven.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VectorPrim {
    /// `vector_access#`: Elementzugriff `v[i]`.
    Access,
    /// `vector_subrange#`: Slice `v[hi .. lo]`.
    Subrange,
    /// `vector_update#`: funktionales Update `[v with i = x]`.
    Update,
    /// `vector_update_subrange#`: funktionales Slice-Update `[v with hi .. lo = x]`.
    UpdateSubrange,
}

/// Interner Parser-/OCaml-Name von [`VectorPrim::Access`].
pub const VECTOR_ACCESS_HASH: &str = "vector_access#";
/// Interner Parser-/OCaml-Name von [`VectorPrim::Subrange`].
pub const VECTOR_SUBRANGE_HASH: &str = "vector_subrange#";
/// Interner Parser-/OCaml-Name von [`VectorPrim::Update`].
pub const VECTOR_UPDATE_HASH: &str = "vector_update#";
/// Interner Parser-/OCaml-Name von [`VectorPrim::UpdateSubrange`].
pub const VECTOR_UPDATE_SUBRANGE_HASH: &str = "vector_update_subrange#";

/// Klassifiziert exakt die vier internen Namen; alles andere ergibt `None`.
///
/// Insbesondere wird kein `#`-Suffix von beliebigen Namen entfernt.
#[must_use]
pub fn classify(name: &str) -> Option<VectorPrim> {
    match name {
        VECTOR_ACCESS_HASH => Some(VectorPrim::Access),
        VECTOR_SUBRANGE_HASH => Some(VectorPrim::Subrange),
        VECTOR_UPDATE_HASH => Some(VectorPrim::Update),
        VECTOR_UPDATE_SUBRANGE_HASH => Some(VectorPrim::UpdateSubrange),
        _ => None,
    }
}

/// Interner `#`-Name des Primitivs (Eingabeseite der Normalisierung).
#[must_use]
pub fn raw_name(prim: VectorPrim) -> &'static str {
    match prim {
        VectorPrim::Access => VECTOR_ACCESS_HASH,
        VectorPrim::Subrange => VECTOR_SUBRANGE_HASH,
        VectorPrim::Update => VECTOR_UPDATE_HASH,
        VectorPrim::UpdateSubrange => VECTOR_UPDATE_SUBRANGE_HASH,
    }
}

/// Overload-Name ohne `#` (Ausgabeseite der Normalisierung, vgl.
/// `mk_id "vector_access"` etc. in `type_check.ml`).
#[must_use]
pub fn normalized_name(prim: VectorPrim) -> &'static str {
    match prim {
        VectorPrim::Access => "vector_access",
        VectorPrim::Subrange => "vector_subrange",
        VectorPrim::Update => "vector_update",
        VectorPrim::UpdateSubrange => "vector_update_subrange",
    }
}

/// Erwartete Argumentzahl des internen Aufrufs (vgl. Parser-Lowering).
#[must_use]
pub fn expected_arity(prim: VectorPrim) -> usize {
    match prim {
        VectorPrim::Access => 2,
        VectorPrim::Subrange => 3,
        VectorPrim::Update => 3,
        VectorPrim::UpdateSubrange => 4,
    }
}

/// Normalisiert einen internen Aufruf zum Overload-Namen.
///
/// Gibt genau dann `Some(overload)` zurueck, wenn `name` exakt eines der vier
/// Primitiven bezeichnet **und** `arg_count` zur erwarteten Stelligkeit passt.
/// Falsche Stelligkeit oder fremde Namen (auch andere `#`-Namen) ergeben
/// `None`, damit Fehlanwendungen als Arity-/Unbound-Fehler sichtbar bleiben
/// statt still umgeschrieben zu werden.
#[must_use]
pub fn normalize(name: &str, arg_count: usize) -> Option<&'static str> {
    let prim = classify(name)?;
    if arg_count == expected_arity(prim) {
        Some(normalized_name(prim))
    } else {
        None
    }
}

/// Prueft, ob `name` exakt eines der vier internen Primitiven bezeichnet.
#[must_use]
pub fn is_vector_prim(name: &str) -> bool {
    classify(name).is_some()
}

/// Ob OCaml vor/nach dem Overload-Versuch einen Bitfield-Zugriff in Betracht
/// zieht (`vector_access#`: Zugriffs-Fallback; `vector_update#`:
/// `infer_vector_update` verzweigt ueber den Bitfield-Status der Basis).
/// `vector_subrange#` und `vector_update_subrange#` kennen keinen
/// Bitfield-Pfad (reine Overload-Umschreibung).
#[must_use]
pub fn allows_bitfield_fallback(prim: VectorPrim) -> bool {
    match prim {
        VectorPrim::Access | VectorPrim::Update => true,
        VectorPrim::Subrange | VectorPrim::UpdateSubrange => false,
    }
}

/// Aufgeloeste Call-Site eines internen `vector_*#`-Aufrufs.
///
/// Ergebnis von [`resolve_app`]: der Overload-Name, unter dem der App-Arm in
/// `canonical_body.rs` die Kandidatensuche wiederholen soll, plus ob bei
/// fehlgeschlagener Overload-Suche der Bitfield-Pfad versucht werden darf
/// (statt sofort mit Unbound/Arity abzubrechen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppResolution {
    /// Das erkannte Primitiv (Eingabeseite).
    pub prim: VectorPrim,
    /// Overload-Name ohne `#` (vgl. [`normalized_name`]).
    pub overload: &'static str,
    /// Ob OCaml fuer dieses Primitiv einen Bitfield-Pfad vorsieht (vgl.
    /// [`allows_bitfield_fallback`]).
    pub bitfield_fallback: bool,
}

/// Call-Site-Helper fuer den `ExpKind::App`-Arm in `canonical_body.rs`.
///
/// Fasst [`normalize`] und [`allows_bitfield_fallback`] in einem Aufruf
/// zusammen: `name` ist der aufgeloeste Funktionsname der Applikation,
/// `arg_count` die Argumentzahl (`args.len()`). Gibt genau dann
/// `Some(AppResolution)` zurueck, wenn der Aufruf exakt einem der vier
/// internen Primitive mit passender Stelligkeit entspricht; sonst `None`
/// (der App-Arm faehrt dann mit der bisherigen Kandidatensuche fort).
///
/// Rein funktional: kein `Program`-Zugriff, keine Seiteneffekte. Die
/// Overload-Kandidatensuche unter `resolution.overload` sowie der
/// Bitfield-Fallback selbst bleiben Aufgabe der Call-Site (siehe Insert-Spec
/// im Lane-16-Bericht).
#[must_use]
pub fn resolve_app(name: &str, arg_count: usize) -> Option<AppResolution> {
    let prim = classify(name)?;
    if arg_count != expected_arity(prim) {
        return None;
    }
    Some(AppResolution {
        prim,
        overload: normalized_name(prim),
        bitfield_fallback: allows_bitfield_fallback(prim),
    })
}
