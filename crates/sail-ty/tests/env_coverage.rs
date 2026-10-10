#![forbid(unsafe_code)]
//! KindEnv/TypeEnv-Endzustand (Lane 111, Env.t-Entscheid).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Friert den Endzustand ein:
//! - `KindEnv`: binden/suchen/iterieren/zaehlen/entfernen (vollstaendig).
//! - `TypeEnv`: val-specs + registers + locals sind vollstaendig
//!   (hinzufuegen/suchen/iterieren/sortierte Namen/zaehlen/entfernen,
//!   val/register-Namensraeume getrennt, `Clone` billig via `Arc`).
//! - Scattered: EINGEFROREN — kein offenes/impl/end-State im Checker.
//!   Offene/impl/end-Verkettung gehoert `sail-resolve`
//!   (`scattered_functions/_mappings/_variants/_enums`,
//!   `insert_scattered`/`lookup_scattered`); der Checker sieht nur
//!   geschlossene Schemes via `add_val`. Dieser Test haelt fest, dass
//!   `TypeEnv` dafuer bewusst keine API anbietet (val/register reicht).
//!   Details: `ENV_COVERAGE.md` (Repo-Root).

use sail_ty::kind::KindEnv;
use sail_ty::{Kind, MiniTyp, TypeEnv, TypQuant};

fn mono_int() -> TypQuant {
    TypQuant::monomorphic(MiniTyp::Int)
}

#[test]
fn kind_env_final_state() {
    let mut env = KindEnv::new();
    assert!(env.is_empty());
    env.bind("n", Kind::KInt);
    env.bind("b", Kind::KBool);
    env.bind("t", Kind::KType);
    assert_eq!(env.len(), 3);
    assert_eq!(env.lookup("n"), Some(Kind::KInt));
    assert_eq!(env.lookup("missing"), None);
    assert_eq!(env.iter().count(), 3);
    assert_eq!(env.remove("n"), Some(Kind::KInt));
    assert_eq!(env.len(), 2);
}

#[test]
fn val_specs_final_state() {
    let mut env = TypeEnv::new();
    assert_eq!(env.val_count(), 0);
    assert!(!env.contains_val("f"));
    env.add_val("f", mono_int());
    env.add_val("g", mono_int());
    assert_eq!(env.val_count(), 2);
    assert_eq!(env.val_names(), vec!["f".to_string(), "g".to_string()]);
    assert_eq!(env.iter_vals().count(), 2);
    // Re-Deklaration: last wins, keine Duplikate.
    env.add_val("f", mono_int());
    assert_eq!(env.val_count(), 2);
    assert!(env.remove_val("f").is_some());
    assert!(!env.contains_val("f"));
    assert_eq!(env.remove_val("f"), None);
}

#[test]
fn registers_final_state() {
    let mut env = TypeEnv::new();
    assert_eq!(env.register_count(), 0);
    env.add_register("xlen", mono_int());
    env.add_register("pc", mono_int());
    assert_eq!(env.register_count(), 2);
    assert_eq!(env.register_names(), vec!["pc".to_string(), "xlen".to_string()]);
    assert_eq!(env.iter_registers().count(), 2);
    assert!(env.lookup_register("xlen").is_some());
    assert!(env.lookup_register("missing").is_none());
    assert!(env.remove_register("pc").is_some());
    assert!(!env.contains_register("pc"));
}

#[test]
fn val_register_namespaces_are_independent() {
    let mut env = TypeEnv::new();
    env.add_val("r", mono_int());
    env.add_register("r", mono_int());
    // Gleicher Name in beiden Raeumen: beide sichtbar, getrennt loeschbar.
    assert!(env.lookup_val("r").is_some());
    assert!(env.lookup_register("r").is_some());
    assert!(env.remove_val("r").is_some());
    assert!(env.lookup_register("r").is_some());
    // Scattered-Freeze: offene/impl/end-Zustaende gibt es hier nicht —
    // der Checker konsumiert nur geschlossene Schemes (s. `add_val`).
    // Aufgeloeste Streu-Definitionen landen als normale vals:
    env.add_val("scattered_f_closed", mono_int());
    assert!(env.contains_val("scattered_f_closed"));
    assert_eq!(env.lookup("scattered_f_closed"), Some(MiniTyp::Int));
}

#[test]
fn locals_scope_pop_keeps_program_tables() {
    let mut env = TypeEnv::new();
    env.add_val("f", mono_int());
    env.add_register("xlen", mono_int());
    env.add_local("x", MiniTyp::Bool);
    assert_eq!(env.local_count(), 1);
    let fork = env.clone();
    assert_eq!(fork.val_count(), 1);
    assert_eq!(fork.register_count(), 1);
    let mut fork = fork;
    fork.clear_locals();
    assert_eq!(fork.local_count(), 0);
    // Programmtabellen ueberleben den Scope-Pop.
    assert!(fork.contains_val("f"));
    assert!(fork.contains_register("xlen"));
}
