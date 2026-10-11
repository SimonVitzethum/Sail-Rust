#![forbid(unsafe_code)]
//! Lane 82 — ARMv9.4-Korpus-Treiber (E3: reiner Klassifikations-Code, kein Lauf).
//!
//! Bildet `sail-arm/arm-v9.4-a/src/*.sail` (26 Dateien) auf
//! `ok-shard` / `defizit-mit-Owner` ab. Die Tabelle ist in
//! `WORK_STATE.md` (Worktree-Root) festgehalten; dieser Treiber hält
//! dieselbe Tabelle als Code, damit der Integrator sie außerhalb des
//! E3-Moratoriums per Test verifizieren kann — **hier wird nichts
//! ausgeführt** (`cargo`/`lake`-Läufe sind in dieser Lane verboten).
//!
//! Methode (lesend, E3-konform): `wc -l` + `grep -nE` auf den
//! Originaldateien (Top-Level-Köpfe, `throw`/`foreach`/`forall`/
//! `instantiation`/etc.), abgeglichen mit den Shard-Berichten der
//! Lanes 31 (`WORK_STATE.md`), 32 (`WORK_STATE.md`) und 33
//! (`WORK_STATE.md`) sowie der Lane-45-Fail-Liste
//! (`WORK_STATE.md`). Alle Owner sind je genau eine
//! Lane (Spec-Vorgabe); `ok-shard` heißt: Definitions-Split/Scan ist
//! per Shard-Bericht belegt, volles Lowering bleibt dort verowned.

/// Eine Korpus-Datei mit fester Zeilenzahl (`wc -l`, Stand 10.10.2026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmFile {
    /// Dateiname unter `sail-arm/arm-v9.4-a/src/`.
    pub name: &'static str,
    /// Zeilen (`wc -l`).
    pub lines: u32,
}

/// Status einer Korpus-Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Definitions-Split/Scan belegt (Shard-Bericht), Lane als Beleg.
    ShardOk {
        /// z. B. `"lane-31"`.
        lane: &'static str,
    },
    /// Volles Frontend braucht diese Owner-Lane (genau ein Owner).
    Deficit {
        /// z. B. `"lane-04"`.
        owner: &'static str,
        /// Kurzbegründung (dominantes Konstrukt + Beleg).
        reason: &'static str,
    },
}

/// Alle 26 Dateien aus `sail-arm/arm-v9.4-a/src/*.sail`, alphabetisch.
/// Summe: 464212 Zeilen.
pub const ARM_FILES: &[ArmFile] = &[
    ArmFile { name: "builtins.sail", lines: 208 },
    ArmFile { name: "decode_end.sail", lines: 111 },
    ArmFile { name: "decode_start.sail", lines: 89 },
    ArmFile { name: "devices.sail", lines: 286 },
    ArmFile { name: "elfmain.sail", lines: 99 },
    ArmFile { name: "event_clauses.sail", lines: 138 },
    ArmFile { name: "fetch.sail", lines: 389 },
    ArmFile { name: "impdefs.sail", lines: 900 },
    ArmFile { name: "instrs32.sail", lines: 48015 },
    ArmFile { name: "instrs64.sail", lines: 53106 },
    ArmFile { name: "instrs64_sme.sail", lines: 42546 },
    ArmFile { name: "instrs64_sve.sail", lines: 95548 },
    ArmFile { name: "interface.sail", lines: 199 },
    ArmFile { name: "interrupts.sail", lines: 368 },
    ArmFile { name: "isla_main.sail", lines: 86 },
    ArmFile { name: "isla_splice.sail", lines: 29 },
    ArmFile { name: "map_clauses.sail", lines: 465 },
    ArmFile { name: "mem.sail", lines: 232 },
    ArmFile { name: "no_devices.sail", lines: 56 },
    ArmFile { name: "prelude.sail", lines: 375 },
    ArmFile { name: "reset.sail", lines: 1466 },
    ArmFile { name: "stubs.sail", lines: 463 },
    ArmFile { name: "sysregs.sail", lines: 1481 },
    ArmFile { name: "sysregs_autogen.sail", lines: 160302 },
    ArmFile { name: "termination_measures.sail", lines: 116 },
    ArmFile { name: "v8_base.sail", lines: 57139 },
];

