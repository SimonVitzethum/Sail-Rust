# Bauen, testen und messen

Stand: 2026-10-11. Aktuelle Ergebnisse und offene Fehler stehen in
[WORK_STATE.md](WORK_STATE.md). Dieser Text beschreibt die Prüfungen und enthält
keine pauschale Behauptung einer grünen Workspace- oder ARM-Pipeline.

## Voraussetzungen und Speicherregeln

Linux mit systemd-User-Manager und aktivierten cgroup-Speicherlimits, Rust/Cargo,
Z3 für abhängige Beweise; Lean/Lake für die Lean-Abnahme. OCaml Sail dient als
Vergleichsreferenz. Lokale Referenzpfade stehen in `WORK_STATE.md`.
Die MSRV-Angabe steht in `Cargo.toml`; sie ist ein CI-Ziel und kein Nachweis,
dass der neueste Worktree bereits mit jeder angegebenen Toolchain geprüft wurde.

Alle Compiler-, Build-, Test- und rustfmt-Läufe erhalten `MemoryMax` und
`MemorySwapMax=0`. Für relative Dateipfade unbedingt `WorkingDirectory` setzen.
`profile.ci` erbt Release-Optimierung, deaktiviert LTO und Debuginfos. Release
verwendet aktuell thin-LTO. Cargo-Jobs bei schweren Builds zusätzlich begrenzen;
die Workspace-Konfiguration setzt standardmäßig vier.

## Standardprüfungen

Vom Repositoryroot:

```bash
# Testbuild unter 4 GiB, jedes Testprogramm separat unter 128 MiB ohne Swap:
scripts/test-memory-limited.sh -p sail-ty -p sail-parse -p sail-grammar -p sail-cli -p sail-solver

# Nur die kanonische Typprüfung:
scripts/test-memory-limited.sh -p sail-ty --test canonical_program

# Gesamter Workspace, einschließlich Doctests:
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$PWD" \
  cargo test --workspace --profile ci -j 2

# Gezieltes Bauen mit lesbaren Compilerdiagnosen:
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$PWD" \
  cargo test -p sail-ty --test canonical_program --no-run --profile ci -j 2

# Formatierung ausgewählter Dateien:
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=256M -p MemorySwapMax=0 -p "WorkingDirectory=$PWD" \
  rustfmt --edition 2021 --config skip_children=true \
  crates/sail-ty/src/canonical_body.rs crates/sail-ty/tests/canonical_program.rs
```

Der Wrapper baut einmal und führt danach jedes Testprogramm mit vier Testthreads
und 180 Sekunden Laufzeitlimit aus. Er enthält keine Doctests. Konfigurierbar sind
`SAIL_BUILD_MEMORY`, `SAIL_TEST_MEMORY`, `SAIL_BUILD_JOBS`. Beim Debuggen eines
Speicherfehlers zuerst das betroffene Programm bzw. den betroffenen Test einzeln
unter dem ursprünglichen Limit messen; ein höheres Limit beseitigt den Befund
nicht. Die aktuellen 128-MiB-OOM-Ergebnisse stehen in `WORK_STATE.md`.

Builddiagnosen im Wrapper können wegen Cargo-JSON verkürzt erscheinen. In diesem
Fall den gezielten Build mit normaler Textausgabe benutzen. Bestehende laufende
Handles bzw. Prozesse prüfen; Beobachtungs-Timeouts sind kein Anlass zum Neustart.

## Out-of-core-Teilmengenprüfung

```bash
SAIL_RS_THREADS=16 scripts/typecheck-ooc-gate.sh
```

