#![forbid(unsafe_code)]
//! Lane-127-Paritaet LoopKind (Code, E3: NICHT ausfuehren).
//!
//! Wahrheit: `sail_loops::LoopKind` (voll: While/WhileFuel/Until/
//! UntilFuel/Foreach/ForSurface). `sail_termination::fuel::LoopKind`
//! ist die Fuel-Teilsicht; Abbildung ueber Kombinator-Namen und
//! `needs_fuel`.

use sail_loops::{FuelKind, LoopKind as LoopsKind, is_loop_name};
use sail_termination::fuel::LoopKind as TermLoop;

/// Fuel-Kombinatoren stimmen in Namen und Fuel-Bedarf ueberein.
#[test]
fn fuel_combinators_agree() {
    assert_eq!(TermLoop::WhileFuel.combinator(), Some("whileFuelM"));
    assert_eq!(TermLoop::UntilFuel.combinator(), Some("untilFuelM"));
    assert_eq!(FuelKind::WhileFuel.combinator(), "whileFuelM");
    assert_eq!(FuelKind::UntilFuel.combinator(), "untilFuelM");
    // Besitz-Wechsel beidseitig gleich: nur Fuel-Varianten brauchen Fuel.
    for k in [TermLoop::WhileFuel, TermLoop::UntilFuel] {
        assert!(k.needs_fuel());
    }
    for k in [TermLoop::While, TermLoop::Until] {
        assert!(!k.needs_fuel());
    }
    for k in [
        LoopsKind::WhileFuel,
        LoopsKind::UntilFuel,
    ] {
        assert!(k.needs_fuel());
    }
    for k in [
        LoopsKind::While,
        LoopsKind::Until,
        LoopsKind::Foreach,
        LoopsKind::ForSurface,
    ] {
        assert!(!k.needs_fuel());
    }
    // `From<FuelKind>`-Bruecke der Wahrheit trifft die Fuel-Namen.
    assert_eq!(LoopsKind::from(FuelKind::WhileFuel).sail_name(), Some("while#t"));
    assert_eq!(LoopsKind::from(FuelKind::UntilFuel).sail_name(), Some("until#t"));
    // Alle `#`-Kombinatoren der Wahrheit sind Loop-Namen.
    for k in [
        LoopsKind::While,
        LoopsKind::WhileFuel,
        LoopsKind::Until,
        LoopsKind::UntilFuel,
        LoopsKind::Foreach,
    ] {
        assert!(is_loop_name(k.sail_name().expect("kombinator")), "{k:?}");
    }
    assert_eq!(LoopsKind::ForSurface.sail_name(), None);
}