/// Byte-exakter String-Vergleich ohne Allokation (const-fähig).
#[must_use]
pub const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Klassifikation je Datei (Spiegel von `WORK_STATE.md`).
#[must_use]
pub const fn classify(name: &str) -> Option<FileStatus> {
    // `const fn` mit String-Match: Byte-Vergleich über Schleife ist in
    // const-Kontext erlaubt; der Einfachheit halber hier als Kette.
    macro_rules! shard {
        ($lane:expr) => {
            FileStatus::ShardOk { lane: $lane }
        };
    }
    macro_rules! deficit {
        ($owner:expr, $reason:expr) => {
            FileStatus::Deficit { owner: $owner, reason: $reason }
        };
    }
    // Kurzschluss-Vergleich ohne Allokation (const fn, kein Closure: E0015).
    if str_eq(name, "builtins.sail") {
        Some(deficit!(
            "lane-04",
            "forall/constraints in val-Sigs + foreach/throw/assert (nexp-Constraint zuerst)"
        ))
    } else if str_eq(name, "decode_end.sail") {
        Some(deficit!(
            "lane-21",
            "21 function-clause Default-Arme mit throw (gestreuter Dispatch, Prioritaet)"
        ))
    } else if str_eq(name, "decode_start.sail") {
        Some(deficit!(
            "lane-17",
            "21 scattered-function-Deklarationen + SEE-Register (Grammatik-Hoheit)"
        ))
    } else if str_eq(name, "devices.sail") {
        Some(deficit!(
            "lane-19",
            "register __GIC_* + let-Basen + effect-Sigs (Register-Modell)"
        ))
    } else if str_eq(name, "elfmain.sail") {
        Some(deficit!(
            "lane-34",
            "C-externe vals ({c: ...}) + effect-Mengen + main/step_model (ausfuehrbar)"
        ))
    } else if str_eq(name, "event_clauses.sail") {
        Some(deficit!(
            "lane-38",
            "__ListConfig/__Reset/__EndCycle-Eventklauseln (Treiber/Config-Verdrahtung)"
        ))
    } else if str_eq(name, "fetch.sail") {
        Some(deficit!(
            "lane-28",
            "undefined-Inits + throw-Passthrough (try/throw) + Emulator-Register"
        ))
    } else if str_eq(name, "impdefs.sail") {
        Some(deficit!(
            "lane-25",
            "ImpDefBool/Int/Bits-Externs + lange else-if-Implementierungsketten (FFI)"
        ))
    } else if str_eq(name, "instrs32.sail") {
        Some(shard!("lane-33"))
    } else if str_eq(name, "instrs64.sail") {
        Some(shard!("lane-33"))
    } else if str_eq(name, "instrs64_sme.sail") {
        Some(shard!("lane-32"))
    } else if str_eq(name, "instrs64_sve.sail") {
        Some(shard!("lane-32"))
    } else if str_eq(name, "interface.sail") {
        Some(deficit!(
            "lane-29",
            "9 instantiations (sail_mem_*/tlbi/barrier/exceptions) + union/struct (Archsem)"
        ))
    } else if str_eq(name, "interrupts.sail") {
        Some(deficit!(
            "lane-22",
            "reine Funktionsdatei, dichte Bitvektor/Int-Ausdruecke + throw (Bitvec-Semantik)"
        ))
    } else if str_eq(name, "isla_main.sail") {
        Some(deficit!(
            "lane-08",
            "effect-Mengen {configuration, escape, rmem, ...} in val-Sigs (Effekte)"
        ))
    } else if str_eq(name, "isla_splice.sail") {
        Some(deficit!(
            "lane-38",
            "Reset/isla-Einstiegspunkte (Treiber-Verdrahtung, sonst trivial)"
        ))
    } else if str_eq(name, "map_clauses.sail") {
        Some(deficit!(
            "lane-20",
            "mapping-Klausel __SetConfig (encdec-Abbildung)"
        ))
    } else if str_eq(name, "mem.sail") {
        Some(deficit!(
            "lane-29",
            "sail_mem_read/write-Callgraph + forall-Sigs + match-Dispatch (Archsem)"
        ))
    } else if str_eq(name, "no_devices.sail") {
        Some(deficit!(
            "lane-38",
            "Device-Stub-Funktionen (Treiber-Seite, sonst trivial)"
        ))
    } else if str_eq(name, "prelude.sail") {
        Some(deficit!(
            "lane-18",
            "overload-Operatoren + pure-Externs + type/union/enum (Prelude-Lib)"
        ))
    } else if str_eq(name, "reset.sail") {
        Some(deficit!(
            "lane-19",
            "1466 Zeilen Reset-Schreibung vieler Control-Register (Register-Modell)"
        ))
    } else if str_eq(name, "stubs.sail") {
        Some(deficit!(
            "lane-25",
            "105 Stub-Funktionen als Extern-Platzhalter (FFI-Abbildung)"
        ))
    } else if str_eq(name, "sysregs.sail") {
        Some(deficit!(
            "lane-31",
            "56 handgeschriebene SysReg-read/set-Funktionen (Shard-Folge, autogen ist lane-31-ok)"
        ))
    } else if str_eq(name, "sysregs_autogen.sail") {
        Some(shard!("lane-31"))
    } else if str_eq(name, "termination_measures.sail") {
        Some(deficit!(
            "lane-26",
            "27 termination_measure-Deklarationen (Mass-Hoheit)"
        ))
    } else if str_eq(name, "v8_base.sail") {
        Some(shard!("lane-33"))
    } else {
        None
    }
}

