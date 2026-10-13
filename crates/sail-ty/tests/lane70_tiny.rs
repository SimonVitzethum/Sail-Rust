//! Lane 70 — Tiny-Modell Stufe 1+2: Parse -> Declare -> Check (Harness).
//!
//! Das Modell uebt Funktionen, Vektor-Literale, Record-/Enum-/Union-Dekla-
//! rationen, Record-Konstruktion, Feldzugriff und `match` ein. Erwartung am
//! Base-Stand (`a972e3a`): Frontend (inkl. `match`/`struct`-Parse) und
//! Deklaration sind gruen; `check_definition` ist gruen fuer die reine
//! Funktions-/Vektor-Teilmenge und scheitert exakt an den noch nicht
//! implementierten kanonischen Ausdrucksregeln (`E_match`, `E_struct`/Feld,
//! Owner: Lane 02 full-`Exp`-Port, vgl. `sail-ty/src/pat.rs`).
//!
//! Der Test schreibt das Modell (`tiny.sail`) und das Stufenprotokoll
//! (`stage-typecheck.log`, inkl. Scheme-Snapshots) nach `LANE70_OUT_DIR`
//! (Default: `<workspace>/target/lane70-tiny`). Der Treiber kopiert
//! `tiny.sail` als Single Source of Truth nach
//! `evidence/2026-10-13/lane-70/` und faehrt damit den CLI-Lauf.

use sail_ast::Span;
use sail_parse::{ParserContext, RealParser};
use sail_ty::program::Program;
use std::sync::Arc;

/// Das Tiny-Modell (Single Source of Truth; wird auch als `tiny.sail`
/// ausgeschrieben). Vor Stufen mit Prelude bewusst prelude-frei, damit der
/// Lauf nicht in `vector.sail` maskiert (vgl. Lane-31 B2).
pub const TINY_MODEL: &str = r#"default Order dec

struct Point = { x : int, y : int }

enum Colour = { Red, Green }

union Shape = { Circle : int, Square : bool }

val const_one : unit -> int
function const_one() = 1

val choose : bool -> int
function choose(b) = if b then 1 else 0

val swap_pair : (int, bool) -> (bool, int)
function swap_pair(x, y) = (y, x)

val two_nums : unit -> vector(2, int)
function two_nums() = [1, 2]

val byte_pair : unit -> bitvector(2)
function byte_pair() = [bitone, bitzero]

val is_red : Colour -> bool
function is_red(Red) = true
and is_red(Green) = false

val mk_point : (int, int) -> Point
function mk_point(x, y) = struct Point { x = x, y = y }

val get_x : Point -> int
function get_x(p) = p.x

val describe : Shape -> int
function describe(s) = match s { Circle(n) => n, Square(_) => 0 }
"#;

/// Namen der Gruen-Teilmenge (muss am Base-Stand komplett checken).
const GREEN_FUNS: &[&str] = &[
    "const_one",
    "choose",
    "swap_pair",
    "two_nums",
    "byte_pair",
    "is_red",
    "describe",
];

fn out_dir() -> std::path::PathBuf {
    std::env::var_os("LANE70_OUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lane70-tiny")
        })
}

