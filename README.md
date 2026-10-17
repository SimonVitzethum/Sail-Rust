# Sail-Rust

Rust-Rewrite des Sail-Compilers mit dem Ziel, die vollständige ARMv9.4-A-
Spezifikation nach Lean zu übersetzen und erfolgreich mit Lake zu bauen —
ohne `sorry`, unter 16 GiB RAM ohne Swap. 28 GiB sind die erlaubte Ausweichgrenze.

**Der Compiler ist noch unvollständig.** Der reale Frontend-Pfad verarbeitet
bereits das vollständige ARM-Modell auf Platte mit begrenzter Parallelität.
Die Typprüfung und die ausführbare Lean-Ausgabe sind noch nicht vollständig.
Ein erfolgreicher ARM-`lake build` ist nicht nachgewiesen.

Frontend, Typprüfung und Rewrites arbeiten mit begrenzten Worker-Fenstern.
`SAIL_RS_THREADS` steuert die Parallelität (automatisch maximal 16);
`SAIL_REWRITE_MEMORY` steuert das gemeinsame Rewrite-Eingabebudget.
ASTs und temporäre Gate-Daten liegen auf der Projektplatte. Der aktuelle
ARM-Blocker und die gemessenen Speicher-Peaks stehen im Arbeitsstand.

## Aktuelle Dokumentation

| Datei | Inhalt |
| --- | --- |
| [WORK_STATE.md](WORK_STATE.md) | Bestätigter Stand, laufende Änderungen, Fehler, offene Aufgaben und Fortsetzung. |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Compilerpfad, Out-of-core, Typprüfung, Solver und zentrale Verträge. |
| [TESTING.md](TESTING.md) | Voraussetzungen, systemd-Limits, reproduzierbare Prüfungen und gesicherte Logs. |
| [THIRD_PARTY.md](THIRD_PARTY.md) | Herkunft und Lizenzen eingebetteter Bibliotheken und Testfixtures. |

Die Projektdokumentation liegt ausschließlich in diesen fünf Markdown-Dateien
im Root. Alte Pläne und Berichte bleiben über die Git-Historie verfügbar.
Compilerquellen, ausführbare Testfixtures und Logs liegen in ihren jeweiligen
Unterverzeichnissen. Aktuelle Ergebnisse werden in `WORK_STATE.md` gepflegt;
Infrastruktur- oder Teilmengentests ersetzen keinen vollständigen ARM-Build.

## Bauen und prüfen

Voraussetzungen: Rust/Cargo, Linux mit erreichbarem systemd-User-Manager und
cgroup-Speicherlimits; Z3 für abhängige Typbeweise, Lean/Lake für die Lean-Abnahme.
OCaml Sail wird nur als Referenz für Vergleichstests benötigt.

```bash
mkdir -p target/tmp
/tmp/opencode/sail-gate/gated-run.sh 4G systemd-run --user --wait --pipe --collect --setenv="TMPDIR=$PWD/target/tmp" \
  -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$PWD" \
  cargo build --profile ci -j 2 -p sail-cli

/tmp/opencode/sail-gate/gated-run.sh 4G scripts/test-memory-limited.sh -p sail-ty -p sail-parse -p sail-grammar -p sail-cli -p sail-solver
/tmp/opencode/sail-gate/gated-run.sh 16G env SAIL_RS_THREADS=16 scripts/arm-lean-gate.sh
```

Das ARM-Gate ist das Akzeptanzziel; sein aktueller Fehlschlag ist im Arbeitsstand
dokumentiert. Builds und Tests werden mit systemd-Speicherlimit ausgeführt.
Für die übrigen Prüfungen siehe [TESTING.md](TESTING.md).