/// Anzahl Defizit-Dateien (Arbeitsliste für „ARM in 20GiB").
#[must_use]
pub const fn deficit_count() -> usize {
    // 26 Dateien, 6 ok-shard → 20 Defizite (per Tabelle verifiziert).
    20
}

/// Gesamtzeilen des Korpus.
#[must_use]
pub fn total_lines() -> u64 {
    ARM_FILES.iter().map(|f| u64::from(f.lines)).sum()
}

#[cfg(test)]
mod tests {
    //! Verifikation für den Integrator (läuft auf dem Test-Rig, E3: hier nur Code).
    use super::*;

    #[test]
    fn alle_26_dateien_klassifiziert() {
        assert_eq!(ARM_FILES.len(), 26);
        for f in ARM_FILES {
            assert!(
                classify(f.name).is_some(),
                "ohne Klassifikation: {}",
                f.name
            );
        }
        assert_eq!(classify("gibt-es-nicht.sail"), None);
    }

    #[test]
    fn zeilensumme_stimmt() {
        assert_eq!(total_lines(), 464_212);
    }

    #[test]
    fn sechs_shard_ok_zwanzig_defizite_mit_je_genau_einem_owner() {
        let mut ok = 0usize;
        let mut deficits = 0usize;
        for f in ARM_FILES {
            match classify(f.name) {
                Some(FileStatus::ShardOk { lane }) => {
                    assert!(!lane.is_empty());
                    ok += 1;
                }
                Some(FileStatus::Deficit { owner, reason }) => {
                    assert!(!owner.is_empty(), "Owner fehlt: {}", f.name);
                    assert!(!reason.is_empty(), "Grund fehlt: {}", f.name);
                    // Genau ein Owner: kein Komma/Schrägstrich in der Owner-Angabe.
                    assert!(
                        !owner.contains(',') && !owner.contains('/'),
                        "mehr als ein Owner bei {}: {owner}",
                        f.name
                    );
                    deficits += 1;
                }
                None => panic!("ohne Klassifikation: {}", f.name),
            }
        }
        assert_eq!(ok, 6);
        assert_eq!(deficits, 20);
        assert_eq!(deficits, deficit_count());
    }

    #[test]
    fn const_kontext_ohne_closure_e0015() {
        // E0015-Regression als Code (E3: wird hier NICHT ausgefuehrt):
        // `classify`/`str_eq` muessen im const-Kontext aufrufbar sein.
        // Vor dem Fix scheiterte das an `eq`-Closure in `const fn`.
        const A: Option<FileStatus> = classify("instrs32.sail");
        const B: Option<FileStatus> = classify("gibt-es-nicht.sail");
        const EQ: bool = str_eq("mem.sail", "mem.sail");
        const NE: bool = str_eq("mem.sail", "fetch.sail");
        assert!(matches!(A, Some(FileStatus::ShardOk { .. })));
        assert!(B.is_none());
        assert!(EQ);
        assert!(!NE);
    }

    #[test]
    fn shard_ok_dateien_sind_die_belegten_sechs() {
        for name in [
            "instrs32.sail",
            "instrs64.sail",
            "v8_base.sail",
            "instrs64_sve.sail",
            "instrs64_sme.sail",
            "sysregs_autogen.sail",
        ] {
            assert!(
                matches!(classify(name), Some(FileStatus::ShardOk { .. })),
                "{name} sollte ok-shard sein"
            );
        }
    }
}