#[test]
fn lane70_tiny_parse_declare_check() {
    let out = out_dir();
    std::fs::create_dir_all(&out).expect("LANE70_OUT_DIR anlegen");
    std::fs::write(out.join("tiny.sail"), TINY_MODEL).expect("tiny.sail schreiben");

    // Stufe 1a: Parsen. Jede nicht-leere Zeile des Modells ist genau eine
    // Def (alle Modell-Defs sind einzeilig), ausser `and`-Folgeklauseln:
    // Mehrklausel-Funktionen gehoeren per `and` an EINE `function`-Def
    // (Sail-Form, vgl. `sail/test/typecheck/pass/funcl_guard.sail`;
    // wiederholte `function`-Zeilen lehnen OCaml wie unser Resolver ab).
    let mut sources: Vec<String> = Vec::new();
    for line in TINY_MODEL.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("and ") {
            sources
                .last_mut()
                .expect("and ohne function-Kopf")
                .push_str("\n");
            sources.last_mut().expect("and ohne function-Kopf").push_str(line);
        } else {
            sources.push(line.to_string());
        }
    }
    assert_eq!(
        sources.len(),
        22,
        "Modell-Drift: {} Defs statt 22",
        sources.len()
    );

    let mut context = ParserContext::default();
    let mut defs = Vec::new();
    let mut log = String::new();
    for (position, source) in sources.iter().enumerate() {
        let mut parser = RealParser::with_context(context.clone());
        let def = parser
            .parse_def(source, Span::DUMMY)
            .unwrap_or_else(|e| panic!("parse #{position} fehlgeschlagen: {source:?}: {e}"));
        context = parser.context();
        defs.push((def, Arc::new(parser.into_interner())));
        log.push_str(&format!("parse ok #{position}: {source}\n"));
    }

    // Stufe 1b: Deklarieren in Source-Order.
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program
            .declare(def, it.clone(), position)
            .unwrap_or_else(|e| panic!("declare #{position} fehlgeschlagen: {e}"));
        log.push_str(&format!("declare ok #{position}\n"));
    }

    // Stufe 2: Bodies pruefen; Gruen/Rot-Tabelle.
    let mut green_names: Vec<&str> = Vec::new();
    let mut red = Vec::new();
    let mut expressions = 0usize;
    for (position, (def, it)) in defs.iter().enumerate() {
        match program.check_definition(def, it.clone(), position) {
            Ok(count) => {
                expressions += count;
                log.push_str(&format!("check ok #{position} (+{count} exprs)\n"));
                let name = fun_name(position);
                if let Some(name) = name {
                    if GREEN_FUNS.contains(&name) {
                        green_names.push(name);
                    }
                }
            }
            Err(e) => {
                log.push_str(&format!("check FAIL #{position}: {e}\n"));
                red.push((position, format!("{e}")));
            }
        }
    }
    // Scheme-Snapshots der deklarierten Werte (fuer die Emit-Stufe).
    for name in GREEN_FUNS
        .iter()
        .chain(["mk_point", "get_x", "describe"].iter())
    {
        match program.value(name, usize::MAX) {
            Some(scheme) => log.push_str(&format!(
                "scheme {name}: {:?}\n",
                scheme.typschm.typ.kind
            )),
            None => log.push_str(&format!("scheme {name}: <kein Wert-Eintrag>\n")),
        }
    }
    log.push_str(&format!(
        "SUMMARY defs={} exprs={} green_funs={}/{} red_defs={}\n",
        defs.len(),
        expressions,
        {
            let mut names = green_names.clone();
            names.sort_unstable();
            names.dedup();
            names.len()
        },
        GREEN_FUNS.len(),
        red.len()
    ));
    std::fs::write(out.join("stage-typecheck.log"), &log).expect("log schreiben");

    // Erwartung mit gemergten Armen: alle 7 Gruen-Funktionen checken
    // (describe via Konstruktor-Pattern-Bindung `Circle(n)`/`Square(_)`).
    // Rot bleibt nur get_x (E_field, Arm fehlt).
    let mut names = green_names.clone();
    names.sort_unstable();
    names.dedup();
    let mut expected: Vec<&str> = GREEN_FUNS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected, "Gruen-Teilmenge unvollstaendig:\n{log}");
    assert_eq!(red.len(), 1, "Rot-Tabelle unerwartet:\n{log}");
    for (position, message) in &red {
        assert!(
            message.contains("not yet implemented in the canonical checker")
                || message.contains("constructor, vector, list, disjunctive or dependent pattern"),
            "Rot-Def #{position} mit unerwarteter Meldung: {message}"
        );
    }

    // Zusatzsonde (kein Modell-Bestandteil): wiederholte `function`-Zeilen
    // fuer denselben Namen. OCaml (`Function ... has already been declared`)
    // und unser Frontend-Resolver (`duplicate function`) lehnen das ab;
    // `sail-ty` declare+check akzeptiert es still — dokumentierte Laxheit,
    // keine Freigabe (Owner am Base-Stand undokumentiert).
    let dup_sources = [
        "val dup_probe : int -> int",
        "function dup_probe(x) = x",
        "function dup_probe(y) = y",
    ];
    let mut dup_context = ParserContext::default();
    let mut dup_program = Program::new();
    let mut dup_accepted = true;
    for (position, source) in dup_sources.iter().enumerate() {
        let mut parser = RealParser::with_context(dup_context.clone());
        let def = parser
            .parse_def(source, Span::DUMMY)
            .unwrap_or_else(|e| panic!("dup-probe parse #{position}: {e}"));
        dup_context = parser.context();
        let it = Arc::new(parser.into_interner());
        if dup_program.declare(&def, it.clone(), position).is_err() {
            dup_accepted = false;
            break;
        }
        if dup_program
            .check_definition(&def, it, position)
            .is_err()
        {
            dup_accepted = false;
            break;
        }
    }
    let mut log = std::fs::read_to_string(out.join("stage-typecheck.log")).expect("log lesen");
    log.push_str(&format!("DUP-PROBE repeated-function-lines accepted={dup_accepted} (ocaml=reject, resolver=reject)\n"));
    std::fs::write(out.join("stage-typecheck.log"), &log).expect("log schreiben");
    assert!(
        dup_accepted,
        "Dup-Sonde unerwartet streng — Verhalten dokumentieren, nicht still aendern:\n{log}"
    );
}

/// Ordnet Def-Positionen auf Funktionsnamen ab (`and`-Klauseln sind Teil
/// der `function`-Def; bricht bei Drift lautstark). Gezaehlt werden die
/// `function`-Defs (val-Specs und Typ-Deklarationen sind keine Bodies).
fn fun_name(position: usize) -> Option<&'static str> {
    match position {
        5 => Some("const_one"),
        7 => Some("choose"),
        9 => Some("swap_pair"),
        11 => Some("two_nums"),
        13 => Some("byte_pair"),
        15 => Some("is_red"),
        17 => Some("mk_point"),
        19 => Some("get_x"),
        21 => Some("describe"),
        _ => None,
    }
}