Dieser Test generiert einen unterstützten Ausschnitt mit mehr AST-Bytes als
cgroup-Speicher, prüft Definitionen und Ausdrücke und erwartet danach den
expliziten Fehler wegen fehlender Backend-Integration. Seine Zählungen und
die Größe werden im Skript kontrolliert. Er ist kein erfolgreicher ARM- oder
Lean-Build. Spool-Daten müssen auf echter Platte liegen; siehe
[ARCHITECTURE.md](ARCHITECTURE.md#out-of-core-und-parallelisierung).

## Vollständige ARM-/Lean-Abnahme

```bash
scripts/arm-lean-gate.sh
# Ausschließlich die erlaubte Ausweichgrenze:
SAIL_ARM_MEMORY=28G scripts/arm-lean-gate.sh
```

Das Gate baut Rust unter 4 GiB und liest die vollständige Eingabeliste aus dem
ARMv9.4-A-Makefile einschließlich Terminierungsmaßen. Es erstellt einen frischen
Ausgabeordner und führt Compiler, `lake build` und die elaborierte No-Sorry-Prüfung
jeweils unter 16 GiB ohne Swap aus; alternativ 28 GiB. Alte Ausgaben oder Stubs
werden nicht als Erfolg akzeptiert. Ausgaben liegen unter `target/arm-lean-gate/`.
`scripts/CheckLeanNoSorry.lean` untersucht Typen und Werte aller erfassten Module
sowie transitiver Imports. `scripts/test-lean-no-sorry.sh` enthält positive und
negative Regressionen für dieses Gate. Es setzt passende Lean-Versionen voraus.

Ein grüner Lake-Lauf, der Proof-Holes akzeptiert, genügt nicht. Ebenso sind
Frontend-Durchlauf, Source-Census, Shard-Scan, String-Snapshots oder ein
Untergruppen-Gate keine vollständige Abnahme.

## OCaml-Vergleich

OCaml-Regeln in `../sail/src/lib/{parser.mly,initial_check.ml,type_check.ml}`
lesen und gültige sowie gezielt ungültige Fixtures prüfen. Lokales Beispiel:

```bash
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=256M -p MemorySwapMax=0 -p "WorkingDirectory=$PWD" \
  /home/simon/.opam/sail/bin/sail -just_check target/mutable-reference/positive.sail
```

Diese Fixture wird während der Arbeit aus den kanonischen Testfällen erzeugt;
`target/` ist keine dauerhafte Quelle. Positive und negative Prozessausgänge
getrennt festhalten. Referenzwarnungen sind nicht automatisch Fehler, aber
Annahme/Ablehnung müssen tatsächlich mit der gewünschten Semantik übereinstimmen.
Die Herkunft unveränderter Quellfixtures steht in [THIRD_PARTY.md](THIRD_PARTY.md).

## Gesicherte Prüfnachweise

Die Logs unter `evidence/2026-10-11/` bleiben unverändert erhalten. Sie dokumentieren
verschiedene Implementierungsstände; sie belegen nicht automatisch den neuesten
Worktree. Aktuelle Interpretation, Zahlen und der offene OOM-Befund stehen in
`WORK_STATE.md`.

| Log | Umfang |
| --- | --- |
| [md-cleanup-tests.log](evidence/2026-10-11/md-cleanup-tests.log) | Gezielte Tests nach Dokumentationskonsolidierung und Verweisänderungen. |
| [md-cleanup-check.log](evidence/2026-10-11/md-cleanup-check.log) | Root-Layout, Links, unveränderte Originalnachweise/Fixtures und erhaltene Compilerarbeit. |
| [mutable-focused-128M.log](evidence/2026-10-11/mutable-focused-128M.log) | Neuester Mutable-Fokustest: kanonisches Programm durch OOM beendet. |
| [mutable-before-deep-test-128M.log](evidence/2026-10-11/mutable-before-deep-test-128M.log) | Früherer Zwischenstand ohne neuen tiefen Mutable-/Loop-Test. |
| [mutable-ocaml-positive-256M.log](evidence/2026-10-11/mutable-ocaml-positive-256M.log) | OCaml-Annahme der damaligen positiven Fälle. |
| [mutable-ocaml-negative-256M.log](evidence/2026-10-11/mutable-ocaml-negative-256M.log) | Einzelablehnung der damaligen negativen Fälle. |
| [arm-vectors-16G.log](evidence/2026-10-11/arm-vectors-16G.log) | Vollständiger ARM-Eingang, Abbruch bei `to_bytes_le`, vor Mutable-Arbeit. |
| [dependent-calls-ooc-128M.log](evidence/2026-10-11/dependent-calls-ooc-128M.log) | 16-Worker-Teilmengenprüfung, erwarteter Backend-Fehler. |
| [vector-focused-128M.log](evidence/2026-10-11/vector-focused-128M.log) | Vektor-Fokussuite vor Mutable-Arbeit. |
| [vector-canonical-128M.log](evidence/2026-10-11/vector-canonical-128M.log) | Separate kanonische Vektorsuite. |

Maßgeblich sind Prozessausgang und systemd-cgroup-Peak einschließlich Swap-Wert.
Eine Reihe erfolgreicher Einzeltests ohne erfolgreichen Programmabschluss ist
kein grünes Suite-Ergebnis. `/proc`-/CLI-RSS und cgroup-Peak sind unterschiedliche
Messungen; letztere schließt im Limit die zugehörigen Prozesse mit ein.
SMT-Nullzähler im CLI-Profil sind derzeit nicht vollständig verdrahtet.

`sail-perf` definiert die Modellbudgets im Code: `aarch64_small` 4 GiB,
ARMv8.5-A und ARMv9.4-A jeweils 16 GiB. Das ARM-Gate ist für das vollständige Ziel
maßgeblich; seine explizite Ausweichgrenze bleibt 28 GiB. `perf/timings.jsonl`
enthält ergänzende Messdaten. Die warnende CI-Prüfung dieses Logs gegen das
kleinste Budget ersetzt keinen vollständigen systemd-Modelllauf. Weitere
CI-/Audit-Jobs sind in `.github/workflows/ci.yml` definiert; ihre Existenz ist
kein aktueller Erfolgsausweis.
